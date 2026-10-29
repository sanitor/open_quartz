use std::path::{Path, PathBuf};

/// Resolve runtime resources from the portable folder or macOS app bundle.
pub fn runtime_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            directories.push(parent.join("runtime"));
            if parent.file_name().is_some_and(|name| name == "MacOS") {
                if let Some(contents) = parent.parent() {
                    directories.push(contents.join("Resources/runtime"));
                }
            }
        }
    }
    directories.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime"));
    directories
}

pub fn runtime_file(name: &str) -> Result<PathBuf, String> {
    runtime_directories()
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("Required desktop runtime resource is missing: {name}"))
}

/// Configure native libraries before renderer/model workers are created.
pub fn configure_runtime() -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut paths = runtime_directories();
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        std::env::set_var(
            "PATH",
            std::env::join_paths(paths).map_err(|error| error.to_string())?,
        );
    }
    if std::env::var_os("ORT_DYLIB_PATH").is_none() {
        let name = if cfg!(windows) {
            "onnxruntime.dll"
        } else if cfg!(target_os = "macos") {
            "libonnxruntime.1.21.0.dylib"
        } else {
            "libonnxruntime.so.1.21.0"
        };
        if let Ok(path) = runtime_file(name) {
            std::env::set_var("ORT_DYLIB_PATH", path);
        }
    }
    if std::env::var_os("OPEN_QUARTZ_FFMPEG_PATH").is_none() {
        if let Ok(path) = runtime_file(if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        }) {
            std::env::set_var("OPEN_QUARTZ_FFMPEG_PATH", path);
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn export_screen_saver(
    project: &open_quartz_sdk::Project,
    renderer_node_id: &str,
    path: &Path,
) -> Result<(), String> {
    {
        use open_quartz_screensaver::{
            export_windows, ScreenSaverExportRequest, ScreenSaverExposedInput,
            ScreenSaverModelFiles, ScreenSaverRuntimeFiles,
        };
        use open_quartz_sdk::{InputMode, NodeType};
        let graph = project
            .graph()
            .upstream_subgraph(&open_quartz_sdk::NodeId::new(renderer_node_id))
            .map_err(|error| error.to_json())?;
        let exposed_inputs = graph
            .nodes()
            .iter()
            .filter_map(|node| {
                if node.node_type != NodeType::Input {
                    return None;
                }
                let kind = match node.data.input_mode {
                    Some(InputMode::Image) => "image",
                    Some(InputMode::Video) => "video",
                    _ => return None,
                };
                Some(ScreenSaverExposedInput {
                    node_id: node.id.clone(),
                    label: node.data.label.clone(),
                    kind: kind.into(),
                })
            })
            .collect();
        let base = directories::BaseDirs::new().ok_or("Cannot resolve model cache directory")?;
        let runtime = runtime_directories()
            .into_iter()
            .find(|directory| directory.join("open-quartz-screensaver-stub.exe").is_file())
            .ok_or("Screen Saver launcher is not bundled; build the desktop distribution first")?;
        export_windows(ScreenSaverExportRequest {
            output_path: path.to_owned(),
            name: project.name().to_owned(),
            project_json: prepare_screen_saver_project(project, renderer_node_id)?,
            renderer_node_id: renderer_node_id.to_owned(),
            exposed_inputs,
            stub_path: runtime.join("open-quartz-screensaver-stub.exe"),
            runtime_files: ScreenSaverRuntimeFiles {
                ffmpeg: runtime.join("ffmpeg.exe"),
                onnxruntime: runtime.join("onnxruntime.dll"),
                directml: runtime.join("DirectML.dll"),
            },
            model_files: ScreenSaverModelFiles {
                cache_dir: base.data_dir().join("com.openquartz.app/models"),
                resolved: std::collections::HashMap::new(),
            },
        })
    }
}

#[cfg(any(windows, test))]
fn prepare_screen_saver_project(
    project: &open_quartz_sdk::Project,
    renderer_node_id: &str,
) -> Result<String, String> {
    use base64::Engine as _;
    use image::ImageEncoder as _;
    use open_quartz_sdk::{FramebufferFormat, InputMode, NodeId, NodeType};
    let mut file = project.to_file();
    file.graph = file
        .graph
        .upstream_subgraph(&NodeId::new(renderer_node_id))
        .map_err(|error| error.to_json())?;
    for node in &mut file.graph.nodes {
        if node.node_type != NodeType::Input || node.data.input_mode != Some(InputMode::Framebuffer)
        {
            continue;
        }
        let data = &mut node.data;
        let source = data
            .raw_data_url
            .as_deref()
            .ok_or_else(|| format!("Framebuffer '{}' has no raw resource", data.label))?;
        let (Some(width), Some(height)) = (data.fb_width, data.fb_height) else {
            return Err(format!("Framebuffer '{}' has no dimensions", data.label));
        };
        let asset = super::file_service::decode_framebuffer_data_url(
            source,
            "framebuffer.png".into(),
            data.fb_format.unwrap_or(FramebufferFormat::Rgba8),
            width,
            height,
            data.fb_stride,
        )?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&asset.rgba, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|error| error.to_string())?;
        data.input_mode = Some(InputMode::Image);
        data.image_data_url = Some(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ));
        data.image_width = Some(width);
        data.image_height = Some(height);
        data.raw_data_url = None;
        data.fb_format = None;
        data.fb_width = None;
        data.fb_height = None;
        data.fb_stride = None;
    }
    serde_json::to_string(&file).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn screen_saver_bakes_padded_framebuffer_without_changing_project() {
        use base64::Engine as _;
        use open_quartz_sdk::{
            DataType, Environment, GraphCommand, InputMode, NodeFactoryRequest, NodeId, OpenQuartz,
            PortId, PortKey,
        };
        let sdk = OpenQuartz::new(Environment::headless());
        let mut project = sdk.create_project("Raw screen saver");
        let (input, _) = project
            .create_graph_node(
                NodeFactoryRequest::Input {
                    position: None,
                    data_type: DataType::Sampler2d,
                    input_mode: Some(InputMode::Framebuffer),
                },
                project.graph_revision(),
            )
            .unwrap();
        let (renderer, _) = project
            .create_graph_node(
                NodeFactoryRequest::Renderer { position: None },
                project.graph_revision(),
            )
            .unwrap();
        let mut data = input.data.clone();
        data.fb_width = Some(1);
        data.fb_height = Some(2);
        data.fb_stride = Some(8);
        data.raw_data_url = Some(format!(
            "data:application/octet-stream;base64,{}",
            base64::engine::general_purpose::STANDARD
                .encode([255u8, 0, 0, 255, 7, 7, 7, 7, 0, 255, 0, 255])
        ));
        project
            .apply_graph_command(
                GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&input.id),
                    data,
                },
                project.graph_revision(),
            )
            .unwrap();
        project
            .apply_graph_command(
                GraphCommand::Connect {
                    source: PortKey::new(
                        NodeId::new(&input.id),
                        PortId::new(&input.data.outputs[0].id),
                    ),
                    target: PortKey::new(
                        NodeId::new(&renderer.id),
                        PortId::new(&renderer.data.inputs[0].id),
                    ),
                },
                project.graph_revision(),
            )
            .unwrap();
        let before = project.to_file();
        let json = super::prepare_screen_saver_project(&project, &renderer.id).unwrap();
        let exported = sdk.open_project_json(&json).unwrap();
        let data = &exported
            .graph()
            .nodes()
            .iter()
            .find(|node| node.id == input.id)
            .unwrap()
            .data;
        assert_eq!(data.input_mode, Some(InputMode::Image));
        assert!(data.raw_data_url.is_none());
        let asset = crate::services::file_service::decode_image_data_url(
            data.image_data_url.as_deref().unwrap(),
            "baked.png".into(),
        )
        .unwrap();
        assert_eq!(asset.rgba.as_slice(), &[255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(project.to_file(), before);
    }
}
