use super::*;
use std::collections::BTreeMap;

use crate::compiler::build_execution_plan;
use crate::project::model::Graph;
use serde_json::json;

async fn shared_device() -> (RenderDevice, RenderDeviceInfo, GpuBackend, wgpu::Adapter) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: native_backends(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: !cfg!(target_os = "macos"),
        compatible_surface: None,
        apply_limit_buckets: false,
    }).await.unwrap();
    let (device, queue) = adapter.request_device(&wgpu::DeviceDescriptor::default()).await.unwrap();
    let original = GpuBackend::from_device(device.clone(), queue.clone());
    let (render, info) = RenderDevice::from_shared_wgpu(device, queue, &adapter).unwrap();
    assert_eq!(info.adapter_name, adapter.get_info().name);
    assert_eq!(info.backend, format!("{:?}", adapter.get_info().backend));
    (render, info, original, adapter)
}

fn command(node_id: &str, kind: &str) -> ExecutionCommand {
    ExecutionCommand {
        node_id: node_id.to_owned(),
        kind: kind.to_owned(),
        output_port_id: None,
        texture_inputs: BTreeMap::new(),
        uniforms: BTreeMap::new(),
        target_width: None,
        target_height: None,
        feedback_read_index: None,
        feedback_write_index: None,
        clear_feedback: false,
        scalar_output: None,
    }
}

fn graph() -> Graph {
    serde_json::from_value(json!({
        "nodes": [
            {
                "id": "shader", "type": "shader", "position": {"x": 0, "y": 0},
                "data": {
                    "type": "shader", "label": "Shader",
                    "shaderCode": "@fragment fn main() -> @location(0) vec4f { return vec4f(0.0, 0.0, 1.0, 1.0); }",
                    "inputs": [],
                    "outputs": [{"id": "out", "label": "output", "dataType": "sampler2D", "direction": "output"}],
                    "uniforms": {}, "autoSize": false, "width": 4, "height": 2
                }
            },
            {
                "id": "renderer", "type": "renderer", "position": {"x": 1, "y": 0},
                "data": {
                    "type": "renderer", "label": "Renderer", "shaderCode": "",
                    "inputs": [{"id": "in", "label": "image", "dataType": "sampler2D", "direction": "input"}],
                    "outputs": [], "uniforms": {}
                }
            }
        ],
        "edges": [{"id": "edge", "source": "shader", "sourceHandle": "out", "target": "renderer", "targetHandle": "in"}]
    })).unwrap()
}

#[test]
fn output_payload_prefixes_little_endian_dimensions() {
    let payload = RenderDevice::output_payload(&[1, 2, 3, 4], 640, 360);
    assert_eq!(&payload[..4], &640_u32.to_le_bytes());
    assert_eq!(&payload[4..8], &360_u32.to_le_bytes());
    assert_eq!(&payload[8..], &[1, 2, 3, 4]);
}

#[test]
fn native_backend_matches_platform() {
    #[cfg(target_os = "windows")]
    assert_eq!(native_backends(), wgpu::Backends::DX12);
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    assert_eq!(native_backends(), wgpu::Backends::METAL);
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
    assert_eq!(native_backends(), wgpu::Backends::VULKAN);
}

#[test]
fn shared_device_resources_keep_pixels_and_outlive_owner() {
    pollster::block_on(async {
        let (mut render, info, original, _) = shared_device().await;
        assert_eq!(render.info().adapter_name, info.adapter_name);
        assert_eq!(render.info().shared_texture, info.shared_texture);
        assert_eq!(render.device().features(), original.device.features());
        assert!(Arc::ptr_eq(&render.backend(), &render.backend()));

        // Resources created and written through the caller's original device/queue
        // must remain usable for readback on RenderDevice's device.
        let texture = original.create_texture(2, 1, TextureFormat::Rgba8Unorm);
        let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
        original.upload_rgba(&texture, &rgba).unwrap();
        render.register_external_texture("image", GpuOutputHandle {
            texture: texture.texture,
            view: texture.view,
            sampler: texture.sampler,
            lease: texture.lease,
            width: texture.width,
            height: texture.height,
            format: texture.format,
        }).unwrap();
        let handle = render.preview_source("image").unwrap();
        assert!(Arc::ptr_eq(&handle.texture, &render.output_handle("image").unwrap().texture));
        assert_eq!(render.read_output_rgba_async("image").await.unwrap(), (rgba.to_vec(), 2, 1));
        render.remove_texture("image");
        assert!(!render.has_output_texture("image"));
        drop(render);
        assert_eq!(original.read_texture_rgba(&handle.texture, 2, 1).await.unwrap(), rgba);
    });
}

