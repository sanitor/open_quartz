use std::sync::Arc;

use open_quartz_execution::gpu::{
    GpuBackend, GpuPresentationFrame, GpuPresenter, LatestFrameMailbox, SharedTextureExporter,
    SharedTextureFrame, SharedTexturePlatform, SharedTexturePresenter, TextureFormat,
};

async fn output() -> open_quartz_execution::gpu::GpuOutputHandle {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: !cfg!(target_os = "macos"),
            compatible_surface: None,
        })
        .await
        .expect("a GPU adapter is required for presenter tests");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("GPU device creation must succeed");
    let backend = Arc::new(GpuBackend::from_device(device, queue));
    let texture = backend.create_texture(4, 4, TextureFormat::Rgba8Unorm);
    open_quartz_execution::gpu::GpuOutputHandle {
        texture: texture.texture,
        view: texture.view,
        sampler: texture.sampler,
        width: texture.width,
        height: texture.height,
        format: texture.format,
    }
}

fn frame(output: &open_quartz_execution::gpu::GpuOutputHandle, number: u64) -> GpuPresentationFrame {
    GpuPresentationFrame {
        node_id: "renderer".to_owned(),
        frame: number,
        timeline_ns: number * 16_666_667,
        output: output.clone(),
    }
}

#[test]
fn latest_frame_mailbox_replaces_slow_presenter_work() {
    pollster::block_on(async {
        let output = output().await;
        let mailbox = LatestFrameMailbox::default();

        assert!(!mailbox.submit(frame(&output, 1)).unwrap());
        assert!(mailbox.submit(frame(&output, 2)).unwrap());
        assert!(mailbox.submit(frame(&output, 3)).unwrap());

        assert_eq!(mailbox.try_take().unwrap().frame, 3);
        assert_eq!(
            mailbox.stats(),
            open_quartz_execution::gpu::PresentationQueueStats {
                submitted: 3,
                replaced: 2,
                consumed: 1,
            }
        );
    });
}

struct TestExporter {
    released: Vec<u64>,
}

impl SharedTextureExporter for TestExporter {
    fn export(&mut self, frame: &GpuPresentationFrame) -> Result<SharedTextureFrame, String> {
        Ok(SharedTextureFrame {
            lease_id: frame.frame,
            platform: SharedTexturePlatform::Dxgi,
            resource_handle: 42,
            sync_handle: Some(43),
            sync_value: frame.frame,
            width: frame.output.width,
            height: frame.output.height,
            frame: frame.frame,
            timeline_ns: frame.timeline_ns,
        })
    }

    fn release(&mut self, lease_id: u64) -> Result<(), String> {
        self.released.push(lease_id);
        Ok(())
    }
}

#[test]
fn shared_texture_presenter_exports_only_the_latest_frame() {
    pollster::block_on(async {
        let output = output().await;
        let mut presenter = SharedTexturePresenter::new(TestExporter {
            released: Vec::new(),
        });

        presenter.submit(frame(&output, 7)).unwrap();
        presenter.submit(frame(&output, 8)).unwrap();
        assert!(presenter.process_latest().unwrap());

        let exported = presenter.latest().unwrap();
        assert_eq!(exported.resource_handle, 42);
        assert_eq!(exported.sync_handle, Some(43));
        assert_eq!(exported.sync_value, 8);
        assert_eq!((exported.width, exported.height), (4, 4));
        let lease_id = exported.lease_id;
        presenter.release(lease_id).unwrap();
    });
}
