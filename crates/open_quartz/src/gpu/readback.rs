use std::sync::{Arc, Mutex};

use super::backend::GpuBackend;
pub const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

pub fn align_bytes_per_row(bytes_per_row: u32) -> u32 {
    bytes_per_row.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT
}

const READBACK_RING_SIZE: usize = 3;

struct ReadbackSlot {
    buffer: wgpu::Buffer,
    capacity: u64,
    busy: bool,
}

#[derive(Default)]
struct ReadbackRingState {
    slots: Vec<ReadbackSlot>,
    cursor: usize,
}

pub(crate) struct ReadbackStagingRing {
    state: Mutex<ReadbackRingState>,
}

struct ReadbackSlotGuard<'a> {
    ring: &'a ReadbackStagingRing,
    index: usize,
}

impl Drop for ReadbackSlotGuard<'_> {
    fn drop(&mut self) {
        self.ring.release(self.index);
    }
}

impl ReadbackStagingRing {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(ReadbackRingState::default()),
        }
    }

    fn acquire(&self, device: &wgpu::Device, size: u64) -> Result<(usize, wgpu::Buffer), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Readback staging ring lock is poisoned".to_owned())?;
        let slot_count = state.slots.len();
        for offset in 0..slot_count {
            let index = (state.cursor + offset) % slot_count;
            let slot = &mut state.slots[index];
            if slot.busy {
                continue;
            }
            if slot.capacity < size {
                slot.buffer = create_readback_buffer(device, size, index);
                slot.capacity = size;
            }
            slot.busy = true;
            let buffer = slot.buffer.clone();
            state.cursor = (index + 1) % READBACK_RING_SIZE;
            return Ok((index, buffer));
        }
        if state.slots.len() >= READBACK_RING_SIZE {
            return Err("Readback staging ring is saturated".to_owned());
        }
        let index = state.slots.len();
        let buffer = create_readback_buffer(device, size, index);
        state.slots.push(ReadbackSlot {
            buffer: buffer.clone(),
            capacity: size,
            busy: true,
        });
        state.cursor = (index + 1) % READBACK_RING_SIZE;
        Ok((index, buffer))
    }

    fn release(&self, index: usize) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(slot) = state.slots.get_mut(index) {
                slot.busy = false;
            }
        }
    }
}

fn create_readback_buffer(device: &wgpu::Device, size: u64, index: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("open-quartz-readback-{index}")),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    })
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
        let (slot_index, readback) = self.readback_ring.acquire(&self.device, buffer_size)?;
        let _slot_guard = ReadbackSlotGuard {
            ring: &self.readback_ring,
            index: slot_index,
        };
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

        let slice = readback.slice(..buffer_size);
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
