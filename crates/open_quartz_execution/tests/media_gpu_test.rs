use std::sync::Arc;

use open_quartz_execution::gpu::{GpuBackend, GpuExecutor, GpuOutputHandle, TextureFormat};
use open_quartz_execution::media::{
    ExternalFrameSync, ExternalVideoFrame, ExternalVideoFrameImporter, ExternalVideoPlane,
    ImportedVideoFrame, NativeDecoderCapabilities, NativeVideoDecoder, NativeVideoGpuSource,
    NativeVideoSurfaceKind, VideoPixelFormat,
};

async fn output() -> (Arc<GpuBackend>, GpuOutputHandle) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: !cfg!(target_os = "macos"),
            compatible_surface: None,
        })
        .await
        .expect("a GPU adapter is required for media tests");
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .expect("GPU device creation must succeed");
    let backend = Arc::new(GpuBackend::from_device(device, queue));
    let texture = backend.create_texture(16, 16, TextureFormat::Rgba8Unorm);
    let output = GpuOutputHandle {
        texture: texture.texture,
        view: texture.view,
        sampler: texture.sampler,
        width: texture.width,
        height: texture.height,
        format: texture.format,
    };
    (backend, output)
}

struct TestDecoder(bool);

impl NativeVideoDecoder for TestDecoder {
    fn capabilities(&self) -> NativeDecoderCapabilities {
        NativeDecoderCapabilities {
            decoder: "ffmpeg-hw".to_owned(),
            surface_kinds: vec![NativeVideoSurfaceKind::Dxgi],
            pixel_formats: vec![VideoPixelFormat::Nv12],
            zero_cpu_copy: true,
        }
    }

    fn next_frame(&mut self) -> Result<Option<ExternalVideoFrame>, String> {
        if self.0 {
            return Ok(None);
        }
        self.0 = true;
        Ok(Some(ExternalVideoFrame {
            surface_kind: NativeVideoSurfaceKind::Dxgi,
            pixel_format: VideoPixelFormat::Nv12,
            width: 16,
            height: 16,
            planes: vec![ExternalVideoPlane {
                handle: 7,
                offset: 0,
                stride: 16,
            }],
            pts_ns: 33_000_000,
            duration_ns: 33_000_000,
            sync: ExternalFrameSync::Fence {
                handle: 8,
                value: 9,
            },
        }))
    }

    fn pause(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn resume(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn seek(&mut self, _timestamp_ns: i64) -> Result<(), String> {
        self.0 = false;
        Ok(())
    }
}

struct TestImporter(GpuOutputHandle);

impl ExternalVideoFrameImporter for TestImporter {
    fn supported_surface_kinds(&self) -> &[NativeVideoSurfaceKind] {
        &[NativeVideoSurfaceKind::Dxgi]
    }

    fn import(&mut self, frame: ExternalVideoFrame) -> Result<ImportedVideoFrame, String> {
        assert_eq!(
            frame.sync,
            ExternalFrameSync::Fence {
                handle: 8,
                value: 9
            }
        );
        Ok(ImportedVideoFrame {
            texture: self.0.clone(),
            pts_ns: frame.pts_ns,
            duration_ns: frame.duration_ns,
        })
    }
}

#[test]
fn imported_decoder_surface_registers_as_gpu_node_texture() {
    pollster::block_on(async {
        let (backend, output) = output().await;
        let mut source = NativeVideoGpuSource::new(TestDecoder(false), TestImporter(output));
        let frame = source
            .next_frame()
            .expect("decode/import must succeed")
            .unwrap();
        assert_eq!(frame.pts_ns, 33_000_000);

        let mut executor = GpuExecutor::new(backend);
        executor
            .register_external_texture("video", frame.texture)
            .expect("imported texture must register");
        let registered = executor
            .output_handle("video")
            .expect("video output must be visible");
        assert_eq!((registered.width, registered.height), (16, 16));
    });
}
