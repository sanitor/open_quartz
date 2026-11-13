use crate::document::{self, ClipTransform, EditorDocument, MediaAsset, Rational};
use crate::preview::{PreviewBridge, PreviewRequest};
use crate::{AssetRow, ClipRow, EditorWindow};
use open_quartz::rendering::{GpuBackend, GpuOutputHandle, StableFrame, TextureFormat};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

enum IoResult {
    Import(u64, Result<MediaAsset, String>),
    Open(u64, Result<EditorDocument, String>),
    Relink(u64, String, PathBuf, Result<(), String>),
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
    generation: u64,
    ticket: u64,
    selected_asset: Option<String>,
    selected_clip: Option<String>,
    asset_preview: bool,
    playhead: u64,
    playing: bool,
    clock: Option<(Instant, u64)>,
    requested: Option<(u64, u64, bool)>,
    displayed: Option<StableFrame>,
    thumbnail_order: VecDeque<String>,
    thumbnails: HashMap<String, Thumbnail>,
    asset_model: Rc<VecModel<AssetRow>>,
    clip_model: Rc<VecModel<ClipRow>>,
    timer: slint::Timer,
    deferred_refresh: bool,
    busy: bool,
}
impl Controller {
    pub fn new(
        window: &EditorWindow,
        preview: PreviewBridge,
        backend: Arc<GpuBackend>,
        description: String,
    ) -> Rc<RefCell<Self>> {
        let (io_send, io_receive) = mpsc::channel();
        let asset_model = Rc::new(VecModel::default());
        let clip_model = Rc::new(VecModel::default());
        window.set_assets(ModelRc::from(asset_model.clone()));
        window.set_clips(ModelRc::from(clip_model.clone()));
        let this = Rc::new(RefCell::new(Self {
            window: window.as_weak(),
            document: EditorDocument::default(),
            preview,
            backend,
            io_send,
            io_receive,
            generation: 0,
            ticket: 0,
            selected_asset: None,
            selected_clip: None,
            asset_preview: false,
            playhead: 0,
            playing: false,
            clock: None,
            requested: None,
            displayed: None,
            thumbnail_order: VecDeque::new(),
            thumbnails: HashMap::new(),
            asset_model,
            clip_model,
            timer: slint::Timer::default(),
            deferred_refresh: false,
            busy: false,
        }));
        window.set_project_status(description.into());
        Self::connect(&this, window);
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
        let weak = Rc::downgrade(this);
        window.on_asset_select(move |id| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                controller.selected_asset = Some(id.to_string());
                controller.selected_clip = None;
                controller.asset_preview = true;
                controller.requested = None;
                controller.pause();
                controller.refresh();
                controller.request_preview();
            }
        });
        let weak = Rc::downgrade(this);
        window.on_add_to_timeline(move || {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().add_clip();
            }
        });
        let weak=Rc::downgrade(this);
        window.on_add_asset_to_timeline(move|asset_id|{if let Some(controller)=weak.upgrade(){controller.borrow_mut().add_asset_from_card(&asset_id);}});
        let weak = Rc::downgrade(this);
        window.on_clip_select(move |id| {
            if let Some(controller) = weak.upgrade() {
                let mut controller = controller.borrow_mut();
                if let Some(clip) = controller.document.clip(&id).cloned() {
                    controller.selected_clip = Some(id.to_string());
                    controller.selected_asset = Some(clip.asset_id);
                    controller.seek(clip.timeline_start);
                }
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
        window.on_clip_gesture(move |id, kind, delta| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().gesture(&id, &kind, delta);
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
        self.pause();
        self.asset_preview = false;
        self.requested = None;
        if self.selected_clip.as_ref().is_some_and(|id| self.document.clip(id).is_none()) { self.selected_clip = None; }
        self.refresh();
        self.request_preview();
    }
    fn add_asset_from_card(&mut self,id:&str){
        if !self.document.asset(id).is_some_and(|asset|asset.online){self.status("Selected media is offline and cannot be placed on the timeline");return;}
        match self.document.add_clip(id){
            Ok(clip_id)=>{let clip=self.document.clip(&clip_id).expect("inserted clip exists");self.playhead=clip.timeline_start;self.selected_asset=Some(id.to_owned());self.selected_clip=Some(clip_id);self.asset_preview=false;self.requested=None;self.pause();self.deferred_refresh=true;self.status("Clip added · unsaved changes");},
            Err(error)=>self.status(error),
        }
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
        self.status("Importing · hardware decode and source validation…");
        std::thread::spawn(move || {
            let _ = sender.send(IoResult::Import(generation, crate::media::probe(&path)));
        });
    }
    fn import_dialog(&mut self) {
        self.path_dialog("import");
    }
    fn confirm_discard(&self) -> bool {
        !self.document.is_dirty()
            || rfd::MessageDialog::new()
                .set_title("Unsaved project")
                .set_description("Discard unsaved changes to this project?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                == rfd::MessageDialogResult::Yes
    }
    fn action(&mut self, action: &str) {
        match action {
            "import" | "media" => self.import_dialog(),
            "new" => {
                if self.confirm_discard() {
                    self.pause();
                    self.generation += 1;
                    self.busy = false;
                    self.document = EditorDocument::default();
                    self.selected_asset = None;
                    self.selected_clip = None;
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
            "delete" => self.lift(),
            "play" => {
                if self.playing {
                    self.pause();
                    self.refresh();
                } else {
                    self.play();
                }
            }
            "step-back" => self.seek(self.playhead.saturating_sub(1)),
            "step-forward" => self.seek(self.playhead.saturating_add(1)),
            "cancel" => {
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
            window.set_file_path(if kind == "save-as" {
                self.document
                    .path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled.oqvideo.json".into())
                    .into()
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
                .add_filter("Video", &["mp4", "mov", "m4v", "mkv"])
                .pick_file(),
            "open" => rfd::FileDialog::new()
                .add_filter("Video project", &["json"])
                .pick_file(),
            "save-as" => rfd::FileDialog::new()
                .set_file_name("Untitled.oqvideo.json")
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
        if path.as_os_str().is_empty() {
            self.status("Enter a local file path");
            return;
        }
        if let Some(window) = self.window.upgrade() {
            window.set_file_dialog_open(false);
        }
        match kind {
            "import" => self.import_path(path),
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
        match self.document.add_clip(&asset) {
            Ok(id) => {
                self.playhead = self.document.clip(&id).unwrap().timeline_start;
                self.selected_clip = Some(id);
                self.selected_asset = Some(asset);
                self.changed();
            }
            Err(error) => self.status(error),
        }
    }
    fn split(&mut self) {
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
    fn gesture(&mut self, id: &str, kind: &str, delta: i32) {
        let Some(clip) = self.document.clip(id).cloned() else {
            return;
        };
        self.selected_clip = Some(id.to_owned());
        let result = match kind {
            "move" => self.document.move_clip(
                id,
                clip.timeline_start.saturating_add_signed(i64::from(delta)),
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
                let ticks = out
                    .checked_sub(clip.source_in_pts)
                    .ok_or("Source range overflow")?;
                if ticks <= 0 {
                    return Err("Source out must follow source in".into());
                }
                let ns = (ticks as u128) * (asset.time_base.numerator as u128) * 1_000_000_000
                    / (asset.time_base.denominator as u128);
                let duration =
                    self.document.data.fps.ns_to_frames_floor(
                        u64::try_from(ns).map_err(|_| "Source out overflow")?,
                    )?;
                self.document
                    .set_timing(&id, clip.timeline_start, duration, clip.source_in_pts)
            }
            _ => Err("Unknown clip timing field".into()),
        })();
        self.finish_edit(result);
    }
    fn pause(&mut self) {
        self.playing = false;
        self.clock = None;
    }
    fn seek(&mut self, frame: u64) {
        self.pause();
        self.asset_preview = false;
        self.playhead = frame.min(self.document.duration_frames().saturating_sub(1));
        self.requested = None;
        self.refresh();
        self.request_preview();
    }
    fn play(&mut self) {
        if self.document.duration_frames() == 0 {
            self.status("Add media to the timeline first");
            return;
        }
        self.asset_preview = false;
        self.playing = true;
        self.clock = Some((Instant::now(), self.playhead));
        self.requested = None;
        self.refresh();
        self.request_preview();
    }
    fn request_preview(&mut self) {
        let identity = (self.document.revision, self.playhead, self.asset_preview);
        if self.requested == Some(identity) {
            return;
        }
        let plan: Result<(Option<MediaAsset>, u64, ClipTransform), String> = (|| {
            if self.asset_preview {
                let asset = self
                    .selected_asset
                    .as_ref()
                    .and_then(|id| self.document.asset(id))
                    .cloned()
                    .ok_or("Select a media asset")?;
                return Ok((Some(asset), 0, ClipTransform::default()));
            }
            if let Some(clip) = self.document.clip_at(self.playhead) {
                let asset = self
                    .document
                    .asset(&clip.asset_id)
                    .cloned()
                    .ok_or("Clip media is missing")?;
                let source_ns =
                    document::source_time_ns(&asset, clip, self.playhead, self.document.data.fps)?;
                Ok((Some(asset), source_ns, clip.transform))
            } else {
                Ok((None, 0, ClipTransform::default()))
            }
        })();
        match plan {
            Ok((asset, source_ns, transform)) => {
                self.ticket += 1;
                let request = PreviewRequest {
                    ticket: self.ticket,
                    revision: self.document.revision,
                    frame: self.playhead,
                    source_ns,
                    width: self.document.data.width,
                    height: self.document.data.height,
                    asset,
                    transform,
                };
                if let Err(error) = self.preview.request(request) {
                    self.status(error);
                } else {
                    self.requested = Some(identity);
                }
            }
            Err(error) => self.status(error),
        }
    }
    fn tick(&mut self) {
        if self.deferred_refresh { self.deferred_refresh=false; self.changed(); }
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
                            self.selected_asset =
                                self.document.data.assets.first().map(|a| a.id.clone());
                            self.asset_preview = false;
                            self.requested = None;
                            self.refresh();
                            self.request_preview();
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
                    self.finish_edit(result);
                }
                _ => {}
            }
        }
        if let Some(update) = self.preview.take_update() {
            if update.ticket == self.ticket && update.revision == self.document.revision {
                match update.image {
                    Ok(frame) => {
                        if let Some(window) = self.window.upgrade() {
                            match slint::Image::try_from((*frame.output().texture).clone()) {
                                Ok(image) => {
                                    window.set_monitor_image(image);
                                    window.set_has_monitor_image(true);
                                    window.window().request_redraw();
                                }
                                Err(e) => {
                                    self.status(format!("Cannot present GPU frame: {e:?}"));
                                }
                            }
                        }
                        let thumbnail_asset = if self.asset_preview {
                            self.selected_asset.clone()
                        } else {
                            self.document
                                .clip_at(update.frame)
                                .map(|clip| clip.asset_id.clone())
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
                        self.status(format!(
                            "Frame {} · source PTS {} ns · CPU image bytes {}",
                            update.frame,
                            update
                                .actual_pts_ns
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "gap".into()),
                            update.cpu_copy_bytes
                        ));
                    }
                    Err(error) => {
                        self.pause();
                        if let Some(window) = self.window.upgrade() {
                            window.set_monitor_image(slint::Image::default());
                            window.set_has_monitor_image(false);
                        }
                        self.displayed = None;
                        self.status(error);
                    }
                }
            }
        }
        if self.playing {
            if let Some((start, frame)) = self.clock {
                let elapsed = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                let advance = self
                    .document
                    .data
                    .fps
                    .ns_to_frames_floor(elapsed)
                    .unwrap_or(0);
                let next = frame.saturating_add(advance);
                let duration = self.document.duration_frames();
                if next >= duration {
                    self.pause();
                    self.playhead = duration.saturating_sub(1);
                } else {
                    self.playhead = next;
                }
                self.refresh_time();
                self.request_preview();
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
    fn refresh(&mut self) {
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
        let zoom = window.get_timeline_zoom().max(1) as f32;
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
        let clips = self
            .document
            .data
            .clips
            .iter()
            .map(|clip| {
                let asset = self.document.asset(&clip.asset_id);
                let thumbnail = self.thumbnails.get(&clip.asset_id);
                ClipRow {
                    id: clip.id.clone().into(),
                    asset_name: asset.map(|a| a.name.as_str()).unwrap_or("Missing").into(),
                    start: clip.timeline_start.min(i32::MAX as u64) as i32,
                    duration: clip.duration.min(i32::MAX as u64) as i32,
                    x: clip.timeline_start as f32 * zoom,
                    width: clip.duration as f32 * zoom,
                    selected: self.selected_clip.as_ref() == Some(&clip.id),
                    online: asset.is_some_and(|a| a.online),
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
                "Import local H.264 / HEVC SDR video · video only".into(),
            );
        }
        if let Some(clip) = self
            .selected_clip
            .as_ref()
            .and_then(|id| self.document.clip(id))
        {
            window.set_selected_clip_id(clip.id.clone().into());
            window.set_selected_clip_name(
                self.document
                    .asset(&clip.asset_id)
                    .map(|a| a.name.as_str())
                    .unwrap_or("Offline media")
                    .into(),
            );
            window.set_selected_clip_start(clip.timeline_start.to_string().into());
            window.set_selected_clip_duration(clip.duration.to_string().into());
            window.set_selected_clip_end(clip.timeline_end().unwrap_or(0).to_string().into());
            window.set_selected_clip_source_in(clip.source_in_pts.to_string().into());
            let out = self
                .document
                .asset(&clip.asset_id)
                .map(|asset| {
                    clip.source_in_pts as i128
                        + clip.duration as i128
                            * self.document.data.fps.denominator as i128
                            * asset.time_base.denominator as i128
                            / self.document.data.fps.numerator as i128
                            / asset.time_base.numerator as i128
                })
                .unwrap_or(0);
            window.set_selected_clip_source_out(out.to_string().into());
        } else {
            window.set_selected_clip_id("".into());
            window.set_selected_clip_name("No clip selected".into());
        }
        self.refresh_time();
        if self.document.is_dirty() {
            self.status("Unsaved changes");
        }
    }
    fn clear_images(&mut self) {
        if let Some(window) = self.window.upgrade() {
            window.set_monitor_image(slint::Image::default());
            window.set_has_monitor_image(false);
        }
        self.asset_model.set_vec(Vec::new());
        self.clip_model.set_vec(Vec::new());
        self.displayed = None;
        self.thumbnails.clear();
        self.thumbnail_order.clear();
    }
    pub fn shutdown(&mut self) {
        self.timer.stop();
        self.clear_images();
        self.preview.shutdown();
    }
    pub fn may_close(&self) -> bool {
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
fn asset_details(asset: &MediaAsset) -> String {
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
        if asset.has_audio {
            "Video only · audio unused"
        } else {
            "No audio"
        }
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
}
