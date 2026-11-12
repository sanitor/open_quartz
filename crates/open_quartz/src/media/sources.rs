use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use crate::resources::{FrameBudget, FrameCredit};

use super::native::{
    NativeVideoConfig, NativeVideoFrame, NativeVideoInfo, NativeVideoMetrics, NativeVideoSource,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStageKind { Enqueued, DecodeStarted, DecodeReady, Cancelled }

/// Monotonic timestamps can be compared directly with GPU submission events.
#[derive(Clone, Debug)]
pub struct SourceStageEvent {
    pub request_id: u64,
    pub epoch: u64,
    pub timeline_ns: u64,
    pub node_id: Option<String>,
    pub stage: SourceStageKind,
    pub at: Instant,
    pub actual_pts_ns: Option<i64>,
    pub sequence: Option<u64>,
}


/// Native cursors are selected on a dedicated worker. Upload callbacks never run
/// under its lock, and a request retains its upload position when a callback fails.
pub struct VideoSources {
    sources: HashMap<String, NativeVideoSource>,
    requests: HashMap<u64, RequestProgress>,
    capacity: usize,
    max_surfaces: usize,
    max_bytes: u64,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
    frame_budget: Option<Arc<FrameBudget>>,
    trace_enabled: bool,
    #[cfg(target_os = "macos")]
    worker: Option<precise::Worker>,
}

struct RequestProgress {
    timeline_ns: u64,
    epoch: u64,
    excluded: Vec<String>,
    nodes: Vec<String>,
    selected: Vec<(String, Option<i64>)>,
    #[cfg(target_os = "macos")]
    failure: Option<String>,
    #[cfg(target_os = "macos")]
    ready: Option<Vec<precise::ReadyFrame>>,
}

impl Default for VideoSources {
    fn default() -> Self {
        Self {
            sources: HashMap::new(),
            requests: HashMap::new(),
            capacity: 3,
            max_surfaces: usize::MAX,
            max_bytes: u64::MAX,
            waker: None,
            frame_budget: None,
            trace_enabled: false,
            #[cfg(target_os = "macos")]
            worker: None,
        }
    }
}

pub struct PreparedVideoSource {
    source: Option<NativeVideoSource>,
    info: NativeVideoInfo,
    frame_budget: Option<Arc<FrameBudget>>,
    #[cfg(target_os = "macos")]
    preparation: Option<precise::Preparation>,
}

impl PreparedVideoSource {
    pub fn open_precise(config: NativeVideoConfig, preview_only: bool) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        let source = NativeVideoSource::open_hardware_precise(config, preview_only)?;
        #[cfg(not(target_os = "macos"))]
        let source = NativeVideoSource::open_hardware_required(config, preview_only)?;
        let info = source.info().clone();
        Ok(Self { source: Some(source), info, frame_budget: None,
            #[cfg(target_os = "macos")]
            preparation: None,
        })
    }

    pub fn info(&self) -> &NativeVideoInfo {
        &self.info
    }

    pub fn set_frame_budget(&mut self, budget: Arc<FrameBudget>) -> Result<(), String> {
        if self.frame_budget.as_ref().is_some_and(|current| Arc::ptr_eq(current, &budget)) { return Ok(()); }
        #[cfg(target_os = "macos")]
        if self.preparation.is_some() {
            return Err("Install the prepared source budget before prewarming".to_owned());
        }
        self.frame_budget = Some(budget);
        Ok(())
    }

    /// Starts/polls preparation without selecting on the calling thread. True
    /// means the exact selected surface is cached and ready to transfer at attach.
    pub fn prefetch_at(&mut self, timeline_ns: u64, epoch: u64) -> Result<bool, String> {
        #[cfg(target_os = "macos")]
        {
            if self.preparation.is_none() {
                let Some(worker) = precise::Preparation::pooled_worker(self.frame_budget.clone()) else { return Ok(false); };
                self.preparation = Some(precise::Preparation::new(worker, self.source.take().ok_or("Prepared source was already transferred")?));
            }
            return self.preparation.as_mut().expect("preparation exists").poll(timeline_ns, epoch);
        }
        #[cfg(not(target_os = "macos"))]
        { let _ = (timeline_ns, epoch); Ok(true) }
    }
}

