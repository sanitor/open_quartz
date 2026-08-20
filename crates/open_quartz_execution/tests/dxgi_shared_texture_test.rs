#![cfg(windows)]

use std::sync::Arc;

use open_quartz_execution::gpu::{
    DxgiSharedTextureExporter, GpuBackend, GpuPresentationFrame, SharedTextureExporter,
    TextureFormat,
};
use windows_graphics::Win32::Foundation::{CloseHandle, BOOL, HANDLE, WAIT_OBJECT_0};
use windows_graphics::Win32::Graphics::Direct3D12::{ID3D12Fence, ID3D12Resource};
use windows_graphics::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
use windows_graphics::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

async fn dx12_backend() -> Arc<GpuBackend> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .expect("a DX12 adapter is required for shared texture tests");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("DX12 device creation must succeed");
    Arc::new(GpuBackend::from_device(device, queue))
}

#[test]
fn dxgi_export_opens_resource_and_signals_shared_fence() {
    pollster::block_on(async {
        let backend = dx12_backend().await;
        let source = backend.create_texture(8, 6, TextureFormat::Rgba8Unorm);
        backend
            .upload_rgba(&source, &vec![127; 8 * 6 * 4])
            .expect("source upload must succeed");
        let mut exporter = DxgiSharedTextureExporter::new(backend.clone()).unwrap();
        let frame = GpuPresentationFrame {
            node_id: "renderer".to_owned(),
            frame: 1,
            timeline_ns: 16_666_667,
            output: open_quartz_execution::gpu::GpuOutputHandle {
                texture: source.texture,
                view: source.view,
                sampler: source.sampler,
                width: source.width,
                height: source.height,
                format: source.format,
            },
        };
        let exported = exporter.export(&frame).expect("DXGI export must succeed");

        let hal_device = unsafe { backend.device.as_hal::<wgpu::hal::api::Dx12>() }.unwrap();
        let raw_device = hal_device.raw_device();
        let mut resource: Option<ID3D12Resource> = None;
        let mut fence: Option<ID3D12Fence> = None;
        unsafe {
            raw_device
                .OpenSharedHandle(
                    HANDLE(exported.resource_handle as usize as *mut _),
                    &mut resource,
                )
                .expect("shared texture handle must reopen");
            raw_device
                .OpenSharedHandle(
                    HANDLE(exported.sync_handle.unwrap() as usize as *mut _),
                    &mut fence,
                )
                .expect("shared fence handle must reopen");
        }
        let resource = resource.unwrap();
        let desc = unsafe { resource.GetDesc() };
        assert_eq!((desc.Width, desc.Height), (8, 6));
        assert_eq!(desc.Format, DXGI_FORMAT_R8G8B8A8_UNORM);

        let event = unsafe { CreateEventW(None, BOOL(0), BOOL(0), None) }.unwrap();
        unsafe {
            fence
                .unwrap()
                .SetEventOnCompletion(exported.sync_value, event)
                .expect("consumer fence wait must register");
            assert_eq!(WaitForSingleObject(event, 5_000), WAIT_OBJECT_0);
            CloseHandle(event).unwrap();
        }
        exporter.release(exported.lease_id).unwrap();
    });
}

#[test]
fn dxgi_pool_requires_consumer_release_before_reuse() {
    pollster::block_on(async {
        let backend = dx12_backend().await;
        let source = backend.create_texture(2, 2, TextureFormat::Rgba8Unorm);
        let output = open_quartz_execution::gpu::GpuOutputHandle {
            texture: source.texture,
            view: source.view,
            sampler: source.sampler,
            width: source.width,
            height: source.height,
            format: source.format,
        };
        let mut exporter = DxgiSharedTextureExporter::new(backend).unwrap();
        let mut leases = Vec::new();
        for frame in 1..=3 {
            leases.push(
                exporter
                    .export(&GpuPresentationFrame {
                        node_id: "renderer".to_owned(),
                        frame,
                        timeline_ns: frame,
                        output: output.clone(),
                    })
                    .unwrap(),
            );
        }
        assert!(exporter
            .export(&GpuPresentationFrame {
                node_id: "renderer".to_owned(),
                frame: 4,
                timeline_ns: 4,
                output: output.clone(),
            })
            .unwrap_err()
            .contains("pool is saturated"));

        let released = leases.remove(0);
        exporter.release(released.lease_id).unwrap();
        let reused = exporter
            .export(&GpuPresentationFrame {
                node_id: "renderer".to_owned(),
                frame: 5,
                timeline_ns: 5,
                output,
            })
            .unwrap();
        assert_eq!(reused.resource_handle, released.resource_handle);
        assert!(reused.sync_value > released.sync_value);
    });
}
