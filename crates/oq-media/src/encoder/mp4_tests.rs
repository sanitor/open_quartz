use super::*;
use std::{
    io::{self, Seek, SeekFrom, Write},
    sync::atomic::AtomicBool,
    time::Instant,
};

fn config() -> EncoderConfig {
    EncoderConfig {
        width: 64,
        height: 64,
        time_base: RationalTimeBase {
            numerator: 1001,
            denominator: 30000,
        },
        bitrate: 300_000,
        queue_capacity: 4,
        max_b_frames: 2,
        overwrite: true,
    }
}
fn until<T>(mut action: impl FnMut() -> Option<T>) -> T {
    let limit = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(value) = action() {
            return value;
        }
        assert!(Instant::now() < limit, "hardware encoder stalled");
        thread::sleep(Duration::from_millis(1));
    }
}
fn drain(encoder: &mut Mp4Encoder) -> EncoderReport {
    until(|| encoder.finish().unwrap().then_some(()));
    until(|| match encoder.poll().unwrap() {
        EncoderStatus::ReadyToCommit(report) => Some(report),
        EncoderStatus::Failed(error) => panic!("{error}"),
        _ => None,
    })
}

struct FailWriter {
    file: File,
    remaining: usize,
}
impl Write for FailWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::from_raw_os_error(28));
        }
        let size = self.file.write(&data[..data.len().min(self.remaining)])?;
        self.remaining -= size;
        Ok(size)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
impl Seek for FailWriter {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

#[test]
fn real_mux_disk_full_at_header_packet_or_trailer_never_replaces_target() {
    for allowance in [0, 48, 160] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("protected.mp4");
        fs::write(&path, b"original").unwrap();
        let opened = Mp4Encoder::open_with_writer(&path, config(), move |file| FailWriter {
            file,
            remaining: allowance,
        });
        if let Ok(mut encoder) = opened {
            let surface = EncoderSurface::new(64, 64).unwrap();
            until(|| {
                match encoder.submit(EncodeFrame {
                    surface: surface.clone(),
                    pts: 0,
                    duration: 1,
                }) {
                    Ok(EncodeAdmission::Accepted) => Some(()),
                    Ok(EncodeAdmission::WouldBlock(_)) => None,
                    Err(_) => Some(()),
                }
            });
            let _ = encoder.finish();
            until(|| match encoder.poll().unwrap() {
                EncoderStatus::Failed(_) => Some(()),
                EncoderStatus::ReadyToCommit(_) => panic!("disk full must fail before commit"),
                _ => None,
            });
            drop(encoder);
        }
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(
            fs::read_dir(directory.path()).unwrap().count(),
            1,
            "failed writer must remove reserved temporary"
        );
    }
}

