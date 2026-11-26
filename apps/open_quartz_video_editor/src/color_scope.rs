use open_quartz::outputs::FrameSnapshotIdentity;
use open_quartz::rendering::{GpuBackend, GpuPreviewImage, GpuPreviewReader};
use crate::hdr_monitor::DisplayFrame;
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
    RgbParade,
    Vectorscope,
}

impl Mode {
    pub fn from_index(index: i32) -> Self {
        match index {
            1 => Self::RgbHistogram,
            2 => Self::LumaWaveform,
            3 => Self::RgbParade,
            4 => Self::Vectorscope,
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
    send: mpsc::SyncSender<(u64, Mode, DisplayFrame)>,
    receive: mpsc::Receiver<(u64, Result<Plot, String>)>,
}

impl Worker {
    fn start(backend: Arc<GpuBackend>) -> Self {
        let (send, jobs) = mpsc::sync_channel::<(u64, Mode, DisplayFrame)>(1);
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

    pub fn update(&mut self, backend: &Arc<GpuBackend>, frame: Option<&DisplayFrame>) -> Option<Result<Plot, String>> {
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
        Mode::RgbParade => parade(image, &mut pixels),
        Mode::Vectorscope => vectorscope(image, &mut pixels),
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

fn parade(image: &GpuPreviewImage, plot: &mut [u8]) -> String {
    let mut minima = [255; 3];
    let mut maxima = [0; 3];
    let panel = WIDTH / 3;
    for (index, pixel) in image.rgba.chunks_exact(4).enumerate() {
        let x = index as u32 % image.width * panel / image.width;
        for channel in 0..3 {
            let level = pixel[channel];
            minima[channel] = minima[channel].min(level);
            maxima[channel] = maxima[channel].max(level);
            let row = HEIGHT - 1 - u32::from(level) * (HEIGHT - 1) / 255;
            let at = ((row * WIDTH + x + channel as u32 * panel) * 4) as usize;
            plot[at..at + 3].copy_from_slice(&[42; 3]);
            plot[at + channel] = 247;
        }
    }
    format!("RGB parade levels · R {}–{} · G {}–{} · B {}–{}",
        minima[0], maxima[0], minima[1], maxima[1], minima[2], maxima[2])
}

fn vectorscope(image: &GpuPreviewImage, plot: &mut [u8]) -> String {
    let cx = WIDTH as i32 / 2;
    let cy = HEIGHT as i32 / 2;
    let radius = cy - 4;
    for y in -radius..=radius {
        for x in -radius..=radius {
            if x == 0 || y == 0 || ((x * x + y * y) - radius * radius).abs() <= radius {
                let at = ((cy + y) * WIDTH as i32 + cx + x) as usize * 4;
                plot[at..at + 3].copy_from_slice(&[47, 60, 61]);
            }
        }
    }
    let (mut cb_min, mut cb_max, mut cr_min, mut cr_max) = (127, -128, 127, -128);
    for pixel in image.rgba.chunks_exact(4) {
        let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(f32::from);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let cb = ((b - luma) / 1.8556).round().clamp(-128.0, 127.0) as i32;
        let cr = ((r - luma) / 1.5748).round().clamp(-128.0, 127.0) as i32;
        cb_min = cb_min.min(cb); cb_max = cb_max.max(cb);
        cr_min = cr_min.min(cr); cr_max = cr_max.max(cr);
        let x = cx + cb * radius / 128;
        let y = cy - cr * radius / 128;
        let at = (y * WIDTH as i32 + x) as usize * 4;
        plot[at..at + 3].copy_from_slice(&[89, 242, 191]);
    }
    format!("Vectorscope · Cb {cb_min}–{cb_max} · Cr {cr_min}–{cr_max}")
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
        assert_eq!(plot(Mode::RgbParade, &image).summary, "RGB parade levels · R 0–255 · G 0–0 · B 0–255");
        assert_eq!(plot(Mode::Vectorscope, &image).summary, "Vectorscope · Cb -29–127 · Cr -12–127");
    }
}
