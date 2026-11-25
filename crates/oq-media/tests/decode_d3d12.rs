#![cfg(all(windows, feature = "d3d12"))]

use oq_media::{ErrorKind, PixelFormat, Session};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

#[test]
fn d3d12_decodes_and_seeks_by_source_pts() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/source-time.mp4");
    let mut session = Session::open(fixture).unwrap();
    assert_eq!(session.source_info().start_pts, Some(2000));
    assert!(session.source_info().seekable);
    assert_eq!(session.actual_pipeline().hardware, None);

    let first = session.frame_at_ns(0, 0).unwrap().unwrap();
    assert_eq!(first.actual_ns, 0);
    assert_eq!(first.frame.frame.timestamp().unwrap().pts, 2000);
    assert_eq!(first.frame.frame.format(), PixelFormat::Nv12);
    assert_eq!(session.actual_pipeline().hardware, Some(true));
    assert_eq!(session.actual_pipeline().decoder, "ffmpeg-d3d12va");

    let later = session.frame_at_ns(500_000_000, 0).unwrap().unwrap();
    assert_eq!(later.actual_ns, 500_000_000);
    assert_eq!(later.frame.frame.timestamp().unwrap().pts, 2500);
    assert_eq!(later.frame.source_epoch, first.frame.source_epoch);
    assert!(later.frame.sequence > first.frame.sequence);
    assert_eq!(
        session.frame_at_ns(0, 0).err().unwrap().kind,
        ErrorKind::DiscontinuousTarget
    );

    let cancelled = Arc::new(AtomicBool::new(true));
    assert_eq!(
        session.frame_at_ns_cancellable(0, 1, cancelled).err().unwrap().kind,
        ErrorKind::Cancelled
    );
    let reselected = session.frame_at_ns(0, 1).unwrap().unwrap();
    assert_eq!(reselected.actual_ns, first.actual_ns);
    assert_eq!(reselected.frame.timeline_epoch, 1);
    assert_eq!(first.frame.frame.pts_ns(), Some(0));

    session.reset().unwrap();
    let mut timestamps = Vec::new();
    while let Some(frame) = session.next_frame().unwrap() {
        timestamps.push(frame.frame.timestamp().unwrap().pts);
    }
    // The fixture has a nonzero origin, variable cadence, and reordered B-frames.
    assert_eq!(
        timestamps,
        [
            2000, 2100, 2200, 2400, 2500, 2600, 2700, 2800, 2900, 3100, 3200, 3300, 3400, 3500,
            3600, 3800, 3900, 4000, 4100, 4200, 4300, 4500, 4600, 4700, 4800, 4900, 5000, 5200,
            5300, 5400, 5500, 5600, 5700, 5900,
        ],
    );
}
