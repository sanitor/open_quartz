use super::*;

// Metadata, completed GPU leases and core admissions share one host high-water
// limit. Draining core terminals never opens a second, unbounded frame queue.
fn playback_limits(width: u32, height: u32, fps: Rational) -> Result<(FrameWindowConfig, StableFrameBudget), String> {
    let rgba = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4)).ok_or("Preview extent overflow")?;
    let high = if rgba >= 64 * 1024 * 1024 { 2 } else { 3 };
    let stable_bytes = rgba.checked_mul((high + 2) as u64).ok_or("Stable output extent overflow")?;
    if stable_bytes > 1024 * 1024 * 1024 { return Err("Full-resolution outputs exceed the bounded 1 GiB preview pool".into()); }
    Ok((FrameWindowConfig {
        low_water: 1, high_water: high, prefill: 2,
        max_span_ns: fps.frames_to_ns(high as u64 + 1)?.max(1),
        max_bytes: rgba.saturating_mul(12).clamp(256 * 1024 * 1024, 1024 * 1024 * 1024), max_surfaces: 12,
    }, StableFrameBudget { max_frames: high + 2, max_bytes: stable_bytes }))
}
fn request_limits(request: &PreviewRequest, fps: Rational) -> Result<(FrameWindowConfig, StableFrameBudget), String> {
    let (width, height) = request.sources().fold((request.width, request.height), |(w, h), (_, asset, _)| {
        (w.max(asset.width), h.max(asset.height))
    });
    playback_limits(width, height, fps)
}
pub(super) fn configure_limits(pipeline: &mut FramePipeline, request: &PreviewRequest, fps: Rational) -> Result<FrameWindowConfig, String> {
    let (window, mut stable) = request_limits(request, fps)?;
    // Extent changes preserve outstanding display/UI leases. Retained local
    // capacity is a bounded lifetime high-water, never an unlimited pool.
    let previous = pipeline.stable_output_budget();
    stable.max_frames = stable.max_frames.max(previous.max_frames);
    stable.max_bytes = stable.max_bytes.max(previous.max_bytes);
    pipeline.configure_stable_output_budget(stable)?;
    pipeline.configure_window(window)?;
    Ok(window)
}
struct FrameMetadata { id: u64, frame: u64, deadline_ns: u64 }
impl FrameMetadata {
    fn is_due(&self, elapsed_ns: u64) -> bool { self.deadline_ns <= elapsed_ns }
}
struct ReadyFrame { metadata: FrameMetadata, image: StableFrame, source_pts_ns: Vec<(String, Option<i64>)> }
struct WarmRegion { request: PreviewRequest, end: u64, key: PreviewConfiguration, graph: PreparedNativeGraph }
struct PlaybackSession {
    project: Option<ProjectData>, request: PreviewRequest, end: u64, duration: u64,
    start_frame: u64, next_frame: u64, clock: Option<Instant>, fps: Rational,
    #[cfg(target_os = "macos")]
    audio_clock: Option<Arc<crate::audio_output::AudioClock>>,
    config: FrameWindowConfig, pending: VecDeque<FrameMetadata>, ready: VecDeque<ReadyFrame>,
    warm: Option<WarmRegion>, warm_error: Option<String>, refill: bool,
}
impl PlaybackSession {
    fn new(command: PreviewCommand) -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        let audio_clock = match &command {
            PreviewCommand::Exact(_) => None,
            PreviewCommand::Play(play) => play.audio_clock.clone(),
        };
        let (project, request, end, duration, fps) = match command {
            PreviewCommand::Exact(request) => {
                let end = request.frame.saturating_add(1);
                (None, request, end, end, Rational::new(30, 1)?)
            }
            PreviewCommand::Play(play) => {
                let duration = play.project.duration_frames();
                if duration == 0 { return Err("Add media to the timeline first".into()); }
                let frame = play.start_frame.min(duration - 1);
                let (request, end) = sequence_region(&play.project, play.ticket, play.revision, frame, duration)?;
                let fps = play.project.fps;
                (Some(play.project), request, end, duration, fps)
            }
        };
        let (config, _) = request_limits(&request, fps)?;
        let start_frame = request.frame;
        Ok(Self { project, request, end, duration, start_frame, next_frame: start_frame, clock: None, fps, config,
            #[cfg(target_os = "macos")]
            audio_clock,
            pending: VecDeque::with_capacity(config.high_water), ready: VecDeque::with_capacity(config.high_water), warm: None, warm_error: None, refill: true })
    }
    fn position_at(&self, elapsed_ns: u64) -> u64 {
        self.start_frame.saturating_add(self.fps.ns_to_frames_floor(elapsed_ns).unwrap_or(0)).min(self.duration.saturating_sub(1))
    }
    fn elapsed_ns(&self) -> u64 {
        #[cfg(target_os = "macos")]
        if let Some(clock) = &self.audio_clock { return clock.elapsed_ns(); }
        self.clock.map(|clock| clock.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64).unwrap_or(0)
    }
    fn source_times(&self, frame: u64) -> Result<(u64, BTreeMap<String, u64>), String> {
        let Some(project) = &self.project else {
            return Ok((self.request.source_ns, self.request.sources()
                .map(|(node, _, time)| (node, time)).collect()));
        };
        sequence_source_times(project, &self.request, frame)
    }
    fn occupancy(&self) -> usize { self.pending.len() + self.ready.len() }
    fn skip_to_position(&mut self, position: u64) -> u64 {
        let target = if position >= self.end && self.end < self.duration { self.end }
            else { position.min(self.end.saturating_sub(1)) };
        let skipped = target.saturating_sub(self.next_frame);
        self.next_frame = self.next_frame.max(target);
        skipped
    }
    fn should_refill(&mut self) -> bool {
        if self.occupancy() <= self.config.low_water { self.refill = true; }
        if self.occupancy() >= self.config.high_water { self.refill = false; }
        self.refill
    }
    fn prefill_required(&self) -> usize { self.config.prefill.min((self.end - self.start_frame) as usize) }
    fn prefill_satisfied(&self, ready: usize) -> bool { ready >= self.prefill_required() }
    fn prefilled(&self) -> bool {
        self.prefill_satisfied(self.ready.iter().filter(|frame| frame.image.ready.is_ready()).count())
    }
}
pub(super) fn configuration(request: &PreviewRequest) -> Result<PreviewConfiguration, String> {
    let mut sources = BTreeMap::new();
    for (node, asset, _) in request.sources() {
        if !asset.online {
            let track = request.layers.iter().find(|layer| composition::source_id(&layer.clip_id) == node)
                .map_or("", |layer| layer.track_name.as_str());
            return Err(format!("{track} Media offline: {} — use Relink", asset.name));
        }
        sources.insert(node, (asset.path.clone(), asset.fingerprint.clone()));
    }
    let mut images = BTreeMap::new();
    for layer in &request.layers {
        let image = match &layer.source {
            LayerSource::Video { .. } => continue,
            LayerSource::Title(title) => StaticImage::Title {
                title: title.clone(), width: request.width, height: request.height,
            },
            LayerSource::Still(asset) => {
                if !asset.online {
                    return Err(format!("{} Media offline: {} — use Relink", layer.track_name, asset.name));
                }
                StaticImage::File(asset.clone())
            }
        };
        images.insert(composition::source_id(&layer.clip_id), image);
    }
    Ok(PreviewConfiguration { sources, images, graph: graph(request)? })
}

