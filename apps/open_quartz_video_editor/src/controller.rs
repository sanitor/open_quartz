use crate::document::{self, ClipTransform, EditorDocument, MediaAsset, Placement, Rational};
use crate::preview::{asset_request, sequence_request, PlaybackRequest, PreviewBridge};
use crate::{AssetRow, AudioClipRow, AudioTrackRow, ClipRow, DissolveRow, EditorWindow, TrackRow};
use open_quartz::rendering::{GpuBackend, GpuOutputHandle, StableFrame, TextureFormat};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;
#[path = "pipeline_editor.rs"]
mod pipeline_editor;
#[path = "monitor_transform.rs"]
mod monitor_transform;
use monitor_transform::MonitorGesture;
use pipeline_editor::{PipelineCommand, PipelineEditor};

enum IoResult {
    Import(u64, Result<MediaAsset, String>),
    Open(u64, Result<EditorDocument, String>),
    Relink(u64, String, PathBuf, Result<(), String>),
}
enum UiCommand {
    SelectAsset(String),
    AddAsset(String),
    AddSelectedAsset,
    SelectClip(String),
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
    selected_track: String,
    monitor_gesture: Option<MonitorGesture>,
    timeline_capture: Option<(String, u64)>,
    displayed_ticket: Option<u64>,
    asset_preview: bool,
    playhead: u64,
    playing: bool,
    requested: Option<(u64, u64, bool)>,
    displayed: Option<StableFrame>,
    displayed_frame: Option<u64>,
    thumbnail_order: VecDeque<String>,
    thumbnails: HashMap<String, Thumbnail>,
    asset_model: Rc<VecModel<AssetRow>>,
    clip_model: Rc<VecModel<ClipRow>>,
    audio_track_model: Rc<VecModel<AudioTrackRow>>,
    audio_clip_model: Rc<VecModel<AudioClipRow>>,
    timer: slint::Timer,
    deferred_refresh: bool,
    busy: bool,
    #[cfg(target_os = "macos")]
    export: Option<crate::preview::export::ExportTask>,
    #[cfg(target_os = "macos")]
    audio: Option<crate::audio_output::AudioPlayback>,
    #[cfg(target_os = "macos")]
    waveforms: crate::audio_waveform::Waveforms,
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
        let this = Rc::new(RefCell::new(Self {
            window: window.as_weak(),
            document: EditorDocument::default(),
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
            selected_track: crate::document::DEFAULT_TRACK.into(),
            monitor_gesture: None,
            timeline_capture: None,
            displayed_ticket: None,
            asset_preview: false,
            playhead: 0,
            playing: false,
            requested: None,
            displayed: None,
            displayed_frame: None,
            thumbnail_order: VecDeque::new(),
            thumbnails: HashMap::new(),
            asset_model,
            clip_model,
            audio_track_model,
            audio_clip_model,
            timer: slint::Timer::default(),
            deferred_refresh: false,
            busy: false,
            #[cfg(target_os = "macos")]
            export: None,
            #[cfg(target_os = "macos")]
            audio: None,
            #[cfg(target_os = "macos")]
            waveforms: Default::default(),
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
            let _ = clip_sender.send(UiCommand::SelectClip(clip_id.to_string()));
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
            weak.upgrade().map_or(delta, |controller| controller.borrow()
                .snap_clip(&id, &kind, delta, &track, zoom))
        });
        let weak = Rc::downgrade(this);
        window.on_asset_place(move |asset, track, frame| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().place_asset(&asset, &track, Some(frame.max(0) as u64));
            }
        });
        Self::connect_monitor(this, window);
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
        if self.selected_clip.as_ref().is_some_and(|id| self.document.clip(id).is_none()) { self.selected_clip = None; }
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
        match self.document.place_visual(&asset, &track, placement) {
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
                    self.document = EditorDocument::default();
                    self.selected_asset = None;
                    self.selected_clip = None;
                    self.selected_audio = None;
                    self.playhead = 0;
                    self.clear_images();
                    self.changed();
                }
            }
            "open" => self.path_dialog("open"),
            "save" => {
                if self.document.path().is_some() {
                    let result = self.document.save();
                    match result {
                        Ok(()) => {
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
            "export" => {
                if self.document.data.duration_frames() == 0 {
                    self.status("Add media to the timeline before exporting");
                } else if self.pipeline_has_drafts() {
                    self.status("Apply or discard the pending node drafts before exporting");
                } else {
                    self.path_dialog("export");
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
                self.document.undo();
                self.changed();
            }
            "redo" => {
                self.document.redo();
                self.changed();
            }
            "split" => self.split(),
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
            if kind == "export" {
                window.set_export_start_frame("0".into());
                window.set_export_end_frame(self.document.duration_frames().to_string().into());
            }
            window.set_file_path(if kind == "save-as" {
                self.document
                    .path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled.oqvideo.json".into())
                    .into()
            } else if kind == "export" {
                self.document.path().and_then(|path| path.parent())
                    .map(|parent| parent.join("export.mp4").to_string_lossy().into_owned())
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
        let selected = run_reentrant_native_dialog(state, || match kind {
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
            "export" => rfd::FileDialog::new()
                .add_filter("MP4 video", &["mp4"])
                .set_file_name("export.mp4")
                .save_file(),
            "relink" => rfd::FileDialog::new()
                .set_title("Relink original media")
                .pick_file(),
            _ => None,
        });
        let Ok(mut state) = controller.try_borrow_mut() else {
            return;
        };
        state.busy = false;
        if let Some(path) = selected {
            state.file_submit(kind, path);
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
        if kind == "export" {
            self.submit_export(path);
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
            .or_else(|| self.document.data.assets.first().map(|a| a.id.clone()))
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
        let result = self.document.lift(&id);
        self.finish_edit(result);
    }
    fn snap_clip(&self, id: &str, kind: &str, delta: i32, track: &str, zoom: f32) -> i32 {
        let Some(clip) = self.document.clip(id) else { return delta; };
        let Some(track) = self.document.data.tracks.iter().find(|item| item.id == track) else { return delta; };
        if delta == 0 || zoom <= 0.0 { return delta; }
        let end = clip.timeline_end().unwrap_or(clip.timeline_start);
        let start = i128::from(clip.timeline_start);
        let end = i128::from(end);
        let mut anchors = match kind {
            "trim-end" => vec![end],
            "move" => vec![start, end],
            _ => vec![start],
        };
        anchors.iter_mut().for_each(|anchor| *anchor += i128::from(delta));
        let targets = std::iter::once(self.playhead).chain(track.clips.iter().filter(|other| other.id != id)
            .flat_map(|other| [other.timeline_start, other.timeline_end().unwrap_or(other.timeline_start)]));
        let mut best = None;
        for target in targets {
            for anchor in &anchors {
                let correction = i128::from(target) - anchor;
                if correction.abs() as f32 * zoom <= 7.0
                    && best.is_none_or(|old: i128| correction.abs() < old.abs()) {
                    best = Some(correction);
                }
            }
        }
        best.and_then(|correction| i32::try_from(i128::from(delta) + correction).ok()).unwrap_or(delta)
    }

    fn gesture(&mut self, id: &str, kind: &str, delta: i32, track: &str) {
        let Some(clip) = self.document.clip(id).cloned() else {
            return;
        };
        self.selected_clip = Some(id.to_owned());
        self.selected_audio = None;
        self.selected_asset = clip.title.is_none().then(|| clip.asset_id.clone());
        let result = match kind {
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
    fn request_preview(&mut self) {
        if self.playing {
            if self.requested.is_some_and(|(revision, _, asset)| revision == self.document.revision && !asset) { return; }
            self.ticket += 1;
            let request = PlaybackRequest { ticket: self.ticket, revision: self.document.revision,
                start_frame: self.playhead, project: self.document.data.clone(),
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
                .map(|asset| asset_request(asset, self.ticket, self.document.revision, self.playhead,
                    self.document.data.width, self.document.data.height))
        } else {
            sequence_request(&self.document.data, self.ticket, self.document.revision, self.playhead,
                |id| self.pipeline_preview_for(id),
                self.monitor_gesture.as_ref().map(|draft| (draft.clip.as_str(), draft.value)))
        };
        match plan.and_then(|request| self.preview.request(request)) {
            Ok(()) => { self.requested = Some(identity); self.refresh_monitor(); }
            Err(error) => self.status(error),
        }
    }
    fn submit_export(&mut self, path: PathBuf) {
        let Some(window) = self.window.upgrade() else { return; };
        let range = if !path.is_absolute()
            || !path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
        {
            Err("Enter an absolute output path ending in .mp4".into())
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
                self.start_export(path, frames);
            }
            Err(error) => {
                window.set_file_path(path.to_string_lossy().as_ref().into());
                window.set_file_dialog_open(true);
                self.status(error);
            }
        }
    }

    fn start_export(&mut self, path: PathBuf, frames: Range<u64>) {
        #[cfg(target_os = "macos")]
        {
            if self.export.is_some() {
                return;
            }
            let result = self.preview.export_device().and_then(|render| {
                crate::preview::export::ExportTask::start(
                    render, self.document.data.clone(), self.document.revision, path, frames,
                )
            });
            match result {
                Ok(task) => {
                    if let Some(window) = self.window.upgrade() {
                        window.set_export_status(task.progress().0.into());
                        window.set_export_active(true);
                    }
                    self.export = Some(task);
                }
                Err(error) => self.status(error),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (path, frames);
            self.status("MP4 export requires macOS VideoToolbox");
        }
    }

    fn tick(&mut self) {
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
            let status = self.audio.as_ref().map_or(String::new(), |audio| {
                format!("Audio output {}%", (audio.clock.peak() * 100.0).round() as u32)
            });
            if window.get_audio_level().as_str() != status { window.set_audio_level(status.into()); }
        }
        #[cfg(target_os = "macos")]
        if let Some(export) = &self.export {
            let (status, finished) = export.progress();
            if let Some(window) = self.window.upgrade() {
                if window.get_export_status().as_str() != status {
                    window.set_export_status(status.into());
                }
                if finished {
                    window.set_export_active(false);
                }
            }
            if finished {
                self.export.take();
            }
        }
        while let Ok(command) = self.ui_receive.try_recv() {
            match command {
                UiCommand::Pipeline(command) => self.pipeline_command(command),
                UiCommand::SelectAsset(id) => {
                    self.selected_asset = Some(id);
                    self.selected_clip = None;
                    self.selected_audio = None;
                    self.asset_preview = true;
                    self.requested = None;
                    self.pause();
                    self.refresh();
                    self.request_preview();
                }
                UiCommand::AddAsset(id) => self.add_asset_from_card(&id),
                UiCommand::AddSelectedAsset => self.add_clip(),
                UiCommand::SelectClip(id) => {
                    if let Some(clip) = self.document.clip(&id).cloned() {
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
                IoResult::Open(generation, result) if generation == self.generation => {
                    self.busy = false;
                    match result {
                        Ok(document) => {
                            self.clear_images();
                            self.document = document;
                            self.playhead = 0;
                            self.selected_clip = None;
                            self.selected_audio = None;
                            self.selected_asset =
                                self.document.data.assets.first().map(|a| a.id.clone());
                            self.asset_preview = false;
                            self.requested = None;
                            self.refresh();
                            self.request_preview();
                            self.status("Project opened");
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
            window.set_selected_frame(self.playhead.min(i32::MAX as u64) as i32);
            window.set_timecode(timecode(self.playhead, self.document.data.fps).into());
            window.set_playing(self.playing);
        }
    }
    fn refresh_audio(&self, window: &EditorWindow) {
        let data = &self.document.data;
        let tracks = data.audio_tracks.iter().map(|track| AudioTrackRow {
            id: track.id.clone().into(), name: track.name.clone().into(), muted: track.muted,
            locked: track.locked, gain: (track.gain * 100.0).round().to_string().into(),
        }).collect::<Vec<_>>();
        reconcile(&self.audio_track_model, tracks, |row| row.id.as_str(), |a, b| a == b);
        let scale = f64::from(data.fps.numerator) / f64::from(data.fps.denominator)
            / crate::document::SAMPLE_RATE as f64;
        let mut rows = Vec::new();
        let mut selection = None;
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
                    selected, muted: track.muted, editable: !linked && !track.locked,
                    waveform: waveform.into(),
                    fade_in: (clip.fade_in as f64 * scale) as f32,
                    fade_out: (clip.fade_out as f64 * scale) as f32,
                });
                if selected { selection = Some((clip, asset_name, interval, track.locked, linked)); }
            }
        }
        reconcile(&self.audio_clip_model, rows, |row| row.id.as_str(), |a, b| a == b);
        if let Some((clip, name, interval, locked, linked)) = selection {
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
        window.set_project_name(self.document.data.name.clone().into());
        window.set_project_path(
            self.document
                .path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Not saved".into())
                .into(),
        );
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
            .filter(|asset| asset.name.to_lowercase().contains(&search))
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
                    selected: self.selected_clip.as_ref() == Some(&clip.id),
                    online: title.is_some() || asset.is_some_and(|a| a.online),
                    thumbnail: thumbnail.map(|t| t.image.clone()).unwrap_or_default(),
                    has_thumbnail: thumbnail.is_some(),
                }
            })
            .collect::<Vec<_>>();
        reconcile(
            &self.clip_model,
            clips,
            |row| row.id.as_str(),
            |a, b| {
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
            },
        );
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
        self.publish_pipeline();
        self.refresh_time();
        self.refresh_monitor();
        if self.document.is_dirty() {
            self.status("Unsaved changes");
        }
    }
    fn clear_images(&mut self) {
        #[cfg(target_os = "macos")]
        { self.waveforms = Default::default(); }
        self.monitor_gesture = None;
        self.timeline_capture = None;
        self.displayed_ticket = None;
        self.pipeline_editor.reset();
        if let Some(window) = self.window.upgrade() {
            window.set_monitor_image(slint::Image::default());
            window.set_has_monitor_image(false);
            window.set_monitor_status("".into());
        }
        self.asset_model.set_vec(Vec::new());
        self.clip_model.set_vec(Vec::new());
        self.displayed = None;
        self.displayed_frame = None;
        self.thumbnails.clear();
        self.thumbnail_order.clear();
    }
    pub fn shutdown(&mut self) {
        self.timer.stop();
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
    format!(
        "{} × {} · {} · {}",
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
            window: slint::Weak::default(), document, preview, backend,
            io_send, io_receive, ui_send, ui_receive, generation: 0, ticket: 0,
            selected_asset: None, selected_clip: Some(first), asset_preview: false,
            selected_audio: None,
            playhead: 0, playing: false, requested: None,
            displayed: None, displayed_frame: None, thumbnail_order: VecDeque::new(), thumbnails: HashMap::new(),
            asset_model: Rc::new(VecModel::default()), clip_model: Rc::new(VecModel::default()),
            audio_track_model: Rc::new(VecModel::default()), audio_clip_model: Rc::new(VecModel::default()),
            timer: slint::Timer::default(), deferred_refresh: false, busy: false,
            #[cfg(target_os = "macos")]
            export: None,
            #[cfg(target_os = "macos")]
            audio: None,
            #[cfg(target_os = "macos")]
            waveforms: Default::default(),
            pipeline_editor: PipelineEditor::default(),
            selected_track: crate::document::DEFAULT_TRACK.into(),
            monitor_gesture: None,
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