#[test]
fn reorder_small_input_credit_pool_drains_all_pts_and_releases_surface_owners() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reorder.mp4");
    let mut encoder = Mp4Encoder::open(&path, config()).unwrap();
    let wake = Arc::new(AtomicBool::new(false));
    let notified = wake.clone();
    encoder.set_waker(Arc::new(move || {
        notified.store(true, Ordering::Release);
    }));
    let surface = EncoderSurface::new(64, 64).unwrap();
    let mut blocked = false;
    for pts in 0..24 {
        until(|| {
            match encoder
                .submit(EncodeFrame {
                    surface: surface.clone(),
                    pts,
                    duration: 1,
                })
                .unwrap()
            {
                EncodeAdmission::Accepted => Some(()),
                EncodeAdmission::WouldBlock(_) => {
                    blocked = true;
                    None
                }
            }
        });
        assert!(encoder.in_flight() <= 4);
    }
    let report = drain(&mut encoder);
    assert_eq!(report.input_frames, 24);
    assert_eq!(report.packets, 24);
    assert_eq!(
        encoder.in_flight(),
        0,
        "drain must release all native source AVBuffer references"
    );
    assert!(wake.load(Ordering::Acquire));
    encoder.commit().unwrap();
    until(|| match encoder.poll().unwrap() {
        EncoderStatus::Completed(_) => Some(()),
        EncoderStatus::Failed(e) => panic!("{e}"),
        _ => None,
    });
    let mut input = ffmpeg_next::format::input(&path).unwrap();
    let stream = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .unwrap();
    let id = stream.index();
    let tb = stream.time_base();
    let mut pts = Vec::new();
    let mut dts = Vec::new();
    for (stream, packet) in input.packets() {
        if stream.index() == id {
            pts.push(
                checked_rescale(
                    packet.pts().unwrap(),
                    RationalTimeBase {
                        numerator: tb.0,
                        denominator: tb.1,
                    },
                    config().time_base,
                )
                .unwrap(),
            );
            dts.push(packet.dts().unwrap());
        }
    }
    pts.sort_unstable();
    assert_eq!(pts, (0..24).collect::<Vec<_>>());
    assert!(dts.windows(2).all(|pair| pair[0] < pair[1]));
    eprintln!(
        "small-pool native reorder: blocked={blocked}, packets={}",
        report.packets
    );
    let artifacts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-results/phase5");
    fs::create_dir_all(&artifacts).unwrap();
    fs::copy(&path, artifacts.join("reordered-export.mp4")).unwrap();
}

#[test]
fn cancellation_after_drain_and_target_race_preserve_winner() {
    for cancel in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("target.mp4");
        let mut settings = config();
        settings.overwrite = false;
        let mut encoder = Mp4Encoder::open(&path, settings).unwrap();
        encoder
            .submit(EncodeFrame {
                surface: EncoderSurface::new(64, 64).unwrap(),
                pts: 0,
                duration: 1,
            })
            .unwrap();
        drain(&mut encoder);
        fs::write(&path, b"other-writer").unwrap();
        if cancel {
            encoder.cancel();
        } else {
            encoder.commit().unwrap();
        }
        until(|| match encoder.poll().unwrap() {
            EncoderStatus::Cancelled if cancel => Some(()),
            EncoderStatus::Failed(_) if !cancel => Some(()),
            EncoderStatus::Completed(_) => panic!("must not overwrite competing target"),
            _ => None,
        });
        drop(encoder);
        assert_eq!(fs::read(&path).unwrap(), b"other-writer");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}

#[test]
fn invalid_timestamps_do_not_advance_admission_and_cancel_commit_has_one_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("race.mp4");
    let mut settings = config();
    settings.max_b_frames = 0;
    settings.queue_capacity = 2;
    let mut encoder = Mp4Encoder::open(&path, settings).unwrap();
    let surface = EncoderSurface::new(64, 64).unwrap();
    for (pts, duration) in [(-1, 1), (0, 0), (i64::MAX, 1)] {
        assert!(encoder
            .submit(EncodeFrame {
                surface: surface.clone(),
                pts,
                duration
            })
            .is_err());
        assert_eq!(encoder.in_flight(), 0);
    }
    assert!(matches!(
        encoder
            .submit(EncodeFrame {
                surface: surface.clone(),
                pts: 0,
                duration: 1
            })
            .unwrap(),
        EncodeAdmission::Accepted
    ));
    assert!(encoder
        .submit(EncodeFrame {
            surface,
            pts: 0,
            duration: 1
        })
        .is_err());
    drain(&mut encoder);
    assert!(encoder.commit().unwrap());
    encoder.cancel();
    let committed = until(|| match encoder.poll().unwrap() {
        EncoderStatus::Completed(_) => Some(true),
        EncoderStatus::Cancelled => Some(false),
        EncoderStatus::Failed(e) => panic!("{e}"),
        _ => None,
    });
    assert_eq!(path.exists(), committed);
    assert!(matches!(
        (committed, encoder.poll().unwrap()),
        (true, EncoderStatus::Completed(_)) | (false, EncoderStatus::Cancelled)
    ));
}
