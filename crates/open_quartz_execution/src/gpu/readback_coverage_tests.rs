use super::*;

async fn device() -> (wgpu::Device, wgpu::Queue) {
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
    adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .unwrap()
}

#[test]
fn staging_ring_reuses_resizes_saturates_releases_and_reports_poisoning() {
    pollster::block_on(async {
        let (device, _) = device().await;
        let ring = ReadbackStagingRing::new();
        let (first, _) = ring.acquire(&device, 4).unwrap();
        ring.release(first);
        let (first_again, _) = ring.acquire(&device, 8).unwrap();
        assert_eq!(first_again, first);
        let _ = ring.acquire(&device, 8).unwrap();
        let _ = ring.acquire(&device, 8).unwrap();
        assert!(ring.acquire(&device, 8).is_err());
        ring.release(99);

        let poisoned = ReadbackStagingRing::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.state.lock().unwrap();
            panic!("poison readback ring");
        }));
        assert!(poisoned.acquire(&device, 4).is_err());
        poisoned.release(0);
    });
}

#[test]
fn shared_backend_constructor_and_float_format_conversion_are_covered() {
    pollster::block_on(async {
        let (device, queue) = device().await;
        let backend = shared_backend(device, queue);
        assert_eq!(backend.device.features(), backend.supported_features());
        assert_eq!(
            wgpu::TextureFormat::from(crate::gpu::TextureFormat::Rgba32Float),
            wgpu::TextureFormat::Rgba32Float
        );
    });
}

#[test]
fn readback_propagates_map_poll_callback_and_copy_failures() {
    pollster::block_on(async {
        use std::sync::atomic::Ordering;
        use std::sync::Arc;
        let (device, queue) = device().await;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        for fault in [FAULT_MAP, FAULT_POLL, FAULT_CALLBACK_DROP, FAULT_COPY] {
            let backend = GpuBackend::from_shared(device.clone(), queue.clone());
            let target = backend.create_target(1, 1, crate::gpu::TextureFormat::Rgba8Unorm);
            backend.readback_fault.store(fault, Ordering::Release);
            assert!(backend.read_target_rgba(&target).await.is_err());
        }
    });
}