pub(super) fn prepare_region(
    pipeline: &FramePipeline, configured: &Option<PreviewConfiguration>, request: &PreviewRequest,
    key: &PreviewConfiguration, discontinuity: bool,
) -> Result<(PreparedNativeGraph, u64), String> {
    validate_dimensions(pipeline, request.width, request.height)?;
    let mut prepared = pipeline.prepare_graph(&key.graph)?;
    let (limits, _) = request_limits(request, Rational::new(30, 1)?)?;
    prepared.configure_video_prefetch_limits(limits.max_surfaces, limits.max_bytes / 2)?;
    let mut opened = 0;
    for (node, image) in &key.images {
        let previous = configured.as_ref().and_then(|previous| previous.images.get(node));
        if previous == Some(image) { continue; }
        match image {
            StaticImage::Title { title, width, height } => {
                let rgba = crate::title_raster::rasterize(title, *width, *height)?;
                prepared.stage_image(node, &rgba, *width, *height)?;
            }
            StaticImage::File(asset) => {
                validate_dimensions(pipeline, asset.width, asset.height)?;
                let rgba = crate::media::decode_still(std::path::Path::new(&asset.path))?;
                if rgba.dimensions() != (asset.width, asset.height) {
                    return Err(format!("{} dimensions changed since import", asset.name));
                }
                prepared.stage_image(node, rgba.as_raw(), asset.width, asset.height)?;
            }
        }
    }
    for (node, (path, _)) in &key.sources {
        if (discontinuity && node == SOURCE_ID)
            || configured.as_ref().and_then(|previous| previous.sources.get(node)) != key.sources.get(node) {
            prepared.stage_video(node, NativeVideoConfig { kind: NativeVideoSourceKind::File,
                source: path.clone(), looping: false, playback_rate: 1.0 }, false)?;
            opened += 1;
        }
    }
    Ok((prepared, opened))
}

