use super::*;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use image::ImageEncoder;
use open_quartz::rendering::GpuPreviewReader;
use std::io::Cursor;
use crate::document::{ReferenceStill, MAX_REFERENCES};

const REFERENCE_EDGE: u32 = 384;

impl ReferenceStill {
    fn pixels(&self) -> Result<image::RgbaImage, String> {
        let bytes = STANDARD.decode(self.png.as_ref()).map_err(|error| format!("Invalid still data: {error}"))?;
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(REFERENCE_EDGE);
        limits.max_image_height = Some(REFERENCE_EDGE);
        limits.max_alloc = Some(4 * 1024 * 1024);
        reader.limits(limits);
        Ok(reader.decode().map_err(|error| format!("Invalid reference PNG: {error}"))?.into_rgba8())
    }

    fn decode(&self) -> Result<slint::Image, String> {
        let pixels = self.pixels()?;
        Ok(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&pixels, pixels.width(), pixels.height())))
    }
}

#[derive(Default)]
pub(super) struct ReferenceGallery {
    selected: Option<u64>,
    viewing: bool,
    pending: Option<mpsc::Receiver<Result<ReferenceStill, String>>>,
    captured_revision: u64,
    pub(super) copied_grade: Option<(crate::clip_pipeline::ClipPipeline, Vec<document::EffectKeys>)>,
    matching: Option<ShotMatch>,
}

struct ShotMatch {
    revision: u64,
    clip: String,
    receive: mpsc::Receiver<Result<crate::clip_pipeline::ClipPipeline, String>>,
}

struct Midtones {
    luma: f32,
    rgb: [f32; 3],
}

fn midtones(rgba: &[u8]) -> Result<Midtones, String> {
    let linear = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    let mut values: Vec<_> = rgba.chunks_exact(4).filter(|pixel| pixel[3] >= 128)
        .map(|pixel| {
            let rgb = [linear(pixel[0]), linear(pixel[1]), linear(pixel[2])];
            (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2], rgb)
        })
        .filter(|(luma, _)| *luma > 0.002 && *luma < 0.98).collect();
    if values.len() < 16 { return Err("Shot match needs visible midtones in both frames".into()); }
    let middle = values.len() / 2;
    let luma = values.select_nth_unstable_by(middle, |a, b| a.0.total_cmp(&b.0)).1.0;
    let mut rgb = [0.0; 3];
    let mut count = 0.0;
    for (_, sample) in values.iter().filter(|(lightness, _)| (luma / 2.0..=luma * 2.0).contains(lightness)) {
        for channel in 0..3 { rgb[channel] += sample[channel]; }
        count += 1.0;
    }
    let [r, g, b] = rgb.map(|value| value / count);
    // Linear sRGB to ACEScg, the gain stage's working space.
    let rgb = [
        0.6131289 * r + 0.3395173 * g + 0.0473538 * b,
        0.07010676 * r + 0.91636576 * g + 0.01352747 * b,
        0.02058661 * r + 0.10956772 * g + 0.8698457 * b,
    ];
    Ok(Midtones { luma, rgb })
}

fn match_grade(mut pipeline: crate::clip_pipeline::ClipPipeline, source: Midtones, target: Midtones)
    -> Result<crate::clip_pipeline::ClipPipeline, String> {
    use crate::clip_pipeline::OUTPUT_ID;
    use open_quartz::project::{GraphCommand, NodeId};
    let id = pipeline.add_corrector_serial(OUTPUT_ID)?;
    let mut data = pipeline.graph().nodes.iter().find(|node| node.id == id)
        .ok_or("Shot match adjustment is missing")?.data.clone();
    let primaries = &mut data.color_corrector.as_mut().ok_or("Shot match correction is missing")?.primaries;
    primaries.exposure = (target.luma / source.luma).log2().clamp(-3.0, 3.0);
    for channel in 0..3 {
        let relative_source = source.rgb[channel] / source.luma;
        let relative_target = target.rgb[channel] / target.luma;
        primaries.gain[channel] = (relative_target / relative_source.max(0.01)).clamp(0.67, 1.5);
    }
    pipeline.apply_command(GraphCommand::UpdateNodeData { node_id: NodeId::new(&id), data })?;
    Ok(pipeline)
}

impl Controller {
    fn start_shot_match(&mut self) -> Result<(), String> {
        if self.playing || self.asset_preview || self.references.viewing || self.hdr_monitor_open
            || self.displayed_ticket != Some(self.ticket) || self.displayed_frame != Some(self.playhead)
            || self.pipeline_has_drafts()
            || self.window.upgrade().is_some_and(|window| window.get_multicam_open() || window.get_pipeline_compare_active())
        {
            return Err("Pause on a current SDR program frame without comparison or node drafts".into());
        }
        if self.references.matching.is_some() { return Err("Shot match is already running".into()); }
        let (track, clip) = self.selected_clip.as_deref().and_then(|id| {
            let mut inputs = self.document.data.video_inputs(self.playhead);
            let input = inputs.next()?;
            (inputs.next().is_none() && input.clip.id == id).then_some((input.track, input.clip))
        }).ok_or("Select the only visible video clip at the playhead")?;
        if track.locked || clip.title.is_some() { return Err("Select an unlocked picture clip".into()); }
        let reference = self.references.selected.and_then(|id| self.document.data.reference_stills
            .iter().find(|still| still.id == id)).ok_or("Select a reference still")?.clone();
        let frame = self.displayed.clone().ok_or("Wait for the current frame to render")?;
        let backend = self.backend.clone();
        let pipeline = clip.pipeline.clone();
        let (send, receive) = mpsc::channel();
        let worker = std::thread::Builder::new().name("oq-shot-match".into()).spawn(move || {
            let result = (|| {
                let target = reference.pixels()?;
                let mut reader = GpuPreviewReader::new(backend);
                let source = pollster::block_on(reader.read(frame.output(), REFERENCE_EDGE))?;
                match_grade(pipeline, midtones(&source.rgba)?, midtones(&target)?)
            })();
            let _ = send.send(result);
        });
        worker.map_err(|error| error.to_string())?;
        self.references.matching = Some(ShotMatch {
            revision: self.document.revision, clip: clip.id.clone(), receive,
        });
        self.status("Matching shot…");
        Ok(())
    }

