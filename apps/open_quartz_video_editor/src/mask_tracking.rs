use super::*;
use open_quartz::rendering::{GpuPreviewImage, GpuPreviewReader};

const TRACK_EDGE: u32 = 128;
const SEARCH: i32 = 10;

pub(crate) fn track_mask(
    render: RenderDevice,
    asset: MediaAsset,
    fps: Rational,
    mask: document::Mask,
    source_times: &[u64],
    similarity: bool,
    cancel: &AtomicBool,
) -> Result<Vec<document::MaskPosition>, String> {
    let mut pipeline =
        FramePipeline::from_render_device(render, PlaybackCapabilities { data_paths: vec![] });
    let mut reader = GpuPreviewReader::new(pipeline.backend());
    let owner = thread::current();
    pipeline.set_waker(Arc::new(move || owner.unpark()))?;
    let longest = asset.width.max(asset.height).max(1);
    let dimension = |pixels: u32| {
        ((u64::from(pixels) * u64::from(TRACK_EDGE) / u64::from(longest)) as u32)
            .max(2)
            .next_multiple_of(2)
    };
    let (width, height) = (dimension(asset.width), dimension(asset.height));
    let mut request = asset_request(asset, 0, 0, 0, width, height);
    request.source_ns = *source_times
        .first()
        .ok_or("No frames are available for mask tracking")?;
    playback::configure_limits(&mut pipeline, &request, fps)?;
    configure(&mut pipeline, &mut None, &request)?;
    let mut trail = Vec::with_capacity(source_times.len());
    let mut tracker = None;
    for (index, &time) in source_times.iter().enumerate() {
        if cancel.load(Ordering::Acquire) {
            return Err("Mask tracking cancelled".into());
        }
        let frame = tracking_frame(&mut pipeline, width, height, index as u64 + 1, time, cancel)?;
        let pixels = pollster::block_on(reader.read(frame.output(), TRACK_EDGE))?;
        let mut gray = Vec::with_capacity(pixels.rgba.len() / 4);
        for pixel in pixels.rgba.chunks_exact(4) {
            gray.push(
                ((54 * i32::from(pixel[0]) + 183 * i32::from(pixel[1]) + 19 * i32::from(pixel[2]))
                    / 256) as u8,
            );
        }
        let (x, y, scale, rotation) = match &mut tracker {
            Some(template) if similarity => Template::locate_similarity(template, &pixels, &gray)?,
            Some(template) => {
                let (x, y) = Template::locate(template, &pixels, &gray)?;
                (x, y, 1.0, 0.0)
            }
            None => {
                tracker = Some(Template::new(mask, &pixels, &gray)?);
                (mask.x, mask.y, 1.0, 0.0)
            }
        };
        trail.push(document::MaskPosition {
            source_ns: time,
            x,
            y,
            scale,
            rotation,
        });
    }
    Ok(trail)
}

fn tracking_frame(
    pipeline: &mut FramePipeline,
    width: u32,
    height: u32,
    request: u64,
    time: u64,
    cancel: &AtomicBool,
) -> Result<StableFrame, String> {
    let times = BTreeMap::from([(SOURCE_ID.to_owned(), time)]);
    let demand = FrameDemand {
        request_id: request,
        graph_revision: pipeline.revision(),
        output_node_id: OUTPUT_ID.into(),
        sources: Vec::new(),
        input: FrameEvaluationInput {
            stamp: FrameStamp {
                epoch: 1,
                frame: request,
                timeline_ns: time,
                deadline_ns: 0,
            },
            previous_timeline_ns: time,
            date: [0.0; 4],
            mouse: [0.0; 4],
            resolution: [width as f32, height as f32, 1.0],
        },
    };
    let until = Instant::now() + Duration::from_secs(4);
    let mut pending = Some(demand);
    while Instant::now() < until && !cancel.load(Ordering::Acquire) {
        if let Some(demand) = pending.take() {
            match pipeline.submit_window_with_source_times(demand, &times) {
                FrameAdmission::Accepted => {}
                FrameAdmission::WouldBlock(demand) => pending = Some(demand),
                FrameAdmission::Rejected { error, .. } => return Err(error),
            }
        }
        pipeline.progress();
        if let Some(completion) = pipeline.poll_completion() {
            return match completion.outcome {
                FrameOutcome::Submitted(frame) => Ok(frame),
                FrameOutcome::Failed(error) => Err(error),
                FrameOutcome::Cancelled => Err("Mask tracking cancelled".into()),
            };
        }
        thread::park_timeout(Duration::from_millis(2));
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Mask tracking cancelled".into());
    }
    Err("Timed out decoding a frame for mask tracking".into())
}

