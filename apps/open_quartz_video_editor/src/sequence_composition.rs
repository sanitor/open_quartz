use open_quartz::project::{Edge, Graph, NodeFactoryRequest};

use super::{OpacityAnimation, PreviewRequest, OUTPUT_ID, SOURCE_ID};

fn opacity_function(index: usize, animation: &OpacityAnimation, origin: u64) -> String {
    let seconds = |source_ns: u64| {
        (i128::from(animation.timeline_start_ns) - i128::from(origin)
            + i128::from(source_ns) - i128::from(animation.source_in_ns)) as f64 / 1_000_000_000.0
    };
    let mut shader = format!("fn animated_alpha{index}() -> f32 {{\n");
    for pair in animation.keys.windows(2) {
        let left = pair[0];
        let right = pair[1];
        let span = (right.source_ns - left.source_ns) as f64 / 1_000_000_000.0;
        shader.push_str(&format!(
            "if (iTime < {:.9e}) {{ return mix({:.9e}, {:.9e}, clamp((iTime - ({:.9e})) / {:.9e}, 0.0, 1.0)); }}\n",
            seconds(right.source_ns), left.opacity, right.opacity, seconds(left.source_ns), span,
        ));
    }
    let last = animation.keys.last().expect("animation has at least one key");
    shader.push_str(&format!("return {:.9e};\n}}\n", last.opacity));
    shader
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
    let origin = request.animation_origin_ns();
    if slots.iter().any(|layer| layer.opacity.as_ref().is_some_and(|animation| animation.keys.len() > 1)) {
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
        let order = request
            .layers
            .iter()
            .rev()
            .position(|item| item.clip_id == layer.clip_id)
            .expect("layer belongs to the frame plan");
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
                animation_code.push_str(&opacity_function(index, animation, origin));
                format!("animated_alpha{index}()")
            }
            None => format!("placement{index}.w"),
        };
        draws.push_str(&format!(
            "if (i32(blend{index}.w) == order) {{\n\
                let p = source_uv(uv, vec2f(textureDimensions(layer{index})), placement{index}, blend{index});\n\
                let rgb = textureSampleLevel(layer{index}, layer{index}Sampler, p, 0.0).rgb;\n\
                if (all(p >= vec2f(0.0)) && all(p <= vec2f(1.0))) {{\n\
                    let alpha = {alpha};\n\
                    result = linear(clamp(rgb * blend{index}.z, vec3f(0.0), vec3f(1.0))) * alpha + result * (1.0 - alpha);\n\
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
        {draws}
    }}
    return vec4f(encoded(result), 1.0);
}}", request.width, request.height, slots.len());
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
