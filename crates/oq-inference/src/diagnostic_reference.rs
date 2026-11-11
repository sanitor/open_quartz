//! Explicit diagnostic CPU oracle. Never used by production model preparation.
//!
//! Source protobuf fields and initializer payloads are copied verbatim. Only
//! InstanceNormalization nodes are replaced with centered FP64 population
//! statistics/affine arithmetic followed by FP32 activation export. A semantic
//! graph may additionally expose its existing pre-ArgMax tensor as an output.
use crate::gpu_model::proto::{Attribute, Node, Tensor};
use prost::Message;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct ReferenceModel {
    pub bytes: Vec<u8>,
    pub normalized_nodes: Vec<String>,
    pub logits_output: Option<String>,
    pub source_sha256: String,
    pub oracle_sha256: String,
}

struct Field<'a> { number: u64, wire: u64, raw: &'a [u8], payload: &'a [u8] }
fn varint(bytes: &[u8], at: &mut usize) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*at).ok_or("truncated protobuf varint")?; *at += 1;
        if shift == 63 && byte > 1 { return Err("protobuf varint overflow".into()); }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 { return Ok(value); }
    }
    Err("protobuf varint overflow".into())
}
fn fields(bytes: &[u8]) -> Result<Vec<Field<'_>>, String> {
    let mut at = 0; let mut result = Vec::new();
    while at < bytes.len() {
        let start = at; let key = varint(bytes, &mut at)?; let wire = key & 7;
        if key >> 3 == 0 { return Err("invalid protobuf field number".into()); }
        let payload = match wire {
            0 => { let begin = at; varint(bytes, &mut at)?; &bytes[begin..at] }
            1 | 5 => { let begin = at; at = at.checked_add(if wire == 1 { 8 } else { 4 }).ok_or("protobuf size overflow")?;
                bytes.get(begin..at).ok_or("truncated fixed protobuf field")? }
            2 => { let length = usize::try_from(varint(bytes, &mut at)?).map_err(|_| "protobuf length overflow")?;
                let begin = at; at = at.checked_add(length).ok_or("protobuf size overflow")?;
                bytes.get(begin..at).ok_or("truncated protobuf bytes")? }
            _ => return Err("unsupported protobuf wire type".into()),
        };
        result.push(Field { number:key >> 3, wire, raw:&bytes[start..at], payload });
    }
    Ok(result)
}
fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 { out.push(value as u8 | 128); value >>= 7; } out.push(value as u8);
}
fn put_bytes(out: &mut Vec<u8>, number: u64, bytes: &[u8]) {
    put_varint(out, number << 3 | 2); put_varint(out, bytes.len() as u64); out.extend_from_slice(bytes);
}
fn put_int(out: &mut Vec<u8>, number: u64, value: u64) { put_varint(out, number << 3); put_varint(out, value); }
fn attr_int(name: &str, value: i64) -> Attribute {
    Attribute { name:name.into(), i:Some(value), r#type:Some(2), ..Default::default() }
}
fn node(op: &str, inputs: Vec<String>, output: String, attributes: Vec<Attribute>) -> Node {
    Node { op_type:op.into(), input:inputs, output:vec![output], attribute:attributes, ..Default::default() }
}
fn append_node(graph: &mut Vec<u8>, n: Node) { put_bytes(graph, 1, &n.encode_to_vec()); }
fn constant(graph: &mut Vec<u8>, name: &str, dtype: i32, dims: Vec<i64>, data: Vec<u8>) {
    append_node(graph, node("Constant", vec![], name.into(), vec![Attribute { name:"value".into(), r#type:Some(4),
        t:Some(Tensor { dims, data_type:Some(dtype), raw_data:Some(data), ..Default::default() }), ..Default::default() }]));
}
fn expand_norm(graph: &mut Vec<u8>, n: &Node, prefix: &str) -> Result<(), String> {
    if n.input.len() != 3 || n.output.len() != 1 { return Err("InstanceNormalization oracle requires three inputs and one output".into()); }
    let mut epsilon = 1e-5f32;
    for a in &n.attribute {
        if a.name != "epsilon" || a.r#type != Some(1) { return Err("unsupported InstanceNormalization oracle attribute".into()); }
        epsilon = a.f.ok_or("missing normalization epsilon")?;
    }
    if !epsilon.is_finite() || epsilon < 0.0 { return Err("invalid normalization epsilon".into()); }
    let name = |suffix: &str| format!("{prefix}/{suffix}");
    let emit = |graph: &mut Vec<u8>, op: &str, inputs: Vec<String>, suffix: &str, attrs: Vec<Attribute>| {
        let output = name(suffix); append_node(graph, node(op, inputs, output.clone(), attrs)); output
    };
    let x = emit(graph, "Cast", vec![n.input[0].clone()], "double", vec![attr_int("to", 11)]);
    let axes = || vec![Attribute { name:"axes".into(), ints:vec![2,3], r#type:Some(7), ..Default::default() }, attr_int("keepdims",1)];
    let mean = emit(graph,"ReduceMean",vec![x.clone()],"mean",axes());
    let centered = emit(graph,"Sub",vec![x,mean],"centered",vec![]);
    let square = emit(graph,"Mul",vec![centered.clone(),centered.clone()],"square",vec![]);
    let variance = emit(graph,"ReduceMean",vec![square],"variance",axes());
    constant(graph,&name("epsilon"),11,vec![],f64::from(epsilon).to_le_bytes().to_vec());
    let adjusted = emit(graph,"Add",vec![variance,name("epsilon")],"adjusted",vec![]);
    let std = emit(graph,"Sqrt",vec![adjusted],"std",vec![]);
    let normalized = emit(graph,"Div",vec![centered,std],"normalized",vec![]);
    constant(graph,&name("channel_shape"),7,vec![4],[1i64,-1,1,1].into_iter().flat_map(i64::to_le_bytes).collect());
    let scale = emit(graph,"Cast",vec![n.input[1].clone()],"double_scale",vec![attr_int("to",11)]);
    let bias = emit(graph,"Cast",vec![n.input[2].clone()],"double_bias",vec![attr_int("to",11)]);
    let scale = emit(graph,"Reshape",vec![scale,name("channel_shape")],"scale",vec![]);
    let bias = emit(graph,"Reshape",vec![bias,name("channel_shape")],"bias",vec![]);
    let scaled = emit(graph,"Mul",vec![normalized,scale],"scaled",vec![]);
    let biased = emit(graph,"Add",vec![scaled,bias],"biased",vec![]);
    append_node(graph,node("Cast",vec![biased],n.output[0].clone(),vec![attr_int("to",1)]));
    Ok(())
}

/// Prepare the explicitly approved rank-four image diagnostic oracle, retaining
/// original model metadata/weights and every non-normalization node byte-for-byte.
pub fn prepare_reference_model(original: &[u8]) -> Result<ReferenceModel, String> {
    let model_fields = fields(original)?;
    let graphs: Vec<_> = model_fields.iter().filter(|f| f.number == 7 && f.wire == 2).collect();
    if graphs.len() != 1 { return Err("diagnostic oracle requires exactly one graph".into()); }
    let graph_fields = fields(graphs[0].payload)?;
    let mut graph = Vec::new(); let mut normalized_nodes = Vec::new(); let mut argmax = Vec::new();
    let mut namespace = "__oq_stable_reference".to_owned();
    while original.windows(namespace.len()).any(|w| w == namespace.as_bytes()) { namespace.push('_'); }
    let interface = crate::inspect_onnx(original).map_err(|e| e.to_string())?;
    let opset = interface.opset_imports.iter().find(|o| o.domain.as_deref().unwrap_or("").is_empty()).and_then(|o| o.version).ok_or("missing standard opset")?;
    let mut ranks = BTreeMap::new();
    for input in &interface.inputs {
        if let Some(crate::artifact::ArtifactValueKind::Tensor {shape:crate::artifact::ArtifactShape::KnownRank(dims),..}) = input.value.value_type.as_ref().map(|v|&v.kind) {
            ranks.insert(input.value.name.clone(),dims.len());
        }
    }
    for f in graph_fields.iter().filter(|f|f.number==5 && f.wire==2) {
        let t=Tensor::decode(f.payload).map_err(|e|e.to_string())?;ranks.insert(t.name,t.dims.len());
    }
    for (index,f) in graph_fields.iter().enumerate() {
        if f.number != 1 || f.wire != 2 { graph.extend_from_slice(f.raw); continue; }
        let n = Node::decode(f.payload).map_err(|e| e.to_string())?;
        let input_rank=n.input.first().and_then(|name|ranks.get(name)).copied();
        if n.op_type=="InstanceNormalization" && n.domain.is_empty() && input_rank!=Some(4) {
            return Err("stable image normalization oracle requires proven rank-four activation".into());
        }
        let rank=match (n.domain.as_str(),n.op_type.as_str()) {
            ("","Conv")=>n.input.get(1).and_then(|name|ranks.get(name)).copied(),
            ("","Identity"|"Relu"|"PRelu"|"Sigmoid"|"Clip"|"Slice"|"Resize"|"Transpose"|"Cast"|"Neg"|"BatchNormalization"|"InstanceNormalization"|"GlobalAveragePool"|"MaxPool"|"Split"|"Pad"|"DepthToSpace")=>input_rank,
            ("","Concat")=>input_rank.filter(|r|n.input.iter().all(|v|ranks.get(v)==Some(r))),
            ("","Add"|"Sub"|"Mul"|"Div")=>n.input.iter().map(|v|ranks.get(v).copied()).collect::<Option<Vec<_>>>().and_then(|v|v.into_iter().max()),
            ("","Shape")=>Some(1),
            ("","Constant")=>n.attribute.iter().find_map(|a|a.t.as_ref().map(|t|t.dims.len())),
            _=>None,
        };
        if let Some(rank)=rank {for output in &n.output {ranks.insert(output.clone(),rank);}}
        if n.domain.is_empty() && n.op_type == "InstanceNormalization" {
            if !(11..=17).contains(&opset) { return Err("stable rank-four normalization oracle requires ONNX opset11..17".into()); }
            let raw = fields(f.payload)?;
            let source_name = raw.iter().find(|v| v.number == 3 && v.wire == 2).map(|v| String::from_utf8_lossy(v.payload).into_owned()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("node_{index}"));
            normalized_nodes.push(source_name);
            expand_norm(&mut graph,&n,&format!("{namespace}/{index}"))?;
        } else {
            if n.domain.is_empty() && n.op_type == "ArgMax" { argmax.push(n); }
            graph.extend_from_slice(f.raw);
        }
    }
    // Only an original UInt8 semantic output produced by Cast(ArgMax) exposes logits.
    let logits_output = if interface.outputs.len() == 1 && argmax.len() == 1 {
        let a = &argmax[0];
        let cast = graph_fields.iter().filter(|f| f.number == 1 && f.wire == 2).filter_map(|f| Node::decode(f.payload).ok()).find(|n|
            n.op_type == "Cast" && n.domain.is_empty() && n.input == a.output && n.output == [interface.outputs[0].name.clone()] && n.attribute.iter().any(|v| v.name == "to" && v.i == Some(2)));
        if cast.is_some() && a.input.len() == 1 && a.attribute.iter().any(|v| v.name == "axis" && v.i == Some(1)) {
            let logits = a.input[0].clone();
            // ValueInfoProto: FP32 tensor with symbolic dimensions. Invocation supplies
            // exact expected concrete NCHW dimensions independently.
            let mut value = Vec::new(); put_bytes(&mut value,1,logits.as_bytes());
            let mut tensor = Vec::new(); put_int(&mut tensor,1,1);
            let mut shape = Vec::new();
            for axis in ["batch","classes","height","width"] { let mut dim = Vec::new(); put_bytes(&mut dim,2,format!("{namespace}/{axis}").as_bytes()); put_bytes(&mut shape,1,&dim); }
            put_bytes(&mut tensor,2,&shape); let mut ty = Vec::new();put_bytes(&mut ty,1,&tensor);put_bytes(&mut value,2,&ty);put_bytes(&mut graph,12,&value);
            Some(logits)
        } else { None }
    } else { None };
    let mut bytes = Vec::with_capacity(original.len()+graph.len().saturating_sub(graphs[0].payload.len()));
    for f in model_fields { if f.number == 7 { put_bytes(&mut bytes,7,&graph); } else { bytes.extend_from_slice(f.raw); } }
    Ok(ReferenceModel { source_sha256:format!("{:x}",Sha256::digest(original)),oracle_sha256:format!("{:x}",Sha256::digest(&bytes)),bytes, normalized_nodes, logits_output })
}

#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct ClassBoundaryDifference {
    pub coordinate: [usize;3], pub cpu_class: usize, pub gpu_class: usize,
    pub cpu_logits: [f32;2], pub gpu_logits: [f32;2],
    pub cpu_margin: f64, pub gpu_margin: f64,
    pub observed_uncertainty_bound: f64, pub logical_uncertainty_bound: f64,
    pub intervals_overlap: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all="camelCase")]
pub struct SemanticParity {
    pub passed: bool, pub exact_id_passed: bool, pub boundary_accepted: usize,
    pub exact_id_differences: usize, pub class_differences: Vec<ClassBoundaryDifference>,
    pub logits_mismatches: usize, pub wrong_gpu_argmax: usize, pub wrong_cpu_argmax: usize,
    pub maximum_logit_absolute_error: f64, pub absolute_tolerance: f64, pub relative_tolerance: f64,
}

/// Validate original class IDs against each backend's own first-index ArgMax,
/// unchanged FP32 logit tolerance, and overlapping top-class error intervals.
pub fn semantic_parity(cpu_ids: &[f32], gpu_ids: &[f32], cpu: &[f32], gpu: &[f32], shape: &[i64], absolute: f64, relative: f64) -> Result<SemanticParity,String> {
    if shape.len()!=4 || shape.iter().any(|&d|d<=0) || !absolute.is_finite() || !relative.is_finite() || absolute<0.0 || relative<0.0 { return Err("invalid semantic logits shape/tolerance".into()); }
    let dims: Vec<usize> = shape.iter().map(|&d|usize::try_from(d).map_err(|_|"semantic dimension overflow")).collect::<Result<_,_>>()?;
    let count = dims.iter().try_fold(1usize,|n,&d|n.checked_mul(d)).ok_or("semantic shape overflow")?;
    let plane = dims[2].checked_mul(dims[3]).ok_or("semantic plane overflow")?;
    let pixels = dims[0].checked_mul(plane).ok_or("semantic pixels overflow")?;
    if cpu.len()!=count || gpu.len()!=count || cpu_ids.len()!=pixels || gpu_ids.len()!=pixels { return Err("semantic tensor length/shape mismatch".into()); }
    if cpu.iter().chain(gpu).chain(cpu_ids).chain(gpu_ids).any(|v|!v.is_finite()) { return Err("nonfinite semantic tensor".into()); }
    let mut result = SemanticParity { passed:true,exact_id_passed:true,boundary_accepted:0,exact_id_differences:0,class_differences:Vec::new(),logits_mismatches:0,wrong_gpu_argmax:0,wrong_cpu_argmax:0,maximum_logit_absolute_error:0.0,absolute_tolerance:absolute,relative_tolerance:relative };
    for (&c,&g) in cpu.iter().zip(gpu) { let e=(f64::from(c)-f64::from(g)).abs(); result.maximum_logit_absolute_error=result.maximum_logit_absolute_error.max(e);result.logits_mismatches+=usize::from(e>absolute+relative*f64::from(c).abs()); }
    for pixel in 0..pixels {
        let batch=pixel/plane; let offset=pixel%plane;
        let at = |class:usize| batch*dims[1]*plane+class*plane+offset;
        let argmax = |values:&[f32]| { let mut best=0;for class in 1..dims[1] {if values[at(class)]>values[at(best)] {best=class;}}best };
        let ci=cpu_ids[pixel];let gi=gpu_ids[pixel];
        if ci<0.0 || gi<0.0 || ci.fract()!=0.0 || gi.fract()!=0.0 || ci>=dims[1] as f32 || gi>=dims[1] as f32 { return Err("invalid original semantic class ID".into()); }
        let ci=ci as usize;let gi=gi as usize;
        result.wrong_cpu_argmax+=usize::from(ci!=argmax(cpu));result.wrong_gpu_argmax+=usize::from(gi!=argmax(gpu));
        if ci!=gi {
            let c=[cpu[at(ci)],cpu[at(gi)]];let g=[gpu[at(ci)],gpu[at(gi)]];
            let radii=c.map(|v|absolute+relative*f64::from(v).abs());
            let cpu_margin=f64::from(c[0])-f64::from(c[1]);let gpu_margin=f64::from(g[1])-f64::from(g[0]);
            let observed=(f64::from(c[0])-f64::from(g[0])).abs()+(f64::from(c[1])-f64::from(g[1])).abs();
            let logical=radii[0]+radii[1];
            let overlap=(f64::from(c[0])-radii[0])<=(f64::from(c[1])+radii[1]);
            result.class_differences.push(ClassBoundaryDifference {coordinate:[batch,offset/dims[3],offset%dims[3]],cpu_class:ci,gpu_class:gi,cpu_logits:c,gpu_logits:g,cpu_margin,gpu_margin,observed_uncertainty_bound:observed,logical_uncertainty_bound:logical,intervals_overlap:overlap});
            // Acceptance uses only predeclared numerical error intervals. Observed
            // discrepancy is evidence, never a retroactive acceptance threshold.
            result.passed &= overlap;
        }
    }
    result.exact_id_passed=result.class_differences.is_empty();
    result.exact_id_differences=result.class_differences.len();
    result.passed &= result.logits_mismatches==0 && result.wrong_gpu_argmax==0 && result.wrong_cpu_argmax==0;
    if result.passed { result.boundary_accepted=result.exact_id_differences; }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn semantic_boundary_requires_valid_numerics_and_own_argmax() {
        let check=|c:&[f32],g:&[f32],ci:&[f32],gi:&[f32]|semantic_parity(ci,gi,c,g,&[1,2,1,1],1e-3,1e-3);
        assert!(check(&[1.0001,1.0],&[1.0,1.0001],&[0.0],&[1.0]).unwrap().passed);
        assert!(!check(&[2.0,1.0],&[1.0,2.0],&[0.0],&[1.0]).unwrap().passed);
        assert!(!check(&[1.0001,1.0],&[1.0001,1.0],&[0.0],&[1.0]).unwrap().passed);
        assert!(!check(&[1.0,1.0],&[1.0,1.0],&[0.0],&[1.0]).unwrap().passed);
        assert!(check(&[1.0],&[1.0],&[0.0],&[0.0]).is_err());
        assert!(check(&[f32::NAN,1.0],&[1.0,1.0],&[0.0],&[0.0]).is_err());
        assert!(check(&[1.0,1.0],&[f32::INFINITY,1.0],&[0.0],&[0.0]).is_err());
        assert!(check(&[1.0,1.0],&[1.0,1.0],&[f32::NAN],&[0.0]).is_err());
        assert!(check(&[1.0,1.0],&[1.0,1.0],&[0.5],&[0.0]).is_err());
        assert!(semantic_parity(&[0.0],&[0.0],&[1.0,1.0],&[1.0,1.0],&[1,2,1],1e-3,1e-3).is_err());
        let accepted=check(&[1.0001,1.0],&[1.0,1.0001],&[0.0],&[1.0]).unwrap();
        assert!(!accepted.exact_id_passed);assert_eq!(accepted.boundary_accepted,1);
        assert!(accepted.class_differences[0].cpu_margin<=accepted.class_differences[0].logical_uncertainty_bound);
        assert!(semantic_parity(&[],&[],&[],&[],&[1,2,0,1],1e-3,1e-3).is_err());
    }
    #[cfg(all(feature="ort",not(target_arch="wasm32")))]
    fn value_info(name:&str,dimensions:&[u64])->Vec<u8> { value_info_typed(name,dimensions,1) }
    #[cfg(all(feature="ort",not(target_arch="wasm32")))]
    fn value_info_typed(name:&str,dimensions:&[u64],dtype:u64)->Vec<u8> {
        let mut shape=Vec::new();for &size in dimensions {let mut dim=Vec::new();put_int(&mut dim,1,size);put_bytes(&mut shape,1,&dim);}
        let mut tensor=Vec::new();put_int(&mut tensor,1,dtype);put_bytes(&mut tensor,2,&shape);
        let mut ty=Vec::new();put_bytes(&mut ty,1,&tensor);let mut value=Vec::new();put_bytes(&mut value,1,name.as_bytes());put_bytes(&mut value,2,&ty);value
    }
    #[cfg(all(feature="ort",not(target_arch="wasm32")))]
    fn model_bytes(graph: &[u8]) -> Vec<u8> {
        let mut model=Vec::new();put_int(&mut model,1,8);put_bytes(&mut model,7,graph);
        let mut opset=Vec::new();put_int(&mut opset,2,11);put_bytes(&mut model,8,&opset);model
    }
    #[cfg(all(feature="ort",not(target_arch="wasm32")))]
    #[test]
    fn stable_oracle_normalizes_high_dc_low_variance_with_original_affine_and_epsilon() {
        const SIDE:usize=256;
        let shape=[1,2,SIDE as i64,SIDE as i64];
        let mut graph=Vec::new();put_bytes(&mut graph,2,b"stable normalization behavioral regression");
        put_bytes(&mut graph,11,&value_info("x",&[1,2,SIDE as u64,SIDE as u64]));
        for name in ["first","result"] {put_bytes(&mut graph,12,&value_info(name,&[1,2,SIDE as u64,SIDE as u64]));}
        let scales=[[1.75f32,-0.625],[0.875,1.25]];
        let biases=[[-0.2f32,0.1],[0.3,-0.4]];
        let epsilons=[3e-4f32,0.0025];
        for stage in 0..2 {
            for (suffix,values) in [("scale",scales[stage]),("bias",biases[stage])] {
                let tensor=Tensor {name:format!("stage{stage}/{suffix}"),dims:vec![2],data_type:Some(1),
                    raw_data:Some(values.into_iter().flat_map(f32::to_le_bytes).collect()),..Default::default()};
                put_bytes(&mut graph,5,&tensor.encode_to_vec());
            }
        }
        append_node(&mut graph,node("InstanceNormalization",vec!["x".into(),"stage0/scale".into(),"stage0/bias".into()],"first".into(),
            vec![Attribute {name:"epsilon".into(),f:Some(epsilons[0]),r#type:Some(1),..Default::default()}]));
        append_node(&mut graph,node("Relu",vec!["first".into()],"activated".into(),vec![]));
        append_node(&mut graph,node("InstanceNormalization",vec!["activated".into(),"stage1/scale".into(),"stage1/bias".into()],"result".into(),
            vec![Attribute {name:"epsilon".into(),f:Some(epsilons[1]),r#type:Some(1),..Default::default()}]));
        let original=model_bytes(&graph);let prepared=prepare_reference_model(&original).unwrap();
        let plane=SIDE*SIDE;
        let input:Vec<f32>=(0..2*plane).map(|i| {
            let dc=if i<plane {-1000.123f32} else {900.456f32};
            dc+((i%29) as f32-14.0)*0.0005
        }).collect();
        // Deliberately independent from ONNX graph construction: centered f64
        // population statistics over the actual representable FP32 samples.
        let normalize=|samples:&[f32],scale:[f32;2],bias:[f32;2],epsilon:f32| {
            let mut result=Vec::with_capacity(samples.len());
            for channel in 0..2 {
                let values=&samples[channel*plane..(channel+1)*plane];
                let mean=values.iter().map(|&v|f64::from(v)).sum::<f64>()/plane as f64;
                let variance=values.iter().map(|&v|{let centered=f64::from(v)-mean;centered*centered}).sum::<f64>()/plane as f64;
                let denominator=(variance+f64::from(epsilon)).sqrt();
                result.extend(values.iter().map(|&v|((f64::from(v)-mean)/denominator*f64::from(scale[channel])+f64::from(bias[channel])) as f32));
            }
            result
        };
        let first=normalize(&input,scales[0],biases[0],epsilons[0]);
        let activated:Vec<_>=first.iter().map(|&v|v.max(0.0)).collect();
        let last=normalize(&activated,scales[1],biases[1],epsilons[1]);
        let mut session=crate::InferenceSession::from_memory(&prepared.bytes).unwrap();
        let stamp=crate::RequestStamp {request_id:1,session_generation:session.generation()};
        let outputs=["first".to_owned(),"result".to_owned()];
        let actual=session.invoke_with_output_shapes(stamp,vec![crate::NamedTensor {name:"x".into(),data:input,shape:shape.to_vec()}],
            &outputs,&[("first",shape.as_slice()),("result",shape.as_slice())]).unwrap();
        for (name,expected) in [("first",first),("result",last)] {
            let output=actual.outputs.iter().find(|v|v.name==name).unwrap();
            assert_eq!(output.shape,shape);
            assert_eq!(output.data.len(),expected.len());
            for (index,(&actual,&expected)) in output.data.iter().zip(&expected).enumerate() {
                let error=(f64::from(actual)-f64::from(expected)).abs();
                assert!(actual.is_finite() && error<=1e-5+1e-5*f64::from(expected).abs(),"{name}[{index}]={actual}, independent stable value={expected}, error={error}");
            }
        }
    }
    #[cfg(all(feature="ort",not(target_arch="wasm32")))]
    #[test]
    fn semantic_reference_returns_original_uint8_ids_and_same_run_logits() {
        let mut graph=Vec::new();put_bytes(&mut graph,2,b"semantic mixed-output behavioral regression");
        put_bytes(&mut graph,11,&value_info("logits",&[1,3,1,2]));
        put_bytes(&mut graph,12,&value_info_typed("classes",&[1,1,2],2));
        append_node(&mut graph,node("ArgMax",vec!["logits".into()],"indices".into(),vec![attr_int("axis",1),attr_int("keepdims",0)]));
        append_node(&mut graph,node("Cast",vec!["indices".into()],"classes".into(),vec![attr_int("to",2)]));
        let prepared=prepare_reference_model(&model_bytes(&graph)).unwrap();
        let logits_name=prepared.logits_output.as_deref().unwrap();
        let mut session=crate::InferenceSession::from_memory(&prepared.bytes).unwrap();
        let input=vec![0.5,5.0,1.5,5.0,-1.0,4.0];let shape=vec![1,3,1,2];
        let ((ids,id_shape),logits)=session.run_u8_with_logits(input.clone(),shape.clone(),"classes",logits_name,&shape).unwrap();
        assert_eq!(ids,[1,0],"original UInt8 result must retain first-index tie selection");
        assert_eq!(id_shape,[1,1,2]);assert_eq!(logits.shape,shape);assert_eq!(logits.data,input);
        assert!(session.run_u8_with_logits(input.clone(),shape.clone(),"missing",logits_name,&shape).is_err());
        assert!(session.run_u8_with_logits(input.clone(),shape.clone(),"classes","missing",&shape).is_err());
        assert!(session.run_u8_with_logits(input.clone(),shape.clone(),logits_name,"classes",&shape).is_err());
        assert!(session.run_u8_with_logits(input,shape,"classes",logits_name,&[1,3,2,2]).is_err());
    }
    #[test] fn malformed_reference_protobuf_is_rejected() {
        assert!(prepare_reference_model(&[0x3a,0xff]).is_err());
    }
}
