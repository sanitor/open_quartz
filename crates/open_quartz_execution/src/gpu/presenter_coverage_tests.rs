use super::*;
use std::sync::Arc;

async fn output() -> GpuOutputHandle {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: !cfg!(target_os = "macos"),
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .unwrap();
    let backend = Arc::new(crate::gpu::GpuBackend::from_device(device, queue));
    let texture = backend.create_texture(1, 1, crate::gpu::TextureFormat::Rgba8Unorm);
    GpuOutputHandle {
        texture: texture.texture,
        view: texture.view,
        sampler: texture.sampler,
        width: 1,
        height: 1,
        format: texture.format,
    }
}

fn frame(output: &GpuOutputHandle, number: u64) -> GpuPresentationFrame {
    GpuPresentationFrame {
        node_id: "node".to_owned(),
        frame: number,
        timeline_ns: number,
        output: output.clone(),
    }
}

struct Presenter {
    mailbox: LatestFrameMailbox,
    fail: bool,
}
impl Presenter {
    fn new(fail: bool) -> Self {
        Self {
            mailbox: LatestFrameMailbox::default(),
            fail,
        }
    }
}
impl GpuPresenter for Presenter {
    fn capabilities(&self) -> PresentationCapabilities {
        PresentationCapabilities {
            kind: PresentationBackendKind::RgbaReadback,
            zero_cpu_copy: false,
            preserves_exact_pixels: true,
            supports_dom_composition: true,
            estimated_latency_frames: 1,
        }
    }
    fn mailbox(&self) -> &LatestFrameMailbox {
        &self.mailbox
    }
    fn process_latest(&mut self) -> Result<bool, String> {
        if self.fail {
            Err("failure".to_owned())
        } else {
            Ok(self.mailbox.try_take().is_some())
        }
    }
}

#[test]
fn mailbox_close_wait_and_registry_success_failure_paths() {
    pollster::block_on(async {
        let output = output().await;
        let mailbox = LatestFrameMailbox::default();
        assert!(mailbox.wait_take(Duration::from_millis(1)).is_none());
        mailbox.submit(frame(&output, 1)).unwrap();
        let poisoned = LatestFrameMailbox::default();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.state.lock().unwrap();
            panic!("poison mailbox");
        }));
        assert!(poisoned.submit(frame(&output, 0)).is_err());
        assert!(poisoned.try_take().is_none());
        assert!(poisoned.wait_take(Duration::from_millis(1)).is_none());
        assert_eq!(poisoned.stats(), PresentationQueueStats::default());
        poisoned.close();
        assert_eq!(
            mailbox.wait_take(Duration::from_millis(1)).unwrap().frame,
            1
        );
        mailbox.close();
        assert!(mailbox.submit(frame(&output, 2)).is_err());
        assert!(mailbox.try_take().is_none());

        let mut registry = PresenterRegistry::default();
        let closed = Presenter::new(false);
        closed.mailbox.close();
        registry.register("closed", Box::new(closed)).unwrap();
        assert!(registry
            .register("", Box::new(Presenter::new(false)))
            .is_err());
        registry
            .register("ok", Box::new(Presenter::new(false)))
            .unwrap();
        assert!(registry
            .register("ok", Box::new(Presenter::new(false)))
            .is_err());
        registry
            .register("fail", Box::new(Presenter::new(true)))
            .unwrap();
        let capabilities = registry.capabilities();
        assert_eq!(capabilities.len(), 3);
        let stats = registry.submit(frame(&output, 3));
        assert_eq!(stats.accepted, 2);
        assert_eq!(stats.failed, 1);
        let stats = registry.submit(frame(&output, 4));
        assert_eq!(stats.replaced, 2);
        let results = registry.process_latest();
        assert!(results.iter().any(|(_, result)| result.is_err()));
        assert!(registry.remove("ok"));
        assert!(!registry.remove("missing"));
    });
}

struct Exporter {
    fail: bool,
}
impl SharedTextureExporter for Exporter {
    fn export(&mut self, frame: &GpuPresentationFrame) -> Result<SharedTextureFrame, String> {
        if self.fail {
            return Err("export".to_owned());
        }
        Ok(SharedTextureFrame {
            lease_id: frame.frame,
            platform: SharedTexturePlatform::IoSurface,
            resource_handle: 1,
            sync_handle: None,
            sync_value: 0,
            width: 1,
            height: 1,
            frame: frame.frame,
            timeline_ns: frame.timeline_ns,
        })
    }
    fn release(&mut self, _: u64) -> Result<(), String> {
        if self.fail {
            Err("release".to_owned())
        } else {
            Ok(())
        }
    }
}

#[test]
fn shared_presenter_no_work_pending_take_and_export_failure_paths() {
    pollster::block_on(async {
        let output = output().await;
        let mut presenter = SharedTexturePresenter::new(Exporter { fail: false });
        assert!(!presenter.process_latest().unwrap());
        presenter.submit(frame(&output, 1)).unwrap();
        assert!(presenter.process_latest().unwrap());
        assert!(!presenter.process_latest().unwrap());
        assert!(presenter.take_latest().is_some());
        assert!(presenter.take_latest().is_none());
        presenter.release(1).unwrap();
        let capabilities = presenter.capabilities();
        assert_eq!(capabilities.kind, PresentationBackendKind::SharedTexture);

        let mut failing = SharedTexturePresenter::new(Exporter { fail: true });
        failing.submit(frame(&output, 2)).unwrap();
        assert!(failing.process_latest().is_err());
        assert!(failing.release(2).is_err());
    });
}
