use open_quartz::project::{Edge, Graph, InputMode, NodeFactoryRequest};

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
        let progress = left.curve.wgsl();
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

pub(super) fn source_id(clip: &str) -> String {
    format!("{}:{clip}/{SOURCE_ID}", clip.len())
}

/// Lower the current timeline stack into the existing graph executor. The
/// protected clip outputs are replaced by one program output for all layers.
pub(super) fn graph(request: &PreviewRequest) -> Result<Graph, String> {
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
    if slots.iter().any(|layer| layer.dissolve.is_some()
        || layer.opacity.as_ref().is_some_and(|animation| animation.keys.len() > 1)
        || layer.spatial.as_ref().is_some_and(|animation| animation.keys.len() > 1)
        || layer.crop_animation.as_ref().is_some_and(|animation| animation.keys.len() > 1)) {
        bindings.push_str(&format!("@group(0) @binding({}) var<uniform> iTime:f32;\n", slots.len() * 4));
    }
    for (index, layer) in slots.iter().enumerate() {
        let clip = layer.pipeline.graph();
        let namespace = |id: &str| format!("{}:{}/{id}", layer.clip_id.len(), layer.clip_id);
        let edge = clip
            .edges
            .iter()
            .find(|edge| edge.target == OUTPUT_ID)
            .ok_or_else(|| format!("{} has no connected output", layer.track_name))?;
        inputs.push((namespace(&edge.source), namespace(&edge.source_handle)));
        for node in clip.nodes.iter().filter(|node| node.id != OUTPUT_ID) {
            let mut node = node.clone();
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
            edge.source = namespace(&edge.source);
            edge.source_handle = namespace(&edge.source_handle);
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
            serde_json::json!([cos, sin, 2.0f32.powf(t.exposure), order]),
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
        let alpha = if !matches!(layer.source, LayerSource::Video { .. }) { format!("({alpha}) * sample.a") } else { alpha };
        draws.push_str(&format!(
            "if (i32(blend{index}.w) == order) {{\n\
                {placement}
                {crop_setup}
                let p = source_uv(uv, vec2f(textureDimensions(layer{index})), layer_place, layer_blend);\n\
                let sample = textureSampleLevel(layer{index}, layer{index}Sampler, p, 0.0);\n\
                if (all(p >= {minimum}) && all(p <= {maximum})) {{\n\
                    let alpha = {alpha};\n\
                    let color = linear(clamp(sample.rgb * blend{index}.z, vec3f(0.0), vec3f(1.0)));\n\
                    track_pixel += vec4f(color * alpha, alpha) * {weight};\n\
                }}\n\
            }}\n"
        ));
    }
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
@fragment fn main(@location(0) uv:vec2f) -> @location(0) vec4f {{
    var result = vec3f(0.0);
    for (var order = 0; order < {}; order++) {{
        var track_pixel = vec4f(0.0);
        {draws}
        result = track_pixel.rgb + result * (1.0 - track_pixel.a);
    }}
    return vec4f(encoded(result), 1.0);
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
    output.id = OUTPUT_ID.into();
    output.data.width = Some(request.width);
    output.data.height = Some(request.height);
    output.data.auto_size = Some(false);
    output.data.uniforms = parameters;
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