#[test]
fn execute_preserves_renderer_pixels_across_static_frames_and_sync() {
    pollster::block_on(async {
        let (mut render, _, _, _) = shared_device().await;
        let graph = graph();
        let plan = build_execution_plan(&graph.nodes, &graph.edges);
        render.sync_plan(&plan).unwrap();
        render.sync_sampling(&graph);
        let mut renderer = command("renderer", "renderer");
        renderer.texture_inputs.insert("in".to_owned(), "shader".to_owned());
        GpuFacade::execute(&mut render, &plan, &[command("shader", "shader"), renderer]).unwrap();
        let (rgba, width, height) = render.read_output_rgba_async("renderer").await.unwrap();
        assert_eq!((width, height), (4, 2));
        assert_eq!(rgba.len(), 4 * 2 * 4);
        assert!(rgba.chunks_exact(4).all(|pixel| pixel == [0, 0, 255, 255]));
        let output = render.output_handle("renderer").unwrap();
        render.execute(&plan, &[]).unwrap();
        assert!(Arc::ptr_eq(&output.texture, &render.output_handle("renderer").unwrap().texture));
        assert_eq!(render.read_output_rgba_async("renderer").await.unwrap().0, rgba);
        assert_eq!(render.read_output_rgba_async("shader").await.unwrap().0, rgba);
        #[cfg(feature = "native-runtime")]
        {
            assert_eq!(render.read_output_rgba("renderer").unwrap(), (rgba.clone(), width, height));
            assert_eq!(render.read_output("renderer").unwrap(), RenderDevice::output_payload(&rgba, width, height));
        }
    });
}

#[test]
fn execution_upload_preview_and_readback_keep_error_attribution() {
    pollster::block_on(async {
        let (mut render, _, original, adapter) = shared_device().await;
        let graph = graph();
        let plan = build_execution_plan(&graph.nodes, &graph.edges);
        let error = render.execute(&plan, &[command("bad-node", "unsupported")]).unwrap_err();
        assert_eq!(error.node_id.as_deref(), Some("bad-node"));
        assert_eq!(error.code, crate::project::model::SdkErrorCode::InvalidResource);
        let error = render.upload_rgba("bad-image", &[0; 3], 1, 1).unwrap_err();
        assert_eq!(error.node_id.as_deref(), Some("bad-image"));
        assert_eq!(render.preview_source("missing").err().unwrap(), "Output node missing has no native GPU texture");
        assert_eq!(render.read_output_rgba_async("missing").await.unwrap_err(), "Output node missing has no native GPU texture");

        let target = render.backend().create_target(1, 1, TextureFormat::Rgba32Float);
        render.register_external_texture("float", GpuOutputHandle {
            texture: target.texture,
            view: target.view,
            sampler: Arc::new(render.device().create_sampler(&wgpu::SamplerDescriptor::default())),
            lease: None,
            width: 1,
            height: 1,
            format: TextureFormat::Rgba32Float,
        }).unwrap();
        assert!(render.preview_source("float").err().unwrap().contains("native preview requires rgba8unorm"));
        assert!(render.read_output_rgba_async("float").await.unwrap_err().contains("native readback requires rgba8unorm"));

        // A fresh owner per injected failure preserves the executor's existing
        // readback error contract without reusing failed staging buffers.
        for fault in [1, 2, 3, 4] {
            let (mut render, _) = RenderDevice::from_shared_wgpu(
                original.device.as_ref().clone(),
                original.queue.as_ref().clone(),
                &adapter,
            ).unwrap();
            render.upload_rgba("image", &[0, 0, 0, 255], 1, 1).unwrap();
            render.backend().readback_fault.store(fault, std::sync::atomic::Ordering::Release);
            let error = render.read_output_rgba_async("image").await.unwrap_err();
            assert!(error.starts_with("GPU node image:"), "{error}");
        }
    });
}

