use crate::output_queue::{ExportSpec, OutputQueue, Phase as JobPhase};
use crate::ExportJobRow;
use crate::timeline_viewport::TimelineViewport;
use crate::delivery::ExportOptions;
use crate::document::{self, AudioDestination, ClipTransform, EditorDocument, MediaAsset, Placement, Rational};
use crate::preview::{asset_request, sequence_request, PlaybackRequest, PreviewBridge};
use crate::{AssetRow, AudioClipRow, AudioTrackRow, ClipRow, DissolveRow, EditorWindow, MarkerRow, SnapResult, TrackRow};
use open_quartz::rendering::{GpuBackend, GpuOutputHandle, StableFrame, TextureFormat};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;
#[path = "pipeline_editor.rs"]
mod pipeline_editor;
#[path = "monitor_transform.rs"]
mod monitor_transform;
#[path = "motion_editor.rs"]
mod motion_editor;
use motion_editor::MotionEditor;
use monitor_transform::MonitorGesture;
use pipeline_editor::{PipelineCommand, PipelineEditor};

#[derive(Clone, Copy)]
struct SourceMarks {
    fps: Rational,
    cursor: u64,
    start: Option<u64>,
    end: Option<u64>,
}

impl SourceMarks {
    fn new(fps: Rational) -> Self {
        Self { fps, cursor: 0, start: None, end: None }
    }

    fn range(self, duration: u64) -> Option<Range<u64>> {
        self.start.or(self.end).map(|_| self.start.unwrap_or(0)..self.end.unwrap_or(duration))
    }
}

