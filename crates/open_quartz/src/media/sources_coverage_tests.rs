use super::*;
use crate::media::native::NativeVideoSourceKind;
#[cfg(any(target_os = "macos", all(windows, feature = "native-video-d3d12")))]
use std::time::{Duration, Instant};

#[test]
fn camera_preparation_rejects_cpu_fallback() {
    for preview in [false, true] {
        let error = PreparedVideoSource::open_precise(
            NativeVideoConfig {
                kind: NativeVideoSourceKind::Camera,
                source: "__fail_reader_spawn__".to_owned(),
                looping: false,
                playback_rate: 1.0,
            },
            preview,
        )
        .err()
        .expect("camera preparation must reject CPU fallback");
        assert!(error.contains("camera capture"), "{error}");
        assert!(
            error.contains("CPU decode/upload fallback is disabled"),
            "{error}"
        );
    }
}

#[cfg(not(any(target_os = "macos", all(windows, feature = "native-video-d3d12"))))]
#[test]
fn prepared_file_rejects_unsupported_backend_before_file_access() {
    let error = PreparedVideoSource::open_precise(
        NativeVideoConfig {
            kind: NativeVideoSourceKind::File,
            source: "missing".to_owned(),
            looping: false,
            playback_rate: 1.0,
        },
        true,
    )
    .err()
    .expect("unsupported backend must reject before file access");
    assert!(
        error.contains("no hardware-resident video decoder"),
        "{error}"
    );
}