    pub(super) fn reference_action(&mut self, action: &str, index: i32) {
        match action {
            "grab" => {
                if self.playing || self.references.viewing || self.displayed_ticket != Some(self.ticket) || self.asset_preview {
                    self.status("Pause on a current timeline frame to grab a reference still");
                    return;
                }
                if self.references.pending.is_some() || self.document.data.reference_stills.len() >= MAX_REFERENCES {
                    self.status("The reference gallery holds up to eight stills");
                    return;
                }
                let Some(frame) = self.displayed.clone() else { return; };
                let Some(number) = self.displayed_frame else { return; };
                let next = self.document.data.reference_stills.iter().map(|still| still.id).max().unwrap_or(0);
                let Some(id) = next.checked_add(1) else {
                    self.status("No more reference still IDs are available");
                    return;
                };
                let label = format!("Still {id} · Frame {number}");
                let backend = self.backend.clone();
                let (send, receive) = mpsc::channel();
                self.references.pending = Some(receive);
                self.references.captured_revision = self.document.revision;
                self.status("Capturing the reference still…");
                std::thread::spawn(move || {
                    let result = (|| {
                        let mut reader = GpuPreviewReader::new(backend);
                        let pixels = pollster::block_on(reader.read(frame.output(), REFERENCE_EDGE))?;
                        let mut png = Vec::new();
                        image::codecs::png::PngEncoder::new(&mut png)
                            .write_image(&pixels.rgba, pixels.width, pixels.height, image::ExtendedColorType::Rgba8)
                            .map_err(|error| error.to_string())?;
                        Ok(ReferenceStill { id, label, png: STANDARD.encode(png).into() })
                    })();
                    let _ = send.send(result);
                });
            }
            "select" => {
                self.references.selected = usize::try_from(index).ok()
                    .and_then(|index| self.document.data.reference_stills.get(index)).map(|still| still.id);
                self.publish_reference_image();
            }
            "view" => {
                self.references.viewing = !self.references.viewing;
                if self.references.viewing { self.pause(); }
                self.publish_reference_image();
            }
            "remove" => {
                let Some(id) = self.references.selected else { return; };
                match self.document.transact_at(self.document.revision, |data| {
                    data.reference_stills.retain(|still| still.id != id);
                    Ok(())
                }) {
                    Ok(()) => { self.references.viewing = false; self.changed(); }
                    Err(error) => self.status(error),
                }
            }
            "match" => {
                if let Err(error) = self.start_shot_match() { self.status(error); }
            }
            _ => {}
        }
        self.publish_references();
    }

    pub(super) fn poll_references(&mut self) {
        if let Some(task) = &self.references.matching {
            let result = match task.receive.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("Shot matching stopped".into())),
            };
            if let Some(result) = result {
                let task = self.references.matching.take().unwrap();
                match result.and_then(|pipeline| self.document.set_clip_pipeline(&task.clip, pipeline, task.revision)) {
                    Ok(()) => { self.pipeline_editor.reset(); self.changed(); self.status("Shot grade matched"); }
                    Err(error) => self.status(error),
                }
            }
        }
        let Some(pending) = &self.references.pending else { return; };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Reference capture stopped".into()),
        };
        self.references.pending = None;
        match result.and_then(|still| {
            let id = still.id;
            self.document.transact_at(self.references.captured_revision, |data| {
                data.reference_stills.push(still);
                Ok(())
            })?;
            Ok(id)
        }) {
            Ok(id) => {
                self.references.selected = Some(id);
                self.changed();
                self.status("Reference still captured");
            }
            Err(error) => self.status(error),
        }
        self.publish_references();
    }

    pub(super) fn hide_reference(&mut self) {
        self.references.viewing = false;
        if let Some(window) = self.window.upgrade() { window.set_reference_viewing(false); }
    }

    pub(super) fn publish_references(&mut self) {
        let Some(window) = self.window.upgrade() else { return; };
        let stills = &self.document.data.reference_stills;
        if !stills.iter().any(|still| Some(still.id) == self.references.selected) {
            self.references.selected = stills.first().map(|still| still.id);
            self.references.viewing = false;
        }
        window.set_reference_names(ModelRc::new(VecModel::from(
            stills.iter().map(|still| slint::SharedString::from(&still.label)).collect::<Vec<_>>(),
        )));
        window.set_reference_index(stills.iter().position(|still| Some(still.id) == self.references.selected)
            .map_or(-1, |index| index as i32));
        window.set_reference_capture_busy(self.references.pending.is_some());
        if !self.references.viewing { window.set_reference_viewing(false); }
    }

    fn publish_reference_image(&mut self) {
        let Some(window) = self.window.upgrade() else { return; };
        if !self.references.viewing {
            window.set_reference_viewing(false);
            return;
        }
        let image = self.references.selected.and_then(|id| self.document.data.reference_stills.iter().find(|still| still.id == id))
            .map(ReferenceStill::decode);
        if self.references.viewing {
            match image {
                Some(Ok(image)) => window.set_reference_image(image),
                Some(Err(error)) => { self.references.viewing = false; self.status(error); }
                None => self.references.viewing = false,
            }
        }
        window.set_reference_viewing(self.references.viewing);
    }
}
