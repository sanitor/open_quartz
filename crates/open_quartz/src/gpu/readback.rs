use std::sync::Arc;

use super::backend::GpuBackend;
pub const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

pub fn align_bytes_per_row(bytes_per_row: u32) -> u32 {
    bytes_per_row.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT
}

pub fn copy_padded_rgba(
    padded: &[u8],
    width: u32,
    height: u32,
    bytes_per_row: u32,
) -> Result<Vec<u8>, String> {
    let aligned = align_bytes_per_row(width * 4);
    if bytes_per_row < aligned {
        return Err(format!(
            "bytes_per_row {bytes_per_row} is below required alignment {aligned}"
        ));
    }
    let required = bytes_per_row as usize * height as usize;
    if padded.len() < required {
        return Err(format!(
            "readback buffer has {} bytes, expected at least {required}",
            padded.len()
        ));
    }
    let row_size = width as usize * 4;
    let mut rgba = vec![0; row_size * height as usize];
    for row in 0..height as usize {
        let src = row * bytes_per_row as usize;
        let dst = row * row_size;
        rgba[dst..dst + row_size].copy_from_slice(&padded[src..src + row_size]);
    }
    Ok(rgba)
}
use super::target::RenderTarget;

impl GpuBackend {
    pub async fn read_target_rgba(&self, target: &RenderTarget) -> Result<Vec<u8>, String> {
        self.read_texture_rgba(&target.texture, target.width, target.height)
            .await
    }

    pub async fn read_texture_rgba(
        &self,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, String> {
        let bytes_per_row = align_bytes_per_row(width * 4);
        let buffer_size = bytes_per_row as u64 * height as u64;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("open-quartz-readback"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("open-quartz-readback-copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = futures_channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|error| format!("GPU poll failed: {error:?}"))?;
        receiver
            .await
            .map_err(|error| format!("GPU readback callback dropped: {error}"))??;

        let mapped = slice.get_mapped_range();
        let rgba = copy_padded_rgba(&mapped, width, height, bytes_per_row)?;
        drop(mapped);
        readback.unmap();
        Ok(rgba)
    }
}

pub fn shared_backend(device: wgpu::Device, queue: wgpu::Queue) -> Arc<GpuBackend> {
    Arc::new(GpuBackend::from_device(device, queue))
}