fn prewarm(graph: &mut PreparedNativeGraph, request: &PreviewRequest, epoch: u64) -> Result<(), String> {
    let times = request.sources().map(|(node, _, time)| (node, time)).collect();
    graph.prewarm_videos_with_source_times(request.source_ns, epoch, &times)?;
    Ok(())
}

fn publish(updates: &Mutex<Option<PreviewUpdate>>, request: &PreviewRequest, epoch: u64, frame: u64, image: Result<StableFrame, String>, source_pts_ns: Vec<(String, Option<i64>)>, cpu_copy_bytes: u64) -> bool {
    let actual_pts_ns = image.as_ref().ok().and_then(|frame| frame.identity.content_stamp.as_ref().and_then(|stamp| stamp.media_pts_ns));
    if let Ok(mut slot) = updates.lock() {
        let replaced = slot.is_some();
        let layer_pts_ns = request.layers.iter().map(|layer| {
            let node = composition::source_id(&layer.clip_id);
            let pts = source_pts_ns.iter().find(|(id, _)| *id == node).and_then(|(_, pts)| *pts);
            (layer.track_name.clone(), pts)
        }).collect();
        *slot = Some(PreviewUpdate { ticket: request.ticket, revision: request.revision, epoch, frame, image, actual_pts_ns, layer_pts_ns, cpu_copy_bytes });
        return replaced;
    }
    false
}
fn worker_step(pipeline: &mut FramePipeline, session: &mut PlaybackSession, configured: &mut Option<PreviewConfiguration>,
    epoch: &mut u64, sequence: &mut u64, stats: &mut PreviewProgress, updates: &Mutex<Option<PreviewUpdate>>) -> Result<bool, String> {
    // Preparation/prewarm cannot commit or mutate the current clip's graph.
    if session.project.is_some() && session.end < session.duration && session.warm.is_none()
        && session.warm_error.is_none()
        && session.next_frame.saturating_add(session.config.high_water as u64) >= session.end {
        let result = (|| {
            let (request, end) = sequence_region(session.project.as_ref().unwrap(), session.request.ticket,
                session.request.revision, session.end, session.duration)?;
            let previous_ns = session.source_times(session.end.saturating_sub(1))?.0;
            let continuous = request.source_ns.checked_sub(previous_ns)
                .is_some_and(|delta| delta <= session.fps.frames_to_ns(1).unwrap_or(0).saturating_add(1));
            let key = configuration(&request)?;
            let (graph, opened) = prepare_region(pipeline, configured, &request, &key, !continuous)?;
            stats.source_opens += opened;
            Ok(WarmRegion { request, end, key, graph })
        })();
        match result {
            Ok(warm) => session.warm = Some(warm),
            Err(error) => session.warm_error = Some(error),
        }
    }
    if let Some(warm) = &mut session.warm {
        if let Err(error) = prewarm(&mut warm.graph, &warm.request, epoch.saturating_add(1)) {
            session.warm_error = Some(error);
            session.warm = None;
        }
    }
    if session.clock.is_some() {
        let position = session.position_at(session.elapsed_ns());
        // Unadmitted past frames have no GPU/source identity to preserve. Skip
        // them rather than rendering an ever-growing backlog after the clock;
        // the final sequence frame always remains an exact required target.
        stats.dropped += session.skip_to_position(position);
    }
    if session.should_refill() {
        while session.occupancy() < session.config.high_water && session.next_frame < session.end {
            let frame = session.next_frame;
            let (source_ns, source_times) = session.source_times(frame)?;
            let deadline_ns = session.fps.frames_to_ns(frame.saturating_sub(session.start_frame))?;
            let id = sequence.saturating_add(1);
            let demand = FrameDemand { request_id: id, graph_revision: pipeline.revision(), output_node_id: OUTPUT_ID.into(), sources: Vec::new(),
                input: FrameEvaluationInput { stamp: FrameStamp { epoch: *epoch, frame: id, timeline_ns: source_ns, deadline_ns },
                    previous_timeline_ns: source_ns, date: [0.0; 4], mouse: [0.0; 4],
                    resolution: [session.request.width as f32, session.request.height as f32, 1.0] } };
            match pipeline.submit_window_with_source_times(demand, &source_times) {
                FrameAdmission::Accepted => {
                    *sequence = id; session.next_frame += 1; stats.submitted += 1;
                    session.pending.push_back(FrameMetadata { id, frame, deadline_ns });
                }
                FrameAdmission::WouldBlock(_) => break,
                FrameAdmission::Rejected { error, .. } => return Err(error),
            }
        }
    }
    stats.queue_peak = stats.queue_peak.max(session.occupancy());
    pipeline.progress();
    while let Some(completion) = pipeline.poll_completion() {
        let metadata = session.pending.pop_front().ok_or("Completion has no bounded host metadata")?;
        if metadata.id != completion.request_id || completion.stamp.epoch != *epoch { return Err("Preview completion identity mismatch".into()); }
        match completion.outcome {
            FrameOutcome::Submitted(image) => session.ready.push_back(ReadyFrame { metadata, image, source_pts_ns: completion.source_pts_ns }),
            FrameOutcome::Failed(error) => return Err(error),
            FrameOutcome::Cancelled => return Err("Active preview frame unexpectedly cancelled".into()),
        }
    }
    stats.ready_peak = stats.ready_peak.max(session.ready.len());
    if session.clock.is_none() && session.prefilled() {
        session.clock = Some(Instant::now());
        #[cfg(target_os = "macos")]
        if let Some(clock) = &session.audio_clock { clock.start(); }
    }
    let elapsed = session.elapsed_ns();
    stats.frame = session.position_at(elapsed); stats.preparing = session.clock.is_none();
    if session.clock.is_some() {
        let mut selected: Option<ReadyFrame> = None;
        // Only ready/due frames in this clip epoch coalesce, never future work.
        while session.ready.front().is_some_and(|frame| frame.image.ready.is_ready() && frame.metadata.is_due(elapsed)) {
            if selected.is_some() { stats.dropped += 1; }
            selected = session.ready.pop_front();
        }
        if let Some(selected) = selected {
            if stats.frame >= session.end && session.end < session.duration {
                stats.dropped += 1;
            } else {
                if elapsed.saturating_sub(selected.metadata.deadline_ns) >= session.fps.frames_to_ns(1)? { stats.late += 1; }
                stats.presented += 1;
                if publish(updates, &session.request, *epoch, selected.metadata.frame, Ok(selected.image), selected.source_pts_ns, stats.cpu_copy_bytes) { stats.coalesced += 1; }
            }
        }
    }
    stats.queue = session.occupancy(); stats.queue_peak = stats.queue_peak.max(stats.queue);
    stats.ready = session.ready.len(); stats.ready_peak = stats.ready_peak.max(stats.ready);
    if session.next_frame == session.end && session.occupancy() == 0 {
        if session.end == session.duration {
            #[cfg(target_os = "macos")]
            if session.audio_clock.as_ref().is_some_and(|clock| !clock.finished()) { return Ok(false); }
            stats.playing = false;
            return Ok(true);
        }
        if session.elapsed_ns() < session.fps.frames_to_ns(session.end.saturating_sub(session.start_frame))? { return Ok(false); }
        if let Some(error) = session.warm_error.take() { return Err(error); }
        if let Some(warm) = session.warm.take() {
            if let Ok(mut slot) = updates.lock() { slot.take(); }
            session.config = configure_limits(pipeline, &warm.request, session.fps)?;
            pipeline.commit_graph(warm.graph)?; *configured = Some(warm.key);
            session.request = warm.request; session.next_frame = session.end; session.end = warm.end;
            *epoch = epoch.saturating_add(1); session.refill = true;
        }
    }
    Ok(false)
}
pub(super) fn run(render: RenderDevice, validation_receiver: mpsc::Receiver<PipelineValidation>, updates: Arc<Mutex<Option<PreviewUpdate>>>,
    latest: Arc<Mutex<Option<PreviewCommand>>>, closed: Arc<AtomicBool>, progress: Arc<Mutex<PreviewProgress>>,
    delivered: Arc<AtomicU64>, coalesced: Arc<AtomicU64>) {
    let mut pipeline = FramePipeline::from_render_device(render, PlaybackCapabilities { data_paths: vec![] });
    let owner = thread::current();
    let wake_result = pipeline.set_waker(Arc::new(move || owner.unpark()));
    let mut current: Option<PlaybackSession> = None;
    let mut configured: Option<PreviewConfiguration> = None;
    let mut epoch = 0u64; let mut sequence = 0u64;
    let mut stats = PreviewProgress::default();
    loop {
        if closed.load(Ordering::Acquire) { break; }
        for validation in validation_receiver.try_iter() {
            let result = (|| { validate_dimensions(&pipeline, validation.width, validation.height)?;
                pipeline.prepare_graph(&pipeline_graph(&validation.pipeline, validation.width, validation.height)?)?; Ok(()) })();
            let _ = validation.result.send(result);
        }
        if let Some(command) = latest.lock().ok().and_then(|mut latest| latest.take()) {
            pipeline.cancel_pending(); while pipeline.poll_completion().is_some() {}
            if let Some(old) = current.take() { stats.dropped += old.occupancy() as u64; }
            if let Ok(mut slot) = updates.lock() { slot.take(); }
            epoch = epoch.saturating_add(1);
            let (ticket, revision, frame) = match &command {
                PreviewCommand::Exact(request) => (request.ticket, request.revision, request.frame),
                PreviewCommand::Play(request) => (request.ticket, request.revision, request.start_frame),
            };
            stats.ticket = ticket; stats.revision = revision; stats.frame = frame; stats.preparing = true;
            stats.queue = 0; stats.ready = 0; stats.playing = matches!(&command, PreviewCommand::Play(_));
            if let Ok(mut progress) = progress.lock() { *progress = stats.clone(); }
            let session = PlaybackSession::new(command).and_then(|session| {
                wake_result.clone()?;
                configure_limits(&mut pipeline, &session.request, session.fps)?;
                stats.source_opens += configure(&mut pipeline, &mut configured, &session.request)?.1;
                stats.width = session.request.width; stats.height = session.request.height; stats.playing = session.project.is_some();
                Ok(session)
            });
            match session {
                Ok(session) => current = Some(session),
                Err(error) => {
                    if let Ok(mut slot) = updates.lock() { *slot = Some(PreviewUpdate { ticket, revision, epoch, frame, image: Err(error), actual_pts_ns: None, layer_pts_ns: Vec::new(), cpu_copy_bytes: 0 }); }
                    stats.playing = false; stats.preparing = false;
                }
            }
        }
        if let Some(session) = &mut current {
            match worker_step(&mut pipeline, session, &mut configured, &mut epoch, &mut sequence, &mut stats, &updates) {
                Ok(true) => { current = None; }
                Ok(false) => {}
                Err(error) => {
                    publish(&updates, &session.request, epoch, stats.frame, Err(error), Vec::new(), stats.cpu_copy_bytes);
                    pipeline.cancel_pending(); while pipeline.poll_completion().is_some() {}
                    current = None; stats.playing = false; stats.preparing = false;
                }
            }
        }
        let (surfaces, bytes) = pipeline.frame_budget_usage(); stats.surfaces = surfaces; stats.bytes = bytes;
        let (peak_surfaces, peak_bytes) = pipeline.backend().frame_budget().peak_usage();
        stats.surfaces_peak = peak_surfaces; stats.bytes_peak = peak_bytes;
        stats.lifetime_peaks = true;
        stats.byte_limit = pipeline.backend().frame_budget().limits().1;
        stats.stable_byte_limit = pipeline.stable_output_budget().max_bytes;
        stats.window_current_capacity = pipeline.window_config().high_water;
        stats.window_high_water = stats.window_high_water.max(stats.window_current_capacity);
        let (stable_surfaces, stable_bytes) = pipeline.stable_output_usage(); stats.stable_surfaces = stable_surfaces; stats.stable_bytes = stable_bytes;
        let runtime = pipeline.backend().runtime_stats(); stats.pipeline_creations = runtime.pipeline_creations; stats.target_allocations = runtime.target_allocations;
        if let Some(key) = &configured {
            let metrics: Vec<_> = key.sources.keys().filter_map(|id| pipeline.video_metrics(id).ok()).collect();
            if !metrics.is_empty() {
                stats.decoded = metrics.iter().map(|value| value.decoded_frames).sum();
                stats.uploaded = metrics.iter().map(|value| value.uploaded_frames).sum();
                stats.cpu_copy_bytes = metrics.iter().map(|value| value.cpu_copy_bytes).sum();
            }
        }
        stats.epoch = epoch; stats.displayed = delivered.load(Ordering::Relaxed);
        let mut published = stats.clone(); published.coalesced += coalesced.load(Ordering::Relaxed);
        if let Ok(mut progress) = progress.lock() { *progress = published; }
        // Commands and native/GPU/budget callbacks wake this same parked owner.
        let wait = current.as_ref().map_or(Duration::from_millis(50), |session| {
            if session.pending.is_empty() {
                if let Some(frame) = session.ready.front().filter(|frame| frame.image.ready.is_ready()) {
                    return Duration::from_nanos(frame.metadata.deadline_ns.saturating_sub(session.elapsed_ns()))
                        .clamp(Duration::from_micros(100), Duration::from_millis(50));
                }
            }
            // GPU callbacks need device progress; source/command/budget wakes
            // interrupt this bound rather than waiting for the UI timer.
            Duration::from_millis(2)
        });
        thread::park_timeout(wait);
    }
    let _ = pipeline.dispose();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> PlaybackSession {
        let mut session = PlaybackSession::new(PreviewCommand::Exact(PreviewRequest {
            ticket: 7, revision: 9, frame: 10, source_ns: 9_000_000_000,
            width: 64, height: 64, asset: None, transform: ClipTransform::default(), pipeline: ClipPipeline::default(), layers: Vec::new(),
        })).unwrap();
        session.project = Some(ProjectData::default());
        session.duration = 120; session.end = 120;
        session
    }
    #[test]
    fn prefill_waits_for_ready_outputs_and_short_exact_target_needs_only_one() {
        let mut session = session();
        assert!(!session.prefill_satisfied(0));
        assert!(!session.prefill_satisfied(1));
        assert!(session.prefill_satisfied(2));
        session.end = session.start_frame + 1;
        assert_eq!(session.prefill_required(), 1);
        assert!(session.prefill_satisfied(1));
    }
    #[test]
    fn rational_sequence_deadline_is_separate_from_media_pts() {
        let mut session = session();
        session.fps = Rational::new(30_000, 1001).unwrap();
        assert_eq!(session.position_at(1_001_000_000), 40);
        let frame = FrameMetadata { id: 1, frame: 11, deadline_ns: session.fps.frames_to_ns(1).unwrap() };
        assert_eq!(frame.deadline_ns, 33_366_666);
        assert!(!frame.is_due(frame.deadline_ns - 1));
        assert!(frame.is_due(frame.deadline_ns));
        assert_eq!(session.request.source_ns, 9_000_000_000);
    }
    #[test]
    fn host_backpressure_retains_metadata_until_consumed() {
        let mut session = session();
        for id in 0..session.config.high_water {
            session.pending.push_back(FrameMetadata { id: id as u64, frame: id as u64, deadline_ns: 0 });
        }
        assert_eq!(session.occupancy(), session.config.high_water);
        assert!(!session.should_refill());
        session.pending.pop_front();
        assert!(!session.should_refill(), "hysteresis must retain the low-water boundary");
        while session.pending.len() > session.config.low_water { session.pending.pop_front(); }
        assert!(session.should_refill());
    }
    #[test]
    fn late_clock_skips_only_unadmitted_frames_and_preserves_exact_final_target() {
        let mut session = session();
        session.pending.push_back(FrameMetadata { id: 1, frame: 10, deadline_ns: 0 });
        session.next_frame = 11;
        assert_eq!(session.skip_to_position(90), 79);
        assert_eq!(session.pending.front().unwrap().frame, 10);
        assert_eq!(session.skip_to_position(120), 29);
        assert_eq!(session.next_frame, 119, "last sequence frame cannot be skipped");
        session.end = 100; session.next_frame = 95;
        assert_eq!(session.skip_to_position(110), 5, "expired clip work cannot cross a cut");
        assert_eq!(session.next_frame, 100);
    }
    #[test]
    fn high_resolution_limits_are_explicit_and_bounded_without_downscaling() {
        let (window, stable) = playback_limits(7680, 4320, Rational::new(30, 1).unwrap()).unwrap();
        assert_eq!(window.high_water, 2);
        assert_eq!(window.prefill, 2);
        assert_eq!(stable.max_frames, 4);
        assert_eq!(stable.max_bytes, 7680 * 4320 * 4 * 4);
        assert!(window.max_bytes <= 1024 * 1024 * 1024);
        assert!(playback_limits(32_768, 32_768, Rational::new(30, 1).unwrap()).is_err());
    }
}
