use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use windows_graphics::core::Interface;

use windows_graphics::Win32::Foundation::{CloseHandle, BOOL, GENERIC_ALL, HANDLE, WAIT_OBJECT_0};
use windows_graphics::Win32::Graphics::Direct3D12::{
    ID3D12Fence, ID3D12Resource, D3D12_FENCE_FLAG_SHARED, D3D12_HEAP_FLAG_SHARED,
    D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_NONE, D3D12_RESOURCE_STATE_COMMON,
    D3D12_TEXTURE_LAYOUT_UNKNOWN,
};
use windows_graphics::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_P010, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows_graphics::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

use super::{
    GpuBackend, GpuPresentationFrame, SharedTextureExporter, SharedTextureFrame,
    SharedTexturePlatform, TextureFormat, TextureHandle,
};

const SHARED_TEXTURE_POOL_SIZE: usize = 3;

struct SharedTextureSlot {
    texture: wgpu::Texture,
    fence: ID3D12Fence,
    resource_handle: isize,
    fence_handle: isize,
    width: u32,
    height: u32,
    lease_id: Option<u64>,
    fence_value: u64,
}

impl SharedTextureSlot {
    fn wait_for_signal(&self) {
        if self.fence_value == 0 || unsafe { self.fence.GetCompletedValue() } >= self.fence_value {
            return;
        }
        let Ok(event) = (unsafe { CreateEventW(None, BOOL(0), BOOL(0), None) }) else {
            return;
        };
        unsafe {
            if self
                .fence
                .SetEventOnCompletion(self.fence_value, event)
                .is_ok()
            {
                let _ = WaitForSingleObject(event, 5_000) == WAIT_OBJECT_0;
            }
            let _ = CloseHandle(event);
        }
    }
}

impl Drop for SharedTextureSlot {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(HANDLE(self.resource_handle as *mut _));
            let _ = CloseHandle(HANDLE(self.fence_handle as *mut _));
        }
    }
}

pub struct DxgiSharedTextureExporter {
    // Drop order is declaration order: imported textures must die before their device.
    slots: Vec<SharedTextureSlot>,
    backend: Arc<GpuBackend>,
    next_lease_id: u64,
}

impl Drop for DxgiSharedTextureExporter {
    fn drop(&mut self) {
        let _ = self
            .backend
            .device
            .poll(wgpu::PollType::wait_indefinitely());
        for slot in &self.slots {
            slot.wait_for_signal();
        }
    }
}

impl DxgiSharedTextureExporter {
    pub fn new(backend: Arc<GpuBackend>) -> Result<Self, String> {
        if unsafe { backend.device.as_hal::<wgpu::hal::api::Dx12>() }.is_none() {
            return Err("DXGI shared textures require the wgpu DX12 backend".to_owned());
        }
        Ok(Self {
            backend,
            slots: Vec::with_capacity(SHARED_TEXTURE_POOL_SIZE),
            next_lease_id: 1,
        })
    }

    fn create_slot(&self, width: u32, height: u32) -> Result<SharedTextureSlot, String> {
        let hal_device = unsafe { self.backend.device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| "wgpu DX12 device is unavailable".to_owned())?;
        let raw_device = hal_device.raw_device();
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            ..Default::default()
        };
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: u64::from(width),
            Height: height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: D3D12_RESOURCE_FLAG_NONE,
            ..Default::default()
        };
        let mut resource: Option<ID3D12Resource> = None;
        unsafe {
            raw_device
                .CreateCommittedResource(
                    &heap,
                    D3D12_HEAP_FLAG_SHARED,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut resource,
                )
                .map_err(|error| format!("Cannot create shared DXGI texture: {error}"))?;
        }
        let resource = resource.ok_or_else(|| "D3D12 returned no shared texture".to_owned())?;
        let resource_handle = unsafe {
            raw_device
                .CreateSharedHandle(&resource, None, GENERIC_ALL.0, None)
                .map_err(|error| format!("Cannot export DXGI texture handle: {error}"))?
        };
        let fence: ID3D12Fence = unsafe {
            raw_device
                .CreateFence(0, D3D12_FENCE_FLAG_SHARED)
                .map_err(|error| format!("Cannot create shared DXGI fence: {error}"))?
        };
        let fence_handle =
            match unsafe { raw_device.CreateSharedHandle(&fence, None, GENERIC_ALL.0, None) } {
                Ok(handle) => handle,
                Err(error) => {
                    unsafe {
                        let _ = CloseHandle(resource_handle);
                    }
                    return Err(format!("Cannot export DXGI fence handle: {error}"));
                }
            };

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let hal_texture = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            )
        };
        let texture = unsafe {
            self.backend
                .device
                .create_texture_from_hal::<wgpu::hal::api::Dx12>(
                    hal_texture,
                    &wgpu::TextureDescriptor {
                        label: Some("open-quartz-dxgi-shared-texture"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    },
                )
        };

        Ok(SharedTextureSlot {
            texture,
            fence,
            resource_handle: resource_handle.0 as isize,
            fence_handle: fence_handle.0 as isize,
            width,
            height,
            lease_id: None,
            fence_value: 0,
        })
    }

    fn acquire_slot(&mut self, width: u32, height: u32) -> Result<usize, String> {
        if let Some(index) = self.slots.iter().position(|slot| {
            slot.lease_id.is_none() && slot.width == width && slot.height == height
        }) {
            return Ok(index);
        }
        if self.slots.len() < SHARED_TEXTURE_POOL_SIZE {
            let slot = self.create_slot(width, height)?;
            self.slots.push(slot);
            return Ok(self.slots.len() - 1);
        }
        if let Some(index) = self.slots.iter().position(|slot| slot.lease_id.is_none()) {
            self.slots[index] = self.create_slot(width, height)?;
            return Ok(index);
        }
        Err("DXGI shared texture pool is saturated; release a consumed lease".to_owned())
    }
}

