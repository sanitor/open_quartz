use open_quartz::project::{Edge, Graph, InputMode, NodeClock, NodeFactoryRequest};

use super::{ClipAnimation, LayerSource, OpacityAnimation, PreviewRequest, OUTPUT_ID, SOURCE_ID};
use crate::animation_key::Keyframe;
use crate::document::DissolveSide;

fn animated_function<Time: Copy, Value: Copy>(
    name: &str, ty: &str, keys: &[Keyframe<Time, Value>],
    seconds: impl Fn(Time) -> f64, format_value: impl Fn(Value) -> String,
) -> String {
    let mut shader = format!("fn {name}() -> {ty} {{\n");
    for pair in keys.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        let (start, end) = (seconds(left.time), seconds(right.time));
        let span = end - start;
        let progress = crate::animation_key::wgsl(left.curve);
        shader.push_str(&format!(
            "if (iTime < {end:.9e}) {{ let t = clamp((iTime - ({start:.9e})) / {span:.9e}, 0.0, 1.0); return mix({}, {}, {progress}); }}\n",
            format_value(left.value), format_value(right.value),
        ));
    }
    let last = keys.last().expect("animation has at least one key");
    shader.push_str(&format!("return {};\n}}\n", format_value(last.value)));
    shader
}

fn opacity_function(index: usize, animation: &OpacityAnimation, origin: u64, speed_percent: u16) -> String {
    let timeline_scale = 100.0 / f64::from(speed_percent);
    let seconds = |source_ns: u64| {
        ((i128::from(animation.timeline_start_ns) - i128::from(origin)) as f64
            + (i128::from(source_ns) - i128::from(animation.source_in_ns)) as f64 * timeline_scale) / 1_000_000_000.0
    };
    animated_function(&format!("animated_alpha{index}"), "f32", &animation.keys, seconds, |opacity| format!("{opacity:.9e}"))
}

fn clip_function<V: Copy>(
    name: &str, animation: &ClipAnimation<V>, origin: u64, format_value: impl Fn(V) -> String,
) -> String {
    let start = (i128::from(animation.timeline_start_ns) - i128::from(origin)) as f64 / 1_000_000_000.0;
    let seconds = |offset: i64| start + offset as f64 * animation.seconds_per_frame;
    animated_function(name, "vec4f", &animation.keys, seconds, format_value)
}

pub(super) fn namespace(clip: &str, id: &str) -> String {
    format!("{}:{clip}/{id}", clip.len())
}
pub(super) fn source_id(clip: &str) -> String {
    namespace(clip, SOURCE_ID)
}

/// Lower the current timeline stack into the existing graph executor. The
/// protected clip outputs are replaced by one program output for all layers.
pub(super) fn graph(request: &PreviewRequest) -> Result<Graph, String> {
    let mut graph = composition(request, None)?;
    if request.managed_view {
        add_managed_view(&mut graph, request.width, request.height)?;
    }
    Ok(graph)
}

