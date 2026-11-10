use std::sync::Arc;
#[cfg(windows)]
use std::time::Instant;

use crate::compiler::ExecutionPlan;
use crate::evaluation::{ExecutionCommand, GpuFacade};
use crate::project::model::{Graph, SdkError};
use serde::Serialize;

use super::{GpuBackend, GpuExecutionError, GpuExecutor, GpuOutput, GpuOutputHandle, TextureFormat};
#[cfg(windows)]
use super::{
    DxgiSharedTextureExporter, GpuPresentationFrame, GpuPresenter, GpuPreviewReader,
    SharedTextureFrame, SharedTexturePresenter,
};

#[cfg(windows)]
const PRESENTATION_MAX_DIMENSION: u32 = 3_840;

/// Concrete device and presentation capabilities, independent of playback policy.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderDeviceInfo {
    pub adapter_name: String,
    pub backend: String,
    pub device_type: String,
    pub output_mode: String,
    pub shared_texture: bool,
}

/// A staged plan retaining unchanged textures and feedback on its original device.
pub struct PreparedRenderPlan {
    executor: GpuExecutor,
}

impl PreparedRenderPlan {
    /// Upload into a fresh texture so the active plan's shared handles keep their pixels.
    pub fn stage_image(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), GpuExecutionError> {
        let limit = self.executor.backend().device.limits().max_texture_dimension_2d;
        if width == 0 || height == 0 || width > limit || height > limit {
            return Err(GpuExecutionError {
                node_id: Some(node_id.to_owned()),
                message: format!("invalid image dimensions {width}x{height}; maximum dimension is {limit}"),
            });
        }
        let expected = (width as usize).checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if expected != Some(rgba.len()) {
            return Err(GpuExecutionError {
                node_id: Some(node_id.to_owned()),
                message: format!("RGBA byte length {} does not match {width}x{height} texture", rgba.len()),
            });
        }
        self.executor.remove_texture(node_id);
        self.executor.upload_rgba(node_id, rgba, width, height)
    }

    pub fn stage_remove_texture(&mut self, node_id: &str) {
        self.executor.remove_texture(node_id);
    }
}

/// Owns GPU execution, readback, and native presentation on one shared device.
pub struct RenderDevice {
    executor: GpuExecutor,
    info: RenderDeviceInfo,
    #[cfg(windows)]
    shared_presenter: Option<SharedTexturePresenter<DxgiSharedTextureExporter>>,
    #[cfg(windows)]
    presentation_scaler: GpuPreviewReader,
    #[cfg(windows)]
    shared_texture_enabled: bool,
    #[cfg(windows)]
    presentation_started_at: Instant,
}

