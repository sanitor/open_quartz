use open_quartz::outputs::FrameSnapshotIdentity;
use open_quartz::rendering::{GpuBackend, GpuPreviewImage, GpuPreviewReader, StableFrame};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 112;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Off,
    RgbHistogram,
    LumaWaveform,
}

impl Mode {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::RgbHistogram,
            2 => Self::LumaWaveform,
            _ => Self::Off,
        }
    }
}

#[derive(Default)]
pub struct ColorScope {
    mode: Mode,
    generation: u64,
    last_frame: Option<FrameSnapshotIdentity>,
    last_sample: Option<Instant>,
    worker: Option<Worker>,
    busy: bool,
}

struct Worker {
    send: mpsc::SyncSender<(u64, Mode, StableFrame)>,
    receive: mpsc::Receiver<(u64, Result<Plot, String>)>,
}

impl Worker {
    fn start(backend: Arc<GpuBackend>) -> Self {
        let (send, jobs) = mpsc::sync_channel::<(u64, Mode, StableFrame)>(1);
        let (results, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = GpuPreviewReader::new(backend);
            while let Ok((generation, mode, frame)) = jobs.recv() {
                let image = pollster::block_on(reader.read(frame.output(), WIDTH));
                if results.send((generation, image.map(|image| plot(mode, &image)))).is_err() { break; }
            }
        });
        Self { send, receive }
    }
}

pub struct Plot {
    pub pixels: Vec<u8>,
    pub summary: String,
}

impl Plot {
    pub fn image(&self) -> slint::Image {
        slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&self.pixels, WIDTH, HEIGHT))
    }
}

impl ColorScope {
    pub fn select(&mut self, mode: Mode) {
        self.mode = mode;
        self.clear();
    }

    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.last_frame = None;
        self.last_sample = None;
    }

    pub fn update(&mut self, backend: &Arc<GpuBackend>, frame: Option<&StableFrame>) -> Option<Result<Plot, String>> {
        let mut result = None;
        if let Some(worker) = &self.worker {
            match worker.receive.try_recv() {
                Ok((generation, plot)) => {
                    self.busy = false;
                    if generation == self.generation { result = Some(plot); }
                }
                Err(mpsc::TryRecvError::Empty) if self.busy => return None,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.busy = false;
                    result = Some(Err("Color scope worker stopped".into()));
                }
            }
        }
        if let Some(frame) = frame {
            let now = Instant::now();
            if self.mode != Mode::Off
                && self.last_frame.as_ref() != Some(&frame.identity)
                && self.last_sample.is_none_or(|then| now.duration_since(then) >= Duration::from_millis(125))
            {
                self.last_sample = Some(now);
                self.last_frame = Some(frame.identity.clone());
                let worker = self.worker.get_or_insert_with(|| Worker::start(backend.clone()));
                self.busy = worker.send.try_send((self.generation, self.mode, frame.clone())).is_ok();
            }
        }
        result
    }
}

fn plot(mode: Mode, image: &GpuPreviewImage) -> Plot {
    let mut pixels = vec![0; WIDTH as usize * HEIGHT as usize * 4];
    for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
        let grid = index / WIDTH as usize % 28 == 0;
        pixel.copy_from_slice(if grid { &[36, 42, 44, 255] } else { &[12, 16, 18, 255] });
    }
    let summary = match mode {
        Mode::RgbHistogram => histogram(image, &mut pixels),
        Mode::LumaWaveform => waveform(image, &mut pixels),
        Mode::Off => String::new(),
    };
    Plot { pixels, summary }
}

fn histogram(image: &GpuPreviewImage, plot: &mut [u8]) -> String {
    let mut bins = [[0u32; WIDTH as usize]; 3];
    for pixel in image.rgba.chunks_exact(4) {
        for (channel, bins) in bins.iter_mut().enumerate() {
            bins[pixel[channel] as usize] += 1;
        }
    }
    let max = bins.iter().flatten().copied().max().unwrap_or(1).max(1);
    let peaks = bins.each_ref().map(|channel| channel.iter().enumerate()
        .max_by_key(|(_, frequency)| *frequency).map_or(0, |(level, _)| level));
    for (channel, bins) in bins.iter().enumerate() {
        for (x, count) in bins.iter().enumerate() {
            let size = (*count as f32 / max as f32).sqrt() * HEIGHT as f32;
            for y in HEIGHT - size.ceil() as u32..HEIGHT {
                let at = (y as usize * WIDTH as usize + x) * 4;
                plot[at + channel] = 238;
            }
        }
    }
    format!("RGB peak levels · R {} · G {} · B {}", peaks[0], peaks[1], peaks[2])
}

fn waveform(image: &GpuPreviewImage, plot: &mut [u8]) -> String {
    let (mut min, mut max) = (255, 0);
    for (index, pixel) in image.rgba.chunks_exact(4).enumerate() {
        let y = ((2126 * u32::from(pixel[0]) + 7152 * u32::from(pixel[1]) + 722 * u32::from(pixel[2]) + 5000) / 10_000) as u8;
        min = min.min(y);
        max = max.max(y);
        let x = index as u32 % image.width * WIDTH / image.width;
        let row = HEIGHT - 1 - u32::from(y) * (HEIGHT - 1) / 255;
        let at = (row * WIDTH + x) as usize * 4;
        plot[at..at + 3].copy_from_slice(&[71, 221, 187]);
    }
    format!("Luma levels · {}–{}", min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_reports_dominant_codes_and_rec709_luma_of_the_current_frame() {
        let image = GpuPreviewImage {
            rgba: [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255].into(),
            width: 3, height: 1, scale_submit: Duration::ZERO, readback: Duration::ZERO,
        };
        assert_eq!(plot(Mode::RgbHistogram, &image).summary, "RGB peak levels · R 255 · G 0 · B 0");
        assert_eq!(plot(Mode::LumaWaveform, &image).summary, "Luma levels · 18–54");
    }
}
