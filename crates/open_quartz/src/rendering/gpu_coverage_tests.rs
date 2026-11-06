use super::*;
use std::sync::Arc;

async fn backend() -> Arc<GpuBackend> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: !cfg!(target_os = "macos"),
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .expect("coverage requires a GPU adapter");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .unwrap();
    Arc::new(GpuBackend::from_device(device, queue))
}

#[test]
fn backend_preview_scaling_readback_and_blit_cover_resource_paths() {
    pollster::block_on(async {
        let backend = backend().await;
        let shared = Arc::new(GpuBackend::from_shared(
            backend.device.clone(),
            backend.queue.clone(),
        ));
        let texture = shared.create_texture(4, 2, TextureFormat::Rgba8Unorm);
        assert!(shared.upload_rgba(&texture, &[0; 3]).is_err());
        let pixels = vec![255_u8; 4 * 2 * 4];
        shared.upload_rgba(&texture, &pixels).unwrap();
        let target = shared.create_target(4, 2, TextureFormat::Rgba8Unorm);
        shared.blit(&texture, &target);
        let read = shared.read_target_rgba(&target).await.unwrap();
        assert_eq!(read, pixels);

        let output = GpuOutputHandle {
            texture: texture.texture.clone(),
            view: texture.view.clone(),
            sampler: texture.sampler.clone(),
            width: texture.width,
            height: texture.height,
            format: texture.format,
        };
        let mut preview = GpuPreviewReader::new(shared.clone());
        assert!(preview.scale(&output, 0).is_err());
        assert!(preview.read(&output, 0).await.is_err());
        let mut unsupported = output.clone();
        unsupported.format = TextureFormat::Rgba32Float;
        assert!(preview.scale(&unsupported, 2).is_err());
        let scaled = preview.scale(&output, 2).unwrap();
        assert_eq!((scaled.width, scaled.height), (2, 1));
        let same = preview.scale(&output, 2).unwrap();
        assert_eq!((same.width, same.height), (2, 1));
        let image = preview.read(&output, 4).await.unwrap();
        assert_eq!((image.width, image.height, image.rgba.len()), (4, 2, 32));
        for _ in 0..3 {
            shared.readback_ring.acquire(&shared.device, 256).unwrap();
        }
        assert!(preview.read(&output, 4).await.is_err());
        assert!(image.scale_submit <= image.scale_submit + image.readback);
    });
}