impl RenderDevice {
    pub async fn new() -> Result<(Self, RenderDeviceInfo), String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: native_backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| format!("Cannot find a native GPU adapter: {error}"))?;
        let adapter_features = adapter.features();
        let mut required_features = wgpu::Features::empty();
        #[cfg(target_os = "macos")]
        if adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM) {
            required_features |= wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        }
        if adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_NV12) {
            required_features |= wgpu::Features::TEXTURE_FORMAT_NV12;
        }
        if adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_P010)
            && adapter_features.contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
        {
            required_features |=
                wgpu::Features::TEXTURE_FORMAT_P010 | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
        }
        let mut limits = wgpu::Limits::default();
        let adapter_limits = adapter.limits();
        limits.max_storage_buffer_binding_size = adapter_limits.max_storage_buffer_binding_size;
        limits.max_buffer_size = adapter_limits.max_buffer_size;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features,
                required_limits: limits,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("Cannot create native GPU device: {error}"))?;
        Self::from_shared_wgpu(device, queue, &adapter)
    }

    /// Takes ownership of these exact device and queue handles without requesting a device.
    pub fn from_shared_wgpu(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: &wgpu::Adapter,
    ) -> Result<(Self, RenderDeviceInfo), String> {
        let adapter_info = adapter.get_info();
        if adapter_info.device_type == wgpu::DeviceType::Cpu {
            return Err("Hardware GPU rendering is required; software CPU adapters are forbidden".to_owned());
        }
        let backend = Arc::new(GpuBackend::from_device(device, queue));
        let executor = GpuExecutor::new(backend.clone());
        #[cfg(windows)]
        let shared_presenter = DxgiSharedTextureExporter::new(backend.clone())
            .ok()
            .map(SharedTexturePresenter::new);
        #[cfg(windows)]
        let presentation_scaler = GpuPreviewReader::new(backend);
        #[cfg(windows)]
        let shared_texture = shared_presenter.is_some();
        #[cfg(not(windows))]
        let shared_texture = false;
        let info = RenderDeviceInfo {
            adapter_name: adapter_info.name,
            backend: format!("{:?}", adapter_info.backend),
            device_type: format!("{:?}", adapter_info.device_type),
            output_mode: "shared-gpu-texture".to_owned(),
            shared_texture,
        };
        Ok((
            Self {
                executor,
                info: info.clone(),
                #[cfg(windows)]
                shared_presenter,
                #[cfg(windows)]
                presentation_scaler,
                #[cfg(windows)]
                shared_texture_enabled: false,
                #[cfg(windows)]
                presentation_started_at: Instant::now(),
            },
            info,
        ))
    }

    pub fn info(&self) -> &RenderDeviceInfo {
        &self.info
    }

    pub fn backend(&self) -> Arc<GpuBackend> {
        self.executor.backend().clone()
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.executor.backend().device
    }

    pub fn sync_plan(&mut self, plan: &ExecutionPlan) -> Result<(), GpuExecutionError> {
        self.executor.sync_plan(plan)
    }

    pub fn prepare_plan(
        &self,
        plan: &ExecutionPlan,
        graph: &Graph,
    ) -> Result<PreparedRenderPlan, GpuExecutionError> {
        let mut executor = self.executor.prepare_plan(plan)?;
        executor.sync_sampling(graph);
        Ok(PreparedRenderPlan { executor })
    }

    pub fn commit_plan(&mut self, prepared: PreparedRenderPlan) -> Result<(), GpuExecutionError> {
        if !Arc::ptr_eq(self.executor.backend(), prepared.executor.backend()) {
            return Err(GpuExecutionError {
                node_id: None,
                message: "Prepared render plan belongs to another render device".to_owned(),
            });
        }
        self.executor = prepared.executor;
        Ok(())
    }

    /// Clear retained outputs while preserving the device and consumer-held export leases.
    pub fn reset(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            if self.shared_presenter.is_some() {
                self.set_shared_texture_enabled(false)?;
            }
            self.shared_texture_enabled = false;
            self.presentation_scaler.clear();
        }
        self.executor = GpuExecutor::new(self.backend());
        Ok(())
    }

    pub fn clear(&mut self) -> Result<(), String> {
        self.reset()
    }

    pub fn sync_sampling(&mut self, graph: &Graph) {
        self.executor.sync_sampling(graph);
    }

    pub fn execute(
        &mut self,
        plan: &ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError> {
        GpuFacade::execute(&mut self.executor, plan, commands)
    }

    pub fn upload_rgba(
        &mut self,
        node_id: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), GpuExecutionError> {
        self.executor.upload_rgba(node_id, rgba, width, height)
    }

    #[cfg(all(target_os = "macos", feature = "native-video"))]
    pub fn upload_videotoolbox(
        &mut self,
        node_id: &str,
        frame: &oq_media::VideoToolboxFrame,
    ) -> Result<(), GpuExecutionError> {
        self.executor.upload_videotoolbox(node_id, frame)
    }

    #[cfg(windows)]
    pub fn upload_d3d12_yuv(
        &mut self,
        node_id: &str,
        frame: &super::D3d12VideoFrame,
    ) -> Result<(), GpuExecutionError> {
        self.executor.upload_d3d12_yuv(node_id, frame)
    }

    pub fn register_external_texture(
        &mut self,
        node_id: &str,
        texture: GpuOutputHandle,
    ) -> Result<(), GpuExecutionError> {
        self.executor.register_external_texture(node_id, texture)
    }

    pub fn remove_texture(&mut self, node_id: &str) {
        self.executor.remove_texture(node_id);
    }

    pub fn output_texture(&self, node_id: &str) -> Option<GpuOutput<'_>> {
        self.executor.output_texture(node_id)
    }

    pub fn output_handle(&self, node_id: &str) -> Option<GpuOutputHandle> {
        self.executor.output_handle(node_id)
    }

    pub fn has_output_texture(&self, node_id: &str) -> bool {
        self.output_texture(node_id).is_some()
    }

    pub fn preview_source(&self, node_id: &str) -> Result<GpuOutputHandle, String> {
        let output = self
            .output_handle(node_id)
            .ok_or_else(|| format!("Output node {node_id} has no native GPU texture"))?;
        if output.format != TextureFormat::Rgba8Unorm {
            return Err(format!(
                "Output node {node_id} uses {:?}; native preview requires rgba8unorm",
                output.format
            ));
        }
        Ok(output)
    }

    /// Read pixels without requiring the native runtime's blocking executor.
    pub async fn read_output_rgba_async(
        &self,
        node_id: &str,
    ) -> Result<(Vec<u8>, u32, u32), String> {
        let output = self
            .output_texture(node_id)
            .ok_or_else(|| format!("Output node {node_id} has no native GPU texture"))?;
        if output.format != TextureFormat::Rgba8Unorm {
            return Err(format!(
                "Output node {node_id} uses {:?}; native readback requires rgba8unorm",
                output.format
            ));
        }
        let rgba = self.executor.read_output_rgba(node_id).await
            .map_err(|error| error.to_string())?;
        Ok((rgba, output.width, output.height))
    }

    #[cfg(feature = "native-runtime")]
    pub fn read_output_rgba(&self, node_id: &str) -> Result<(Vec<u8>, u32, u32), String> {
        pollster::block_on(self.read_output_rgba_async(node_id))
    }

    #[cfg(feature = "native-runtime")]
    pub fn read_output(&self, node_id: &str) -> Result<Vec<u8>, String> {
        let (rgba, width, height) = self.read_output_rgba(node_id)?;
        Ok(Self::output_payload(&rgba, width, height))
    }

    pub fn output_payload(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
        let mut payload = Vec::with_capacity(8 + rgba.len());
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        payload.extend_from_slice(rgba);
        payload
    }

    #[cfg(windows)]
    pub fn set_shared_texture_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        let presenter = self.shared_presenter.as_mut()
            .ok_or_else(|| "DXGI shared texture Presenter is unavailable".to_owned())?;
        if !enabled {
            if let Some(frame) = presenter.take_latest() {
                presenter.release(frame.lease_id)?;
            }
        }
        self.shared_texture_enabled = enabled;
        Ok(enabled)
    }

    #[cfg(windows)]
    pub fn take_shared_texture(&mut self) -> Option<SharedTextureFrame> {
        self.shared_presenter.as_mut().and_then(SharedTexturePresenter::take_latest)
    }

    #[cfg(windows)]
    pub fn has_shared_texture_pending(&self) -> bool {
        self.shared_presenter.as_ref().and_then(SharedTexturePresenter::latest).is_some()
    }

    #[cfg(windows)]
    pub fn release_shared_texture(&mut self, lease_id: u64) -> Result<(), String> {
        self.shared_presenter.as_mut()
            .ok_or_else(|| "DXGI shared texture Presenter is unavailable".to_owned())?
            .release(lease_id)
    }

    /// Submit the retained output using the presentation clock's original origin.
    /// `timeline_ns` is composition time; shared presentation timestamps use elapsed wall time.
    pub fn present(
        &mut self,
        _output_node_id: &str,
        _frame: u64,
        _timeline_ns: u64,
    ) -> Result<(), String> {
        #[cfg(windows)]
        if self.shared_texture_enabled {
            if let Some(output) = self.output_handle(_output_node_id) {
                let output = if output.width.max(output.height) > PRESENTATION_MAX_DIMENSION {
                    self.presentation_scaler.scale(&output, PRESENTATION_MAX_DIMENSION)?
                } else {
                    output
                };
                if let Some(presenter) = self.shared_presenter.as_mut() {
                    let timeline_ns = self.presentation_started_at.elapsed().as_nanos()
                        .min(u128::from(u64::MAX)) as u64;
                    presenter.submit(GpuPresentationFrame {
                        node_id: _output_node_id.to_owned(),
                        frame: _frame,
                        timeline_ns,
                        output,
                    })?;
                    presenter.process_latest()?;
                }
            }
        }
        Ok(())
    }
}

impl GpuFacade for RenderDevice {
    fn execute(
        &mut self,
        plan: &ExecutionPlan,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError> {
        RenderDevice::execute(self, plan, commands)
    }
}

fn native_backends() -> wgpu::Backends {
    #[cfg(target_os = "windows")]
    return wgpu::Backends::DX12;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    return wgpu::Backends::METAL;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
    return wgpu::Backends::VULKAN;
}

#[cfg(test)]
#[path = "device_coverage_tests.rs"]
mod tests;