#[cfg(any(target_os = "macos", all(windows, feature = "native-video-d3d12")))]
#[test]
fn hardware_source_upload_retries_and_tracks_lifecycle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/white_64x64.mp4");
    assert!(
        path.is_file(),
        "hardware video fixture is a required test prerequisite"
    );
    let config = NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: false,
        playback_rate: 1.0,
    };
    let mut sources = VideoSources::new();
    let prepared = PreparedVideoSource::open_precise(config.clone(), true)
        .expect("fixture must open through the real hardware backend, never software fallback");
    let info = sources.attach_prepared("video", prepared);
    assert_eq!((info.width, info.height), (64, 64));
    assert!(info.fallback_reason.is_none());
    #[cfg(target_os = "macos")]
    assert_eq!(info.decoder, "ffmpeg-videotoolbox");
    #[cfg(all(windows, feature = "native-video-d3d12"))]
    assert_eq!(info.decoder, "ffmpeg-d3d12va");
    let deadline = Instant::now() + Duration::from_secs(5);
    assert!(sources.prefetch_at(1, 0, 0, &[]).unwrap());
    loop {
        let result =
            sources.poll_upload_at(1, |_, _, _, _| Err("retry upload".to_owned()));
        if let Err(error) = result {
            assert_eq!(error, "retry upload");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hardware preview frame did not arrive"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(sources.metrics("video").unwrap().uploaded_frames, 0);
    let dirty = sources
        .poll_upload_at(1, |node_id, frame, width, height| {
            assert_eq!(node_id, "video");
            assert_eq!((width, height), (64, 64));
            match frame {
                NativeVideoFrame::Rgba(_) => panic!("production source delivered a CPU frame"),
                #[cfg(target_os = "macos")]
                NativeVideoFrame::VideoToolbox(frame) => {
                    assert_eq!((frame.width(), frame.height()), (width, height));
                    assert!(!frame.pixel_buffer().is_null());
                }
                #[cfg(all(windows, feature = "native-video-d3d12"))]
                NativeVideoFrame::D3d12(frame) => {
                    assert_eq!((frame.width, frame.height), (width, height))
                }
            }
            Ok(true)
        })
        .unwrap();
    assert_eq!(dirty, Some(Some(0)));
    let metrics = sources.metrics("video").unwrap();
    assert_eq!(metrics.uploaded_frames, 1);
    assert_eq!(metrics.cpu_copy_bytes, 0);
    assert!(metrics.hardware_decode);
    assert!(sources.prefetch_at(2, 0, 0, &[]).unwrap());
    loop {
        if sources.poll_upload_at(2, |_, _, _, _| panic!("unexpected second source import")).unwrap().is_some() { break; }
        assert!(Instant::now() < deadline, "repeated source request did not complete");
        std::thread::yield_now();
    }
    sources.pause();
    sources.retain(&HashSet::new());
    assert!(sources.node_ids().is_empty());
    assert!(!sources.is_active());
    assert_eq!(sources.frame_counts(), (0, 0));
    assert!(sources.metrics("video").is_err());
    sources.attach_prepared(
        "video",
        PreparedVideoSource::open_precise(config, true).unwrap(),
    );
    assert_eq!(sources.node_ids(), ["video"]);
    sources.clear();
    assert!(sources.node_ids().is_empty());
    assert!(!sources.is_active());
    assert!(sources.metrics("video").is_err());
}

#[test]
fn prefetch_admission_is_bounded_idempotent_and_cancelled() {
    let mut sources = VideoSources::new();
    assert!(sources.set_prefetch_limits(0, 1, 1).is_err());
    assert!(sources.set_prefetch_limits(1, 0, 1).is_err());
    sources.set_prefetch_limits(1, 1, 1).unwrap();
    assert!(sources.prefetch_at(7, 0, 0, &[]).unwrap());
    assert!(sources.prefetch_at(7, 0, 0, &[]).unwrap());
    assert!(sources.prefetch_at(7, 1, 0, &[]).is_err());
    assert!(!sources.prefetch_at(8, 0, 0, &[]).unwrap());
    assert_eq!(sources.poll_upload_at(7, |_, _, _, _| panic!("no source")).unwrap(), Some(None));
    assert!(sources.prefetch_at(8, 0, 0, &[]).unwrap());
    sources.cancel_prefetch();
    assert!(sources.poll_upload_at(8, |_, _, _, _| Ok(true)).is_err());
    assert!(sources.prefetch_at(8, 0, 1, &[]).unwrap());
    assert_eq!(sources.prefetch_usage(), (0, 0));
}

#[cfg(target_os = "macos")]
fn fixture_source() -> PreparedVideoSource {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/white_64x64.mp4");
    PreparedVideoSource::open_precise(NativeVideoConfig {
        kind: NativeVideoSourceKind::File,
        source: path.to_string_lossy().into_owned(),
        looping: true,
        playback_rate: 1.0,
    }, true).expect("real VideoToolbox fixture source")
}

#[cfg(target_os = "macos")]
fn poll_until_ready(
    sources: &mut VideoSources,
    request: u64,
    mut upload: impl for<'frame> FnMut(&str, NativeVideoFrame<'frame>, u32, u32) -> Result<bool, String>,
) -> Option<i64> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(pts) = sources.poll_upload_at(request, &mut upload).unwrap() { return pts; }
        assert!(Instant::now() < deadline, "source request did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(target_os = "macos")]
#[test]
fn real_prefetch_preserves_partial_upload_credit_and_surface_reuse() {
    let mut sources = VideoSources::new();
    sources.attach_prepared("a", fixture_source());
    sources.attach_prepared("b", fixture_source());
    sources.set_prefetch_limits(2, 6, 6 * 64 * 64 * 4).unwrap();
    assert!(sources.prefetch_at(1, 0, 0, &[]).unwrap());
    assert!(sources.prefetch_at(2, 0, 0, &[]).unwrap());
    assert!(!sources.prefetch_at(3, 0, 0, &[]).unwrap());
    let mut a_uploads = 0;
    let mut b_attempts = 0;
    let mut native_bytes = HashMap::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(sources.poll_upload_at(1, |node, frame, _, _| {
            let NativeVideoFrame::VideoToolbox(frame) = frame else { panic!("CPU fallback"); };
            native_bytes.insert(frame.pixel_buffer() as usize, frame.allocation_size());
            if node == "a" { a_uploads += 1; Ok(true) }
            else { b_attempts += 1; Ok(false) }
        }).unwrap().is_none());
        if b_attempts == 2 { break; }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(a_uploads, 1, "successful prefix must not upload twice");
    assert_eq!(sources.metrics("a").unwrap().uploaded_frames, 1);
    assert_eq!(sources.metrics("b").unwrap().uploaded_frames, 0);
    assert_eq!(poll_until_ready(&mut sources, 1, |node, _, _, _| {
        assert_eq!(node, "b"); Ok(true)
    }), None);
    poll_until_ready(&mut sources, 2, |_, _, _, _| panic!("repeat target must reuse imported surface"));
    let (surfaces, bytes) = sources.prefetch_usage();
    assert_eq!(surfaces, 2, "cursor leases for repeat targets are the same physical surfaces");
    assert_eq!(bytes, native_bytes.values().copied().sum::<u64>());
    assert_eq!(sources.frame_counts().1, 2);
    assert_eq!(sources.metrics("a").unwrap().cpu_copy_bytes, 0);
}

#[cfg(target_os = "macos")]
#[test]
fn real_prefetch_forward_epoch_backseek_and_cancelled_final_request() {
    use std::sync::atomic::{AtomicU64, Ordering};
    let mut sources = VideoSources::new();
    sources.set_trace_enabled(true);
    let wakes = Arc::new(AtomicU64::new(0));
    let notified = wakes.clone();
    sources.set_waker(Arc::new(move || { notified.fetch_add(1, Ordering::AcqRel); }));
    sources.attach_prepared("video", fixture_source());
    sources.set_prefetch_limits(3, 4, 4 * 64 * 64 * 4).unwrap();
    assert!(sources.prefetch_at(1, 0, 0, &[]).unwrap());
    assert_eq!(poll_until_ready(&mut sources, 1, |_, _, _, _| Ok(true)), Some(0));
    let decoded = sources.metrics("video").unwrap().decoded_frames;
    assert!(sources.prefetch_at(2, 100_000_000, 0, &[]).unwrap());
    let later = poll_until_ready(&mut sources, 2, |_, _, _, _| Ok(true)).unwrap();
    assert!(later > 0);
    assert!(sources.metrics("video").unwrap().decoded_frames > decoded);
    for epoch in 1..40 {
        sources.cancel_prefetch();
        let _ = sources.prefetch_at(9, 100_000_000, epoch, &[]).unwrap();
    }
    sources.cancel_prefetch();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !sources.prefetch_at(9, 0, 40, &[]).unwrap() {
        assert!(Instant::now() < deadline, "cancelled work starved the final demand");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(poll_until_ready(&mut sources, 9, |_, _, _, _| Ok(true)), Some(0));
    assert!(wakes.load(Ordering::Acquire) > 0);
    assert!(sources.prefetch_usage().0 <= 4);
    assert_eq!(sources.prefetch_pending_requests(), 0);
    assert_eq!(sources.metrics("video").unwrap().cpu_copy_bytes, 0);
    let events = sources.drain_prefetch_events();
    assert!(events.len() <= 512);
    let started = events.iter().find(|event| event.request_id == 9 && event.epoch == 40 && event.stage == SourceStageKind::DecodeStarted).unwrap();
    let ready = events.iter().find(|event| event.request_id == 9 && event.epoch == 40 && event.stage == SourceStageKind::DecodeReady).unwrap();
    assert!(started.at <= ready.at);
    assert_eq!(ready.actual_pts_ns, Some(0));
    assert!(ready.sequence.is_some());
    assert!(events.iter().any(|event| event.stage == SourceStageKind::Cancelled));
    assert!(sources.drain_prefetch_events().is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn shared_budget_deduplicates_native_upload_retention_until_last_owner() {
    let budget = FrameBudget::new(8, 8 * 64 * 64 * 4).unwrap();
    let mut sources = VideoSources::new();
    sources.set_frame_budget(budget.clone()).unwrap();
    sources.attach_prepared("video", fixture_source());
    assert_eq!(sources.dimensions("video"), Some((64, 64)));
    assert!(sources.prefetch_at(1, 0, 0, &[]).unwrap());
    let mut retained = None;
    poll_until_ready(&mut sources, 1, |_, frame, _, _| {
        let NativeVideoFrame::VideoToolbox(frame) = frame else { panic!("CPU fallback"); };
        retained = Some(budget.try_acquire((0, frame.pixel_buffer() as usize), 1, frame.allocation_size()).unwrap().unwrap());
        Ok(true)
    });
    assert_eq!(budget.usage(), sources.prefetch_usage(), "source and upload share physical credit");
    sources.clear();
    assert_eq!(budget.usage().0, 1, "upload owner retains native credit after detach");
    drop(retained);
    assert_eq!(budget.usage(), (0, 0));
}

#[cfg(target_os = "macos")]
#[test]
fn prepared_cut_prewarm_transfers_the_same_cursor_and_selected_surface() {
    let budget = FrameBudget::new(12, 12 * 64 * 64 * 4).unwrap();
    let mut active = VideoSources::new();
    active.set_frame_budget(budget.clone()).unwrap();
    active.attach_prepared("a", fixture_source());
    assert!(active.prefetch_at(1, 0, 0, &[]).unwrap());
    poll_until_ready(&mut active, 1, |_, _, _, _| Ok(true));
    let mut upcoming = fixture_source();
    upcoming.set_frame_budget(budget.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request = 2;
    while !upcoming.prefetch_at(0, 0).unwrap() {
        // The current material remains usable while the next cursor selects on
        // its own bounded preparation worker.
        assert!(active.prefetch_at(request, 0, 0, &[]).unwrap());
        poll_until_ready(&mut active, request, |_, _, _, _| panic!("active repeat must reuse surface"));
        request += 1;
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let (surface, decoded) = {
        let preparation = upcoming.preparation.as_ref().unwrap();
        let state = preparation.worker.lock();
        let source = &state.sources[&preparation.node];
        (source.surface.unwrap().0, source.metrics.decoded_frames)
    };
    active.clear();
    active.attach_prepared("b", upcoming);
    assert!(active.prefetch_at(request, 0, 0, &[]).unwrap());
    assert_eq!(poll_until_ready(&mut active, request, |_, frame, _, _| {
        let NativeVideoFrame::VideoToolbox(frame) = frame else { panic!("CPU fallback"); };
        assert_eq!(frame.pixel_buffer() as usize, surface, "cut uses prewarmed physical surface");
        Ok(true)
    }), Some(0));
    assert_eq!(active.metrics("b").unwrap().decoded_frames, decoded, "first demand must not decode or seek again");
    active.clear();
    assert_eq!(budget.usage(), (0, 0));
}

#[cfg(target_os = "macos")]
#[test]
fn shared_source_budget_bounds_two_sessions_before_decode_and_rejects_impossible_minimum() {
    let impossible = FrameBudget::new(1, 64 * 64 * 4).unwrap();
    let mut oversized = VideoSources::new();
    oversized.set_frame_budget(impossible.clone()).unwrap();
    oversized.attach_prepared("video", fixture_source());
    assert!(oversized.prefetch_at(1, 0, 0, &[]).is_err());
    assert_eq!(oversized.frame_counts(), (0, 0));
    assert_eq!(impossible.usage(), (0, 0));

    let budget = FrameBudget::new(4, 4 * 64 * 64 * 4).unwrap();
    let mut a = VideoSources::new();
    let mut b = VideoSources::new();
    for sources in [&mut a, &mut b] {
        sources.set_frame_budget(budget.clone()).unwrap();
        sources.attach_prepared("video", fixture_source());
        assert!(sources.prefetch_at(1, 0, 0, &[]).unwrap());
        assert!(sources.prefetch_at(2, 100_000_000, 0, &[]).unwrap());
    }
    assert!(!a.prefetch_at(3, 200_000_000, 0, &[]).unwrap());
    assert!(budget.usage().0 <= 4);
    assert!(budget.usage().1 <= 4 * 64 * 64 * 4);
    for sources in [&mut a, &mut b] {
        poll_until_ready(sources, 1, |_, _, _, _| Ok(true));
        poll_until_ready(sources, 2, |_, _, _, _| Ok(true));
    }
    a.clear();
    b.clear();
    assert_eq!(budget.usage(), (0, 0));
}

#[cfg(target_os = "macos")]
#[test]
fn more_prepared_materials_than_worker_slots_all_become_ready() {
    let budget = FrameBudget::new(16, 16 * 64 * 64 * 4).unwrap();
    let mut prepared: Vec<_> = (0..4).map(|_| {
        let mut source = fixture_source();
        source.set_frame_budget(budget.clone()).unwrap();
        source
    }).collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut ready = true;
        for source in &mut prepared {
            ready &= source.prefetch_at(0, 0).unwrap();
        }
        if ready { break; }
        assert!(Instant::now() < deadline, "ready prepared cursors must release request slots for later materials");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(budget.usage().0, 4);
    drop(prepared);
    assert_eq!(budget.usage(), (0, 0));
}


#[cfg(target_os = "macos")]
#[test]
fn cancelled_speculative_target_backseeks_without_changing_composition_epoch() {
    let mut sources = VideoSources::new();
    sources.attach_prepared("video", fixture_source());
    assert!(sources.prefetch_at(1, 100_000_000, 0, &[]).unwrap());
    assert!(poll_until_ready(&mut sources, 1, |_, _, _, _| Ok(true)).unwrap() > 0);
    sources.cancel_prefetch();
    assert!(sources.prefetch_at(2, 0, 0, &[]).unwrap());
    assert_eq!(poll_until_ready(&mut sources, 2, |_, _, _, _| Ok(true)), Some(0));
    sources.cancel_prefetch();
    assert!(sources.prefetch_at(3, 0, 0, &[]).unwrap());
    assert_eq!(poll_until_ready(&mut sources, 3, |_, _, _, _| panic!("same presented PTS must stay imported")), Some(0));
}

#[cfg(target_os = "macos")]
#[test]
fn pending_prepared_handoff_can_cancel_and_complete_a_new_demand() {
    let budget = FrameBudget::new(12, 12 * 64 * 64 * 4).unwrap();
    let mut prepared = fixture_source();
    prepared.set_frame_budget(budget.clone()).unwrap();
    let _ = prepared.prefetch_at(100_000_000, 0).unwrap();
    let mut sources = VideoSources::new();
    sources.set_frame_budget(budget).unwrap();
    sources.attach_prepared("video", prepared);
    assert!(sources.prefetch_at(1, 100_000_000, 0, &[]).unwrap());
    sources.cancel_prefetch();
    assert!(sources.prefetch_at(2, 0, 1, &[]).unwrap());
    assert_eq!(poll_until_ready(&mut sources, 2, |_, _, _, _| Ok(true)), Some(0));
}