impl VideoSources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach_prepared(&mut self, node_id: &str, mut prepared: PreparedVideoSource) -> NativeVideoInfo {
        self.cancel_prefetch();
        let info = prepared.info.clone();
        #[cfg(target_os = "macos")]
        if let Some(preparation) = prepared.preparation.take() {
            self.sources.remove(node_id);
            let worker = self.worker.get_or_insert_with(|| precise::Worker::new(self.waker.clone(), self.frame_budget.clone()));
            worker.set_trace_enabled(self.trace_enabled);
            worker.attach_preparation(node_id, preparation, info.clone());
            return info;
        }
        let source = prepared.source.take().expect("prepared source owns its native cursor");
        #[cfg(target_os = "macos")]
        if source.is_precise() {
            self.sources.remove(node_id);
            let worker = self.worker.get_or_insert_with(|| precise::Worker::new(self.waker.clone(), self.frame_budget.clone()));
            worker.set_trace_enabled(self.trace_enabled);
            worker.attach(node_id, source);
            return info;
        }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            worker.detach(node_id);
        }
        self.sources.insert(node_id.to_owned(), source);
        info
    }

    pub fn detach(&mut self, node_id: &str) {
        self.cancel_prefetch();
        self.sources.remove(node_id);
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            worker.detach(node_id);
        }
    }

    pub fn retain(&mut self, node_ids: &HashSet<String>) {
        for node_id in self.node_ids() {
            if !node_ids.contains(&node_id) {
                self.detach(&node_id);
            }
        }
    }

    pub fn clear(&mut self) {
        self.cancel_prefetch();
        self.sources.clear();
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            worker.clear();
        }
    }

    pub fn node_ids(&self) -> Vec<String> {
        let mut nodes: Vec<_> = self.sources.keys().cloned().collect();
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            nodes.extend(worker.node_ids());
        }
        nodes
    }

    pub fn set_waker(&mut self, waker: Arc<dyn Fn() + Send + Sync>) {
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            worker.lock().waker = Some(waker.clone());
        }
        self.waker = Some(waker);
    }

    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker { worker.set_trace_enabled(enabled); }
    }

    /// Install the process-wide physical budget before admitting source work.
    pub fn set_frame_budget(&mut self, budget: Arc<FrameBudget>) -> Result<(), String> {
        if self.frame_budget.as_ref().is_some_and(|current| Arc::ptr_eq(current, &budget)) { return Ok(()); }
        if !self.requests.is_empty() || self.prefetch_usage().0 != 0 {
            return Err("Cannot replace the frame budget while native frames are retained".to_owned());
        }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker { worker.lock().frame_budget = Some(budget.clone()); }
        self.frame_budget = Some(budget);
        Ok(())
    }

    pub fn dimensions(&self, node_id: &str) -> Option<(u32, u32)> {
        if let Some(source) = self.sources.get(node_id) { return Some((source.info().width, source.info().height)); }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            return worker.lock().sources.get(node_id).map(|source| (source.info.width, source.info.height));
        }
        None
    }


    /// Limits include a conservative four-byte-per-pixel reservation for each
    /// cursor cache and requested surface. A cancelled executing selection keeps
    /// its reservation until it finishes; it cannot publish into a new generation.
    pub fn set_prefetch_limits(
        &mut self,
        capacity: usize,
        max_surfaces: usize,
        max_bytes: u64,
    ) -> Result<(), String> {
        if capacity == 0 || max_surfaces == 0 || max_bytes == 0 {
            return Err("Video prefetch limits must be nonzero".to_owned());
        }
        if self.requests.len() > capacity {
            return Err("Video prefetch capacity is below admitted requests".to_owned());
        }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            let (surfaces, bytes) = worker.reserved_usage();
            if surfaces > max_surfaces || bytes > max_bytes {
                return Err("Video prefetch limits are below the retained working set".to_owned());
            }
        }
        self.capacity = capacity;
        self.max_surfaces = max_surfaces;
        self.max_bytes = max_bytes;
        Ok(())
    }

    pub fn prefetch_at(
        &mut self,
        request_id: u64,
        timeline_ns: u64,
        epoch: u64,
        excluded: &[String],
    ) -> Result<bool, String> {
        if let Some(request) = self.requests.get(&request_id) {
            if request.timeline_ns != timeline_ns || request.epoch != epoch || request.excluded != excluded {
                return Err("Video prefetch request ID was reused with a different demand".to_owned());
            }
            return Ok(true);
        }
        if self.requests.len() >= self.capacity {
            return Ok(false);
        }
        let mut nodes = self.node_ids();
        nodes.retain(|node| !excluded.contains(node));
        nodes.sort();
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            if !worker.enqueue(request_id, timeline_ns, epoch, &nodes, self.capacity, self.max_surfaces, self.max_bytes)? {
                return Ok(false);
            }
        }
        self.requests.insert(request_id, RequestProgress {
            timeline_ns,
            epoch,
            excluded: excluded.to_vec(),
            nodes,
            selected: Vec::new(),
            #[cfg(target_os = "macos")]
            failure: None,
            #[cfg(target_os = "macos")]
            ready: None,
        });
        Ok(true)
    }

    pub fn cancel_prefetch(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            for (&request_id, request) in self.requests.iter().filter(|_| self.trace_enabled) {
                worker.record_event(SourceStageEvent {
                    request_id, epoch: request.epoch, timeline_ns: request.timeline_ns,
                    node_id: None, stage: SourceStageKind::Cancelled, at: Instant::now(),
                    actual_pts_ns: None, sequence: None,
                });
            }
        }
        self.requests.clear();
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
    }

    pub fn drain_prefetch_events(&mut self) -> Vec<SourceStageEvent> {
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker { return worker.lock().events.drain(..).collect(); }
        Vec::new()
    }

    /// Counts unique retained CVPixelBuffers and their IOSurface allocation bytes,
    /// not AVFrame references. Decoder-private pools and GPU submission retention
    /// are owned/accounted by their respective stages.
    pub fn prefetch_usage(&self) -> (usize, u64) {
        #[cfg(target_os = "macos")]
        {
            let mut surfaces = HashMap::new();
            if let Some(worker) = &self.worker {
                worker.collect_surfaces(&mut surfaces);
            }
            for request in self.requests.values() {
                if let Some(frames) = &request.ready {
                    for frame in frames {
                        surfaces.insert(frame.identity(), frame.bytes());
                    }
                }
            }
            return (surfaces.len(), surfaces.values().copied().fold(0, u64::saturating_add));
        }
        #[cfg(not(target_os = "macos"))]
        (0, 0)
    }

    pub fn prefetch_pending_requests(&self) -> usize {
        self.requests.len()
    }

    pub fn poll_upload_at(
        &mut self,
        request_id: u64,
        upload: impl for<'frame> FnMut(&str, NativeVideoFrame<'frame>, u32, u32) -> Result<bool, String>,
    ) -> Result<Option<Option<i64>>, String> {
        self.poll_upload_selected(request_id, upload).map(|selected| selected.map(|selected| {
            if selected.len() == 1 { selected[0].1 } else { None }
        }))
    }

    fn poll_upload_selected(
        &mut self,
        request_id: u64,
        mut upload: impl for<'frame> FnMut(&str, NativeVideoFrame<'frame>, u32, u32) -> Result<bool, String>,
    ) -> Result<Option<Vec<(String, Option<i64>)>>, String> {
        let request = self.requests.get_mut(&request_id).ok_or("Video prefetch request is not admitted")?;
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            if let Some(error) = &request.failure { return Err(error.clone()); }
            if request.ready.is_none() {
                let mut state = worker.lock();
                match state.results.remove(&request_id) {
                    Some(Ok(frames)) => request.ready = Some(frames),
                    Some(Err(error)) => {
                        request.failure = Some(error.clone());
                        return Err(error);
                    }
                    None => return Ok(None),
                }
            }
        }
        while request.selected.len() < request.nodes.len() {
            let node = &request.nodes[request.selected.len()];
            #[cfg(target_os = "macos")]
            if let Some(frame) = request.ready.as_ref().and_then(|frames| frames.iter().find(|frame| &frame.node == node)) {
                let worker = self.worker.as_ref().expect("precise result requires worker");
                let needed = worker.lock().sources.get(node).is_some_and(|source| source.uploaded_pts != Some(frame.selected.actual_ns));
                if needed {
                    if !upload(node, NativeVideoFrame::VideoToolbox(&frame.selected.frame.frame), frame.width, frame.height)? {
                        return Ok(None);
                    }
                    if let Some(source) = worker.lock().sources.get_mut(node) {
                        source.uploaded_pts = Some(frame.selected.actual_ns);
                        source.metrics.uploaded_frames = source.metrics.uploaded_frames.saturating_add(1);
                    }
                }
                request.selected.push((node.clone(), Some(frame.selected.actual_ns)));
                continue;
            }
            if let Some(source) = self.sources.get_mut(node) {
                let mut blocked = false;
                let result = source.upload_at(request.timeline_ns, request.epoch, &mut |frame, width, height| {
                    if matches!(&frame, NativeVideoFrame::Rgba(_)) {
                        return Err(format!("Hardware-resident video is required for node {node}; CPU RGBA upload is disabled"));
                    }
                    if upload(node, frame, width, height)? {
                        Ok(())
                    } else {
                        blocked = true;
                        Err(String::new())
                    }
                });
                if blocked { return Ok(None); }
                let pts = result?;
                request.selected.push((node.clone(), pts));
            } else {
                return Err(format!("Video node {node} was detached during prefetch"));
            }
        }
        let selected = self.requests.remove(&request_id).expect("admitted request").selected;
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            worker.lock().reservations.remove(&request_id);
        }
        Ok(Some(selected))
    }


    pub fn pause(&mut self) {
        for source in self.sources.values_mut() { source.pause(); }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker { worker.set_active(false); }
    }

    pub fn resume(&mut self) -> Result<(), String> {
        for source in self.sources.values_mut() { source.resume()?; }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker { worker.set_active(true); }
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        if self.sources.values().any(NativeVideoSource::is_active) { return true; }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker { return worker.lock().sources.values().any(|source| source.active); }
        false
    }

    pub fn metrics(&self, node_id: &str) -> Result<NativeVideoMetrics, String> {
        if let Some(source) = self.sources.get(node_id) { return Ok(source.metrics()); }
        #[cfg(target_os = "macos")]
        if let Some(worker) = &self.worker {
            if let Some(source) = worker.lock().sources.get(node_id) { return Ok(source.metrics.clone()); }
        }
        Err(format!("Video node {node_id} is not attached"))
    }

    pub fn frame_counts(&self) -> (u64, u64) {
        self.node_ids().iter().filter_map(|node| self.metrics(node).ok()).fold((0_u64, 0_u64), |(decoded, uploaded), metrics| {
            (decoded.saturating_add(metrics.decoded_frames), uploaded.saturating_add(metrics.uploaded_frames))
        })
    }
}

