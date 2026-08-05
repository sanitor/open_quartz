use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;

use super::GpuOutputHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationBackendKind {
    SharedTexture,
    WebViewTextureStream,
    RgbaReadback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationCapabilities {
    pub kind: PresentationBackendKind,
    pub zero_cpu_copy: bool,
    pub preserves_exact_pixels: bool,
    pub supports_dom_composition: bool,
    pub estimated_latency_frames: u8,
}

#[derive(Clone)]
pub struct GpuPresentationFrame {
    pub node_id: String,
    pub frame: u64,
    pub timeline_ns: u64,
    pub output: GpuOutputHandle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentationQueueStats {
    pub submitted: u64,
    pub replaced: u64,
    pub consumed: u64,
}

#[derive(Default)]
struct MailboxState {
    latest: Option<GpuPresentationFrame>,
    closed: bool,
    stats: PresentationQueueStats,
}

#[derive(Default)]
pub struct LatestFrameMailbox {
    state: Mutex<MailboxState>,
    ready: Condvar,
}

impl LatestFrameMailbox {
    pub fn submit(&self, frame: GpuPresentationFrame) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "Presentation mailbox lock is poisoned".to_owned())?;
        if state.closed {
            return Err("Presentation mailbox is closed".to_owned());
        }
        state.stats.submitted = state.stats.submitted.saturating_add(1);
        let replaced = state.latest.replace(frame).is_some();
        if replaced {
            state.stats.replaced = state.stats.replaced.saturating_add(1);
        }
        self.ready.notify_one();
        Ok(replaced)
    }

    pub fn try_take(&self) -> Option<GpuPresentationFrame> {
        let mut state = self.state.lock().ok()?;
        let frame = state.latest.take()?;
        state.stats.consumed = state.stats.consumed.saturating_add(1);
        Some(frame)
    }

    pub fn wait_take(&self, timeout: Duration) -> Option<GpuPresentationFrame> {
        let state = self.state.lock().ok()?;
        let (mut state, _) = self
            .ready
            .wait_timeout_while(state, timeout, |state| {
                state.latest.is_none() && !state.closed
            })
            .ok()?;
        let frame = state.latest.take()?;
        state.stats.consumed = state.stats.consumed.saturating_add(1);
        Some(frame)
    }

    pub fn stats(&self) -> PresentationQueueStats {
        self.state
            .lock()
            .map(|state| state.stats)
            .unwrap_or_default()
    }

    pub fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.latest = None;
            self.ready.notify_all();
        }
    }
}

pub trait GpuPresenter: Send {
    fn capabilities(&self) -> PresentationCapabilities;
    fn mailbox(&self) -> &LatestFrameMailbox;
    fn process_latest(&mut self) -> Result<bool, String>;

    fn submit(&self, frame: GpuPresentationFrame) -> Result<bool, String> {
        self.mailbox().submit(frame)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentationSubmitStats {
    pub accepted: u64,
    pub replaced: u64,
    pub failed: u64,
}

#[derive(Default)]
pub struct PresenterRegistry {
    presenters: BTreeMap<String, Box<dyn GpuPresenter>>,
}

impl PresenterRegistry {
    pub fn register(
        &mut self,
        presenter_id: impl Into<String>,
        presenter: Box<dyn GpuPresenter>,
    ) -> Result<(), String> {
        let presenter_id = presenter_id.into();
        if presenter_id.is_empty() || self.presenters.contains_key(&presenter_id) {
            return Err("Presenter ID must be non-empty and unique".to_owned());
        }
        self.presenters.insert(presenter_id, presenter);
        Ok(())
    }

    pub fn remove(&mut self, presenter_id: &str) -> bool {
        self.presenters.remove(presenter_id).is_some()
    }

    pub fn submit(&self, frame: GpuPresentationFrame) -> PresentationSubmitStats {
        let mut stats = PresentationSubmitStats::default();
        for presenter in self.presenters.values() {
            match presenter.submit(frame.clone()) {
                Ok(replaced) => {
                    stats.accepted = stats.accepted.saturating_add(1);
                    stats.replaced = stats.replaced.saturating_add(u64::from(replaced));
                }
                Err(_) => stats.failed = stats.failed.saturating_add(1),
            }
        }
        stats
    }

    pub fn process_latest(&mut self) -> Vec<(String, Result<bool, String>)> {
        self.presenters
            .iter_mut()
            .map(|(id, presenter)| (id.clone(), presenter.process_latest()))
            .collect()
    }

    pub fn capabilities(&self) -> BTreeMap<String, PresentationCapabilities> {
        self.presenters
            .iter()
            .map(|(id, presenter)| (id.clone(), presenter.capabilities()))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SharedTexturePlatform {
    Dxgi,
    IoSurface,
    DmaBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedTextureFrame {
    pub lease_id: u64,
    pub platform: SharedTexturePlatform,
    pub resource_handle: u64,
    pub sync_handle: Option<u64>,
    pub sync_value: u64,
    pub width: u32,
    pub height: u32,
    pub frame: u64,
    pub timeline_ns: u64,
}

pub trait SharedTextureExporter: Send {
    fn export(&mut self, frame: &GpuPresentationFrame) -> Result<SharedTextureFrame, String>;
    fn release(&mut self, lease_id: u64) -> Result<(), String>;
}

pub struct SharedTexturePresenter<E> {
    mailbox: LatestFrameMailbox,
    exporter: E,
    latest: Option<SharedTextureFrame>,
}

impl<E> SharedTexturePresenter<E> {
    pub fn new(exporter: E) -> Self {
        Self {
            mailbox: LatestFrameMailbox::default(),
            exporter,
            latest: None,
        }
    }

    pub fn latest(&self) -> Option<&SharedTextureFrame> {
        self.latest.as_ref()
    }

    pub fn take_latest(&mut self) -> Option<SharedTextureFrame> {
        self.latest.take()
    }

    pub fn release(&mut self, lease_id: u64) -> Result<(), String>
    where
        E: SharedTextureExporter,
    {
        self.exporter.release(lease_id)
    }
}

impl<E: SharedTextureExporter> GpuPresenter for SharedTexturePresenter<E> {
    fn capabilities(&self) -> PresentationCapabilities {
        PresentationCapabilities {
            kind: PresentationBackendKind::SharedTexture,
            zero_cpu_copy: true,
            preserves_exact_pixels: true,
            supports_dom_composition: false,
            estimated_latency_frames: 0,
        }
    }

    fn mailbox(&self) -> &LatestFrameMailbox {
        &self.mailbox
    }

    fn process_latest(&mut self) -> Result<bool, String> {
        if self.latest.is_some() {
            return Ok(false);
        }
        let Some(frame) = self.mailbox.try_take() else {
            return Ok(false);
        };
        self.latest = Some(self.exporter.export(&frame)?);
        Ok(true)
    }
}
