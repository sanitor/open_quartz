use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct MaskTask {
    revision: u64,
    clip: String,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<Result<Vec<document::MaskPosition>, String>>,
}

impl Controller {
    pub(super) fn start_mask_tracking(&mut self, similarity: bool) {
        if self.mask_tracking.is_some() {
            return;
        }
        let result = self.make_mask_tracking_task(similarity);
        match result {
            Ok(task) => {
                self.pause();
                self.mask_tracking = Some(task);
                if let Some(window) = self.window.upgrade() {
                    window.set_mask_tracking_busy(true);
                }
                self.status("Tracking mask forward…");
            }
            Err(error) => self.status(error),
        }
    }

    fn make_mask_tracking_task(&self, similarity: bool) -> Result<MaskTask, String> {
        let id = self
            .selected_clip
            .as_deref()
            .ok_or("Select a video clip with a mask")?;
        if self
            .document
            .clip_track(id)
            .is_some_and(|track| track.locked)
        {
            return Err("Unlock the track before following a mask".into());
        }
        let clip = self
            .document
            .clip(id)
            .ok_or("Select a video clip with a mask")?;
        let mask = clip.mask.ok_or("Choose a mask shape before tracking")?;
        let asset = self
            .document
            .asset(&clip.asset_id)
            .filter(|asset| asset.online && asset.has_video() && asset.sequence_source().is_none())
            .cloned()
            .ok_or("Mask tracking requires online video")?;
        let start = self.playhead;
        if start < clip.timeline_start {
            return Err("Move the playhead onto the selected clip".into());
        }
        let end = start
            .saturating_add(document::MAX_TRACKED_FRAMES)
            .min(clip.timeline_end()?);
        let mut times = (start..end)
            .map(|frame| self.document.data.clip_source_time(clip, frame))
            .collect::<Result<Vec<_>, _>>()?;
        times.dedup();
        if times.len() < 2 {
            return Err("Mask tracking needs two or more video frames".into());
        }
        let render = self.preview.offscreen_device()?;
        let fps = self.document.data.fps;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_flag = cancel.clone();
        let (send, receive) = mpsc::channel();
        std::thread::Builder::new()
            .name("oq-editor-mask-tracking".into())
            .spawn(move || {
                let result = crate::preview::mask_tracking::track_mask(
                    render,
                    asset,
                    fps,
                    mask,
                    &times,
                    similarity,
                    &worker_flag,
                );
                let _ = send.send(result);
            })
            .map_err(|error| error.to_string())?;
        Ok(MaskTask {
            revision: self.document.revision,
            clip: id.to_owned(),
            cancel,
            receive,
        })
    }

    pub(super) fn cancel_mask_tracking(&mut self) {
        if let Some(task) = &self.mask_tracking {
            task.cancel.store(true, Ordering::Release);
        }
    }

    pub(super) fn reset_mask_tracking(&mut self) {
        self.cancel_mask_tracking();
        self.mask_tracking = None;
        if let Some(window) = self.window.upgrade() {
            window.set_mask_tracking_busy(false);
        }
    }

    pub(super) fn poll_mask_tracking(&mut self) {
        let Some(task) = &self.mask_tracking else {
            return;
        };
        let result = match task.receive.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Mask tracking worker stopped".into()),
        };
        let task = self
            .mask_tracking
            .take()
            .expect("a tracking result has a task");
        if let Some(window) = self.window.upgrade() {
            window.set_mask_tracking_busy(false);
        }
        if task.cancel.load(Ordering::Acquire) {
            self.status("Mask tracking cancelled");
            return;
        }
        let result = result.and_then(|points| {
            self.document
                .set_mask_tracking(&task.clip, task.revision, points)
        });
        match result {
            Ok(()) => {
                self.changed();
                self.status("Mask tracking complete");
            }
            Err(error) => self.status(error),
        }
    }
}