impl SharedTextureExporter for DxgiSharedTextureExporter {
    fn export(&mut self, frame: &GpuPresentationFrame) -> Result<SharedTextureFrame, String> {
        if frame.output.format != TextureFormat::Rgba8Unorm {
            return Err("DXGI shared texture export requires rgba8unorm output".to_owned());
        }
        let slot_index = self.acquire_slot(frame.output.width, frame.output.height)?;
        let slot = &mut self.slots[slot_index];
        let mut encoder =
            self.backend
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("open-quartz-dxgi-shared-copy"),
                });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &frame.output.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &slot.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: frame.output.width,
                height: frame.output.height,
                depth_or_array_layers: 1,
            },
        );
        self.backend.queue.submit([encoder.finish()]);

        slot.fence_value = slot.fence_value.saturating_add(1);
        let hal_device = unsafe { self.backend.device.as_hal::<wgpu::hal::api::Dx12>() }
            .ok_or_else(|| "wgpu DX12 device disappeared".to_owned())?;
        unsafe {
            hal_device
                .raw_queue()
                .Signal(&slot.fence, slot.fence_value)
                .map_err(|error| format!("Cannot signal shared DXGI fence: {error}"))?;
        }
        let lease_id = self.next_lease_id;
        self.next_lease_id = self.next_lease_id.saturating_add(1);
        slot.lease_id = Some(lease_id);

        Ok(SharedTextureFrame {
            lease_id,
            platform: SharedTexturePlatform::Dxgi,
            resource_handle: slot.resource_handle as usize as u64,
            sync_handle: Some(slot.fence_handle as usize as u64),
            sync_value: slot.fence_value,
            width: slot.width,
            height: slot.height,
            frame: frame.frame,
            timeline_ns: frame.timeline_ns,
        })
    }

    fn release(&mut self, lease_id: u64) -> Result<(), String> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.lease_id == Some(lease_id))
            .ok_or_else(|| format!("Unknown or already released DXGI lease {lease_id}"))?;
        slot.lease_id = None;
        Ok(())
    }
}
/// A decoded D3D12 video surface retained independently of its FFmpeg AVFrame.
pub struct D3d12VideoFrame {
    resource: ID3D12Resource,
    pub width: u32,
    pub height: u32,
    pub subresource_index: u32,
    array_layers: u32,
}

unsafe impl Send for D3d12VideoFrame {}
unsafe impl Sync for D3d12VideoFrame {}