struct Template {
    x: i32,
    y: i32,
    half_width: i32,
    half_height: i32,
    samples: Vec<u8>,
    scale: f32,
    rotation: f32,
}

impl Template {
    fn new(mask: document::Mask, image: &GpuPreviewImage, gray: &[u8]) -> Result<Self, String> {
        let half_width = (mask.width * image.width as f32 / 2.0)
            .round()
            .clamp(2.0, 20.0) as i32;
        let half_height = (mask.height * image.height as f32 / 2.0)
            .round()
            .clamp(2.0, 20.0) as i32;
        let (w, h) = (image.width as i32, image.height as i32);
        if w <= 2 * half_width || h <= 2 * half_height {
            return Err("The tracking region is too small".into());
        }
        let x = (mask.x * w as f32).round() as i32;
        let y = (mask.y * h as f32).round() as i32;
        if !(half_width..w - half_width).contains(&x)
            || !(half_height..h - half_height).contains(&y)
        {
            return Err("Move the tracking region away from the frame edge".into());
        }
        let samples = patch(gray, w, x, y, half_width, half_height).collect::<Vec<_>>();
        let contrast = samples.iter().max().copied().unwrap_or_default()
            - samples.iter().min().copied().unwrap_or_default();
        if contrast < 10 {
            return Err("The tracking region needs more visible contrast".into());
        }
        Ok(Self {
            x,
            y,
            half_width,
            half_height,
            samples,
            scale: 1.0,
            rotation: 0.0,
        })
    }

    fn locate(&mut self, image: &GpuPreviewImage, gray: &[u8]) -> Result<(f32, f32), String> {
        let (w, h) = (image.width as i32, image.height as i32);
        let mut best = (u32::MAX, i32::MAX, self.x, self.y);
        for y in (self.y - SEARCH).max(self.half_height)
            ..=(self.y + SEARCH).min(h - self.half_height - 1)
        {
            for x in (self.x - SEARCH).max(self.half_width)
                ..=(self.x + SEARCH).min(w - self.half_width - 1)
            {
                let distance = (x - self.x).pow(2) + (y - self.y).pow(2);
                let error = patch(gray, w, x, y, self.half_width, self.half_height)
                    .zip(&self.samples)
                    .map(|(value, baseline)| u32::from(value.abs_diff(*baseline)))
                    .sum();
                best = best.min((error, distance, x, y));
            }
        }
        if best.0 > self.samples.len() as u32 * 32 {
            return Err("Mask tracker lost the reference pattern".into());
        }
        self.x = best.2;
        self.y = best.3;
        Ok((self.x as f32 / w as f32, self.y as f32 / h as f32))
    }