#[test]
fn failed_plan_staging_preserves_current_output_and_successful_commit_reuses_it() {
    pollster::block_on(async {
        let (mut render, _, _, _) = shared_device().await;
        let graph = graph();
        let plan = build_execution_plan(&graph.nodes, &graph.edges);
        let mut renderer = command("renderer", "renderer");
        renderer.texture_inputs.insert("in".to_owned(), "shader".to_owned());
        render.execute(&plan, &[command("shader", "shader"), renderer]).unwrap();
        let output = render.output_handle("renderer").unwrap();
        let pixels = render.read_output_rgba_async("renderer").await.unwrap();
        for failure in ["validation", "missing-target", "oversized", "zero-size"] {
            let mut invalid = plan.clone();
            let shader = invalid.nodes.iter_mut().find(|node| node.id == "shader").unwrap();
            shader.target.as_mut().unwrap().width = 8;
            let mut bad = shader.clone();
            bad.id = "replacement".to_owned();
            if failure == "missing-target" {
                bad.target = None;
            } else if failure == "oversized" {
                bad.target.as_mut().unwrap().width = render.device().limits().max_texture_dimension_2d + 1;
            } else if failure == "zero-size" {
                bad.target.as_mut().unwrap().height = 0;
            } else {
                bad.validation_errors.push(crate::compiler::shader::WgslCompilationError {
                    message: "invalid replacement shader".to_owned(),
                    line: 1,
                    column: 1,
                    offset: 0,
                    length: 1,
                });
            }
            invalid.nodes.push(bad);
            let error = render.sync_plan(&invalid).unwrap_err();
            assert_eq!(error.node_id.as_deref(), Some("replacement"));
            assert!(render.prepare_plan(&invalid, &graph).is_err());
            assert!(Arc::ptr_eq(&output.texture, &render.output_handle("renderer").unwrap().texture));
            assert_eq!(render.read_output_rgba_async("renderer").await.unwrap(), pixels);
        }
        let prepared = render.prepare_plan(&plan, &graph).unwrap();
        assert!(Arc::ptr_eq(&output.texture, &render.output_handle("renderer").unwrap().texture));
        render.commit_plan(prepared).unwrap();
        assert!(Arc::ptr_eq(&output.texture, &render.output_handle("renderer").unwrap().texture));
        assert_eq!(render.read_output_rgba_async("renderer").await.unwrap(), pixels);
        let backend = render.backend();
        render.reset().unwrap();
        assert!(!render.has_output_texture("renderer"));
        assert!(!render.has_output_texture("shader"));
        assert!(Arc::ptr_eq(&backend, &render.backend()));
        assert_eq!(backend.read_texture_rgba(&output.texture, output.width, output.height).await.unwrap(), pixels.0);
    });
}

#[cfg(windows)]
#[test]
fn shared_presentation_caps_dimensions_uses_origin_and_releases_leases() {
    pollster::block_on(async {
        let (mut render, info) = RenderDevice::new().await.unwrap();
        if !info.shared_texture {
            assert!(render.set_shared_texture_enabled(true).is_err());
            assert!(render.release_shared_texture(1).is_err());
            assert!(!render.has_shared_texture_pending());
            return;
        }
        let width = PRESENTATION_MAX_DIMENSION + 2;
        render.upload_rgba("image", &vec![255; width as usize * 2 * 4], width, 2).unwrap();
        render.set_shared_texture_enabled(true).unwrap();
        render.present("image", 7, u64::MAX).unwrap();
        assert!(render.has_shared_texture_pending());
        let frame = render.take_shared_texture().unwrap();
        assert_eq!(frame.frame, 7);
        assert_eq!(frame.width.max(frame.height), PRESENTATION_MAX_DIMENSION);
        assert!(frame.timeline_ns < u64::MAX);
        assert!(!render.has_shared_texture_pending());
        render.reset().unwrap();
        assert!(!render.has_output_texture("image"));
        // A lease taken by a consumer remains valid after rendering outputs reset.
        render.release_shared_texture(frame.lease_id).unwrap();
        assert!(render.release_shared_texture(frame.lease_id).is_err());
        render.upload_rgba("image", &[0, 0, 0, 255], 1, 1).unwrap();
        render.set_shared_texture_enabled(true).unwrap();
        render.present("image", 8, 0).unwrap();
        let pending_lease = render.shared_presenter.as_ref().unwrap().latest().unwrap().lease_id;
        render.reset().unwrap();
        assert!(!render.has_shared_texture_pending());
        assert!(render.release_shared_texture(pending_lease).is_err());
    });
}

