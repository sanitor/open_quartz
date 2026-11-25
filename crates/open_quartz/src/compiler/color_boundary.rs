//! Explicit managed-AP1 to legacy SDR compatibility boundary.
use std::collections::{BTreeMap, HashSet};
use super::shader::{compile_shader, CompilePort, CompileRequest, CompiledShader};

pub(crate) const AP1_TO_SRGB: &str = "\nfn oq_ap1_to_srgb(value: vec4<f32>) -> vec4<f32> { return vec4<f32>(cc_ap1_to_srgb(value.rgb),value.a); }\n";

const SRGB_TO_AP1: &str = "\nfn oq_srgb_to_ap1(value: vec4<f32>) -> vec4<f32> { return vec4<f32>(cc_srgb_to_ap1(value.rgb),value.a); }\n";

pub(crate) fn display_program(label: &str, source: &str) -> CompiledShader {
    compile_shader(&CompileRequest {
        user_code: format!("{}{AP1_TO_SRGB}\n@fragment fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{ return oq_ap1_to_srgb(textureSample({label}, {label}Sampler, uv)); }}", crate::operators::WGSL_COLOR_TRANSFORMS),
        input_ports: vec![CompilePort { label: label.into(), data_type: "sampler2D".into() }],
        upstream_map: BTreeMap::from([(label.into(), source.into())]),
        video_inputs: Vec::new(),
        target_format: "rgba8unorm".into(),
    })
}

/// Locate actual WGSL image operations, including helper-function arguments.
/// Comments, whitespace and unrelated identifiers cannot alter this boundary.
pub(crate) fn convert_managed_samples(code: &str, managed: &HashSet<&str>) -> Result<String, String> {
    convert_samples(code, managed, "oq_ap1_to_srgb", AP1_TO_SRGB)
}

pub(crate) fn convert_legacy_samples(code: &str, legacy: &HashSet<&str>) -> Result<String, String> {
    convert_samples(code, legacy, "oq_srgb_to_ap1", SRGB_TO_AP1)
}

fn convert_samples(code: &str, selected: &HashSet<&str>, wrapper: &str, helper: &str) -> Result<String, String> {
    if selected.is_empty() { return Ok(code.into()); }
    let module = naga::front::wgsl::parse_str(code).map_err(|error| error.emit_to_string(code))?;
    let mut arguments: Vec<Vec<u8>> = module.functions.iter().map(|(_, function)| vec![0; function.arguments.len()]).collect();
    for _ in 0..=module.functions.len() {
        let previous = arguments.clone();
        for (handle, function) in module.functions.iter() {
            propagate_calls(&function.body, function, &previous[handle.index()], &module, selected, &mut arguments);
        }
        for entry in &module.entry_points {
            propagate_calls(&entry.function.body, &entry.function, &[], &module, selected, &mut arguments);
        }
        if arguments == previous { break; }
    }
    let mut inserts = Vec::<(usize, bool)>::new();
    for (function, domains) in module.functions.iter().map(|(handle, function)| (function, arguments[handle.index()].as_slice()))
        .chain(module.entry_points.iter().map(|entry| (&entry.function, &[][..]))) {
        for (handle, expression) in function.expressions.iter() {
            let (image, gather) = match expression {
                naga::Expression::ImageSample { image, gather, .. } => (*image, gather.is_some()),
                naga::Expression::ImageLoad { image, .. } => (*image, false),
                _ => continue,
            };
            let domain = image_domain(image, function, domains, &module, selected);
            if domain == 3 { return Err("A shader texture helper cannot mix managed AP1 and legacy sRGB textures; use separate helpers".into()); }
            if domain != 2 { continue; }
            if gather { return Err("Managed AP1 textureGather requires an explicit color conversion before gathering channels".into()); }
            let span = function.expressions.get_span(handle).to_range().ok_or("Managed image operation has no source span")?;
            inserts.push((span.start, true));
            inserts.push((image_call_end(code, span.start)?, false));
        }
    }
    if inserts.is_empty() { return Ok(code.to_owned()); }
    // Pick a namespace absent even from user locals, not just module functions.
    // Only generated helpers are renamed; user source remains byte-for-byte intact.
    let mut generation = 0u64;
    let prefix = loop {
        let prefix = format!("oq_internal_color{generation}_");
        if !code.contains(&prefix) { break prefix; }
        generation += 1;
    };
    let call = format!("{prefix}{wrapper}(");
    inserts.sort_by_key(|(position, _)| *position);
    let mut result = String::with_capacity(code.len() + crate::operators::WGSL_COLOR_TRANSFORMS.len()
        + helper.len() + inserts.len() * call.len() + prefix.len() * 16);
    let mut cursor = 0;
    for (position, opening) in inserts {
        result.push_str(&code[cursor..position]);
        result.push_str(if opening { &call } else { ")" });
        cursor = position;
    }
    result.push_str(&code[cursor..]);
    append_namespaced_helpers(&mut result, crate::operators::WGSL_COLOR_TRANSFORMS, &prefix);
    append_namespaced_helpers(&mut result, helper, &prefix);
    Ok(result)
}