impl D3d12VideoFrame {
    /// Takes one borrowed FFmpeg resource/fence reference and waits for decode completion.
    pub unsafe fn from_raw(
        resource: *mut c_void,
        fence: *mut c_void,
        event: *mut c_void,
        fence_value: u64,
        width: u32,
        height: u32,
        subresource_index: u32,
    ) -> Result<Self, String> {
        let borrowed_resource = ID3D12Resource::from_raw(
            NonNull::new(resource)
                .ok_or_else(|| "FFmpeg returned a null D3D12 resource".to_owned())?
                .as_ptr(),
        );
        let resource = borrowed_resource.clone();
        std::mem::forget(borrowed_resource);
        let description = resource.GetDesc();
        if description.Format != DXGI_FORMAT_P010 {
            return Err(format!(
                "D3D12VA produced DXGI format {:?}, expected P010",
                description.Format
            ));
        }
        let array_layers = u32::from(description.DepthOrArraySize);
        if subresource_index >= array_layers {
            return Err(format!(
                "FFmpeg D3D12 subresource {subresource_index} exceeds array size {array_layers}"
            ));
        }
        let borrowed_fence = ID3D12Fence::from_raw(
            NonNull::new(fence)
                .ok_or_else(|| "FFmpeg returned a null D3D12 fence".to_owned())?
                .as_ptr(),
        );
        let fence = borrowed_fence.clone();
        std::mem::forget(borrowed_fence);
        let completed = fence.GetCompletedValue();
        if completed < fence_value {
            let wait_event = CreateEventW(None, BOOL(0), BOOL(0), None)
                .map_err(|error| format!("Cannot create D3D12 decode wait event: {error}"))?;
            fence
                .SetEventOnCompletion(fence_value, wait_event)
                .map_err(|error| format!("Cannot arm D3D12 decode fence: {error}"))?;
            let result = WaitForSingleObject(wait_event, 5_000);
            let _ = CloseHandle(wait_event);
            if result != WAIT_OBJECT_0 {
                return Err("Timed out waiting for D3D12 video decode".to_owned());
            }
        }
        let _ = event;
        Ok(Self {
            resource,
            width,
            height,
            subresource_index,
            array_layers,
        })
    }
}

impl GpuBackend {
    /// Converts an imported P010 D3D12 surface to the normal RGBA graph texture.
    /// The conversion is GPU-only; no decoded bytes cross the CPU.
    pub fn upload_d3d12_p010(
        &self,
        frame: &D3d12VideoFrame,
        output: &TextureHandle,
    ) -> Result<(), String> {
        let size = wgpu::Extent3d {
            width: frame.width,
            height: frame.height,
            depth_or_array_layers: frame.array_layers,
        };
        let hal_texture = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                frame.resource.clone(),
                wgpu::TextureFormat::P010,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            )
        };
        let source = unsafe {
            self.device.create_texture_from_hal::<wgpu::hal::api::Dx12>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("open-quartz-d3d12-p010"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::P010,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        let y_view = source.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::R16Unorm),
            aspect: wgpu::TextureAspect::Plane0,
            base_array_layer: frame.subresource_index,
            array_layer_count: Some(1),
            ..Default::default()
        });
        let uv_view = source.create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rg16Unorm),
            aspect: wgpu::TextureAspect::Plane1,
            base_array_layer: frame.subresource_index,
            array_layer_count: Some(1),
            ..Default::default()
        });
        let (layout, pipeline) = self
            .p010_converter
            .get_or_init(|| create_p010_pipeline(&self.device));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("open-quartz-p010-bindings"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&y_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&uv_view),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("open-quartz-p010-convert"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("open-quartz-p010-convert-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }
}

fn create_p010_pipeline(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("open-quartz-p010-converter"),
        source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(P010_CONVERTER)),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("open-quartz-p010-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("open-quartz-p010-pipeline-layout"),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("open-quartz-p010-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    (layout, pipeline)
}

const P010_CONVERTER: &str = r#"
struct O { @builtin(position) p: vec4f, @location(0) uv: vec2f }
@vertex fn vs(@builtin(vertex_index) i: u32) -> O { var o: O; let x = f32(i / 2u) * 4.0 - 1.0; let y = f32(i % 2u) * 4.0 - 1.0; o.p = vec4f(x,y,0,1); o.uv = vec2f((x+1.0)*0.5,(1.0-y)*0.5); return o; }
@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
@fragment fn fs(@location(0) uv: vec2f) -> @location(0) vec4f {
  let ys = textureDimensions(y_tex); let p = vec2i(clamp(uv * vec2f(ys), vec2f(0.0), vec2f(ys - 1u)));
  let c = vec2i(clamp(uv * vec2f(textureDimensions(uv_tex)), vec2f(0.0), vec2f(textureDimensions(uv_tex) - 1u)));
  let y = (textureLoad(y_tex,p,0).r * 1023.0 - 64.0) / 876.0;
  let u = textureLoad(uv_tex,c,0).r * 1023.0 / 896.0 - 0.5;
  let v = textureLoad(uv_tex,c,0).g * 1023.0 / 896.0 - 0.5;
  return vec4f(clamp(vec3f(y + 1.5748*v, y - 0.1873*u - 0.4681*v, y + 1.8556*u), vec3f(0.0), vec3f(1.0)), 1.0);
}
"#;