    fn locate_similarity(&mut self, image: &GpuPreviewImage, gray: &[u8]) -> Result<(f32, f32, f32, f32), String> {
        let (width, height) = (image.width as i32, image.height as i32);
        let mut pose = (self.x, self.y, self.scale, self.rotation);
        let score = |(x, y, scale, angle): (i32, i32, f32, f32)| -> u32 {
            let (sin, cos) = angle.to_radians().sin_cos();
            let mut error = 0;
            let pitch = (self.half_width * 2 + 1) as usize;
            for v in (-self.half_height..=self.half_height).step_by(2) {
                for u in (-self.half_width..=self.half_width).step_by(2) {
                    let px = x + (scale * (cos * u as f32 - sin * v as f32)).round() as i32;
                    let py = y + (scale * (sin * u as f32 + cos * v as f32)).round() as i32;
                    if !(0..width).contains(&px) || !(0..height).contains(&py) { return u32::MAX; }
                    let baseline = self.samples[(v + self.half_height) as usize * pitch + (u + self.half_width) as usize];
                    error += u32::from(gray[(py * width + px) as usize].abs_diff(baseline));
                }
            }
            error
        };
        let mut best = score(pose);
        for (radius, offset, scale_step, degrees) in [(SEARCH, 2, 0.08, 8.0), (2, 1, 0.04, 4.0)] {
            let (x, y, scale, angle) = pose;
            for dy in (-radius..=radius).step_by(offset) {
                for dx in (-radius..=radius).step_by(offset) {
                    for ds in [0.0, scale_step, -scale_step] {
                        for da in [0.0, degrees, -degrees] {
                            let candidate = (x + dx, y + dy, (scale + ds).clamp(0.5, 2.0), (angle + da).clamp(-360.0, 360.0));
                            let error = score(candidate);
                            if error < best { best = error; pose = candidate; }
                        }
                    }
                }
            }
        }
        if best > (self.half_width as u32 + 1) * (self.half_height as u32 + 1) * 32 {
            return Err("Mask tracker lost the reference pattern".into());
        }
        (self.x, self.y, self.scale, self.rotation) = pose;
        Ok((self.x as f32 / width as f32, self.y as f32 / height as f32, self.scale, self.rotation))
    }
}

fn patch(
    gray: &[u8],
    width: i32,
    x: i32,
    y: i32,
    hw: i32,
    hh: i32,
) -> impl Iterator<Item = u8> + '_ {
    (y - hh..=y + hh).flat_map(move |row| {
        let first = (row * width + x - hw) as usize;
        gray[first..first + (2 * hw + 1) as usize].iter().copied()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_patch_tracks_and_flat_template_is_rejected() {
        let image = GpuPreviewImage {
            rgba: Vec::new(),
            width: 50,
            height: 30,
            scale_submit: Duration::ZERO,
            readback: Duration::ZERO,
        };
        let sample = |offset: usize| {
            let mut gray = vec![0; 50 * 30];
            for y in 11..=19 {
                for x in 10..=20 {
                    gray[y * 50 + x + offset] = (x * 9) as u8;
                }
            }
            gray
        };
        let mask = document::Mask {
            x: 0.30,
            y: 0.50,
            width: 0.28,
            height: 0.4,
            ..Default::default()
        };
        let mut template = Template::new(mask, &image, &sample(0)).unwrap();
        assert_eq!(template.locate(&image, &sample(4)).unwrap(), (0.38, 0.5));
        assert!(Template::new(mask, &image, &vec![0; 1500]).is_err());
    }

    #[test]
    fn similarity_tracker_follows_rotation_and_scale() {
        let image = GpuPreviewImage {
            rgba: Vec::new(), width: 96, height: 80,
            scale_submit: Duration::ZERO, readback: Duration::ZERO,
        };
        let render = |cx: f32, cy: f32, scale: f32, angle: f32| {
            let (sin, cos) = angle.to_radians().sin_cos();
            (0..image.height).flat_map(|y| (0..image.width).map(move |x| {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let u = (dx * cos + dy * sin) / scale;
                let v = (-dx * sin + dy * cos) / scale;
                if u.abs() > 14.0 || v.abs() > 13.0 { return 5; }
                (110.0 + 60.0 * (u * 0.6 + v * 0.24).sin() + 40.0 * (v * 0.62).cos()) as u8
            })).collect::<Vec<_>>()
        };
        let mask = document::Mask { width: 0.35, height: 0.35, ..Default::default() };
        let mut tracker = Template::new(mask, &image, &render(48.0, 40.0, 1.0, 0.0)).unwrap();
        let pose = tracker.locate_similarity(&image, &render(50.0, 42.0, 1.12, 12.0)).unwrap();
        assert!((pose.0 * 96.0 - 50.0).abs() < 1.1 && (pose.1 * 80.0 - 42.0).abs() < 1.1, "{pose:?}");
        assert!((pose.2 - 1.12).abs() < 0.07 && (pose.3 - 12.0).abs() <= 4.1, "{pose:?}");
    }
}
