use std::collections::BTreeMap;
use super::{ColorCorrectorConfig, ColorDomain, OperatorError};

/// Device-independent variant identity. Ordinary parameter values never enter this key,
/// except the exact, proven identity predicate which changes execution to an alias.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub struct OperatorStructuralKey {
    pub kind: &'static str,
    pub schema_version: u32,
    pub algorithm_version: u32,
    pub enabled_stages: u8,
    pub alias: bool,
    pub input_domain: ColorDomain,
    pub output_domain: ColorDomain,
    pub layout_version: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatorUpdate { Unchanged, Parameters, Structural }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorAlias { pub output_port: String, pub input_port: String }

/// Compare validated structured artifacts without generating code or touching a device.
pub fn classify_operator_update(previous_key: &OperatorStructuralKey, previous_parameters: &BTreeMap<String, Vec<f32>>,
    candidate_key: &OperatorStructuralKey, candidate_parameters: &BTreeMap<String, Vec<f32>>) -> OperatorUpdate {
    if previous_key != candidate_key { OperatorUpdate::Structural }
    else if previous_parameters != candidate_parameters { OperatorUpdate::Parameters }
    else { OperatorUpdate::Unchanged }
}
#[derive(Clone, Debug)]
pub struct LoweredOperator {
    pub structural_key: OperatorStructuralKey,
    pub shader_source: Option<String>,
    pub parameters: BTreeMap<String, Vec<f32>>,
    pub parameter_types: BTreeMap<String, String>,
    pub alias: Option<OperatorAlias>,
    pub input_domain: ColorDomain,
    pub output_domain: ColorDomain,
}

impl ColorCorrectorConfig {
    pub fn structure_key(&self, input_domain: ColorDomain) -> Result<OperatorStructuralKey, OperatorError> {
        self.validate()?;
        let alias = self.is_identity();
        Ok(OperatorStructuralKey { kind: "color_corrector", schema_version: self.schema_version,
            algorithm_version: self.algorithm_version,
            enabled_stages: u8::from(self.enabled) | (u8::from(self.primaries.enabled) << 1) | (u8::from(self.hdr_zones.enabled) << 2),
            alias, input_domain, output_domain: if alias { input_domain } else { ColorDomain::AcesCgLinear }, layout_version: 1 })
    }
    pub fn parameter_values(&self) -> Result<BTreeMap<String, Vec<f32>>, OperatorError> {
        self.validate()?;
        let mut values = BTreeMap::new();
        if self.is_identity() { return Ok(values); }
        let p = &self.primaries;
        if p.enabled {
            values.insert("cc_primary".into(), vec![p.white_balance[0],p.white_balance[1],p.exposure,p.contrast]);
            for (name, rgb) in [("cc_lift",p.lift),("cc_gamma",p.gamma),("cc_gain",p.gain),("cc_offset",p.offset)] {
                values.insert(name.into(), vec![rgb[0],rgb[1],rgb[2],0.0]);
            }
            values.insert("cc_finish".into(), vec![p.pivot,p.saturation,p.color_boost,0.0]);
        }
        let z = &self.hdr_zones;
        if z.enabled {
            values.insert("cc_zones_a".into(), vec![z.blacks.exposure,z.shadows.exposure,z.darks.exposure,z.lights.exposure]);
            values.insert("cc_zones_b".into(), vec![z.highlights.exposure,z.specular.exposure,z.blacks.saturation,z.shadows.saturation]);
            values.insert("cc_zones_c".into(), vec![z.darks.saturation,z.lights.saturation,z.highlights.saturation,z.specular.saturation]);
            values.insert("cc_zones_range".into(), vec![z.shadow_pivot,z.highlight_pivot,z.range,z.falloff]);
        }
        Ok(values)
    }
    pub fn classify_update(&self, candidate: &Self, input_domain: ColorDomain) -> Result<OperatorUpdate, OperatorError> {
        let key = self.structure_key(input_domain)?;
        if key != candidate.structure_key(input_domain)? { return Ok(OperatorUpdate::Structural); }
        if !key.alias && ((self.primaries.enabled && self.primaries != candidate.primaries)
            || (self.hdr_zones.enabled && self.hdr_zones != candidate.hdr_zones)) { return Ok(OperatorUpdate::Parameters); }
        Ok(OperatorUpdate::Unchanged)
    }
}

