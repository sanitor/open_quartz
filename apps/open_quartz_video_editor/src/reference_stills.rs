use super::*;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use image::ImageEncoder;
use open_quartz::rendering::GpuPreviewReader;
use std::io::Cursor;
use crate::document::{ReferenceStill, MAX_REFERENCES};

const REFERENCE_EDGE: u32 = 384;

impl ReferenceStill {
    fn decode(&self) -> Result<slint::Image, String> {
        let bytes = STANDARD.decode(self.png.as_ref()).map_err(|error| format!("Invalid still data: {error}"))?;
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(REFERENCE_EDGE);
        limits.max_image_height = Some(REFERENCE_EDGE);
        limits.max_alloc = Some(4 * 1024 * 1024);
        reader.limits(limits);
        let pixels = reader.decode().map_err(|error| format!("Invalid reference PNG: {error}"))?.into_rgba8();
        Ok(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&pixels, pixels.width(), pixels.height())))
    }
}

#[derive(Default)]
pub(super) struct ReferenceGallery {
    selected: Option<u64>,
    viewing: bool,
    pending: Option<mpsc::Receiver<Result<ReferenceStill, String>>>,
    captured_revision: u64,
}

impl Controller {
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
            _ => {}
        }
        self.publish_references();
    }

    pub(super) fn poll_references(&mut self) {
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