fn add_managed_view(graph: &mut Graph, width: u32, height: u32) -> Result<(), String> {
    let program = graph.nodes.iter_mut().find(|node| node.id == OUTPUT_ID).ok_or("Missing managed program output")?;
    program.id = "managed-program".into();
    let source_handle = program.data.outputs.first().ok_or("Missing managed program texture")?.id.clone();
    for edge in graph.edges.iter_mut().filter(|edge| edge.target == OUTPUT_ID) {
        edge.target = program.id.clone();
    }
    let code = format!("{}{}",
        open_quartz::operators::WGSL_COLOR_TRANSFORMS, r#"
        @group(0) @binding(0) var program: texture_2d<f32>;
        @group(0) @binding(1) var programSampler: sampler;
        @fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {
            let p = textureSample(program, programSampler, uv);
            let luma = max(dot(p.rgb, vec3f(0.2722287, 0.6740818, 0.0536895)), 0.0);
            return vec4f(cc_ap1_to_srgb(p.rgb / (1.0 + luma)), p.a);
        }
    "#);
    let (mut output, _) = crate::clip_pipeline::project(Graph::default())
        .create_graph_node(NodeFactoryRequest::Shader {
            position: None, label: "SDR monitor".into(), code,
            template_name: None, shader_template_id: None,
        }, 0).map_err(|error| error.to_string())?;
    output.id = OUTPUT_ID.into();
    output.data.width = Some(width);
    output.data.height = Some(height);
    output.data.auto_size = Some(false);
    output.data.managed_display = true;
    let input = output.data.inputs.iter().find(|port| port.label == "program").ok_or("Missing monitor input")?;
    graph.edges.push(Edge { id: "managed-view".into(), source: program.id.clone(),
        source_handle, target: output.id.clone(), target_handle: input.id.clone() });
    graph.nodes.push(output);
    Ok(())
}

fn composition(request: &PreviewRequest, clock: Option<NodeClock>) -> Result<Graph, String> {
    let mut result = Graph::default();
    let mut bindings = String::new();
    let mut draws = String::new();
    let mut animation_code = String::new();
    let mut parameters = serde_json::Map::new();
    let mut inputs = Vec::new();
    // Keep shader and port order stable when tracks are reordered. A frame's
    // uniforms select the draw order, independently of source bindings.
    let mut slots: Vec<_> = request.layers.iter().collect();
    slots.sort_by_key(|layer| layer.clip_id.as_str());
    let mut tracks: Vec<_> = request.layers.iter().rev().map(|layer| layer.track_id.as_str()).collect();
    tracks.dedup();
    let origin = request.animation_origin_ns();
    let mut mask_binding = slots.len() * 4 + 1;
    let mut mask_animations = std::collections::BTreeMap::new();
    if slots.iter().any(|layer| layer.dissolve.is_some()
        || layer.opacity.as_ref().is_some_and(|animation| animation.keys.len() > 1)
        || layer.spatial.as_ref().is_some_and(|animation| animation.keys.len() > 1)
        || layer.crop_animation.as_ref().is_some_and(|animation| animation.keys.len() > 1)) {
        bindings.push_str(&format!("@group(0) @binding({}) var<uniform> iTime:f32;\n", slots.len() * 4));
    }
    for (index, layer) in slots.iter().enumerate() {
        let clip = layer.pipeline.graph();
        let namespace = |id: &str| namespace(&layer.clip_id, id);
        let nested = if let LayerSource::Nested { request, clock: step, .. } = &layer.source {
            let mut child_clock = clock.clone().unwrap_or(NodeClock {
                root_origin: origin as f64 / 1.0e9, steps: Vec::new(), local_origin: 0.0,
            });
            child_clock.steps.push(*step);
            child_clock.local_origin = request.animation_origin_ns() as f64 / 1.0e9;
            let mut program = composition(request, Some(child_clock))?;
            let nested_output = program.nodes.iter().find(|node| node.id == OUTPUT_ID)
                .ok_or("Nested program has no output")?;
            let output_port = nested_output.data.outputs.first().ok_or("Nested program has no output port")?;
            let source = (namespace(OUTPUT_ID), namespace(&output_port.id));
            for node in &mut program.nodes {
                node.id = namespace(&node.id);
                for port in node.data.inputs.iter_mut().chain(&mut node.data.outputs) {
                    port.id = namespace(&port.id);
                }
            }
            for edge in &mut program.edges {
                edge.id = namespace(&edge.id);
                edge.source = namespace(&edge.source);
                edge.source_handle = namespace(&edge.source_handle);
                edge.target = namespace(&edge.target);
                edge.target_handle = namespace(&edge.target_handle);
            }
            result.nodes.extend(program.nodes);
            result.edges.extend(program.edges);
            Some(source)
        } else { None };
        let mapped_source = |source: &str, port: &str| {
            if source == SOURCE_ID {
                if let Some(source) = &nested { return source.clone(); }
            }
            (namespace(source), namespace(port))
        };
        let edge = clip
            .edges
            .iter()
            .find(|edge| edge.target == OUTPUT_ID)
            .ok_or_else(|| format!("{} has no connected output", layer.track_name))?;
        inputs.push(mapped_source(&edge.source, &edge.source_handle));
        for node in clip.nodes.iter().filter(|node| node.id != OUTPUT_ID) {
            if node.id == SOURCE_ID && nested.is_some() { continue; }
            let mut node = node.clone();
            node.data.clock = clock.clone();
            if node.id == SOURCE_ID {
                if let LayerSource::Video { asset, .. } = &layer.source {
                    node.data.managed_color = request.managed_output;
                    node.data.camera_log = asset.camera_log;
                }
            }
            if let Some(operation) = crate::clip_pipeline::BranchComposite::from_node(&node) {
                node.data.managed_color = request.managed_output;
                node.data.normalize_legacy_inputs = request.managed_output;
                if matches!(operation, crate::clip_pipeline::BranchComposite::LumaMask) && !request.managed_output {
                    node.data.shader_code = node.data.shader_code.replace("0.27222872, 0.67408177, 0.05368952", "0.2126, 0.7152, 0.0722");
                }
            }
            if crate::clip_pipeline::ClipPipeline::is_lut(&node) {
                node.data.managed_color = true;
                node.data.normalize_legacy_inputs = true;
            }
            if let Some(effects) = &layer.effects {
                let start = (i128::from(effects.timeline_start_ns) - i128::from(origin)) as f64 / 1_000_000_000.0;
                for animated in effects.keys.iter().filter(|entry| entry.node == node.id) {
                    node.data.uniform_animations.insert(animated.parameter.clone(), animated.keys.iter().map(|key| {
                        open_quartz::project::UniformAnimationKey {
                            time: start + key.time as f64 * effects.seconds_per_frame,
                            value: key.value,
                            curve: key.curve,
                        }
                    }).collect());
                }
            }
            if node.id == SOURCE_ID && !matches!(layer.source, LayerSource::Video { .. }) {
                node.data.input_mode = Some(InputMode::Image);
            }
            node.id = namespace(&node.id);
            node.position.x = 0.0;
            node.position.y = 0.0;
            node.data.label.clear();
            for port in node.data.inputs.iter_mut().chain(&mut node.data.outputs) {
                port.id = namespace(&port.id);
            }
            result.nodes.push(node);
        }
        for edge in clip.edges.iter().filter(|edge| edge.target != OUTPUT_ID) {
            let mut edge = edge.clone();
            edge.id = namespace(&edge.id);
            (edge.source, edge.source_handle) = mapped_source(&edge.source, &edge.source_handle);
            edge.target = namespace(&edge.target);
            edge.target_handle = namespace(&edge.target_handle);
            result.edges.push(edge);
        }

        let binding = index * 4;
        bindings.push_str(&format!(
            "@group(0) @binding({binding}) var layer{index}:texture_2d<f32>;\n\
             @group(0) @binding({}) var layer{index}Sampler:sampler;\n\
             @group(0) @binding({}) var<uniform> placement{index}:vec4f;\n\
             @group(0) @binding({}) var<uniform> blend{index}:vec4f;\n",
            binding + 1,
            binding + 2,
            binding + 3,
        ));
        let order = tracks.iter().position(|id| *id == layer.track_id)
            .expect("layer belongs to an active track");
        let t = layer.transform;
        let (sin, cos) = t.rotation.to_radians().sin_cos();
        parameters.insert(
            format!("placement{index}"),
            serde_json::json!([t.x, t.y, t.scale, t.opacity]),
        );
        parameters.insert(
            format!("blend{index}"),
            serde_json::json!([cos, sin, 2.0f32.powf(t.exposure), order * 4 + layer.blend.index()]),
        );
        let alpha = match &layer.opacity {
            Some(animation) => {
                animation_code.push_str(&opacity_function(index, animation, origin, layer.speed_percent));
                format!("animated_alpha{index}()")
            }
            None => format!("placement{index}.w"),
        };
        let weight = match layer.dissolve {
            None => "1.0".to_owned(),
            Some(dissolve) => {
                let start = (dissolve.start_ns - origin) as f64 / 1_000_000_000.0;
                let span = (dissolve.end_ns - dissolve.start_ns) as f64 / 1_000_000_000.0;
                let progress = format!("clamp((iTime - {start:.9e}) / {span:.9e}, 0.0, 1.0)");
                match dissolve.side {
                    DissolveSide::Outgoing => format!("(1.0 - {progress})"),
                    DissolveSide::Incoming => progress,
                }
            }
        };
        let placement = if let Some(animation) = &layer.spatial {
            animation_code.push_str(&clip_function(&format!("animated_pose{index}"), animation, origin,
                |value| format!("vec4f({:.9e}, {:.9e}, {:.9e}, {:.9e})",
                    value.x, value.y, value.scale, value.rotation.to_radians())));
            format!("let animated = animated_pose{index}(); let layer_place = vec4f(animated.xyz, placement{index}.w); let layer_blend = vec4f(cos(animated.w), sin(animated.w), blend{index}.zw);")
        } else {
            format!("let layer_place = placement{index}; let layer_blend = blend{index};")
        };
        let (crop_setup, minimum, maximum) = if let Some(animation) = &layer.crop_animation {
            animation_code.push_str(&clip_function(&format!("animated_crop{index}"), animation, origin,
                |crop| format!("vec4f({:.9e}, {:.9e}, {:.9e}, {:.9e})", crop.left, crop.top, 1.0 - crop.right, 1.0 - crop.bottom)));
            (format!("let crop_bounds = animated_crop{index}();"), "crop_bounds.xy".into(), "crop_bounds.zw".into())
        } else {
            let crop = layer.crop;
            (String::new(), format!("vec2f({:.9e}, {:.9e})", crop.left, crop.top),
                format!("vec2f({:.9e}, {:.9e})", 1.0 - crop.right, 1.0 - crop.bottom))
        };
        let mask_center = if let Some(tracked) = &layer.tracked_mask {
            let names = [format!("mask_x{index}"), format!("mask_y{index}")];
            for (axis, name) in names.iter().enumerate() {
                bindings.push_str(&format!("@group(0) @binding({mask_binding}) var<uniform> {name}: f32;\n"));
                mask_binding += 1;
                let start = (i128::from(tracked.timeline_start_ns) - i128::from(origin)) as f64 / 1.0e9;
                let rate = f64::from(layer.speed_percent) / 100.0;
                let keys = tracked.points.iter().map(|point| open_quartz::project::UniformAnimationKey {
                    time: start + (i128::from(point.source_ns) - i128::from(tracked.source_start_ns)) as f64 / (1.0e9 * rate),
                    value: if axis == 0 { point.x } else { point.y },
                    curve: open_quartz::project::Curve::Linear,
                }).collect::<Vec<_>>();
                mask_animations.insert(name.clone(), keys);
            }
            Some(format!("vec2f({}, {})", names[0], names[1]))
        } else { None };
        let mask_setup = if let Some(mask) = layer.mask {
            let distance = match mask.shape {
                crate::document::MaskShape::Rectangle => "max(mask_uv.x, mask_uv.y)",
                crate::document::MaskShape::Ellipse => "length(mask_uv)",
            };
            let gate = if mask.feather == 0.0 {
                format!("select(0.0, 1.0, {distance} <= 1.0)")
            } else {
                format!("(1.0 - smoothstep({:.9e}, 1.0, {distance}))", 1.0 - mask.feather)
            };
            let center = mask_center.unwrap_or_else(|| format!("vec2f({:.9e}, {:.9e})", mask.x, mask.y));
            format!(
                "let mask_uv = abs((p - {center}) * 2.0 / vec2f({:.9e}, {:.9e})); let mask_alpha = {gate};",
                mask.width, mask.height,
            )
        } else { "let mask_alpha = 1.0;".into() };
        let alpha = if !matches!(layer.source, LayerSource::Video { .. }) || clip.nodes.iter()
            .any(|node| crate::clip_pipeline::BranchComposite::from_node(node).is_some()) { format!("({alpha}) * sample.a") } else { alpha };
        let color = if request.managed_output {
            let mut id = edge.source.as_str();
            let working = loop {
                if id == SOURCE_ID {
                    break matches!(&layer.source, LayerSource::Video { .. } | LayerSource::Nested { .. });
                }
                let node = clip.nodes.iter().find(|node| node.id == id).ok_or("Missing layer color producer")?;
                if crate::clip_pipeline::BranchComposite::from_node(node).is_some() || crate::clip_pipeline::ClipPipeline::is_lut(node) { break true; }
                let Some(corrector) = &node.data.color_corrector else { break false; };
                if !corrector.is_identity() { break true; }
                let incoming = clip.edges.iter().find(|edge| edge.target == id).ok_or("Missing Corrector source")?;
                id = &incoming.source;
            };
            if working { format!("sample.rgb * blend{index}.z") }
            else { format!("cc_srgb_to_ap1(sample.rgb) * blend{index}.z") }
        } else { format!("linear(clamp(sample.rgb * blend{index}.z, vec3f(0.0), vec3f(1.0)))") };
        draws.push_str(&format!(
            "if (i32(blend{index}.w) / 4 == order) {{\n\
                track_mode = i32(blend{index}.w) % 4;
                {placement}
                {crop_setup}
                let p = source_uv(uv, vec2f(textureDimensions(layer{index})), layer_place, layer_blend);\n\
                let sample = textureSampleLevel(layer{index}, layer{index}Sampler, p, 0.0);\n\
                if (all(p >= {minimum}) && all(p <= {maximum})) {{\n\
                    {mask_setup}
                    let alpha = ({alpha}) * mask_alpha;\n\
                    let color = {color};\n\
                    track_pixel += vec4f(color * alpha, alpha) * {weight};\n\
                }}\n\
            }}\n"
        ));
    }
    animation_code.push_str(r#"
fn cubic_unit(t: f32, first: f32, second: f32) -> f32 {
    let r = 1.0 - t;
    return 3.0 * r * r * t * first + 3.0 * r * t * t * second + t * t * t;
}
fn bezier_progress(t: f32, points: vec4f) -> f32 {
    if (t <= 0.0 || t >= 1.0) { return t; }
    var low = 0.0;
    var high = 1.0;
    for (var step = 0; step < 16; step = step + 1) {
        let middle = (low + high) / 2.0;
        if (cubic_unit(middle, points.x, points.z) < t) { low = middle; }
        else { high = middle; }
    }
    return cubic_unit((low + high) / 2.0, points.y, points.w);
}
"#);
    if request.managed_output { animation_code.push_str(open_quartz::operators::WGSL_COLOR_TRANSFORMS); }
    let result_color = if request.managed_output && clock.is_some() {
        "return vec4f(result / max(alpha, 0.00001), alpha);"
    } else if request.managed_output {
        "return vec4f(result, 1.0);"
    } else if clock.is_some() {
        "return vec4f(encoded(result / max(alpha, 0.00001)), alpha);"
    } else { "return vec4f(encoded(result), 1.0);" };
    let code = format!("{bindings}
{animation_code}
const canvas = vec2f({}.0, {}.0);
fn source_uv(uv:vec2f, size:vec2f, placement:vec4f, blend:vec4f) -> vec2f {{
    let local = (uv - vec2f(0.5) - placement.xy) * canvas;
    let unrotated = vec2f(blend.x * local.x + blend.y * local.y, -blend.y * local.x + blend.x * local.y);
    let fit = min(canvas.x / size.x, canvas.y / size.y);
    return unrotated / (size * fit * placement.z) + vec2f(0.5);
}}
fn linear(c:vec3f) -> vec3f {{
    return select(pow((c + vec3f(0.055)) / 1.055, vec3f(2.4)), c / 12.92, c <= vec3f(0.04045));
}}
fn encoded(c:vec3f) -> vec3f {{
    return select(1.055 * pow(c, vec3f(1.0 / 2.4)) - vec3f(0.055), 12.92 * c, c <= vec3f(0.0031308));
}}
fn blend_rgb(backdrop: vec3f, source: vec3f, mode: i32) -> vec3f {{
    if (mode == 1) {{ return backdrop * source; }}
    if (mode == 2) {{ return backdrop + source - backdrop * source; }}
    if (mode == 3) {{
        return select(1.0 - 2.0 * (1.0 - backdrop) * (1.0 - source), 2.0 * backdrop * source, backdrop <= vec3f(0.5));
    }}
    return source;
}}
@fragment fn main(@location(0) uv:vec2f) -> @location(0) vec4f {{
    var result = vec3f(0.0);
    var alpha = 0.0;
    for (var order = 0; order < {}; order++) {{
        var track_pixel = vec4f(0.0);
        var track_mode = 0;
        {draws}
        var front = track_pixel.rgb;
        if (track_mode != 0 && track_pixel.a > 0.0 && alpha > 0.0) {{
            let backdrop = result / alpha;
            let source = track_pixel.rgb / track_pixel.a;
            front = mix(source, blend_rgb(backdrop, source, track_mode), alpha) * track_pixel.a;
        }}
        result = front + result * (1.0 - track_pixel.a);
        alpha = track_pixel.a + alpha * (1.0 - track_pixel.a);
    }}
    {result_color}
}}", request.width, request.height, tracks.len());
    let (mut output, _) = crate::clip_pipeline::project(Graph::default())
        .create_graph_node(
            NodeFactoryRequest::Shader {
                position: None,
                code,
                label: "Sequence Output".into(),
                template_name: None,
                shader_template_id: None,
            },
            0,
        )
        .map_err(|error| error.to_string())?;
    if slots.is_empty() {
        // This generated graph has no external uniforms or textures in a gap.
        output.data.inputs.clear();
    }
    output.id = OUTPUT_ID.into();
    output.data.width = Some(request.width);
    output.data.height = Some(request.height);
    output.data.auto_size = Some(false);
    output.data.clock = clock;
    output.data.managed_color = request.managed_output;
    output.data.uniforms = parameters;
    output.data.uniform_animations = mask_animations;
    for (index, (source, source_handle)) in inputs.into_iter().enumerate() {
        let target = output
            .data
            .inputs
            .iter()
            .find(|port| port.label == format!("layer{index}"))
            .ok_or("Could not create the sequence layer input")?;
        result.edges.push(Edge {
            id: format!("sequence-layer-{index}"),
            source,
            source_handle,
            target: output.id.clone(),
            target_handle: target.id.clone(),
        });
    }
    result.nodes.push(output);
    Ok(result)
}