pub fn lower_color_corrector(config: &ColorCorrectorConfig, input_domain: ColorDomain) -> Result<LoweredOperator, OperatorError> {
    let structural_key = config.structure_key(input_domain)?;
    let parameters = config.parameter_values()?;
    let parameter_types = parameters.keys().map(|key| (key.clone(),"vec4<f32>".into())).collect();
    let mut shader_source = None;
    let mut alias = None;
    if structural_key.alias {
        alias = Some(OperatorAlias { output_port: "result".into(), input_port: "source".into() });
    } else {
        let mut shader = String::from("@group(0) @binding(0) var source: texture_2d<f32>;\n@group(0) @binding(1) var sourceSampler: sampler;\n");
        for (index,name) in parameters.keys().enumerate() {
            shader.push_str(&format!("@group(0) @binding({}) var<uniform> {name}: vec4<f32>;\n",index+2));
        }
        shader.push_str("fn cc_signed_pow(v:vec3<f32>,p:vec3<f32>)->vec3<f32>{return sign(v)*pow(abs(v),p);}\nfn cc_luma(v:vec3<f32>)->f32{return dot(v,vec3<f32>(0.2722287168,0.6740817658,0.0536895174));}\nfn cc_saturate(v:vec3<f32>,s:f32)->vec3<f32>{let y=cc_luma(v);return vec3<f32>(y)+(v-vec3<f32>(y))*s;}\n");
        if input_domain == ColorDomain::LegacySrgb { shader.push_str(WGSL_COLOR_TRANSFORMS); }
        shader.push_str("@fragment fn main(@location(0) uv:vec2<f32>)->@location(0) vec4<f32>{\nlet sample=textureSample(source,sourceSampler,uv);\nvar c=sample.rgb;\n");
        if input_domain == ColorDomain::LegacySrgb { shader.push_str("c=cc_srgb_to_ap1(c);\n"); }
        if config.primaries.enabled {
            shader.push_str("let wb=exp2(vec3<f32>(cc_primary.x+cc_primary.y,-cc_primary.y, -cc_primary.x+cc_primary.y));\nc=c*wb/cc_luma(wb);\nc=c*exp2(cc_primary.z);\nc=cc_signed_pow(c+cc_lift.xyz,vec3<f32>(1.0)/cc_gamma.xyz)*cc_gain.xyz+cc_offset.xyz;\nc=cc_finish.x*cc_signed_pow(c/cc_finish.x,vec3<f32>(cc_primary.w));\n");
        }
        if config.hdr_zones.enabled {
            shader.push_str("let stops=log2(max(cc_luma(c),0.00000001)/0.18);\nlet pivots=vec4<f32>(cc_zones_range.x-cc_zones_range.z,cc_zones_range.x,(cc_zones_range.x+cc_zones_range.y)*0.5,cc_zones_range.y);\nlet cuts=smoothstep(pivots-vec4<f32>(cc_zones_range.w),pivots+vec4<f32>(cc_zones_range.w),vec4<f32>(stops));\nlet last=smoothstep(cc_zones_range.y+cc_zones_range.z-cc_zones_range.w,cc_zones_range.y+cc_zones_range.z+cc_zones_range.w,stops);\nlet weights0=vec4<f32>(1.0-cuts.x,cuts.x-cuts.y,cuts.y-cuts.z,cuts.z-cuts.w);\nlet weights1=vec2<f32>(cuts.w-last,last);\nc=c*exp2(dot(weights0,cc_zones_a)+dot(weights1,cc_zones_b.xy));\nc=cc_saturate(c,dot(weights0,vec4<f32>(cc_zones_b.zw,cc_zones_c.xy))+dot(weights1,cc_zones_c.zw));\n");
        }
        if config.primaries.enabled {
            shader.push_str("let maximum=max(max(abs(c.r),abs(c.g)),abs(c.b));\nlet minimum=min(min(abs(c.r),abs(c.g)),abs(c.b));\nlet chroma=(maximum-minimum)/max(maximum,0.000001);\nc=cc_saturate(c,cc_finish.y*(1.0+cc_finish.z*(1.0-clamp(chroma,0.0,1.0))));\n");
        }
        shader.push_str("return vec4<f32>(c,sample.a);\n}\n");
        shader_source = Some(shader);
    }
    Ok(LoweredOperator { input_domain, output_domain: structural_key.output_domain, structural_key,
        shader_source,parameters,parameter_types,alias })
}

/// Explicit SDR compatibility transforms, including D65↔D60 adaptation.
/// Signed transfer functions preserve negative scene values; no hidden clipping/tone map.
pub const WGSL_COLOR_TRANSFORMS: &str = r#"
fn cc_srgb_decode(v:vec3<f32>)->vec3<f32>{
    let a=abs(v);
    return sign(v)*select(pow((a+vec3<f32>(0.055))/1.055,vec3<f32>(2.4)),a/12.92,a<=vec3<f32>(0.04045));
}
fn cc_srgb_encode(v:vec3<f32>)->vec3<f32>{
    let a=abs(v);
    return sign(v)*select(1.055*pow(a,vec3<f32>(1.0/2.4))-vec3<f32>(0.055),12.92*a,a<=vec3<f32>(0.0031308));
}
fn cc_srgb_to_ap1(v:vec3<f32>)->vec3<f32>{
    let c=cc_srgb_decode(v);
    return vec3<f32>(dot(c,vec3<f32>(0.6131288989,0.3395173080,0.0473537931)),dot(c,vec3<f32>(0.0701067622,0.9163657642,0.0135274736)),dot(c,vec3<f32>(0.0205866130,0.1095677170,0.8698456700)));
}
fn cc_ap1_to_srgb(c:vec3<f32>)->vec3<f32>{
    return cc_srgb_encode(vec3<f32>(dot(c,vec3<f32>(1.7048586763,-0.6217160219,-0.0831426544)),dot(c,vec3<f32>(-0.1300768242,1.1407357748,-0.0106589506)),dot(c,vec3<f32>(-0.0239640729,-0.1289755083,1.1529395812))));
}
"#;