#[cfg(target_os = "macos")]
mod precise {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Condvar, LazyLock, Mutex, MutexGuard, Weak};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    static PREPARATION_WORKERS: AtomicUsize = AtomicUsize::new(0);
    static NEXT_PREPARATION: AtomicU64 = AtomicU64::new(1);
    static PREPARATION_POOLS: LazyLock<Mutex<HashMap<usize, Weak<Worker>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    pub(super) struct PreparationSlot;
    impl PreparationSlot {
        pub fn try_acquire() -> Option<Self> {
            PREPARATION_WORKERS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| (count < 3).then_some(count + 1)).ok().map(|_| Self)
        }
    }
    impl Drop for PreparationSlot {
        fn drop(&mut self) { PREPARATION_WORKERS.fetch_sub(1, Ordering::AcqRel); }
    }
    pub(super) struct Preparation {
        pub worker: Arc<Worker>,
        pub node: String,
        request_id: u64,
        target: Option<(u64, u64)>,
        ready: bool,
    }
    impl Preparation {
        pub fn pooled_worker(budget: Option<Arc<FrameBudget>>) -> Option<Arc<Worker>> {
            let key = budget.as_ref().map_or(0, |budget| Arc::as_ptr(budget) as usize);
            let mut pools = PREPARATION_POOLS.lock().unwrap_or_else(|error| error.into_inner());
            pools.retain(|_, worker| worker.strong_count() != 0);
            if let Some(worker) = pools.get(&key).and_then(Weak::upgrade) { return Some(worker); }
            let slot = PreparationSlot::try_acquire()?;
            let worker = Arc::new(Worker::new(None, budget));
            worker.lock().preparation_slot = Some(slot);
            pools.insert(key, Arc::downgrade(&worker));
            Some(worker)
        }
        pub fn new(worker: Arc<Worker>, source: NativeVideoSource) -> Self {
            let request_id = NEXT_PREPARATION.fetch_add(1, Ordering::AcqRel);
            let node = format!("prepared-{request_id}");
            worker.attach(&node, source);
            Self { worker, node, request_id, target: None, ready: false }
        }
        pub fn poll(&mut self, timeline_ns: u64, epoch: u64) -> Result<bool, String> {
            if self.target != Some((timeline_ns, epoch)) {
                self.worker.cancel_request(self.request_id);
                if self.target.is_some() {
                    if let Some(source) = self.worker.lock().sources.get_mut(&self.node).and_then(|entry| entry.source.as_mut()) {
                        source.invalidate_exact_cursor();
                    }
                }
                self.request_id = NEXT_PREPARATION.fetch_add(1, Ordering::AcqRel);
                self.ready = false;
                if !self.worker.enqueue(self.request_id, timeline_ns, epoch, std::slice::from_ref(&self.node), 3, usize::MAX, u64::MAX)? { return Ok(false); }
                self.target = Some((timeline_ns, epoch));
            }
            if self.ready { return Ok(true); }
            let mut state = self.worker.lock();
            match state.results.remove(&self.request_id) {
                Some(Ok(frames)) => {
                    state.reservations.remove(&self.request_id);
                    self.ready = true;
                    drop(state);
                    drop(frames);
                    Ok(true)
                }
                Some(Err(error)) => {
                    state.results.insert(self.request_id, Err(error.clone()));
                    Err(error)
                }
                None => Ok(false),
            }
        }
    }
    impl Drop for Preparation {
        fn drop(&mut self) {
            self.worker.cancel_request(self.request_id);
            self.worker.detach(&self.node);
        }
    }

    pub(super) struct ReadyFrame {
        pub node: String,
        pub selected: oq_media::SelectedFrame,
        pub width: u32,
        pub height: u32,
        _credit: Option<FrameCredit>,
    }
    impl ReadyFrame {
        pub fn identity(&self) -> usize { self.selected.frame.frame.pixel_buffer() as usize }
        pub fn bytes(&self) -> u64 { self.selected.frame.frame.allocation_size() }
    }
    pub(super) struct Source {
        source: Option<NativeVideoSource>,
        version: u64,
        pub info: NativeVideoInfo,
        pub metrics: NativeVideoMetrics,
        pub uploaded_pts: Option<i64>,
        pub active: bool,
        pub surface: Option<(usize, u64)>,
        credit: Option<FrameCredit>,
        handoff: Option<Preparation>,
        failure: Option<String>,
    }
    struct Job {
        id: u64,
        generation: u64,
        timeline_ns: u64,
        epoch: u64,
        nodes: Vec<String>,
        credits: Vec<Option<Reservation>>,
        cancelled: Arc<AtomicBool>,
    }
    struct Reservation { credit: FrameCredit, _identity: Box<u8> }
    pub(super) struct State {
        pub sources: HashMap<String, Source>,
        queue: VecDeque<Job>,
        pub results: HashMap<u64, Result<Vec<ReadyFrame>, String>>,
        pub reservations: HashMap<u64, (usize, u64)>,
        generation: u64,
        version: u64,
        running: Option<(u64, u64, (usize, u64))>,
        running_cancelled: Option<Arc<AtomicBool>>,
        in_flight_surfaces: HashMap<usize, u64>,
        stop: bool,
        startup_error: Option<String>,
        pub frame_budget: Option<Arc<FrameBudget>>,
        preparation_slot: Option<PreparationSlot>,
        pub waker: Option<Arc<dyn Fn() + Send + Sync>>,
        pub events: VecDeque<SourceStageEvent>,
        trace_enabled: bool,
    }
    impl State {
        fn record(&mut self, event: SourceStageEvent) {
            if !self.trace_enabled { return; }
            if self.events.len() == 512 { self.events.pop_front(); }
            self.events.push_back(event);
        }
    }
    struct Shared { state: Mutex<State>, ready: Condvar, trace_enabled: AtomicBool }
    pub(super) struct Worker { shared: Arc<Shared> }

    impl Worker {
        pub fn new(waker: Option<Arc<dyn Fn() + Send + Sync>>, frame_budget: Option<Arc<FrameBudget>>) -> Self {
            let shared = Arc::new(Shared { state: Mutex::new(State {
                sources: HashMap::new(), queue: VecDeque::new(), results: HashMap::new(), reservations: HashMap::new(),
                generation: 0, version: 0, running: None, running_cancelled: None, in_flight_surfaces: HashMap::new(), stop: false, startup_error: None, preparation_slot: None, frame_budget, waker, events: VecDeque::new(), trace_enabled: false,
            }), ready: Condvar::new(), trace_enabled: AtomicBool::new(false) });
            let thread_shared = shared.clone();
            if let Err(error) = std::thread::Builder::new().name("oq-source-prefetch".to_owned()).spawn(move || run(thread_shared)) {
                shared.state.lock().unwrap_or_else(|error| error.into_inner()).startup_error =
                    Some(format!("Cannot start native source prefetch worker: {error}"));
            }
            Self { shared }
        }
        pub fn set_trace_enabled(&self, enabled: bool) {
            self.shared.trace_enabled.store(enabled, Ordering::Release);
            let mut state = self.lock();
            state.trace_enabled = enabled;
            if !enabled { state.events.clear(); }
        }
        pub fn record_event(&self, event: SourceStageEvent) { self.lock().record(event); }
        pub fn lock(&self) -> MutexGuard<'_, State> { self.shared.state.lock().unwrap_or_else(|error| error.into_inner()) }
        pub fn attach(&self, node: &str, source: NativeVideoSource) {
            let mut state = self.lock();
            state.version = state.version.wrapping_add(1);
            let version = state.version;
            let removed = state.sources.insert(node.to_owned(), Source {
                info: source.info().clone(), metrics: source.metrics(), active: source.is_active(),
                source: Some(source), version, uploaded_pts: None, surface: None, credit: None, handoff: None, failure: None,
            });
            drop(state);
            drop(removed);
        }
        pub fn attach_preparation(&self, node: &str, preparation: Preparation, info: NativeVideoInfo) {
            let active = preparation.worker.lock().sources.get(&preparation.node).is_some_and(|source| source.active);
            let transferred = preparation.worker.take_prepared(&preparation.node, preparation.request_id);
            let mut state = self.lock();
            state.version = state.version.wrapping_add(1);
            let version = state.version;
            let mut entry = match transferred {
                Ok(Some(entry)) => entry,
                _ => Source {
                    source: None, version, info,
                    metrics: NativeVideoMetrics { decoded_frames: 0, uploaded_frames: 0, cpu_copy_bytes: 0, hardware_decode: true },
                    uploaded_pts: None, active, surface: None, credit: None, handoff: Some(preparation), failure: None,
                },
            };
            entry.version = version;
            let removed = state.sources.insert(node.to_owned(), entry);
            drop(state);
            drop(removed);
        }
        fn take_prepared(&self, node: &str, request_id: u64) -> Result<Option<Source>, String> {
            let mut state = self.lock();
            if state.running.is_some_and(|(id, _, _)| id == request_id) || state.queue.iter().any(|job| job.id == request_id) { return Ok(None); }
            if let Some(Err(error)) = state.results.get(&request_id) { return Err(error.clone()); }
            if let Some(source) = state.sources.get(node) {
                if let Some(error) = &source.failure { return Err(error.clone()); }
                if source.source.is_none() { return Ok(None); }
            }
            Ok(state.sources.remove(node))
        }
        pub fn detach(&self, node: &str) {
            let removed = self.lock().sources.remove(node);
            drop(removed);
        }
        pub fn clear(&self) {
            let removed = std::mem::take(&mut self.lock().sources);
            drop(removed);
        }
        pub fn node_ids(&self) -> Vec<String> { self.lock().sources.keys().cloned().collect() }
        pub fn set_active(&self, active: bool) { for source in self.lock().sources.values_mut() { source.active = active; } }
        pub fn cancel(&self) {
            let mut state = self.lock();
            state.generation = state.generation.wrapping_add(1);
            for entry in state.sources.values_mut() {
                if let Some(source) = &mut entry.source { source.invalidate_exact_cursor(); }
            }
            if let Some(cancelled) = &state.running_cancelled { cancelled.store(true, Ordering::Release); }
            let queue = std::mem::take(&mut state.queue);
            for job in &queue { job.cancelled.store(true, Ordering::Release); }
            let results = std::mem::take(&mut state.results);
            state.reservations.clear();
            drop(state);
            drop((queue, results));
        }
        fn cancel_request(&self, request_id: u64) {
            let mut state = self.lock();
            if state.running.is_some_and(|(id, _, _)| id == request_id) {
                if let Some(cancelled) = &state.running_cancelled { cancelled.store(true, Ordering::Release); }
            }
            let mut queue = std::mem::take(&mut state.queue);
            let mut retained = VecDeque::new();
            let mut removed = Vec::new();
            for job in queue.drain(..) {
                if job.id == request_id { job.cancelled.store(true, Ordering::Release); removed.push(job); }
                else { retained.push_back(job); }
            }
            state.queue = retained;
            let result = state.results.remove(&request_id);
            state.reservations.remove(&request_id);
            drop(state);
            drop((removed, result));
        }
        pub fn reserved_usage(&self) -> (usize, u64) { reserved(&self.lock()) }
        pub fn enqueue(&self, id: u64, timeline_ns: u64, epoch: u64, nodes: &[String], capacity: usize, max_surfaces: usize, max_bytes: u64) -> Result<bool, String> {
            let mut state = self.lock();
            if let Some(error) = &state.startup_error { return Err(error.clone()); }
            let nodes: Vec<_> = nodes.iter().filter(|node| state.sources.contains_key(*node)).cloned().collect();
            let bytes = nodes.iter().fold(0_u64, |total, node| total.saturating_add(source_bytes(&state.sources[node])));
            let (surfaces, retained_bytes) = reserved(&state);
            let stale_running = usize::from(state.running.is_some_and(|(id, generation, _)| generation != state.generation || !state.reservations.contains_key(&id)));
            let (minimum_surfaces, minimum_bytes) = minimum_working_set(&state, &nodes, bytes);
            if let Some(budget) = &state.frame_budget {
                budget.validate_working_set(minimum_surfaces, minimum_bytes)?;
            }
            if minimum_surfaces > max_surfaces || minimum_bytes > max_bytes {
                return Err("Video prefetch limits cannot fit the source cursor working set and one demand".to_owned());
            }
            if state.reservations.len().saturating_add(stale_running) >= capacity || surfaces.saturating_add(nodes.len()) > max_surfaces || retained_bytes.saturating_add(bytes) > max_bytes {
                return Ok(false);
            }
            let mut credits = Vec::with_capacity(nodes.len());
            for node in &nodes {
                let reservation = if let Some(budget) = &state.frame_budget {
                    let identity = Box::new(0_u8);
                    let key = (2, (&*identity as *const u8) as usize);
                    let credit = match budget.try_acquire(key, 1, source_bytes(&state.sources[node])) {
                        Ok(Some(credit)) => credit,
                        Ok(None) => { drop(state); drop(credits); return Ok(false); },
                        Err(error) => { drop(state); drop(credits); return Err(error); },
                    };
                    Some(Reservation { credit, _identity: identity })
                } else { None };
                credits.push(reservation);
            }
            let generation = state.generation;
            state.reservations.insert(id, (nodes.len(), bytes));
            state.queue.push_back(Job { id, generation, timeline_ns, epoch, nodes, credits, cancelled: Arc::new(AtomicBool::new(false)) });
            if state.trace_enabled {
                state.record(SourceStageEvent { request_id: id, epoch, timeline_ns, node_id: None, stage: SourceStageKind::Enqueued, at: Instant::now(), actual_pts_ns: None, sequence: None });
            }
            drop(state);
            self.shared.ready.notify_one();
            Ok(true)
        }
        pub fn collect_surfaces(&self, output: &mut HashMap<usize, u64>) {
            let state = self.lock();
            for source in state.sources.values() {
                if let Some((identity, bytes)) = source.surface { output.insert(identity, bytes); }
                if let Some(preparation) = &source.handoff {
                    if let Some((identity, bytes)) = preparation.worker.lock().sources.get(&preparation.node).and_then(|source| source.surface) {
                        output.insert(identity, bytes);
                    }
                }
            }
            for frames in state.results.values().filter_map(|result| result.as_ref().ok()) {
                for frame in frames { output.insert(frame.identity(), frame.bytes()); }
            }
            output.extend(state.in_flight_surfaces.iter().map(|(&identity, &bytes)| (identity, bytes)));
        }
    }
    impl Drop for Worker {
        fn drop(&mut self) {
            let mut state = self.lock();
            state.stop = true;
            if let Some(cancelled) = &state.running_cancelled { cancelled.store(true, Ordering::Release); }
            let queue = std::mem::take(&mut state.queue);
            for job in &queue { job.cancelled.store(true, Ordering::Release); }
            let results = std::mem::take(&mut state.results);
            drop(state);
            self.shared.ready.notify_one();
            drop((queue, results));
        }
    }
    fn source_bytes(source: &Source) -> u64 {
        // Reserve aligned BGRA storage, never less than an observed allocation.
        let row = u64::from(source.info.width).saturating_add(63) / 64 * 64 * 4;
        let height = u64::from(source.info.height).saturating_add(15) / 16 * 16;
        let estimate = row.saturating_mul(height).saturating_add(16_383) / 16_384 * 16_384;
        estimate.max(source.surface.map_or(0, |(_, bytes)| bytes))
    }
    fn cached_usage(state: &State) -> (usize, u64) {
        state.sources.values().filter_map(|source| source.surface).fold((0_usize, 0_u64), |(count, bytes), (_, allocation)| (count.saturating_add(1), bytes.saturating_add(allocation)))
    }
    fn minimum_working_set(state: &State, nodes: &[String], demand_bytes: u64) -> (usize, u64) {
        let (mut count, mut bytes) = cached_usage(state);
        for node in nodes {
            let source = &state.sources[node];
            if source.surface.is_none() {
                count = count.saturating_add(1);
                bytes = bytes.saturating_add(source_bytes(source));
            }
        }
        (count.saturating_add(nodes.len()), bytes.saturating_add(demand_bytes))
    }
    fn reserved(state: &State) -> (usize, u64) {
        let mut usage = cached_usage(state);
        for &(count, bytes) in state.reservations.values() { usage.0 = usage.0.saturating_add(count); usage.1 = usage.1.saturating_add(bytes); }
        if let Some((id, generation, (count, bytes))) = state.running {
            if generation != state.generation || !state.reservations.contains_key(&id) { usage.0 = usage.0.saturating_add(count); usage.1 = usage.1.saturating_add(bytes); }
        }
        usage
    }
    fn run(shared: Arc<Shared>) {
        loop {
            let job = {
                let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                while !state.stop && state.queue.is_empty() { state = shared.ready.wait(state).unwrap_or_else(|error| error.into_inner()); }
                if state.stop { return; }
                let job = state.queue.pop_front().expect("nonempty source queue");
                state.running = Some((job.id, job.generation, state.reservations[&job.id]));
                state.running_cancelled = Some(job.cancelled.clone());
                job
            };
            let mut frames = Vec::with_capacity(job.nodes.len());
            let mut failure = None;
            for (node, reservation) in job.nodes.iter().zip(job.credits) {
                let work = {
                    let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                    if state.stop || state.generation != job.generation || !state.reservations.contains_key(&job.id) { break; }
                    state.sources.get_mut(node).map(|entry| (entry.source.take(), entry.handoff.take(), entry.version))
                };
                let Some((source, handoff, version)) = work else { failure = Some(format!("Video node {node} was detached during selection")); break; };
                let source = if let Some(source) = source { Some(source) }
                else if let Some(preparation) = handoff {
                    loop {
                        if job.cancelled.load(Ordering::Acquire) {
                            let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                            if let Some(entry) = state.sources.get_mut(node).filter(|entry| entry.version == version) {
                                entry.handoff = Some(preparation);
                            }
                            break None;
                        }
                        match preparation.worker.take_prepared(&preparation.node, preparation.request_id) {
                            Ok(Some(mut prepared)) => {
                                let source = prepared.source.take();
                                let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                                let mut retired_credit = None;
                                if let Some(entry) = state.sources.get_mut(node).filter(|entry| entry.version == version) {
                                    entry.metrics = prepared.metrics;
                                    entry.surface = prepared.surface;
                                    retired_credit = std::mem::replace(&mut entry.credit, prepared.credit.take());
                                }
                                drop(state);
                                drop(retired_credit);
                                break source;
                            }
                            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(1)),
                            Err(error) => {
                                let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                                if let Some(entry) = state.sources.get_mut(node).filter(|entry| entry.version == version) { entry.failure = Some(error.clone()); }
                                failure = Some(error);
                                break None;
                            }
                        }
                    }
                } else { None };
                let Some(mut source) = source else {
                    if failure.is_none() { failure = Some(format!("Video node {node} has no source cursor")); }
                    break;
                };
                let mut reservation = reservation;
                if shared.trace_enabled.load(Ordering::Acquire) {
                    let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                    if state.trace_enabled {
                        state.record(SourceStageEvent {
                            request_id: job.id, epoch: job.epoch, timeline_ns: job.timeline_ns,
                            node_id: Some(node.clone()), stage: SourceStageKind::DecodeStarted,
                            at: Instant::now(), actual_pts_ns: None, sequence: None,
                        });
                    }
                }
                let mut result = source.prepare_exact_at_cancellable(job.timeline_ns, job.epoch, job.cancelled.clone());
                if shared.trace_enabled.load(Ordering::Acquire) {
                    let finished_at = Instant::now();
                    if let Ok(selected) = &result {
                        let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                        if state.trace_enabled {
                            state.record(SourceStageEvent {
                                request_id: job.id, epoch: job.epoch, timeline_ns: job.timeline_ns,
                                node_id: Some(node.clone()), stage: SourceStageKind::DecodeReady,
                                at: finished_at, actual_pts_ns: Some(selected.actual_ns), sequence: Some(selected.frame.sequence),
                            });
                        }
                    }
                }
                if let Ok(selected) = &result {
                    let bytes = selected.frame.frame.allocation_size();
                    let admitted = if bytes == 0 {
                        Err("Native source returned a zero-sized surface".to_owned())
                    } else if let Some(reservation) = &mut reservation {
                        reservation.credit.reassign((0, selected.frame.frame.pixel_buffer() as usize), 1, bytes)
                    } else { Ok(true) };
                    match admitted {
                        Ok(true) => {},
                        Ok(false) => result = Err("Native surface allocation exceeded its reserved physical working set".to_owned()),
                        Err(error) => result = Err(error),
                    }
                }
                // The unique reservation token stays alive until its identity has
                // been converted; allocator address reuse cannot alias live credit.
                let credit = if result.is_ok() { reservation.take().map(|reservation| reservation.credit) } else { None };
                let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                if job.cancelled.load(Ordering::Acquire) || state.generation != job.generation {
                    source.invalidate_exact_cursor();
                }
                let mut retired_credit = None;
                if let Some(entry) = state.sources.get_mut(node).filter(|entry| entry.version == version) {
                    let uploads = entry.metrics.uploaded_frames;
                    entry.metrics = source.metrics();
                    entry.metrics.uploaded_frames = uploads;
                    if let Ok(selected) = &result {
                        entry.surface = Some((selected.frame.frame.pixel_buffer() as usize, selected.frame.frame.allocation_size()));
                        retired_credit = std::mem::replace(&mut entry.credit, credit.clone());
                    }
                    if result.is_ok() || job.cancelled.load(Ordering::Acquire) {
                        if result.is_err() {
                            entry.surface = None;
                            retired_credit = entry.credit.take();
                        }
                        entry.source = Some(source);
                    } else {
                        // A failed physical conversion must not leave an uncredited
                        // decoder cache alive. Fatal selection errors close this cursor.
                        entry.surface = None;
                        retired_credit = entry.credit.take();
                        entry.failure = result.as_ref().err().cloned();
                        drop(state);
                        drop(source);
                        state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                    }
                }
                drop(state);
                drop(retired_credit);
                let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                if state.stop || state.generation != job.generation || !state.reservations.contains_key(&job.id) { break; }
                match result {
                    Ok(selected) => {
                        let frame = ReadyFrame { node: node.clone(), width: selected.frame.frame.width(), height: selected.frame.frame.height(), selected, _credit: credit };
                        state.in_flight_surfaces.insert(frame.identity(), frame.bytes());
                        frames.push(frame);
                    }
                    Err(error) => { failure = Some(error); break; }
                }
            }
            let waker = {
                let mut state = shared.state.lock().unwrap_or_else(|error| error.into_inner());
                state.running = None;
                state.running_cancelled = None;
                state.in_flight_surfaces.clear();
                if !state.stop && state.generation == job.generation && state.reservations.contains_key(&job.id) {
                    state.results.insert(job.id, match failure.take() { Some(error) => Err(error), None => Ok(std::mem::take(&mut frames)) });
                }
                if state.stop { None } else { state.waker.clone() }
            };
            if let Some(waker) = waker { waker(); }
        }
    }
}

#[cfg(test)]
#[path = "sources_coverage_tests.rs"]
mod tests;
