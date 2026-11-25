use super::*;
use crate::document::{MotionChannel, MotionPoint};

#[derive(Default)]
pub(super) struct MotionEditor {
    open: bool,
    channel: MotionChannel,
    clip: Option<String>,
    revision: u64,
    selection: BTreeSet<usize>,
    clipboard: Option<(MotionChannel, Vec<(u64, MotionPoint)>)>,
}

impl Controller {
    pub(super) fn refresh_motion(&self) {
        let Some(window) = self.window.upgrade() else { return; };
        let Some(clip) = self.selected_clip.as_deref().and_then(|id| self.document.clip(id))
            .filter(|_| self.motion.open) else {
            window.set_motion_open(false);
            return;
        };
        let scoped = self.motion.clip.as_deref() == Some(clip.id.as_str()) && self.motion.revision == self.document.revision;
        let rows: Vec<_> = clip.motion_keys(self.motion.channel).into_iter().enumerate().filter_map(|(index, key)| {
            let frame = key.frame(&self.document.data, clip)?;
            Some(crate::MotionKeyRow {
                index: index as i32,
                frame: frame.min(i32::MAX as u64) as i32,
                selected: scoped && self.motion.selection.contains(&index),
            })
        }).collect();
        if window.get_motion_keys().iter().ne(rows.iter().cloned()) {
            window.set_motion_keys(ModelRc::new(VecModel::from(rows)));
        }
        window.set_motion_start(clip.timeline_start.min(i32::MAX as u64) as i32);
        window.set_motion_duration(clip.duration.min(i32::MAX as u64) as i32);
        window.set_motion_open(true);
        window.set_motion_has_selection(scoped && !self.motion.selection.is_empty());
        window.set_motion_can_paste(self.motion.clipboard.as_ref().is_some_and(|(channel, _)| *channel == self.motion.channel));
        window.set_motion_writable(self.document.clip_track(&clip.id).is_some_and(|track| !track.locked));
    }

    pub(super) fn motion_action(&mut self, action: &str, index: i32, delta: i32, extend: bool) {
        if action == "toggle" {
            self.motion.open = !self.motion.open;
            self.refresh_motion();
            return;
        }
        if action == "channel" {
            if let Some(channel) = MotionChannel::from_index(index) {
                self.motion.channel = channel;
                self.motion.selection.clear();
                self.refresh_motion();
            }
            return;
        }
        let Some(id) = self.selected_clip.clone().filter(|_| self.motion.open) else { return; };
        let revision = self.document.revision;
        if self.motion.clip.as_ref() != Some(&id) || self.motion.revision != revision {
            self.motion.clip = Some(id.clone());
            self.motion.revision = revision;
            self.motion.selection.clear();
        }
        let index = usize::try_from(index).ok();
        if action == "select" {
            if let Some(index) = index {
                if !extend { self.motion.selection.clear(); }
                if !self.motion.selection.insert(index) {
                    self.motion.selection.remove(&index);
                }
                self.refresh_motion();
            }
            return;
        }

        let channel = self.motion.channel;
        if action == "copy" {
            if let Some(clip) = self.document.clip(&id) {
                let mut points: Vec<_> = clip.motion_keys(channel).into_iter().enumerate()
                    .filter(|(index, _)| self.motion.selection.contains(index))
                    .filter_map(|(_, key)| Some((key.frame(&self.document.data, clip)?, key))).collect();
                points.sort_by_key(|(frame, _)| *frame);
                if let Some(&(start, _)) = points.first() {
                    points.iter_mut().for_each(|(frame, _)| *frame -= start);
                    self.motion.clipboard = Some((channel, points));
                    self.refresh_motion();
                }
            }
            return;
        }

        self.pause();
        let selected = &self.motion.selection;
        let playhead = self.playhead;
        let clipboard = self.motion.clipboard.as_ref().filter(|(copied_channel, _)| *copied_channel == channel);
        let result = self.document.edit_motion_keys(&id, channel, revision, |data, clip, points| {
            match action {
                "move" => {
                    let index = index.ok_or("Select a keyframe to move")?;
                    let use_selection = selected.contains(&index);
                    for (at, point) in points.iter_mut().enumerate() {
                        if at == index || (use_selection && selected.contains(&at)) {
                            let origin = point.frame(data, clip).ok_or("Keyframe is outside this clip")?;
                            let frame = origin.checked_add_signed(i64::from(delta))
                                .ok_or("Keyframes must stay inside the selected clip")?;
                            point.set_time(data, clip, frame)?;
                        }
                    }
                }
                "delete" => {
                    let mut index = 0;
                    points.retain(|_| {
                        let keep = !selected.contains(&index);
                        index += 1;
                        keep
                    });
                }
                "paste" => {
                    let (_, copied) = clipboard.ok_or("Copy keys for this property first")?;
                    for &(offset, mut point) in copied {
                        let frame = playhead.checked_add(offset).ok_or("Pasted keyframe exceeds the clip range")?;
                        point.set_time(data, clip, frame)?;
                        points.push(point);
                    }
                }
                _ => return Err("Unknown keyframe action".into()),
            }
            Ok(())
        });
        self.finish_edit(result);
        self.refresh_motion();
    }
}