/// Independent f64 CPU oracle: never evaluates generated code or packed uniforms.
pub fn evaluate_color_corrector(config: &ColorCorrectorConfig, input_domain: ColorDomain, pixel: [f32;4]) -> Result<[f32;4],OperatorError> {
    config.validate()?;
    if pixel.iter().any(|v| !v.is_finite()) { return Err(OperatorError("Pixel must be finite".into())); }
    if config.is_identity() { return Ok(pixel); }
    let mut c = [pixel[0] as f64,pixel[1] as f64,pixel[2] as f64];
    if input_domain == ColorDomain::LegacySrgb {
        c = c.map(|v| v.signum()*if v.abs() <= 0.04045 {v.abs()/12.92} else {((v.abs()+0.055)/1.055).powf(2.4)});
        let m = [[0.613128898877229,0.339517308047594,0.047353793075177],
            [0.070106762186640,0.916365764243620,0.013527473569741],
            [0.020586612954493,0.109567717001633,0.869845670043874]];
        c = m.map(|row| row.into_iter().zip(c).map(|(a,b)|a*b).sum());
    }
    let p = &config.primaries;
    if p.enabled {
        let temperature=p.white_balance[0] as f64;
        let tint=p.white_balance[1] as f64;
        let wb=[(temperature+tint).exp2(),(-tint).exp2(),(-temperature+tint).exp2()];
        let normalization=luma(wb);
        for i in 0..3 {
            c[i] *= wb[i]/normalization * (p.exposure as f64).exp2();
            c[i] = signed_pow(c[i]+p.lift[i] as f64,1.0/p.gamma[i] as f64)*p.gain[i] as f64+p.offset[i] as f64;
            c[i] = p.pivot as f64 * signed_pow(c[i]/p.pivot as f64,p.contrast as f64);
        }
    }
    let z = &config.hdr_zones;
    if z.enabled {
        let stops=(luma(c).max(1e-8)/0.18).log2();
        let shadow=z.shadow_pivot as f64;
        let highlight=z.highlight_pivot as f64;
        let range=z.range as f64;
        let softness=z.falloff as f64;
        let boundaries=[shadow-range,shadow,(shadow+highlight)*0.5,highlight,highlight+range];
        let cuts=boundaries.map(|pivot|smoothstep(pivot-softness,pivot+softness,stops));
        let weights=[1.0-cuts[0],cuts[0]-cuts[1],cuts[1]-cuts[2],cuts[2]-cuts[3],cuts[3]-cuts[4],cuts[4]];
        let zones=[&z.blacks,&z.shadows,&z.darks,&z.lights,&z.highlights,&z.specular];
        let exposure=zones.iter().zip(weights).map(|(zone,w)|zone.exposure as f64*w).sum::<f64>();
        c=c.map(|v|v*exposure.exp2());
        let saturation=zones.iter().zip(weights).map(|(zone,w)|zone.saturation as f64*w).sum();
        c=saturate(c,saturation);
    }
    if p.enabled {
        let maximum=c.iter().map(|v|v.abs()).fold(0.0,f64::max);
        let minimum=c.iter().map(|v|v.abs()).fold(f64::INFINITY,f64::min);
        let chroma=((maximum-minimum)/maximum.max(1e-6)).clamp(0.0,1.0);
        c=saturate(c,p.saturation as f64*(1.0+p.color_boost as f64*(1.0-chroma)));
    }
    let result=[c[0] as f32,c[1] as f32,c[2] as f32,pixel[3]];
    if result.iter().any(|v|!v.is_finite()) { return Err(OperatorError("ColorCorrector result exceeds finite f32 range".into())); }
    Ok(result)
}
fn luma(c:[f64;3])->f64 {c[0]*0.2722287168+c[1]*0.6740817658+c[2]*0.0536895174}
fn signed_pow(v:f64,p:f64)->f64 {v.signum()*v.abs().powf(p)}
fn saturate(c:[f64;3],s:f64)->[f64;3] {let y=luma(c);c.map(|v|y+(v-y)*s)}
fn smoothstep(a:f64,b:f64,x:f64)->f64 {let t=((x-a)/(b-a)).clamp(0.0,1.0);t*t*(3.0-2.0*t)}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