#[cfg(windows)]
#[test]
fn shared_presentation_reports_pool_exhaustion_and_keeps_consumer_leases_releasable() {
    pollster::block_on(async {
        let (mut render, info) = RenderDevice::new().await.unwrap();
        if !info.shared_texture {
            return;
        }
        render.upload_rgba("image", &[0, 0, 0, 255], 1, 1).unwrap();
        render.set_shared_texture_enabled(true).unwrap();
        let mut leases = Vec::new();
        for frame in 0..3 {
            render.present("image", frame, 0).unwrap();
            leases.push(render.take_shared_texture().unwrap().lease_id);
        }
        assert!(render.present("image", 3, 0).is_err());
        assert!(!render.has_shared_texture_pending());
        render.reset().unwrap();
        assert!(!render.has_output_texture("image"));
        for lease in leases {
            render.release_shared_texture(lease).unwrap();
        }
        render.upload_rgba("image", &[0, 0, 0, 255], 1, 1).unwrap();
        render.set_shared_texture_enabled(true).unwrap();
        render.present("image", 4, 0).unwrap();
        let lease = render.take_shared_texture().unwrap().lease_id;
        render.release_shared_texture(lease).unwrap();
        render.shared_presenter.as_ref().unwrap().mailbox().close();
        assert!(render.present("image", 5, 0).is_err());
        render.reset().unwrap();
    });
}

#[test]
fn staged_image_uploads_and_removals_leave_active_pixels_unchanged_until_commit() {
    pollster::block_on(async {
        let (mut render, _, _, _) = shared_device().await;
        let graph = graph();
        let plan = build_execution_plan(&graph.nodes, &graph.edges);
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        render.upload_rgba("image", &red, 1, 1).unwrap();
        let original = render.output_handle("image").unwrap();

        let mut discarded = render.prepare_plan(&plan, &graph).unwrap();
        discarded.stage_image("image", &blue, 1, 1).unwrap();
        assert_eq!(render.read_output_rgba_async("image").await.unwrap().0, red);
        drop(discarded);
        assert!(Arc::ptr_eq(&original.texture, &render.output_handle("image").unwrap().texture));
        assert_eq!(render.read_output_rgba_async("image").await.unwrap().0, red);

        let mut staged = render.prepare_plan(&plan, &graph).unwrap();
        let error = staged.stage_image("image", &[0; 3], 1, 1).unwrap_err();
        assert_eq!(error.node_id.as_deref(), Some("image"));
        let error = staged.stage_image("image", &[], render.device().limits().max_texture_dimension_2d + 1, 1).unwrap_err();
        assert_eq!(error.node_id.as_deref(), Some("image"));
        render.commit_plan(staged).unwrap();
        assert!(Arc::ptr_eq(&original.texture, &render.output_handle("image").unwrap().texture));
        assert_eq!(render.read_output_rgba_async("image").await.unwrap().0, red);

        let mut staged = render.prepare_plan(&plan, &graph).unwrap();
        staged.stage_image("image", &blue, 1, 1).unwrap();
        render.commit_plan(staged).unwrap();
        assert!(!Arc::ptr_eq(&original.texture, &render.output_handle("image").unwrap().texture));
        assert_eq!(render.read_output_rgba_async("image").await.unwrap().0, blue);
        assert_eq!(render.backend().read_texture_rgba(&original.texture, 1, 1).await.unwrap(), red);

        let mut discarded = render.prepare_plan(&plan, &graph).unwrap();
        discarded.stage_remove_texture("image");
        assert_eq!(render.read_output_rgba_async("image").await.unwrap().0, blue);
        drop(discarded);
        assert!(render.has_output_texture("image"));
        let mut staged = render.prepare_plan(&plan, &graph).unwrap();
        staged.stage_remove_texture("image");
        render.commit_plan(staged).unwrap();
        assert!(!render.has_output_texture("image"));
    });
}