enum IoResult {
    BatchImport(u64, Vec<Result<MediaAsset, String>>),
    FolderRelink(u64, u64, Result<Vec<crate::batch_media::RelinkMatch>, String>),
    Import(u64, Result<MediaAsset, String>),
    Open(u64, Result<EditorDocument, String>),
    Recover(u64, Option<PathBuf>, Result<EditorDocument, String>),
    Relink(u64, String, PathBuf, Result<(), String>),
}
enum UiCommand {
    TimelineViewport(TimelineViewport),
    SelectAsset(String),
    AddAsset(String),
    AddSelectedAsset,
    SelectClip(String, bool),
    Pipeline(PipelineCommand),
    Gesture {
        clip_id: String,
        kind: String,
        delta: i32,
        track: String,
        revision: u64,
    },
}
struct Thumbnail {
    image: slint::Image,
    _owner: GpuOutputHandle,
}
pub struct Controller {
    window: slint::Weak<EditorWindow>,
    delivery_queue: OutputQueue,
    delivery_model: Rc<VecModel<ExportJobRow>>,
    pub document: EditorDocument,
    preview: PreviewBridge,
    backend: Arc<GpuBackend>,
    io_send: mpsc::Sender<IoResult>,
    io_receive: mpsc::Receiver<IoResult>,
    ui_send: mpsc::Sender<UiCommand>,
    ui_receive: mpsc::Receiver<UiCommand>,
    generation: u64,
    ticket: u64,
    selected_asset: Option<String>,
    selected_clip: Option<String>,
    selected_audio: Option<String>,
    extra_selection: BTreeSet<String>,
    selected_track: String,
    monitor_gesture: Option<MonitorGesture>,
    motion: MotionEditor,
    timeline_viewport: TimelineViewport,
    timeline_window_dirty: bool,
    timeline_video_model: Rc<VecModel<ClipRow>>,
    timeline_audio_model: Rc<VecModel<AudioClipRow>>,
    timeline_capture: Option<(String, u64)>,
    displayed_ticket: Option<u64>,
    asset_preview: bool,
    source_marks: HashMap<String, SourceMarks>,
    audio_destination: AudioDestination,
    playhead: u64,
    playing: bool,
    requested: Option<(u64, u64, bool)>,
    displayed: Option<StableFrame>,
    color_scope: crate::color_scope::ColorScope,
    displayed_frame: Option<u64>,
    thumbnail_order: VecDeque<String>,
    thumbnails: HashMap<String, Thumbnail>,
    asset_model: Rc<VecModel<AssetRow>>,
    clip_model: Rc<VecModel<ClipRow>>,
    audio_track_model: Rc<VecModel<AudioTrackRow>>,
    audio_clip_model: Rc<VecModel<AudioClipRow>>,
    timer: slint::Timer,
    deferred_refresh: bool,
    recovery: crate::document_recovery::Recovery,
    busy: bool,
    #[cfg(target_os = "macos")]
    export: Option<crate::preview::export::ExportTask>,
    #[cfg(target_os = "macos")]
    audio: Option<crate::audio_output::AudioPlayback>,
    #[cfg(target_os = "macos")]
    waveforms: crate::audio_waveform::Waveforms,
    #[cfg(target_os = "macos")]
    proxies: crate::proxy::Proxies,
    pipeline_editor: PipelineEditor,
}
impl Controller {
    pub fn new(
        window: &EditorWindow,
        preview: PreviewBridge,
        backend: Arc<GpuBackend>,
        description: String,
    ) -> Rc<RefCell<Self>> {
        let (ui_send, ui_receive) = mpsc::channel();
        let (io_send, io_receive) = mpsc::channel();
        let asset_model = Rc::new(VecModel::default());
        let clip_model = Rc::new(VecModel::default());
        let audio_track_model = Rc::new(VecModel::default());
        let audio_clip_model = Rc::new(VecModel::default());
        window.set_assets(ModelRc::from(asset_model.clone()));
        window.set_clips(ModelRc::from(clip_model.clone()));
        window.set_audio_tracks(ModelRc::from(audio_track_model.clone()));
        window.set_audio_clips(ModelRc::from(audio_clip_model.clone()));
        let timeline_video_model = Rc::new(VecModel::default());
        let timeline_audio_model = Rc::new(VecModel::default());
        window.set_timeline_video_clips(ModelRc::from(timeline_video_model.clone()));
        window.set_timeline_audio_clips(ModelRc::from(timeline_audio_model.clone()));
        let delivery_model = Rc::new(VecModel::default());
        window.set_delivery_jobs(ModelRc::from(delivery_model.clone()));
        let this = Rc::new(RefCell::new(Self {
            window: window.as_weak(),
            document: EditorDocument::default(),
            delivery_queue: Default::default(),
            delivery_model,
            preview,
            backend,
            io_send,
            io_receive,
            ui_send,
            ui_receive,
            generation: 0,
            ticket: 0,
            selected_asset: None,
            selected_clip: None,
            selected_audio: None,
            extra_selection: BTreeSet::new(),
            selected_track: crate::document::DEFAULT_TRACK.into(),
            monitor_gesture: None,
            motion: MotionEditor::default(),
            timeline_viewport: TimelineViewport::default(), timeline_window_dirty: false,
            timeline_video_model, timeline_audio_model,
            timeline_capture: None,
            displayed_ticket: None,
            asset_preview: false,
            source_marks: HashMap::new(),
            audio_destination: AudioDestination::Automatic,
            playhead: 0,
            playing: false,
            requested: None,
            displayed: None,
            color_scope: Default::default(),
            displayed_frame: None,
            thumbnail_order: VecDeque::new(),
            thumbnails: HashMap::new(),
            asset_model,
            clip_model,
            audio_track_model,
            audio_clip_model,
            timer: slint::Timer::default(),
            deferred_refresh: false,
            recovery: crate::document_recovery::Recovery::new(),
            busy: false,
            #[cfg(target_os = "macos")]
            export: None,
            #[cfg(target_os = "macos")]
            audio: None,
            #[cfg(target_os = "macos")]
            waveforms: Default::default(),
            #[cfg(target_os = "macos")]
            proxies: Default::default(),
            pipeline_editor: PipelineEditor::default(),
        }));
        window.set_project_status(description.into());
        Self::connect(&this, window);
        Self::connect_pipeline(&this, window);
        let weak = Rc::downgrade(&this);
        this.borrow().timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(16),
            move || {
                if let Some(controller) = weak.upgrade() {
                    controller.borrow_mut().tick();
                }
            },
        );
        this.borrow_mut().refresh();
        this
    }
    fn connect(this: &Rc<RefCell<Self>>, window: &EditorWindow) {
        let weak = Rc::downgrade(this);
        window.on_project_action(move |action| {
            if let Some(controller) = weak.upgrade() {
                if let Some(kind) = action.strip_prefix("browse-") {
                    Self::browse_native_file_dialog(&controller, kind);
                } else {
                    controller.borrow_mut().action(&action);
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_import_media(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().import_dialog();
            }
        });
        let asset_sender = this.borrow().ui_send.clone();
        window.on_asset_select(move |asset_id| {
            let _ = asset_sender.send(UiCommand::SelectAsset(asset_id.to_string()));
        });
        let add_sender = this.borrow().ui_send.clone();
        window.on_add_to_timeline(move || {
            let _ = add_sender.send(UiCommand::AddSelectedAsset);
        });
        let add_sender = this.borrow().ui_send.clone();
        window.on_add_asset_to_timeline(move |asset_id| {
            let _ = add_sender.send(UiCommand::AddAsset(asset_id.to_string()));
        });
        let clip_sender = this.borrow().ui_send.clone();
        window.on_clip_select(move |clip_id| {
            let _ = clip_sender.send(UiCommand::SelectClip(clip_id.to_string(), false));
        });
        let selection_sender = this.borrow().ui_send.clone();
        window.on_clip_select_more(move |clip_id| {
            let _ = selection_sender.send(UiCommand::SelectClip(clip_id.to_string(), true));
        });
        let weak = Rc::downgrade(this);
        window.on_marker_name_changed(move |name| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                let frame = controller.playhead;
                let result = controller.document.set_marker(frame, "name", &name);
                controller.finish_edit(result);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_split_clip(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().split();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_lift_clip(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().lift();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_undo(move || {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                controller.document.undo();
                controller.changed();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_redo(move || {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                controller.document.redo();
                controller.changed();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_playback(move |action| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if action == "play" {
                    controller.play();
                } else {
                    controller.pause();
                    controller.refresh();
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_step_frame(move |delta| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                let target = controller.playhead.saturating_add_signed(i64::from(delta));
                controller.seek(target);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_zoom_changed(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().refresh();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_seek_frame(move |frame| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().seek(frame.max(0) as u64);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_source_action(move |action, value| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().source_action(&action, &value);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_source_audio_changed(move |index| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                controller.audio_destination = match index {
                    0 => AudioDestination::Automatic,
                    1 => AudioDestination::VideoOnly,
                    index => controller.document.data.audio_tracks.get((index - 2) as usize)
                        .map_or(AudioDestination::Automatic, |track| AudioDestination::Track(track.id.clone())),
                };
            }
        });
        let viewport_send = this.borrow().ui_send.clone();
        window.on_timeline_view_changed(move |left, top, width, height| {
            let _ = viewport_send.send(UiCommand::TimelineViewport(TimelineViewport::new(left, top, width, height)));
        });
        let weak = Rc::downgrade(this);
        window.on_timeline_capture(move |id| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                controller.timeline_capture = if id.is_empty() { None } else {
                    Some((id.to_string(), controller.document.revision))
                };
            }
        });
        let weak = Rc::downgrade(this);
        window.on_clip_gesture(move |clip_id, kind, delta, track| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if let Some((id, revision)) = controller.timeline_capture.take().filter(|(id, _)| id == clip_id.as_str()) {
                    let _ = controller.ui_send.send(UiCommand::Gesture {
                        clip_id: id, kind: kind.to_string(), delta, track: track.to_string(), revision,
                    });
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_mask_field(move |field, value| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if let Some(id) = controller.selected_clip.clone() {
                    let result = controller.document.edit_mask(&id, &field, &value);
                    controller.finish_edit(result);
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_color_scope_changed(move |mode| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                controller.color_scope.select(crate::color_scope::Mode::from_index(mode));
                if let Some(window) = controller.window.upgrade() {
                    window.set_color_scope_plot(slint::Image::default());
                    window.set_color_scope_summary("Waiting for a program frame".into());
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_grade_action(move |action, index, value| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().grade_action(&action, index, &value);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_track_action(move |id, action, value| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().track_action(&id, &action, &value);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_audio_track_action(move |id, action, value| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                let result = controller.document.audio_track(&id, &action, &value);
                controller.finish_edit(result);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_audio_clip_action(move |id, action, value| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if action == "select" {
                    controller.selected_clip = None;
                    controller.extra_selection.clear();
                    controller.selected_audio = Some(id.to_string());
                    controller.refresh();
                } else {
                    let result = controller.document.edit_audio_clip(&id, &action, &value);
                    controller.finish_edit(result);
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_audio_clip_gesture(move |id, kind, delta| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if let Some((_, revision)) = controller.timeline_capture.take().filter(|(clip, _)| clip == id.as_str()) {
                    let result = controller.document.audio_gesture(&id, &kind, delta, revision);
                    if result.is_ok() {
                        controller.selected_clip = None;
                        controller.extra_selection.clear();
                        controller.selected_audio = Some(id.to_string());
                    }
                    controller.finish_edit(result);
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_reorder_track(move |id, anchor, after| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if controller.timeline_capture.take().is_some_and(|(source, revision)|
                    source == id.as_str() && revision == controller.document.revision) {
                    let result = controller.document.reorder_track(&id, &anchor, after);
                    controller.finish_edit(result);
                }
            }
        });
        let weak = Rc::downgrade(this);
        window.on_clip_drop_error(move |id, track, delta| {
            weak.upgrade().and_then(|controller| {
                let controller = controller.borrow();
                let clip = controller.document.clip(&id)?;
                controller.document.check_move(&id, &track, clip.timeline_start.saturating_add_signed(i64::from(delta))).err()
            }).unwrap_or_default().into()
        });
        let weak = Rc::downgrade(this);
        window.on_clip_snap(move |id, kind, delta, track, zoom| {
            weak.upgrade().map_or(SnapResult { delta, frame: -1 }, |controller| {
                controller.borrow().snap_clip(&id, &kind, delta, &track, zoom)
            })
        });
        let weak = Rc::downgrade(this);
        window.on_asset_place(move |asset, track, frame| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().place_asset(&asset, &track, Some(frame.max(0) as u64));
            }
        });
        Self::connect_monitor(this, window);
        let weak = Rc::downgrade(this);
        window.on_motion_effect_key(move |value| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().effect_key_action(&value);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_motion_curve_handle(move |handle, x, y| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().adjust_motion_handle(handle, x, y);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_motion_action(move |action, index, delta, extend| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().motion_action(&action, index, delta, extend);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_clip_timing_changed(move |field, value| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().timing(&field, &value);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_search_changed(move |_| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().refresh();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_shortcut(move |action| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().action(&action);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_sequence_slot(move |action, index, name| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().sequence_slot(&action, index, &name);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_sequence_settings(move |width, height, numerator, denominator| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                let result = (|| {
                    let width = width.parse().map_err(|_| "Width must be an integer")?;
                    let height = height.parse().map_err(|_| "Height must be an integer")?;
                    let fps = Rational::new(
                        numerator
                            .parse()
                            .map_err(|_| "FPS numerator must be an integer")?,
                        denominator
                            .parse()
                            .map_err(|_| "FPS denominator must be an integer")?,
                    )?;
                    controller.document.set_sequence(width, height, fps)
                })();
                controller.finish_edit(result);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_rename_project(move |name| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                let result = controller.document.rename(&name);
                controller.finish_edit(result);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_delivery_queue_action(move |action, index| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().delivery_queue_action(&action, index as usize);
            }
        });
        let weak = Rc::downgrade(this);
        window.on_file_submit(move |kind, path| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .file_submit(&kind, PathBuf::from(path.as_str()));
            }
        });
    }
    fn status(&self, text: impl Into<String>) {
        if let Some(window) = self.window.upgrade() {
            window.set_io_status(text.into().into());
        }
    }
    fn finish_edit(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.changed(),
            Err(error) => {
                self.refresh();
                self.status(error);
            }
        }
    }
    fn changed(&mut self) {
        self.monitor_gesture = None;
        self.timeline_capture = None;
        self.pause();
        self.playhead = self.playhead.min(self.document.duration_frames().saturating_sub(1));
        self.asset_preview = false;
        self.requested = None;
        self.extra_selection.retain(|id| self.document.clip(id).is_some());
        if self.selected_clip.as_ref().is_some_and(|id| self.document.clip(id).is_none()) {
            self.selected_clip = self.extra_selection.pop_first();
        }
        if self.selected_audio.as_ref().is_some_and(|id| !self.document.data.audio_tracks.iter().any(|track| track.clips.iter().any(|clip| &clip.id == id))) {
            self.selected_audio = None;
        }
        self.refresh();
        self.request_preview();
    }
    fn add_asset_from_card(&mut self, id: &str) {
        self.place_asset(id, &self.selected_track.clone(), None);
    }

    fn place_asset(&mut self, id: &str, track: &str, start: Option<u64>) {
        if !self.document.asset(id).is_some_and(|asset| asset.online) {
            self.status("Selected media is offline and cannot be placed on the timeline");
            return;
        }
        if self.document.asset(id).is_some_and(|asset| !asset.has_visual()) {
            let result = self.document.place_audio(id, start.unwrap_or(self.playhead));
            self.finish_edit(result);
            return;
        }
        match self.document.place_clip(id, track, start) {
            Ok(clip_id) => self.focus_placed_clip(Some(id), track, clip_id),
            Err(error) => self.status(error),
        }
    }

    fn focus_placed_clip(&mut self, asset: Option<&str>, track: &str, clip_id: String) {
        let start = self.document.clip(&clip_id).expect("placed clip exists").timeline_start;
        self.pause();
        self.playhead = start;
        self.selected_track = track.to_owned();
        self.selected_asset = asset.map(str::to_owned);
        self.extra_selection.clear();
        self.selected_clip = Some(clip_id);
        self.selected_audio = None;
        self.asset_preview = false;
        self.requested = None;
        self.deferred_refresh = true;
        self.status("Clip added · unsaved changes");
    }

    fn place_at_playhead(&mut self, insert: bool) {
        let Some(asset) = self.selected_asset.clone() else { return };
        if !self.document.asset(&asset).is_some_and(|asset| asset.online && asset.has_visual()) {
            return;
        }
        self.pause();
        let track = self.selected_track.clone();
        let placement = if insert { Placement::Insert(self.playhead) } else { Placement::Overwrite(self.playhead) };
        let range = self.document.asset(&asset).filter(|item| item.has_video()).and_then(|item| {
            let duration = item.source_duration_frames(self.document.data.fps).ok()?;
            self.source_state(&asset).range(duration)
        });
        match self.document.place_visual_range(&asset, &track, placement, range, &self.audio_destination) {
            Ok(clip) => self.focus_placed_clip(Some(&asset), &track, clip),
            Err(error) => self.status(error),
        }
    }

    fn track_action(&mut self, id: &str, action: &str, value: &str) {
        if action == "select" {
            self.selected_track = id.into();
            self.refresh();
            return;
        }
        let result = match action {
            "add" => self.document.add_track(&self.selected_track).map(|id| self.selected_track = id),
            "up" | "down" => {
                let tracks = &self.document.data.tracks;
                let neighbor = tracks.iter().position(|track| track.id == id)
                    .and_then(|index| index.checked_add_signed(if action == "up" { -1 } else { 1 }))
                    .and_then(|index| tracks.get(index)).map(|track| track.id.clone());
                match neighbor {
                    Some(anchor) => self.document.reorder_track(id, &anchor, action == "down"),
                    None => return,
                }
            }
            _ => self.document.track_action(id, action, value),
        };
        self.finish_edit(result);
    }

    pub fn open_path(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        self.pause();
        self.busy = true;
        self.generation += 1;
        let generation = self.generation;
        let sender = self.io_send.clone();
        self.status("Opening project and validating media fingerprints…");
        std::thread::spawn(move || {
            let _ = sender.send(IoResult::Open(generation, EditorDocument::open(path)));
        });
    }
    pub fn import_path(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        self.busy = true;
        let generation = self.generation;
        let sender = self.io_send.clone();
        self.status("Importing and validating media…");
        std::thread::spawn(move || {
            let _ = sender.send(IoResult::Import(generation, crate::media::probe(&path)));
        });
    }
    fn import_files(&mut self, paths: Vec<PathBuf>) {
        if self.busy || paths.is_empty() { return; }
        self.busy = true;
        let generation = self.generation;
        let sender = self.io_send.clone();
        self.status(format!("Importing {} files…", paths.len()));
        std::thread::spawn(move || {
            let media = paths.into_iter().map(|path| crate::media::probe(&path)
                .map_err(|error| format!("{} · {error}", path.display()))).collect();
            let _ = sender.send(IoResult::BatchImport(generation, media));
        });
    }

    fn relink_folder(&mut self, folder: PathBuf) {
        if self.busy { return; }
        self.busy = true;
        let generation = self.generation;
        let revision = self.document.revision;
        let sender = self.io_send.clone();
        let assets = self.document.data.assets.clone();
        self.status("Searching the folder for offline originals…");
        std::thread::spawn(move || {
            let matches = crate::batch_media::find_originals(&folder, &assets);
            let _ = sender.send(IoResult::FolderRelink(generation, revision, matches));
        });
    }

    fn import_dialog(&mut self) {
        self.path_dialog("import");
    }
    fn confirm_discard(&self) -> bool {
        (!self.document.is_dirty() && !self.pipeline_has_drafts())
            || rfd::MessageDialog::new()
                .set_title("Unsaved project")
                .set_description("Discard unsaved changes to this project?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }
    fn action(&mut self, action: &str) {
        if matches!(action, "save" | "save-as") && self.pipeline_has_drafts() {
            self.status("Apply or discard the pending node drafts before saving");
            return;
        }
        match action {
            "import" | "media" => self.import_dialog(),
            "import-files" | "relink-folder" => self.path_dialog(action),
            #[cfg(target_os = "macos")]
            "prepare-proxy" => {
                let asset = self.selected_asset.as_deref().and_then(|id| self.document.asset(id)).cloned();
                let result = asset.as_ref().ok_or_else(|| "Select a video asset".to_owned())
                    .and_then(|asset| self.proxies.start(asset));
                self.set_proxy_status(result.map_or_else(|error| error, |_| "Preparing proxy…".into()));
            }
            #[cfg(target_os = "macos")]
            "prepare-proxy-batch" => {
                let result = self.proxies.start_batch(&self.document.data.assets);
                self.set_proxy_status(result.unwrap_or_else(|error| error));
            }
            #[cfg(target_os = "macos")]
            "cancel-proxy" => {
                self.proxies.cancel();
                self.set_proxy_status("Cancelling proxy…".into());
            }
            #[cfg(target_os = "macos")]
            "toggle-proxies" => {
                self.pause();
                self.proxies.enabled = !self.proxies.enabled;
                self.requested = None;
                self.set_proxy_status(if self.proxies.enabled { "Proxy preview enabled" } else { "Original preview enabled" }.into());
                self.request_preview();
            }
            "add-title" => match self.document.add_title(self.playhead) {
                Ok((track, clip)) => self.focus_placed_clip(None, &track, clip),
                Err(error) => self.status(error),
            },
            "add-subtitle" => match self.document.add_subtitle(self.playhead) {
                Ok((track, clip)) => self.focus_placed_clip(None, &track, clip),
                Err(error) => self.status(error),
            },
            "import-srt" => self.path_dialog("srt"),
            "new" => {
                if self.confirm_discard() {
                    self.pause();
                    self.generation += 1;
                    self.busy = false;
                    self.recovery.reset_current();
                    self.document = EditorDocument::default();
                    self.selected_asset = None;
                    self.selected_clip = None;
                    self.extra_selection.clear();
                    self.selected_audio = None;
                    self.playhead = 0;
                    self.clear_images();
                    self.changed();
                }
            }
            "recover-autosave" => {
                if let Some(path) = self.recovery.offer().map(PathBuf::from) {
                    self.begin_recovery(path, true);
                }
            }
            "recover-previous" => {
                if let Some(path) = self.document.path().map(document::previous_project_path) {
                    self.begin_recovery(path, false);
                }
            }
            "discard-autosave" => {
                match self.recovery.discard_offer() {
                    Ok(()) => {
                        self.refresh_recovery();
                        self.status("Recovery snapshot discarded");
                    }
                    Err(error) => self.status(error),
                }
            }
            "open" => self.path_dialog("open"),
            "save" => {
                if self.document.path().is_some() {
                    let result = self.document.save();
                    match result {
                        Ok(()) => {
                            self.recovery.saved();
                            self.refresh();
                            self.status("Project saved");
                        }
                        Err(error) => self.status(error),
                    }
                } else {
                    self.save_as();
                }
            }
            "save-as" => self.save_as(),
            "export" | "export-srt" => {
                if self.document.data.duration_frames() == 0 {
                    self.status("Add media to the timeline before exporting");
                } else if self.pipeline_has_drafts() {
                    self.status("Apply or discard the pending node drafts before exporting");
                } else {
                    self.path_dialog(action);
                }
            }
            #[cfg(target_os = "macos")]
            "cancel-export" => {
                if let Some(export) = &self.export {
                    export.cancel();
                }
            }
            "relink" => self.path_dialog("relink"),
            "undo" => {
                let previous = self.document.data.sequence_id.clone();
                self.document.undo();
                self.refresh_sequence_view(&previous);
                self.changed();
            }
            "redo" => {
                let previous = self.document.data.sequence_id.clone();
                self.document.redo();
                self.refresh_sequence_view(&previous);
                self.changed();
            }
            "toggle-marker" => {
                let result = self.document.set_marker(self.playhead, "toggle", "");
                self.finish_edit(result);
            }
            "split" => self.split(),
            "toggle-crop-key" => {
                self.pause();
                if let Some(id) = self.selected_clip.clone() {
                    let result = self.document.toggle_crop_key(&id, self.playhead);
                    self.finish_edit(result);
                }
            }
            "toggle-transform-key" | "ease-transform-key" => {
                self.pause();
                if let Some(id) = self.selected_clip.clone() {
                    let action = if action == "toggle-transform-key" { "toggle-key" } else { "ease" };
                    let result = self.document.spatial_action(&id, self.playhead, action);
                    self.finish_edit(result);
                }
            }
            "add-opacity-key" | "remove-opacity-key" => {
                self.pause();
                if let Some(id) = self.selected_clip.clone() {
                    let result = if action == "add-opacity-key" {
                        self.document.set_opacity_key(&id, self.playhead, None)
                    } else {
                        self.document.remove_opacity_key(&id, self.playhead)
                    };
                    self.finish_edit(result);
                }
            }
            "delete" => self.lift(),
            "insert" => self.place_at_playhead(true),
            "overwrite" => self.place_at_playhead(false),
            "ripple-delete" => {
                if let Some(id) = self.selected_clip.clone() {
                    self.pause();
                    let start = self.document.clip(&id).map(|clip| clip.timeline_start);
                    let result = self.document.ripple_delete(&id);
                    if let (Ok(()), Some(start)) = (&result, start) { self.playhead = start; }
                    self.finish_edit(result);
                }
            }
            "play" => {
                if self.playing {
                    self.pause();
                    self.refresh();
                } else {
                    self.play();
                }
            }
            "track-up" => self.track_action(&self.selected_track.clone(), "up", ""),
            "track-down" => self.track_action(&self.selected_track.clone(), "down", ""),
            "step-back" => self.seek(self.playhead.saturating_sub(1)),
            "step-forward" => self.seek(self.playhead.saturating_add(1)),
            "cancel" => {
                self.timeline_capture = None;
                self.cancel_monitor();
                self.pause();
                self.refresh();
            }
            _ => {}
        }
    }
    fn save_as(&mut self) {
        self.path_dialog("save-as");
    }
    fn path_dialog(&mut self, kind: &str) {
        if self.busy {
            return;
        }
        self.pause();
        if let Some(window) = self.window.upgrade() {
            window.set_file_dialog_kind(kind.into());
            if matches!(kind, "export" | "export-srt") {
                window.set_export_resolution(0);
                window.set_export_frame_rate(0);
                window.set_export_quality(0);
                window.set_export_profile(0);
                window.set_export_start_frame("0".into());
                window.set_export_end_frame(self.document.duration_frames().to_string().into());
            }
            window.set_file_path(if kind == "save-as" {
                self.document
                    .path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled.oqvideo.json".into())
                    .into()
            } else if matches!(kind, "export" | "export-srt") {
                let name = if kind == "export-srt" { "subtitles.srt" } else { "export.mp4" };
                self.document.path().and_then(|path| path.parent())
                    .map(|parent| parent.join(name).to_string_lossy().into_owned())
                    .unwrap_or_default().into()
            } else {
                "".into()
            });
            window.set_file_dialog_open(true);
        }
    }
    fn browse_native_file_dialog(controller: &Rc<RefCell<Self>>, kind: &str) {
        let mut state = controller.borrow_mut();
        if state.busy {
            return;
        }
        let Some(window) = state.window.upgrade() else {
            return;
        };
        let previous_path = window.get_file_path().to_string();
        state.busy = true;
        window.set_file_dialog_open(false);
        state.status("Choose a local file…");
        let weak_window = state.window.clone();
        let selected = run_reentrant_native_dialog(state, || {
            if kind == "import-files" {
                return rfd::FileDialog::new()
                    .add_filter("Supported media", &["mp4", "mov", "m4v", "mkv", "wav", "m4a", "aac", "png", "jpg", "jpeg"])
                    .pick_files();
            }
            let selected = match kind {
            "relink-folder" => rfd::FileDialog::new().set_title("Find offline originals").pick_folder(),
            "import" => rfd::FileDialog::new()
                .add_filter("Supported media", &["mp4", "mov", "m4v", "mkv", "wav", "m4a", "aac", "png", "jpg", "jpeg"])
                .pick_file(),
            "srt" => rfd::FileDialog::new()
                .add_filter("SubRip subtitles", &["srt"])
                .pick_file(),
            "open" => rfd::FileDialog::new()
                .add_filter("Video project", &["json"])
                .pick_file(),
            "save-as" => rfd::FileDialog::new()
                .set_file_name("Untitled.oqvideo.json")
                .save_file(),
            "export-srt" => rfd::FileDialog::new()
                .add_filter("SubRip subtitles", &["srt"])
                .set_file_name("subtitles.srt")
                .save_file(),
            "export" => rfd::FileDialog::new()
                .add_filter("MP4 video", &["mp4"])
                .set_file_name("export.mp4")
                .save_file(),
            "relink" => rfd::FileDialog::new()
                .set_title("Relink original media")
                .pick_file(),
            _ => None,
            };
            selected.map(|path| vec![path])
        });
        let Ok(mut state) = controller.try_borrow_mut() else {
            return;
        };
        state.busy = false;
        if let Some(mut paths) = selected {
            if kind == "import-files" {
                state.import_files(paths);
            } else if let Some(path) = paths.pop() {
                state.file_submit(kind, path);
            }
        } else if let Some(window) = weak_window.upgrade() {
            window.set_file_path(previous_path.into());
            window.set_file_dialog_open(true);
            state.status("File selection cancelled");
        }
    }
    fn file_submit(&mut self, kind: &str, path: PathBuf) {
        if self.busy {
            return;
        }
        if matches!(kind, "export" | "export-srt" | "queue-export") {
            self.submit_export(path, kind == "export-srt", kind == "queue-export");
            return;
        }
        if path.as_os_str().is_empty() {
            self.status("Enter a local file path");
            return;
        }
        if let Some(window) = self.window.upgrade() {
            window.set_file_dialog_open(false);
        }
        match kind {
            "import" => self.import_path(path),
            "import-files" => {
                let paths = path.to_string_lossy().lines().filter(|line| !line.trim().is_empty())
                    .map(PathBuf::from).collect::<Vec<_>>();
                if paths.is_empty() { self.status("Enter at least one local file path"); }
                else { self.import_files(paths); }
            }
            "relink-folder" => self.relink_folder(path),
            "srt" => match self.document.import_subtitles(&path) {
                Ok(count) => {
                    self.changed();
                    self.status(format!("Imported {count} subtitle cues"));
                }
                Err(error) => self.status(error),
            },
            "open" => {
                if self.confirm_discard() {
                    self.open_path(path);
                }
            }
            "save-as" => {
                let result = self.document.save_as(path);
                match result {
                    Ok(()) => {
                        self.recovery.saved();
                        self.refresh();
                        self.status("Project saved");
                    }
                    Err(error) => self.status(error),
                }
            }
            "relink" => {
                let Some(id) = self.selected_asset.clone() else {
                    self.status("Select a media asset to relink");
                    return;
                };
                let Some(asset) = self.document.asset(&id).cloned() else {
                    return;
                };
                let generation = self.generation;
                let sender = self.io_send.clone();
                self.busy = true;
                self.status("Verifying relinked media fingerprint…");
                std::thread::spawn(move || {
                    let result = document::fingerprint(&path).and_then(|sum| {
                        if sum == asset.fingerprint {
                            Ok(())
                        } else {
                            Err(
                                "Relink target does not match the original media fingerprint"
                                    .into(),
                            )
                        }
                    });
                    let _ = sender.send(IoResult::Relink(generation, id, path, result));
                });
            }
            _ => {}
        }
    }
    fn add_clip(&mut self) {
        let Some(asset) = self
            .selected_asset
            .clone()
            .or_else(|| self.document.data.assets.iter().find(|asset| asset.sequence_source().is_none()).map(|a| a.id.clone()))
        else {
            self.status("Import a media file first");
            return;
        };
        self.add_asset_from_card(&asset);
    }
    fn split(&mut self) {
        if let Some(id) = self.selected_audio.clone() {
            match self.document.split_audio(&id, self.playhead) {
                Ok(right) => {
                    self.selected_audio = Some(right);
                    self.changed();
                }
                Err(error) => self.status(error),
            }
            return;
        }
        let Some(id) = self.selected_clip.clone() else {
            self.status("Select a timeline clip to split");
            return;
        };
        match self.document.split(&id, self.playhead) {
            Ok(right) => {
                self.extra_selection.clear();
                self.selected_clip = Some(right);
                self.changed();
            }
            Err(error) => self.status(error),
        }
    }
    fn lift(&mut self) {
        let Some(id) = self.selected_clip.clone() else {
            return;
        };
        let result = if self.extra_selection.is_empty() {
            self.document.lift(&id)
        } else {
            let mut ids = self.extra_selection.clone();
            ids.insert(id);
            self.document.lift_selected(&ids)
        };
        self.finish_edit(result);
    }
    fn snap_clip(&self, id: &str, kind: &str, delta: i32, track: &str, zoom: f32) -> SnapResult {
        let unsnapped = SnapResult { delta, frame: -1 };
        let Some(clip) = self.document.clip(id) else { return unsnapped; };
        if delta == 0 || !zoom.is_finite() || zoom <= 0.0 {
            return unsnapped;
        }
        let group = kind == "move" && !self.extra_selection.is_empty() && self.in_selection(id);
        let moving = |candidate: &str| candidate == id || (group && self.in_selection(candidate));
        let mut anchors = Vec::new();
        let mut targets = vec![self.playhead];
        targets.extend(self.document.data.markers.iter().map(|marker| marker.frame));
        let mut minimum = -i128::from(clip.timeline_start);
        let mut maximum = i128::from(i32::MAX);
        match kind {
            "move" => {
                for lane in &self.document.data.tracks {
                    let participates = group && lane.clips.iter().any(|item| moving(&item.id));
                    for item in &lane.clips {
                        let Ok(end) = item.timeline_end() else { continue; };
                        if moving(&item.id) {
                            minimum = minimum.max(-i128::from(item.timeline_start));
                            anchors.extend([item.timeline_start, end]);
                        } else if participates || (!group && lane.id == track) {
                            targets.extend([item.timeline_start, end]);
                        }
                    }
                }
                if !group && !self.document.data.tracks.iter().any(|lane| lane.id == track) {
                    return unsnapped;
                }
            }
            "trim-start" | "trim-end" => {
                let Ok(end) = clip.timeline_end() else { return unsnapped; };
                if kind == "trim-start" {
                    anchors.push(clip.timeline_start);
                    maximum = i128::from(clip.duration) - 1;
                } else {
                    anchors.push(end);
                    minimum = 1 - i128::from(clip.duration);
                }
                if let Some(lane) = self.document.clip_track(id) {
                    for item in lane.clips.iter().filter(|item| item.id != id) {
                        if let Ok(end) = item.timeline_end() {
                            targets.extend([item.timeline_start, end]);
                        }
                    }
                }
            }
            _ => return unsnapped,
        }

        // At overview zoom, find the nearest target by frame instead of
        // comparing every selected edge against every timeline target.
        targets.sort_unstable();
        let delta = i128::from(delta);
        let mut best = None;
        for anchor in anchors {
            let base = i128::from(anchor);
            let location = base + delta.clamp(minimum, maximum);
            let first = targets.partition_point(|frame| i128::from(*frame) < base + minimum);
            let last = targets.partition_point(|frame| i128::from(*frame) <= base + maximum);
            let candidates = &targets[first..last];
            let index = candidates.partition_point(|frame| i128::from(*frame) < location);
            for frame in [index.checked_sub(1).and_then(|i| candidates.get(i)), candidates.get(index)]
                .into_iter().flatten()
            {
                let proposed = i128::from(*frame) - base;
                let distance = (proposed - delta).abs();
                if distance as f32 * zoom > 7.0 { continue; }
                let (Ok(frame), Ok(delta)) = (i32::try_from(*frame), i32::try_from(proposed)) else { continue; };
                let score = (distance, frame, delta);
                if best.is_none_or(|old| score < old) {
                    best = Some(score);
                }
            }
        }
        best.map_or(unsnapped, |(_, frame, delta)| SnapResult { delta, frame })
    }

    fn in_selection(&self, id: &str) -> bool {
        self.selected_clip.as_deref() == Some(id) || self.extra_selection.contains(id)
    }

    fn toggle_selection(&mut self, id: String) {
        if self.extra_selection.remove(&id) {
            self.refresh();
            return;
        }
        if self.selected_clip.as_ref() == Some(&id) {
            self.selected_clip = self.extra_selection.pop_first();
        } else {
            if let Some(previous) = self.selected_clip.replace(id) {
                self.extra_selection.insert(previous);
            }
            self.selected_audio = None;
            self.asset_preview = false;
            self.requested = None;
            self.request_preview();
        }
        if let Some(id) = &self.selected_clip {
            self.selected_track = self.document.clip_track(id).map_or_else(|| document::DEFAULT_TRACK.into(), |track| track.id.clone());
        }
        self.selected_asset = self.selected_clip.as_deref().and_then(|id| self.document.clip(id))
            .filter(|clip| clip.title.is_none()).map(|clip| clip.asset_id.clone());
        self.refresh();
    }

    fn gesture(&mut self, id: &str, kind: &str, delta: i32, track: &str) {
        let Some(clip) = self.document.clip(id).cloned() else {
            return;
        };
        let group_move = kind == "move" && !self.extra_selection.is_empty() && self.in_selection(id);
        if group_move {
            let mut ids = self.extra_selection.clone();
            ids.insert(id.to_owned());
            ids.extend(self.selected_clip.iter().cloned());
            let result = if self.document.clip_track(id).is_some_and(|lane| lane.id == track) {
                self.document.move_selected(&ids, delta)
            } else {
                Err("Drag the selection horizontally to keep each clip on its track".into())
            };
            self.finish_edit(result);
            return;
        }
        self.extra_selection.clear();
        self.selected_clip = Some(id.to_owned());
        self.selected_audio = None;
        self.selected_asset = clip.title.is_none().then(|| clip.asset_id.clone());
        let result = match kind {
            "slip" => self.document.slip_clip(id, delta),
            "slide" => self.document.slide_clip(id, delta),
            "move" => self.document.move_clip_to(
                id, track, clip.timeline_start.saturating_add_signed(i64::from(delta)),
            ),
            "trim-start" => self.document.trim_left(
                id,
                clip.timeline_start.saturating_add_signed(i64::from(delta)),
            ),
            "trim-end" => clip.timeline_end().and_then(|end| {
                self.document
                    .trim_right(id, end.saturating_add_signed(i64::from(delta)))
            }),
            _ => Err("Unknown timeline gesture".into()),
        };
        if result.is_ok() {
            if let Some(track) = self.document.clip_track(id) { self.selected_track = track.id.clone(); }
        }
        self.finish_edit(result);
    }
    fn timing(&mut self, field: &str, value: &str) {
        let Some(id) = self.selected_clip.clone() else {
            return;
        };
        let Some(clip) = self.document.clip(&id).cloned() else {
            return;
        };
        let result = (|| match field {
            "title-text" | "subtitle-text" | "title-size" => self.document.edit_title(&id, field, value),
            "start" => self.document.move_clip(
                &id,
                value
                    .parse()
                    .map_err(|_| "Start frame must be a non-negative integer")?,
            ),
            "end" => self.document.trim_right(
                &id,
                value
                    .parse()
                    .map_err(|_| "End frame must be a non-negative integer")?,
            ),
            "roll" => self.document.roll_end(
                &id, value.trim().parse().map_err(|_| "Cut frame must be a non-negative integer")?,
            ),
            "speed" => self.document.set_speed(
                &id, value.trim().parse().map_err(|_| "Speed must be a whole percentage from 25 to 400")?,
            ),
            "dissolve" => self.document.set_dissolve(
                &id, value.trim().parse().map_err(|_| "Enter a dissolve duration in frames, or 0 to remove it")?,
            ),
            "source_in" | "source-in" => self.document.set_timing(
                &id,
                clip.timeline_start,
                clip.duration,
                value
                    .parse()
                    .map_err(|_| "Source in must be an integer stream timestamp")?,
            ),
            "source_out" | "source-out" => {
                let out: i64 = value
                    .parse()
                    .map_err(|_| "Source out must be an integer stream timestamp")?;
                let asset = self
                    .document
                    .asset(&clip.asset_id)
                    .ok_or("Media is missing")?;
                let duration = clip.frames_until_source_out(out, self.document.data.fps, asset.time_base)?;
                self.document
                    .set_timing(&id, clip.timeline_start, duration, clip.source_in_pts)
            }
            _ => Err("Unknown clip timing field".into()),
        })();
        self.finish_edit(result);
    }
    fn pause(&mut self) {
        let was_playing = self.playing;
        self.playing = false;
        #[cfg(target_os = "macos")]
        self.audio.take();
        if was_playing {
            let progress = self.preview.progress();
            if progress.ticket == self.ticket && progress.revision == self.document.revision {
                self.playhead = progress.frame;
            }
            self.requested = None;
            self.request_preview();
        }
    }
    fn seek(&mut self, frame: u64) {
        self.monitor_gesture = None;
        self.pause();
        self.asset_preview = false;
        self.playhead = frame.min(self.document.duration_frames().saturating_sub(1));
        self.requested = None;
        self.refresh();
        self.request_preview();
    }
    fn play(&mut self) {
        self.monitor_gesture = None;
        if self.document.duration_frames() == 0 {
            self.status("Add media to the timeline first");
            return;
        }
        self.asset_preview = false;
        #[cfg(target_os = "macos")]
        match crate::audio_output::AudioPlayback::start(&self.document.data, self.playhead) {
            Ok(playback) => self.audio = playback,
            Err(error) => { self.status(error); return; }
        }
        self.playing = true;
        self.requested = None;
        self.refresh();
        self.request_preview();
    }
    fn sequence_slot(&mut self, action: &str, index: i32, label: &str) {
        self.pause();
        if action == "nest" {
            let sequence = self.document.data.sequence_list().get(index as usize + 1)
                .map(|(id, _)| (*id).to_owned());
            let Some(id) = sequence else { return; };
            let track = self.selected_track.clone();
            match self.document.place_sequence(&id, &track, self.playhead) {
                Ok(clip) => {
                    if let Some(window) = self.window.upgrade() { window.set_settings_open(false); }
                    self.focus_placed_clip(None, &track, clip);
                    self.status("Nested sequence placed");
                }
                Err(error) => self.status(error),
            }
            return;
        }
        let previous = self.document.data.sequence_id.clone();
        let result = match action {
            "create" => self.document.add_sequence(false),
            "duplicate" => self.document.add_sequence(true),
            "rename" => self.document.rename_sequence(label),
            "select" => {
                let sequence = self.document.data.sequence_options().get(index as usize)
                    .map(|(id, _)| (*id).to_owned());
                match sequence {
                    Some(id) => self.document.switch_sequence(&id),
                    None => return,
                }
            }
            _ => return,
        };
        self.refresh_sequence_view(&previous);
        self.finish_edit(result);
    }

    fn refresh_sequence_view(&mut self, previous: &str) {
        if self.document.data.sequence_id != previous {
            self.playhead = 0;
            self.selected_clip = None;
            self.extra_selection.clear();
            self.selected_audio = None;
            self.selected_track = self.document.data.tracks[0].id.clone();
            self.asset_preview = false;
            self.requested = None;
            if let Some(window) = self.window.upgrade() {
                let data = &self.document.data;
                window.set_draft_width(data.width.to_string().into());
                window.set_draft_height(data.height.to_string().into());
                window.set_draft_fps_numerator(data.fps.numerator.to_string().into());
                window.set_draft_fps_denominator(data.fps.denominator.to_string().into());
            }
        }
    }

    fn grade_action(&mut self, action: &str, index: i32, value: &str) {
        let Some(id) = self.selected_clip.clone() else { return; };
        if self.pipeline_has_drafts() {
            self.publish_grades();
            self.status("Apply or discard the pending node drafts before changing versions");
            return;
        }
        let Some(clip) = self.document.clip(&id) else { return; };
        let grade = if action == "select" {
            let choices = clip.grade_options();
            let Some(choice) = choices.get(index as usize) else { return; };
            choice.id.clone()
        } else { value.to_owned() };
        let result = self.document.edit_grade(&id, action, &grade);
        if result.is_ok() && matches!(action, "select" | "remove") { self.pipeline_editor.reset(); }
        self.finish_edit(result);
    }

    fn publish_grades(&self) {
        let Some(window) = self.window.upgrade() else { return; };
        let Some(clip) = self.selected_clip.as_deref().and_then(|id| self.document.clip(id))
            .filter(|clip| clip.title.is_none()) else {
            window.set_grade_available(false);
            window.set_grades_open(false);
            return;
        };
        window.set_grade_available(true);
        let labels = clip.grade_options();
        let active = clip.grade_versions.as_ref().map_or("grade-0", |versions| versions.active.id.as_str());
        let index = labels.iter().position(|label| label.id == active).unwrap_or(0);
        let names: Vec<slint::SharedString> = labels.iter().map(|label| label.name.as_str().into()).collect();
        if window.get_grade_names().iter().ne(names.iter().cloned()) {
            window.set_grade_names(ModelRc::new(VecModel::from(names)));
        }
        window.set_grade_index(index as i32);
        if let Some(label) = labels.get(index) { window.set_grade_name(label.name.as_str().into()); }
    }

    fn publish_sequence_slots(&self, window: &EditorWindow) {
        let slots = self.document.data.sequence_options();
        let names: Vec<slint::SharedString> = slots.iter().map(|(_, label)| (*label).into()).collect();
        if window.get_sequence_names().iter().ne(names.iter().cloned()) {
            window.set_sequence_names(ModelRc::new(VecModel::from(names)));
        }
        let source_names: Vec<slint::SharedString> = self.document.data.sequence_list()
            .into_iter().skip(1).map(|(_, label)| label.into()).collect();
        if window.get_nested_source_names().iter().ne(source_names.iter().cloned()) {
            window.set_nested_source_names(ModelRc::new(VecModel::from(source_names)));
            window.set_nested_source_choice(0);
        }
        if let Some(index) = slots.iter().position(|(id, _)| *id == self.document.data.sequence_id) {
            window.set_sequence_choice(index as i32);
        }
        window.set_active_sequence_name(self.document.data.sequence_name.clone().into());
    }

    #[cfg(target_os = "macos")]
    fn set_proxy_status(&self, text: String) {
        if let Some(window) = self.window.upgrade() {
            window.set_proxy_status(text.into());
        }
        self.refresh_proxy_controls();
    }

    #[cfg(target_os = "macos")]
    fn refresh_proxy_controls(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_proxy_mode(self.proxies.enabled);
            window.set_proxy_building(self.proxies.building());
            window.set_proxy_can_prepare_batch(self.document.data.assets.iter()
                .filter(|asset| asset.online && asset.has_video()).take(2).count() == 2);
            window.set_proxy_can_prepare(self.selected_asset.as_deref().and_then(|id| self.document.asset(id))
                .is_some_and(|asset| asset.online && asset.has_video()));
        }
    }

    fn source_state(&self, id: &str) -> SourceMarks {
        let fps = self.document.data.fps;
        self.source_marks.get(id).copied().filter(|state| state.fps == fps)
            .unwrap_or_else(|| SourceMarks::new(fps))
    }

    fn source_action(&mut self, action: &str, value: &str) {
        let Some(asset) = self.selected_asset.as_deref().and_then(|id| self.document.asset(id))
            .filter(|asset| asset.has_video() && asset.online && self.asset_preview) else { return; };
        let id = asset.id.clone();
        let Ok(duration) = asset.source_duration_frames(self.document.data.fps) else { return; };
        if duration == 0 { return; }
        let mut marks = self.source_state(&id);
        let seek = match action {
            "previous" => { marks.cursor = marks.cursor.saturating_sub(1); true }
            "next" => { marks.cursor = marks.cursor.saturating_add(1).min(duration - 1); true }
            "seek" => match value.trim().parse::<u64>() {
                Ok(frame) if frame < duration => { marks.cursor = frame; true }
                _ => { self.status("Source frame must be inside the available range"); return; }
            },
            "in" if marks.cursor < marks.end.unwrap_or(duration) => {
                marks.start = Some(marks.cursor);
                false
            }
            "out" if marks.cursor >= marks.start.unwrap_or(0) => {
                marks.end = Some(marks.cursor + 1);
                false
            }
            "clear" => {
                marks.start = None;
                marks.end = None;
                false
            }
            "in" | "out" => { self.status("Source In must precede the exclusive Out"); return; }
            _ => return,
        };
        self.source_marks.insert(id, marks);
        self.refresh_source();
        if seek {
            self.requested = None;
            self.request_preview();
        }
    }

    fn refresh_source(&self) {
        let Some(window) = self.window.upgrade() else { return; };
        let source = self.selected_asset.as_deref().and_then(|id| self.document.asset(id))
            .filter(|asset| asset.online && asset.has_video() && self.asset_preview)
            .and_then(|asset| {
                let duration = asset.source_duration_frames(self.document.data.fps).ok()?;
                (duration > 0).then(|| (self.source_state(&asset.id), duration, asset.audio.is_some()))
            });
        window.set_source_visible(source.is_some());
        if let Some((marks, duration, audio)) = source {
            window.set_source_frame(marks.cursor.to_string().into());
            window.set_source_can_previous(marks.cursor > 0);
            window.set_source_can_next(marks.cursor + 1 < duration);
            window.set_source_marks_set(marks.start.or(marks.end).is_some());
            let range = marks.range(duration).unwrap_or(0..duration);
            window.set_source_range(format!("Source {}–{} / {} frames", range.start, range.end, duration).into());
            window.set_source_has_audio(audio);
            if audio {
                let tracks = &self.document.data.audio_tracks;
                let labels = ["Auto audio track".into(), "Video only".into()].into_iter()
                    .chain(tracks.iter().map(|track| track.name.clone().into())).collect::<Vec<slint::SharedString>>();
                if window.get_source_audio_tracks().iter().ne(labels.iter().cloned()) {
                    window.set_source_audio_tracks(ModelRc::new(VecModel::from(labels)));
                }
                let index = match &self.audio_destination {
                    AudioDestination::Automatic => 0,
                    AudioDestination::VideoOnly => 1,
                    AudioDestination::Track(id) => tracks.iter().position(|track| track.id == *id)
                        .map_or(0, |index| index + 2),
                };
                window.set_source_audio_index(index as i32);
            }
        }
    }

    fn request_preview(&mut self) {
        if self.playing {
            if self.requested.is_some_and(|(revision, _, asset)| revision == self.document.revision && !asset) { return; }
            self.ticket += 1;
            let request = PlaybackRequest { ticket: self.ticket, revision: self.document.revision,
                start_frame: self.playhead,
                #[cfg(target_os = "macos")]
                project: self.proxies.project(&self.document.data),
                #[cfg(not(target_os = "macos"))]
                project: self.document.data.clone(),
                #[cfg(target_os = "macos")]
                audio_clock: self.audio.as_ref().map(|audio| audio.clock.clone()),
            };
            match self.preview.play(request) {
                Ok(()) => self.requested = Some((self.document.revision, self.playhead, false)),
                Err(error) => { self.pause(); self.status(error); }
            }
            return;
        }
        let identity = (self.document.revision, self.playhead, self.asset_preview);
        if self.requested == Some(identity) { return; }
        self.ticket += 1;
        let plan = if self.asset_preview {
            self.selected_asset.as_ref().and_then(|id| self.document.asset(id)).cloned()
                .ok_or_else(|| "Select a media asset".to_owned())
                .and_then(|asset| {
                    let source_ns = self.document.data.fps.frames_to_ns(self.source_state(&asset.id).cursor)?;
                    let mut request = asset_request(asset, self.ticket, self.document.revision, self.playhead,
                        self.document.data.width, self.document.data.height);
                    request.source_ns = source_ns;
                    Ok(request)
                })
        } else {
            sequence_request(&self.document.data, self.ticket, self.document.revision, self.playhead,
                |id| self.pipeline_preview_for(id),
                self.monitor_gesture.as_ref().map(|draft| (draft.clip.as_str(), draft.value)))
        };
        match plan.and_then(|request| {
            #[cfg(target_os = "macos")]
            let request = {
                let mut request = request;
                request.map_video_assets(|asset| self.proxies.apply(asset));
                request
            };
            self.preview.request(request)
        }) {
            Ok(()) => { self.requested = Some(identity); self.refresh_monitor(); }
            Err(error) => self.status(error),
        }
    }
    fn submit_export(&mut self, path: PathBuf, subtitles: bool, queued: bool) {
        let Some(window) = self.window.upgrade() else { return; };
        let extension = if subtitles { "srt" } else { "mp4" };
        let range = if !path.is_absolute()
            || !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
        {
            Err(format!("Enter an absolute output path ending in .{extension}"))
        } else {
            export_frames(
                window.get_export_start_frame().as_str(),
                window.get_export_end_frame().as_str(),
                self.document.duration_frames(),
            )
        };
        match range {
            Ok(frames) => {
                window.set_file_dialog_open(false);
                if subtitles {
                    match crate::delivery::export_srt(&self.document.data, frames, &path) {
                        Ok(()) => self.status(format!("Subtitle file exported · {}", path.display())),
                        Err(error) => self.status(format!("SRT export failed · {error}")),
                    }
                } else {
                    let options = ExportOptions {
                        resolution: window.get_export_resolution(),
                        frame_rate: window.get_export_frame_rate(),
                        quality: window.get_export_quality(),
                        hdr10: window.get_export_profile() == 1,
                    };
                    let spec = ExportSpec {
                        project: self.document.data.clone(), revision: self.document.revision,
                        path, frames, options,
                    };
                    let result = if queued {
                        self.delivery_queue.add(spec).map(|()| {
                            self.status("Delivery added to render queue");
                            self.publish_delivery_queue();
                        })
                    } else { self.start_export(spec) };
                    if let Err(error) = result {
                        if queued { window.set_file_dialog_open(true); }
                        self.status(error);
                    }
                }
            }
            Err(error) => {
                window.set_file_path(path.to_string_lossy().as_ref().into());
                window.set_file_dialog_open(true);
                self.status(error);
            }
        }
    }

    fn begin_recovery(&mut self, path: PathBuf, autosave: bool) {
        if self.busy || !self.confirm_discard() { return; }
        self.pause();
        self.busy = true;
        self.generation += 1;
        let generation = self.generation;
        let sender = self.io_send.clone();
        self.status("Opening recovery snapshot…");
        std::thread::spawn(move || {
            let result = EditorDocument::open_as_recovery(path.clone());
            let _ = sender.send(IoResult::Recover(generation, autosave.then_some(path), result));
        });
    }

    fn refresh_recovery(&self) {
        if let Some(window) = self.window.upgrade() {
            window.set_recovery_available(self.recovery.offer().is_some());
            window.set_previous_available(self.document.path()
                .map(document::previous_project_path).is_some_and(|path| path.is_file()));
        }
    }

    fn install_opened_document(&mut self, document: EditorDocument) {
        self.clear_images();
        self.document = document;
        self.playhead = 0;
        self.selected_clip = None;
        self.extra_selection.clear();
        self.selected_audio = None;
        self.selected_asset = self.document.data.assets.iter().find(|asset| asset.sequence_source().is_none()).map(|asset| asset.id.clone());
        self.asset_preview = false;
        self.requested = None;
        self.refresh();
        self.request_preview();
    }

    fn start_export(&mut self, spec: ExportSpec) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        {
            if self.export.is_some() { return Err("A delivery is already rendering".into()); }
            let render = self.preview.export_device()?;
            let task = crate::preview::export::ExportTask::start(
                render, spec.project, spec.revision, spec.path, spec.frames, spec.options,
            )?;
            if let Some(window) = self.window.upgrade() {
                window.set_export_status(task.progress().0.into());
                window.set_export_active(true);
            }
            self.export = Some(task);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = spec;
            Err("MP4 export requires macOS VideoToolbox".into())
        }
    }

    fn delivery_queue_action(&mut self, action: &str, index: usize) {
        match action {
            "start" => self.delivery_queue.running = true,
            "cancel" => {
                if self.delivery_queue.active == Some(index) {
                    #[cfg(target_os = "macos")]
                    if let Some(export) = &self.export { export.cancel(); }
                } else {
                    self.delivery_queue.cancel_pending(index);
                }
            }
            "retry" => {
                if let Err(error) = self.delivery_queue.retry(index) { self.status(error); }
            }
            _ => return,
        }
        self.publish_delivery_queue();
    }

    fn publish_delivery_queue(&self) {
        let Some(window) = self.window.upgrade() else { return; };
        let rows: Vec<_> = self.delivery_queue.jobs.iter().map(|job| ExportJobRow {
            path: job.spec.path.to_string_lossy().as_ref().into(),
            phase: job.phase.label().into(),
            details: if job.status.is_empty() {
                format!("{} · Frames {}–{}", job.spec.project.sequence_name, job.spec.frames.start, job.spec.frames.end).into()
            } else { job.status.as_str().into() },
            can_cancel: matches!(job.phase, JobPhase::Pending | JobPhase::Running),
            can_retry: matches!(job.phase, JobPhase::Cancelled | JobPhase::Failed),
        }).collect();
        // A running job changes each frame. Update the existing row so its
        // cancel button remains mounted while the user presses it.
        for (index, row) in rows.into_iter().enumerate() {
            match self.delivery_model.row_data(index) {
                Some(previous) if previous == row => {}
                Some(_) => self.delivery_model.set_row_data(index, row),
                None => self.delivery_model.push(row),
            }
        }
        window.set_delivery_can_start(!self.delivery_queue.running && self.delivery_queue.has_pending());
    }

    fn tick(&mut self) {
        if let Some(result) = self.color_scope.update(&self.backend, self.displayed.as_ref()) {
            if let Some(window) = self.window.upgrade() {
                match result {
                    Ok(plot) => {
                        window.set_color_scope_plot(plot.image());
                        window.set_color_scope_summary(plot.summary.into());
                    }
                    Err(error) => window.set_color_scope_summary(format!("Color scope unavailable · {error}").into()),
                }
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(status) = self.proxies.poll() {
            self.set_proxy_status(status);
            if self.proxies.enabled && !self.playing {
                self.requested = None;
                self.request_preview();
            }
        }
        if !self.busy {
            if let Some(result) = self.recovery.poll(&self.document) {
                if let Some(window) = self.window.upgrade() {
                    window.set_recovery_status(match result {
                        Ok(()) => "Recovery snapshot saved".into(),
                        Err(error) => format!("Recovery snapshot failed. {error}").into(),
                    });
                }
            }
        }
        #[cfg(target_os = "macos")]
        if self.waveforms.poll(&self.document.data) {
            if let Some(window) = self.window.upgrade() {
                self.refresh_audio(&window);
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(error) = self.audio.as_ref().and_then(|audio| audio.error()) {
            self.pause();
            self.status(error);
        }
        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.upgrade() {
            let (status, peaks) = self.audio.as_ref().map_or_else(
                || (String::new(), Vec::new()),
                |audio| {
                    let clipping = if audio.clock.clipping() { " · clipping" } else { "" };
                    let status = format!("Audio output {}%{clipping}", (audio.clock.peak() * 100.0).round() as u32);
                    let peaks = audio.clock.track_peaks().map(|peak| (peak * 100.0).round() as i32).collect();
                    (status, peaks)
                }
            );
            if window.get_audio_level().as_str() != status { window.set_audio_level(status.into()); }
            if window.get_audio_track_peaks().iter().ne(peaks.iter().copied()) {
                window.set_audio_track_peaks(ModelRc::new(VecModel::from(peaks)));
            }
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(export) = &self.export {
                let (status, outcome) = export.progress();
                if let Some(window) = self.window.upgrade() {
                    if window.get_export_status().as_str() != status {
                        window.set_export_status(status.as_str().into());
                    }
                    if outcome.is_some() { window.set_export_active(false); }
                }
                self.delivery_queue.update_active(&status, outcome);
                if outcome.is_some() { self.export.take(); }
            }
            if self.export.is_none() {
                if let Some(spec) = self.delivery_queue.next() {
                    if let Err(error) = self.start_export(spec) {
                        self.delivery_queue.update_active(&error, Some(JobPhase::Failed));
                    }
                }
            }
            if self.delivery_queue.running || self.window.upgrade().is_some_and(|window| window.get_delivery_queue_open()) {
                self.publish_delivery_queue();
            }
        }
        while let Ok(command) = self.ui_receive.try_recv() {
            match command {
                UiCommand::TimelineViewport(viewport) => {
                    if self.timeline_viewport != viewport {
                        self.timeline_viewport = viewport;
                        self.timeline_window_dirty = true;
                    }
                }
                UiCommand::Pipeline(command) => self.pipeline_command(command),
                UiCommand::SelectAsset(id) => {
                    self.selected_asset = Some(id);
                    self.selected_clip = None;
                    self.extra_selection.clear();
                    self.selected_audio = None;
                    self.asset_preview = true;
                    self.requested = None;
                    self.pause();
                    self.refresh();
                    self.request_preview();
                }
                UiCommand::AddAsset(id) => self.add_asset_from_card(&id),
                UiCommand::AddSelectedAsset => self.add_clip(),
                UiCommand::SelectClip(id, additive) => {
                    if let Some(clip) = self.document.clip(&id).cloned() {
                        if additive {
                            self.toggle_selection(id);
                            continue;
                        }
                        self.extra_selection.clear();
                        if let Some(track) = self.document.clip_track(&id) { self.selected_track = track.id.clone(); }
                        self.selected_clip = Some(id);
                        self.selected_audio = None;
                        self.selected_asset = clip.title.is_none().then(|| clip.asset_id.clone());
                        if self.asset_preview || self.playhead < clip.timeline_start
                            || self.playhead >= clip.timeline_end().unwrap_or(0) {
                            self.seek(clip.timeline_start);
                        } else {
                            self.refresh();
                        }
                    }
                }
                UiCommand::Gesture {
                    clip_id,
                    kind,
                    delta,
                    track,
                    revision,
                } => {
                    if revision == self.document.revision {
                        self.gesture(&clip_id, &kind, delta, &track);
                    } else {
                        self.status("Timeline changed while dragging");
                    }
                }
            }
        }
        self.poll_pipeline();
        if self.deferred_refresh {
            self.deferred_refresh = false;
            self.changed();
        }
        while let Ok(result) = self.io_receive.try_recv() {
            match result {
                IoResult::Import(generation, result) if generation == self.generation => {
                    self.busy = false;
                    match result {
                        Ok(asset) => {
                            let id = asset.id.clone();
                            match self.document.add_asset(asset) {
                                Ok(()) => {
                                    self.selected_asset = Some(id);
                                    self.asset_preview = true;
                                    self.selected_clip = None;
                                    self.extra_selection.clear();
                                    self.selected_audio = None;
                                    self.requested = None;
                                    self.refresh();
                                    self.request_preview();
                                }
                                Err(error) => self.status(error),
                            }
                        }
                        Err(error) => self.status(error),
                    }
                }
                IoResult::BatchImport(generation, results) if generation == self.generation => {
                    self.busy = false;
                    let mut failures = Vec::new();
                    let assets = results.into_iter().filter_map(|result| match result {
                        Ok(asset) => Some(asset),
                        Err(error) => { failures.push(error); None }
                    }).collect::<Vec<_>>();
                    let count = assets.len();
                    match self.document.add_assets(assets) {
                        Ok(added) => {
                            if let Some(id) = added.last() {
                                self.selected_asset = Some(id.clone());
                                self.selected_clip = None;
                                self.extra_selection.clear();
                                self.selected_audio = None;
                                self.asset_preview = true;
                                self.requested = None;
                                self.refresh();
                                self.request_preview();
                            }
                            let first = failures.first().map_or(String::new(), |error| format!(" · {error}"));
                            self.status(format!("Imported {} · {} duplicates · {} failed{first}",
                                added.len(), count - added.len(), failures.len()));
                        }
                        Err(error) => self.status(error),
                    }
                }
                IoResult::FolderRelink(generation, revision, result) if generation == self.generation => {
                    self.busy = false;
                    match result.and_then(|matches| {
                        self.document.relink_folder_matches(revision, &matches)?;
                        Ok(matches)
                    }) {
                        Ok(matches) => {
                            #[cfg(target_os = "macos")]
                            for entry in &matches { self.waveforms.invalidate(&entry.id); }
                            self.changed();
                            let offline = self.document.data.assets.iter().filter(|asset| !asset.online).count();
                            self.status(format!("Relinked {} originals · {offline} still offline", matches.len()));
                        }
                        Err(error) => self.status(error),
                    }
                }
                IoResult::Open(generation, result) if generation == self.generation => {
                    self.busy = false;
                    match result {
                        Ok(document) => {
                            self.recovery.reset_current();
                            self.install_opened_document(document);
                            self.status("Project opened");
                        }
                        Err(error) => self.status(error),
                    }
                }
                IoResult::Recover(generation, source, result) if generation == self.generation => {
                    self.busy = false;
                    match result {
                        Ok(document) => {
                            if let Some(path) = source { self.recovery.mark_restored(path); }
                            else { self.recovery.reset_current(); }
                            self.install_opened_document(document);
                            self.status("Recovered an editable copy");
                        }
                        Err(error) => self.status(error),
                    }
                }
                IoResult::Relink(generation, id, path, result) if generation == self.generation => {
                    self.busy = false;
                    let result = result.and_then(|_| {
                        self.document.transact_at(self.document.revision, |data| {
                            let asset = data
                                .assets
                                .iter_mut()
                                .find(|a| a.id == id)
                                .ok_or("Media asset no longer exists")?;
                            asset.path = path
                                .canonicalize()
                                .map_err(|e| e.to_string())?
                                .to_string_lossy()
                                .into_owned();
                            asset.online = true;
                            Ok(())
                        })
                    });
                    #[cfg(target_os = "macos")]
                    if result.is_ok() { self.waveforms.invalidate(&id); }
                    self.finish_edit(result);
                }
                _ => {}
            }
        }
        let mut progress = self.preview.progress();
        if progress.ticket == self.ticket && progress.revision == self.document.revision {
            if self.playing {
                self.playhead = progress.frame;
                self.playing = progress.playing;
                #[cfg(target_os = "macos")]
                if !self.playing { self.audio.take(); }
                self.refresh_time();
                if let Some(window) = self.window.upgrade() { window.set_playing(self.playing); }
                self.refresh_monitor();
            }
            progress.displayed_frame = self.displayed_frame;
            if let Some(frame) = &self.displayed {
                progress.displayed_width = frame.image.width;
                progress.displayed_height = frame.image.height;
                progress.actual_pts_ns = frame.identity.content_stamp.as_ref().and_then(|stamp| stamp.media_pts_ns);
            }
            if let Some(window) = self.window.upgrade() {
                window.set_playback_summary(format!("{} · queue {}/{} · displayed {} · late {} · skipped {} · {:.0} MiB",
                    if progress.preparing { "Preparing" } else if progress.playing { "Playing" } else { "Paused" },
                    progress.queue, progress.window_current_capacity, progress.displayed, progress.late,
                    progress.dropped + progress.coalesced, progress.bytes as f64 / (1024.0 * 1024.0)).into());
                if let Ok(json) = serde_json::to_string(&progress) {
                    window.set_playback_metrics(format!("Playback metrics {json}").into());
                }
            }
        }
        if let Some(update) = self.preview.take_update() {
            let current_clip = !self.playing || self.document.clip_at(update.frame).map(|clip| &clip.id)
                == self.document.clip_at(self.playhead).map(|clip| &clip.id);
            if update.ticket == self.ticket && update.revision == self.document.revision
                && update.epoch >= progress.epoch && (current_clip || update.image.is_err()) {
                match update.image {
                    Ok(frame) => {
                        if let Some(window) = self.window.upgrade() {
                            match slint::Image::try_from((*frame.output().texture).clone()) {
                                Ok(image) => {
                                    window.set_monitor_image(image);
                                    window.set_has_monitor_image(true);
                                    window.window().request_redraw();
                                    self.preview.acknowledge_presented();
                                }
                                Err(e) => {
                                    self.pause();
                                    self.status(format!("Cannot present GPU frame: {e:?}"));
                                    return;
                                }
                            }
                        }
                        let thumbnail_asset = if self.asset_preview {
                            self.selected_asset.as_ref().filter(|id| self.document.asset(id).is_some_and(MediaAsset::has_visual)).cloned()
                        } else {
                            let mut active = self.document.data.active_clips(update.frame);
                            active.next().filter(|(_, clip)| clip.title.is_none() && clip.transform == ClipTransform::default()
                                && active.next().is_none()).map(|(_, clip)| clip.asset_id.clone())
                        };
                        if let Some(id) = thumbnail_asset {
                            if !self.thumbnails.contains_key(&id) {
                                if let Ok(thumbnail) = self.thumbnail(&frame) {
                                    self.thumbnail_order.push_back(id.clone());
                                    self.thumbnails.insert(id, thumbnail);
                                    while self.thumbnail_order.len() > 6 {
                                        if let Some(old) = self.thumbnail_order.pop_front() {
                                            let retired = self.thumbnails.remove(&old);
                                            self.refresh();
                                            drop(retired);
                                        }
                                    }
                                    self.refresh();
                                }
                            }
                        }
                        self.displayed = Some(frame);
                        self.displayed_frame = Some(update.frame);
                        self.displayed_ticket = Some(update.ticket);
                        self.refresh_monitor();
                        if let Some(window) = self.window.upgrade() {
                            let pts = if update.layer_pts_ns.is_empty() {
                                update.actual_pts_ns.map(|v| v.to_string()).unwrap_or_else(|| "gap".into())
                            } else {
                                update.layer_pts_ns.iter().map(|(track, pts)| {
                                    format!("{track} {}", pts.map(|v| v.to_string()).unwrap_or_else(|| "unknown".into()))
                                }).collect::<Vec<_>>().join(", ")
                            };
                            window.set_monitor_status(format!(
                                "Frame {} · source PTS {pts} ns · CPU image bytes {}",
                                update.frame, update.cpu_copy_bytes
                            ).into());
                        }
                    }
                    Err(error) => {
                        self.playing = false;
                        self.refresh_monitor();
                        self.pipeline_editor.error = format!("Current pipeline failed; showing previous frame: {error}");
                        self.publish_pipeline();
                        self.status(self.pipeline_editor.error.clone());
                    }
                }
            }
        }
        self.refresh_timeline_window();
    }
    fn refresh_timeline_window(&mut self) {
        if !self.timeline_window_dirty || self.timeline_capture.is_some() { return; }
        self.timeline_window_dirty = false;
        let Some(window) = self.window.upgrade() else { return; };
        let zoom = window.get_timeline_zoom();
        let viewport = self.timeline_viewport;
        let crowded = self.clip_model.row_count() + self.audio_clip_model.row_count() > 256;
        let video_rows = self.clip_model.iter().filter(|clip| !crowded || viewport.contains(clip.x, clip.width, clip.lane)).collect();
        reconcile(&self.timeline_video_model, video_rows, |row| row.id.as_str(), same_clip_row);
        let layers = window.get_tracks().row_count() as i32;
        let audio_rows = self.audio_clip_model.iter().filter(|clip| {
            !crowded || viewport.contains(clip.start * zoom, clip.duration * zoom, layers + clip.lane)
        }).collect();
        reconcile(&self.timeline_audio_model, audio_rows, |row| row.id.as_str(), |a, b| a == b);
    }
    fn thumbnail(&self, frame: &StableFrame) -> Result<Thumbnail, String> {
        let bytes = u64::from(frame.image.width) * u64::from(frame.image.height) * 4;
        let credit = self
            .backend
            .frame_budget()
            .try_acquire((3, self.ticket as usize), 1, bytes)?
            .ok_or("Thumbnail GPU budget is full")?;
        let texture = self.backend.create_texture(
            frame.image.width,
            frame.image.height,
            TextureFormat::Rgba8Unorm,
        );
        let mut encoder =
            self.backend
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor immutable media thumbnail"),
                });
        encoder.copy_texture_to_texture(
            frame.output().texture.as_image_copy(),
            texture.texture.as_image_copy(),
            wgpu::Extent3d {
                width: frame.image.width,
                height: frame.image.height,
                depth_or_array_layers: 1,
            },
        );
        self.backend.queue.submit([encoder.finish()]);
        let retained = frame.clone();
        self.backend
            .queue
            .on_submitted_work_done(move || drop(retained));
        let image = slint::Image::try_from((*texture.texture).clone())
            .map_err(|e| format!("Cannot import media thumbnail: {e:?}"))?;
        Ok(Thumbnail {
            image,
            _owner: GpuOutputHandle {
                texture: texture.texture,
                view: texture.view,
                sampler: texture.sampler,
                lease: Some(Arc::new(credit)),
                width: texture.width,
                height: texture.height,
                format: texture.format,
            },
        })
    }
    fn refresh_time(&self) {
        if let Some(window) = self.window.upgrade() {
            let marker = self.document.data.markers.iter().find(|marker| marker.frame == self.playhead);
            window.set_marker_here(marker.is_some());
            window.set_marker_name(marker.map_or("", |marker| marker.name.as_str()).into());
            window.set_selected_frame(self.playhead.min(i32::MAX as u64) as i32);
            window.set_timecode(timecode(self.playhead, self.document.data.fps).into());
            window.set_playing(self.playing);
        }
    }
    fn refresh_audio(&mut self, window: &EditorWindow) {
        let data = &self.document.data;
        let tracks = data.audio_tracks.iter().map(|track| AudioTrackRow {
            id: track.id.clone().into(), name: track.name.clone().into(), muted: track.muted, solo: track.solo,
            locked: track.locked, sync_ripple: track.sync_ripple, gain: (track.gain * 100.0).round().to_string().into(),
        }).collect::<Vec<_>>();
        reconcile(&self.audio_track_model, tracks, |row| row.id.as_str(), |a, b| a == b);
        let scale = f64::from(data.fps.numerator) / f64::from(data.fps.denominator)
            / crate::document::SAMPLE_RATE as f64;
        let mut rows = Vec::new();
        let mut selection = None;
        let has_solo = data.audio_tracks.iter().any(|track| track.solo);
        for (lane, track) in data.audio_tracks.iter().enumerate() {
            for clip in &track.clips {
                let Ok(interval) = clip.interval(data) else { continue };
                let asset_name = data.assets.iter().find(|asset| asset.id == clip.asset_id)
                    .map_or("Missing media", |asset| asset.name.as_str());
                let selected = self.selected_audio.as_ref() == Some(&clip.id);
                let linked = matches!(clip.placement, document::AudioPlacement::Linked { .. });
                #[cfg(target_os = "macos")]
                let waveform = self.waveforms.path(&clip.asset_id, interval).unwrap_or_default();
                #[cfg(not(target_os = "macos"))]
                let waveform = String::new();
                rows.push(AudioClipRow {
                    id: clip.id.clone().into(), lane: lane as i32, name: asset_name.into(),
                    start: (interval.start as f64 * scale) as f32,
                    duration: ((interval.end - interval.start) as f64 * scale) as f32,
                    description: format!("{} samples to {}{}{}", interval.start, interval.end,
                        if interval.speed_percent == document::normal_speed() { String::new() } else { format!(" · {}% speed", interval.speed_percent) },
                        if linked { " · linked video" } else { "" }).into(),
                    selected, muted: track.muted || (has_solo && !track.solo), editable: !linked && !track.locked,
                    waveform: waveform.into(),
                    fade_in: (clip.fade_in as f64 * scale) as f32,
                    fade_out: (clip.fade_out as f64 * scale) as f32,
                });
                if selected { selection = Some((clip, asset_name, interval, track.locked, linked, track.crossfade_samples(&clip.id))); }
            }
        }
        reconcile(&self.audio_clip_model, rows, |row| row.id.as_str(), |a, b| a == b);
        self.timeline_window_dirty = true;
        if let Some((clip, name, interval, locked, linked, crossfade)) = selection {
            window.set_audio_clip_id(clip.id.clone().into());
            window.set_audio_clip_name(name.into());
            window.set_audio_clip_start(interval.start.to_string().into());
            window.set_audio_clip_end(interval.end.to_string().into());
            window.set_audio_clip_source(interval.source.to_string().into());
            window.set_audio_clip_gain((clip.gain * 100.0).round().to_string().into());
            window.set_audio_clip_speed(interval.speed_percent.to_string().into());
            window.set_audio_fade_in(clip.fade_in.to_string().into());
            window.set_audio_fade_out(clip.fade_out.to_string().into());
            window.set_audio_clip_locked(locked);
            window.set_audio_clip_linked(linked);
            window.set_audio_crossfade_length(crossfade.to_string().into());
        } else {
            window.set_audio_clip_id("".into());
        }
    }
    fn refresh(&mut self) {
        #[cfg(target_os = "macos")]
        self.waveforms.request_scan();
        let Some(window) = self.window.upgrade() else {
            return;
        };
        self.refresh_recovery();
        self.publish_sequence_slots(&window);
        window.set_has_offline_assets(self.document.data.assets.iter().any(|asset| !asset.online));
        if matches!(&self.audio_destination, AudioDestination::Track(id)
            if !self.document.data.audio_tracks.iter().any(|track| track.id == *id))
        {
            self.audio_destination = AudioDestination::Automatic;
        }
        self.refresh_source();
        self.refresh_motion();
        #[cfg(target_os = "macos")]
        self.refresh_proxy_controls();
        if !self.document.is_dirty() {
            window.set_recovery_status("".into());
        }
        window.set_project_name(self.document.data.name.clone().into());
        window.set_project_path(
            self.document
                .path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Not saved".into())
                .into(),
        );
        window.set_selection_size((self.extra_selection.len() + usize::from(self.selected_clip.is_some())) as i32);
        let duration = self.document.duration_frames();
        let markers = self.document.data.markers.iter().filter(|marker| marker.frame < duration)
            .map(|marker| MarkerRow { frame: marker.frame as i32, name: marker.name.clone().into() }).collect::<Vec<_>>();
        if window.get_markers().iter().ne(markers.iter().cloned()) {
            window.set_markers(ModelRc::new(VecModel::from(markers)));
        }
        window.set_can_undo(self.document.can_undo());
        window.set_can_redo(self.document.can_redo());
        self.refresh_audio(&window);
        window.set_sequence_duration(self.document.duration_frames().min(i32::MAX as u64) as i32);
        window.set_sequence_timecode(
            timecode(self.document.duration_frames(), self.document.data.fps).into(),
        );
        window.set_sequence_fps(
            format!(
                "{}/{}",
                self.document.data.fps.numerator, self.document.data.fps.denominator
            )
            .into(),
        );
        window.set_project_data_width(self.document.data.width as i32);
        window.set_project_data_height(self.document.data.height as i32);
        let zoom = window.get_timeline_zoom().max(0.0001);
        window.set_timeline_content_width(
            (self.document.duration_frames() as f32 * zoom + 160.0).max(900.0),
        );
        window.set_sequence_fps_numerator(self.document.data.fps.numerator.to_string().into());
        window.set_sequence_fps_denominator(self.document.data.fps.denominator.to_string().into());
        let search = window.get_search_query().to_lowercase();
        let assets = self
            .document
            .data
            .assets
            .iter()
            .filter(|asset| asset.sequence_source().is_none() && asset.name.to_lowercase().contains(&search))
            .map(|asset| {
                let thumbnail = self.thumbnails.get(&asset.id);
                AssetRow {
                    id: asset.id.clone().into(),
                    name: asset.name.clone().into(),
                    details: asset_details(asset).into(),
                    online: asset.online,
                    selected: self.selected_asset.as_ref() == Some(&asset.id),
                    thumbnail: thumbnail.map(|t| t.image.clone()).unwrap_or_default(),
                    has_thumbnail: thumbnail.is_some(),
                }
            })
            .collect::<Vec<_>>();
        reconcile(
            &self.asset_model,
            assets,
            |row| row.id.as_str(),
            |a, b| {
                a.id == b.id
                    && a.name == b.name
                    && a.details == b.details
                    && a.online == b.online
                    && a.selected == b.selected
                    && a.has_thumbnail == b.has_thumbnail
                    && same_image(&a.thumbnail, &b.thumbnail)
            },
        );
        let tracks = &self.document.data.tracks;
        window.set_can_place_visual(tracks.iter().any(|track| track.id == self.selected_track && !track.subtitles)
            && self.selected_asset.as_ref().and_then(|id| self.document.asset(id))
            .is_some_and(|asset| asset.online && asset.has_visual()));
        window.set_can_roll_end(self.selected_clip.as_ref().is_some_and(|id| self.document.can_roll_end(id)));
        window.set_can_dissolve_end(self.selected_clip.as_ref().is_some_and(|id| self.document.can_dissolve_end(id)));
        let dissolves: Vec<_> = tracks.iter().enumerate().flat_map(|(lane, track)| {
            track.dissolve_ranges().map(move |(item, range)| DissolveRow {
                lane: lane as i32,
                start: range.start as f32,
                duration: item.frames as f32,
                description: format!("{} · frames {}–{}", track.name, range.start, range.end - 1).into(),
            })
        }).collect();
        if window.get_dissolves().iter().ne(dissolves.iter().cloned()) {
            window.set_dissolves(ModelRc::new(VecModel::from(dissolves)));
        }
        if !tracks.iter().any(|track| track.id == self.selected_track) {
            self.selected_track = tracks[0].id.clone();
        }
        let tracks_model: Vec<_> = tracks.iter().map(|track| TrackRow {
            id: track.id.clone().into(),
            name: track.name.clone().into(),
            selected: track.id == self.selected_track,
            sync_ripple: track.sync_ripple,
            blend_mode: track.blend.index() as i32,
            subtitles: track.subtitles,
            visible: track.visible,
            locked: track.locked,
        }).collect();
        // Track rows must remain mounted throughout a pointer capture.
        let old_tracks = window.get_tracks();
        if old_tracks.iter().ne(tracks_model.iter().cloned()) {
            window.set_tracks(ModelRc::new(VecModel::from(tracks_model)));
        }
        let clips = tracks.iter().enumerate().flat_map(|(lane, track)| {
            track.clips.iter().map(move |clip| (lane, track, clip))
        }).map(|(lane, track, clip)| {
                let asset = self.document.asset(&clip.asset_id);
                let thumbnail = self.thumbnails.get(&clip.asset_id);
                let title = clip.title.as_ref();
                ClipRow {
                    id: clip.id.clone().into(),
                    lane: lane as i32,
                    track: track.id.clone().into(),
                    locked: track.locked,
                    asset_name: title.map(|text| {
                        if text.subtitle { text.text.lines().next().unwrap_or_default() } else { &text.text }
                    }).or_else(|| asset.map(|a| a.name.as_str())).unwrap_or("Missing").into(),
                    is_subtitle: track.subtitles,
                    is_still: asset.is_some_and(|asset| asset.still),
                    is_title: title.is_some(),
                    start: clip.timeline_start.min(i32::MAX as u64) as i32,
                    duration: clip.duration.min(i32::MAX as u64) as i32,
                    x: clip.timeline_start as f32 * zoom,
                    width: clip.duration as f32 * zoom,
                    selected: self.in_selection(&clip.id),
                    online: title.is_some() || asset.is_some_and(|a| a.online),
                    thumbnail: thumbnail.map(|t| t.image.clone()).unwrap_or_default(),
                    has_thumbnail: thumbnail.is_some(),
                }
            })
            .collect::<Vec<_>>();
        reconcile(&self.clip_model, clips, |row| row.id.as_str(), same_clip_row);
        self.timeline_window_dirty = true;
        self.refresh_timeline_window();
        if let Some(asset) = self
            .selected_asset
            .as_ref()
            .and_then(|id| self.document.asset(id))
        {
            window.set_selected_asset_name(asset.name.clone().into());
            window.set_selected_asset_details(asset_details(asset).into());
        } else {
            window.set_selected_asset_name("No media selected".into());
            window.set_selected_asset_details(
                "Import video, audio, PNG or JPEG".into(),
            );
        }
        if let Some(clip) = self
            .selected_clip
            .as_ref()
            .and_then(|id| self.document.clip(id))
        {
            window.set_selected_clip_id(clip.id.clone().into());
            window.set_selected_clip_is_title(clip.title.is_some());
            window.set_selected_clip_is_still(self.document.asset(&clip.asset_id).is_some_and(|asset| asset.still));
            window.set_selected_clip_is_subtitle(clip.title.as_ref().is_some_and(|text| text.subtitle));
            if let Some(title) = &clip.title {
                window.set_selected_title_text(title.text.as_str().into());
                window.set_selected_title_size(title.size_percent.to_string().into());
            }
            window.set_selected_clip_name(
                clip.title.as_ref().map(|title| title.text.as_str()).or_else(|| {
                    self.document.asset(&clip.asset_id).map(|a| a.name.as_str())
                })
                    .unwrap_or("Offline media")
                    .into(),
            );
            let mask = clip.mask.unwrap_or_default();
            window.set_mask_shape(match clip.mask.map(|mask| mask.shape) {
                None => 0,
                Some(crate::document::MaskShape::Rectangle) => 1,
                Some(crate::document::MaskShape::Ellipse) => 2,
            });
            window.set_mask_x(format!("{:.2}", mask.x * 100.0).into());
            window.set_mask_y(format!("{:.2}", mask.y * 100.0).into());
            window.set_mask_width(format!("{:.2}", mask.width * 100.0).into());
            window.set_mask_height(format!("{:.2}", mask.height * 100.0).into());
            window.set_mask_feather(format!("{:.2}", mask.feather * 100.0).into());
            window.set_selected_clip_start(clip.timeline_start.to_string().into());
            window.set_selected_clip_duration(clip.duration.to_string().into());
            window.set_selected_clip_speed(clip.speed_percent.to_string().into());
            let frames = self.document.clip_track(&clip.id).map_or(0, |track| track.dissolve_duration(&clip.id));
            window.set_selected_dissolve_duration(frames.to_string().into());
            window.set_selected_clip_end(clip.timeline_end().unwrap_or(0).to_string().into());
            window.set_selected_clip_source_in(clip.source_in_pts.to_string().into());
            let out = self
                .document
                .asset(&clip.asset_id)
                .and_then(|asset| clip.source_out(self.document.data.fps, asset.time_base).ok())
                .map(|(ticks, _, _)| ticks)
                .unwrap_or(0);
            window.set_selected_clip_source_out(out.to_string().into());
        } else {
            window.set_selected_clip_is_title(false);
            window.set_selected_clip_is_still(false);
            window.set_selected_clip_is_subtitle(false);
            window.set_selected_clip_id("".into());
            window.set_selected_clip_name("No clip selected".into());
        }
        self.publish_grades();
        self.publish_pipeline();
        self.refresh_time();
        self.refresh_monitor();
        window.set_project_dirty(self.document.is_dirty());
    }
    fn clear_images(&mut self) {
        self.motion = MotionEditor::default();
        self.color_scope.clear();
        self.source_marks.clear();
        self.audio_destination = AudioDestination::Automatic;
        #[cfg(target_os = "macos")]
        {
            self.proxies = Default::default();
            self.set_proxy_status(String::new());
        }
        #[cfg(target_os = "macos")]
        { self.waveforms = Default::default(); }
        self.monitor_gesture = None;
        self.timeline_capture = None;
        self.displayed_ticket = None;
        self.pipeline_editor.reset();
        if let Some(window) = self.window.upgrade() {
            window.set_color_scope_plot(slint::Image::default());
            window.set_color_scope_summary("Waiting for a program frame".into());
            window.set_monitor_image(slint::Image::default());
            window.set_has_monitor_image(false);
            window.set_monitor_status("".into());
        }
        self.asset_model.set_vec(Vec::new());
        self.clip_model.set_vec(Vec::new());
        self.timeline_video_model.set_vec(Vec::new());
        self.timeline_audio_model.set_vec(Vec::new());
        self.timeline_window_dirty = true;
        self.displayed = None;
        self.displayed_frame = None;
        self.thumbnails.clear();
        self.thumbnail_order.clear();
    }
    pub fn shutdown(&mut self) {
        self.timer.stop();
        self.recovery.reset_current();
        #[cfg(target_os = "macos")]
        self.audio.take();
        self.clear_images();
        self.preview.shutdown();
        #[cfg(target_os = "macos")]
        self.export.take();
    }
    pub fn may_close(&self) -> bool {
        #[cfg(target_os = "macos")]
        if self.export.is_some() {
            self.status("Wait for the export to finish or cancel it before closing");
            return false;
        }
        self.confirm_discard()
    }
}

fn same_image(a: &slint::Image, b: &slint::Image) -> bool {
    match (a.to_wgpu_30_texture(), b.to_wgpu_30_texture()) {
        (Some(a), Some(b)) => a == b,
        (None, None) => a.size() == b.size(),
        _ => false,
    }
}
// Preserve rows for an unchanged ID: GPU ready notifications must not cancel a
// pointer gesture or invalidate the user's focused timing field.
fn same_clip_row(a: &ClipRow, b: &ClipRow) -> bool {
    a.id == b.id
        && a.lane == b.lane
        && a.track == b.track
        && a.locked == b.locked
        && a.is_title == b.is_title
        && a.is_subtitle == b.is_subtitle
        && a.is_still == b.is_still
        && a.asset_name == b.asset_name
        && a.start == b.start
        && a.duration == b.duration
        && a.x == b.x
        && a.width == b.width
        && a.online == b.online
        && a.selected == b.selected
        && a.has_thumbnail == b.has_thumbnail
        && same_image(&a.thumbnail, &b.thumbnail)
}

fn reconcile<T: Clone + 'static>(
    model: &VecModel<T>,
    rows: Vec<T>,
    key: impl Fn(&T) -> &str,
    same: impl Fn(&T, &T) -> bool,
) {
    let count = rows.len();
    for (index, row) in rows.into_iter().enumerate() {
        if let Some(current) = model
            .row_data(index)
            .filter(|current| key(current) == key(&row))
        {
            if !same(&current, &row) {
                model.set_row_data(index, row);
            }
        } else {
            if let Some(found) = (index..model.row_count()).find(|&at| {
                model
                    .row_data(at)
                    .is_some_and(|current| key(&current) == key(&row))
            }) {
                model.remove(found);
            }
            model.insert(index, row);
        }
    }
    while model.row_count() > count {
        model.remove(model.row_count() - 1);
    }
}
fn export_frames(start: &str, end: &str, duration: u64) -> Result<Range<u64>, String> {
    let parse = |value: &str| {
        value.trim().parse::<u64>().map_err(|_| "Export frame values must be non-negative integers")
    };
    let (start, end) = (parse(start)?, parse(end)?);
    if start >= end || end > duration {
        return Err(format!("Export range must satisfy 0 ≤ start < end ≤ {duration}"));
    }
    Ok(start..end)
}

fn asset_details(asset: &MediaAsset) -> String {
    if asset.still {
        return format!("{} × {} · Still image", asset.width, asset.height);
    }
    let audio = asset.audio.as_ref().map(|audio| format!("{} · {} Hz · {} ch", audio.codec.to_uppercase(), audio.rate, audio.channels));
    if !asset.has_video() {
        return audio.unwrap_or_default();
    }
    let color = match asset.hdr_transfer {
        Some(crate::document::HdrTransfer::Pq) => " · BT.2020 PQ · 10-bit",
        Some(crate::document::HdrTransfer::Hlg) => " · BT.2020 HLG · 10-bit",
        None => "",
    };
    format!(
        "{} × {} · {} · {}{color}",
        asset.width,
        asset.height,
        asset
            .fps
            .map(|fps| format!(
                "{:.2} fps",
                f64::from(fps.numerator) / f64::from(fps.denominator)
            ))
            .unwrap_or_else(|| "VFR".into()),
        audio.as_deref().unwrap_or(if asset.has_audio { "Audio codec unsupported" } else { "No audio" })
    )
}
fn timecode(frame: u64, fps: Rational) -> String {
    let nominal = (fps.numerator as u64 + fps.denominator as u64 / 2) / fps.denominator as u64;
    let rate = nominal.max(1);
    let seconds = frame / rate;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        frame % rate
    )
}

/// AppKit's synchronous RFD panel runs a nested UI event loop; transfer the guard
/// into this helper so the controller borrow is released for every callback.
fn run_reentrant_native_dialog<T, R>(
    owner: std::cell::RefMut<'_, T>,
    dialog: impl FnOnce() -> R,
) -> R {
    drop(owner);
    dialog()
}

#[cfg(test)]
mod native_dialog_tests {
    use super::*;
    #[test]
    fn appkit_modal_releases_controller_for_nested_slint_callbacks() {
        let state = Rc::new(RefCell::new((false, false)));
        let mut owner = state.borrow_mut();
        owner.0 = true;
        let result = run_reentrant_native_dialog(owner, || {
            let mut callback = state
                .try_borrow_mut()
                .expect("nested AppKit callbacks must re-enter the controller");
            assert!(callback.0);
            callback.1 = true;
            None::<()>
        });
        assert_eq!(result, None);
        assert_eq!(*state.borrow(), (true, true));
    }

    #[test]
    fn worker_owns_playback_clock_and_clip_cuts_but_exact_seek_supersedes() {
        crate::media::configure_runtime().unwrap();
        let (preview, _configuration, _) = PreviewBridge::new().unwrap();
        let backend = preview.backend.clone();
        let asset = crate::media::probe(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_64x64.mp4")).unwrap();
        let mut document = EditorDocument::default();
        document.data.width = 64;
        document.data.height = 64;
        let asset_id = asset.id.clone();
        document.add_asset(asset).unwrap();
        let first = document.add_clip(&asset_id).unwrap();
        let second = document.add_clip(&asset_id).unwrap();
        let cut = document.clip(&second).unwrap().timeline_start;
        let mut grade = document.clip(&first).unwrap().pipeline.clone();
        let id = grade.add_corrector_serial(crate::clip_pipeline::SOURCE_ID).unwrap();
        let mut graph = grade.graph().clone();
        graph.nodes.iter_mut().find(|node| node.id == id).unwrap().data.color_corrector.as_mut().unwrap().primaries.exposure = -1.0;
        grade.replace_graph(graph).unwrap();
        document.set_clip_pipeline(&first, grade, document.revision).unwrap();
        let (io_send, io_receive) = mpsc::channel();
        let (ui_send, ui_receive) = mpsc::channel();
        let mut controller = Controller {
            window: slint::Weak::default(), document, preview, backend, delivery_queue: Default::default(),
            delivery_model: Rc::new(VecModel::default()),
            io_send, io_receive, ui_send, ui_receive, generation: 0, ticket: 0,
            selected_asset: None, selected_clip: Some(first), asset_preview: false,
            source_marks: HashMap::new(), audio_destination: AudioDestination::Automatic,
            selected_audio: None,
            extra_selection: BTreeSet::new(),
            playhead: 0, playing: false, requested: None,
            displayed: None,
            color_scope: Default::default(), displayed_frame: None, thumbnail_order: VecDeque::new(), thumbnails: HashMap::new(),
            asset_model: Rc::new(VecModel::default()), clip_model: Rc::new(VecModel::default()),
            audio_track_model: Rc::new(VecModel::default()), audio_clip_model: Rc::new(VecModel::default()),
            timer: slint::Timer::default(), deferred_refresh: false, busy: false,
            recovery: crate::document_recovery::Recovery::new(),
            #[cfg(target_os = "macos")]
            export: None,
            #[cfg(target_os = "macos")]
            audio: None,
            #[cfg(target_os = "macos")]
            waveforms: Default::default(),
            #[cfg(target_os = "macos")]
            proxies: Default::default(),
            pipeline_editor: PipelineEditor::default(),
            selected_track: crate::document::DEFAULT_TRACK.into(),
            monitor_gesture: None,
            motion: MotionEditor::default(),
            timeline_viewport: TimelineViewport::default(),
            timeline_window_dirty: false,
            timeline_video_model: Rc::new(VecModel::default()),
            timeline_audio_model: Rc::new(VecModel::default()),
            timeline_capture: None,
            displayed_ticket: None,
        };
        controller.play();
        let admitted = controller.ticket;
        // UI polls progress, but never submits a new recipe for clock ticks.
        for frame in 1..=3 {
            controller.playhead = frame;
            controller.request_preview();
            assert_eq!(controller.ticket, admitted);
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while controller.displayed.is_none() {
            controller.tick();
            assert!(std::time::Instant::now() < deadline, "bounded grade was starved");
            std::thread::yield_now();
        }
        let frame = controller.displayed.as_ref().unwrap();
        let pixels = pollster::block_on(controller.backend.read_texture_rgba(
            &frame.output().texture, 64, 64)).unwrap();
        assert!(pixels.chunks_exact(4).all(|pixel| (180..=190).contains(&pixel[0]) && pixel[3] == 255));
        controller.playhead = cut;
        controller.request_preview();
        assert_eq!(controller.ticket, admitted, "worker owns cuts within one immutable play recipe");
        controller.seek(0);
        assert!(controller.ticket > admitted, "explicit seek supersedes playback");
        assert!(!controller.playing);
        controller.preview.shutdown();
    }
}
