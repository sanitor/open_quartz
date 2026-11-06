use super::*;

const SCREEN_SAVER_RESAMPLE_NODE_ID: &str = "__screen_saver_output_resample";
const SCREEN_SAVER_SOURCE_EDGE_ID: &str = "__screen_saver_source_edge";
const SCREEN_SAVER_RENDERER_EDGE_ID: &str = "__screen_saver_renderer_edge";
const SCREEN_SAVER_RESAMPLE_SHADER: &str = r#"
@group(0) @binding(0) var inputImage: texture_2d<f32>;
@group(0) @binding(1) var inputImageSampler: sampler;

@fragment fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
  return textureSample(inputImage, inputImageSampler, v_uv);
}"#;

impl Project {
    pub fn screen_saver_graph(
        &self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<Graph, SdkError> {
        let mut graph = self
            .graph
            .upstream_subgraph(&NodeId::new(renderer_node_id))?;
        graph.insert_screen_saver_resample(renderer_node_id, width, height)?;
        graph
            .validate()
            .expect("screen saver graph transformation should preserve validity");
        Ok(graph)
    }
}

trait ScreenSaverGraphTransform {
    fn insert_screen_saver_resample(
        &mut self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), SdkError>;
}

impl ScreenSaverGraphTransform for Graph {
    fn insert_screen_saver_resample(
        &mut self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), SdkError> {
        if self
            .nodes()
            .iter()
            .any(|node| node.id == SCREEN_SAVER_RESAMPLE_NODE_ID)
        {
            return Err(SdkError::new(
                SdkErrorCode::InvalidGraph,
                "Screen saver resample node ID is already in use",
            )
            .for_node(SCREEN_SAVER_RESAMPLE_NODE_ID));
        }
        let renderer = self
            .nodes()
            .iter()
            .find(|node| node.id == renderer_node_id && node.node_type == NodeType::Renderer)
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::UnknownNode,
                    "The selected Renderer is missing from the exported graph",
                )
                .for_node(renderer_node_id)
            })?
            .clone();
        let renderer_edge_index = self
            .edges()
            .iter()
            .position(|edge| edge.target == renderer_node_id)
            .ok_or_else(|| {
                SdkError::new(
                    SdkErrorCode::InvalidGraph,
                    "The selected Renderer has no connected input",
                )
                .for_node(renderer_node_id)
            })?;
        let renderer_edge = self.edges.remove(renderer_edge_index);
        let resample_node = screen_saver_resample_node(renderer.position, width, height);
        self.nodes.push(resample_node);
        self.edges.push(Edge {
            id: SCREEN_SAVER_SOURCE_EDGE_ID.to_owned(),
            source: renderer_edge.source.clone(),
            source_handle: renderer_edge.source_handle.clone(),
            target: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
            target_handle: "inputImage".to_owned(),
        });
        self.edges.push(Edge {
            id: SCREEN_SAVER_RENDERER_EDGE_ID.to_owned(),
            source: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
            source_handle: "fragColor".to_owned(),
            target: renderer_edge.target,
            target_handle: renderer_edge.target_handle,
        });
        Ok(())
    }
}

fn screen_saver_resample_node(position: Position, width: u32, height: u32) -> ProjectNode {
    ProjectNode {
        id: SCREEN_SAVER_RESAMPLE_NODE_ID.to_owned(),
        node_type: NodeType::Shader,
        position,
        data: NodeData {
            node_type: NodeType::Shader,
            label: "Screen Resolution".to_owned(),
            template_name: Some("Resample".to_owned()),
            shader_template_id: Some("Resample".to_owned()),
            shader_code: SCREEN_SAVER_RESAMPLE_SHADER.to_owned(),
            inputs: vec![Port {
                id: "inputImage".to_owned(),
                label: "inputImage".to_owned(),
                data_type: DataType::Sampler2d,
                direction: PortDirection::Input,
                default_value: None,
                description: None,
            }],
            outputs: vec![Port {
                id: "fragColor".to_owned(),
                label: "fragColor".to_owned(),
                data_type: DataType::Sampler2d,
                direction: PortDirection::Output,
                default_value: None,
                description: None,
            }],
            uniforms: serde_json::Map::new(),
            input_data_type: None,
            input_mode: None,
            image_data_url: None,
            image_width: None,
            image_height: None,
            fb_format: None,
            fb_width: None,
            fb_height: None,
            fb_stride: None,
            raw_data_url: None,
            tex_filter: None,
            tex_wrap: None,
            width: Some(width),
            height: Some(height),
            auto_size: Some(false),
            out_format: Some(FramebufferFormat::Rgba8),
            onnx_model_id: None,
            onnx_score_threshold: None,
            onnx_iou_threshold: None,
            onnx_target_size: None,
            onnx_source: None,
            onnx_catalog_id: None,
            onnx_custom_path: None,
            onnx_params: None,
            video_source_type: None,
            video_url: None,
            video_file_path: None,
            video_device_id: None,
            video_loop: None,
            video_playback_rate: None,
            system_source: None,
            math_op: None,
            feedback_enabled: None,
            feedback_clear_color: None,
            extra: serde_json::Map::new(),
        },
    }
}

#[cfg(test)]
#[path = "screen_saver_coverage_tests.rs"]
mod coverage_tests;