fn append_namespaced_helpers(output: &mut String, snippet: &str, prefix: &str) {
    for token in snippet.split_inclusive(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if token.starts_with("cc_") || token.starts_with("oq_ap1_to_srgb") || token.starts_with("oq_srgb_to_ap1") {
            output.push_str(prefix);
        }
        output.push_str(token);
    }
}

// Naga records the image-expression span at the builtin token, not the entire
// call. Walk balanced WGSL delimiters, excluding nested block/line comments.
fn image_call_end(code: &str, start: usize) -> Result<usize, String> {
    let bytes = code.as_bytes();
    let mut index = start;
    let mut depth = 0usize;
    let mut opened = false;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' { index += 1; }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            let mut comments = 1usize;
            while index < bytes.len() && comments != 0 {
                if bytes[index..].starts_with(b"/*") { comments += 1; index += 2; }
                else if bytes[index..].starts_with(b"*/") { comments -= 1; index += 2; }
                else { index += 1; }
            }
            continue;
        }
        match bytes[index] {
            b'(' => { depth += 1; opened = true; }
            b')' if opened => {
                depth -= 1;
                if depth == 0 { return Ok(index + 1); }
            }
            _ => {}
        }
        index += 1;
    }
    Err("Managed image operation has no complete call".into())
}

fn image_domain(image: naga::Handle<naga::Expression>, function: &naga::Function, arguments: &[u8], module: &naga::Module, managed: &HashSet<&str>) -> u8 {
    match function.expressions[image] {
        naga::Expression::GlobalVariable(global) => if module.global_variables[global].name.as_deref().is_some_and(|name| managed.contains(name)) { 2 } else { 1 },
        naga::Expression::FunctionArgument(index) => arguments.get(index as usize).copied().unwrap_or(0),
        _ => 0,
    }
}

fn propagate_calls(block: &naga::Block, caller: &naga::Function, domains: &[u8], module: &naga::Module, managed: &HashSet<&str>, inferred: &mut [Vec<u8>]) {
    for statement in block.iter() {
        match statement {
            naga::Statement::Call { function, arguments, .. } => {
                for (index, argument) in arguments.iter().enumerate() {
                    inferred[function.index()][index] |= image_domain(*argument, caller, domains, module, managed);
                }
            }
            naga::Statement::Block(block) => propagate_calls(block, caller, domains, module, managed, inferred),
            naga::Statement::If { accept, reject, .. } => {
                propagate_calls(accept, caller, domains, module, managed, inferred);
                propagate_calls(reject, caller, domains, module, managed, inferred);
            }
            naga::Statement::Switch { cases, .. } => for case in cases { propagate_calls(&case.body, caller, domains, module, managed, inferred); },
            naga::Statement::Loop { body, continuing, .. } => {
                propagate_calls(body, caller, domains, module, managed, inferred);
                propagate_calls(continuing, caller, domains, module, managed, inferred);
            }
            _ => {}
        }
    }
}
