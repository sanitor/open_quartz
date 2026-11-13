#[cfg(test)]
mod tests {
    use super::super::*;
    use std::time::Instant;
    fn wait(bridge: &PreviewBridge, ticket: u64) -> PreviewUpdate {
        let limit = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(update) = bridge.take_update() {
                if update.ticket == ticket {
                    return update;
                }
            }
            assert!(Instant::now() < limit, "preview request did not complete");
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn shared_pipeline_displays_real_video_black_gaps_and_retains_previous_frame_after_cut() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/white_64x64.mp4");
        let asset = crate::media::probe(&path).unwrap();
        let mut request = PreviewRequest {
            ticket: 1,
            revision: 4,
            frame: 0,
            source_ns: 0,
            width: 128,
            height: 64,
            asset: Some(asset),
            transform: ClipTransform::default(),
        };
        bridge.request(request.clone()).unwrap();
        let first = wait(&bridge, 1);
        assert_eq!(first.cpu_copy_bytes, 0);
        assert_eq!(first.actual_pts_ns, Some(0));
        let image = first.image.unwrap();
        let pixels = pollster::block_on(bridge.backend.read_texture_rgba(
            &image.output().texture,
            128,
            64,
        ))
        .unwrap();
        assert!(pixels.chunks_exact(4).enumerate().all(|(index, p)| {
            if index % 128 < 32 || index % 128 >= 96 {
                p == [0, 0, 0, 255]
            } else {
                p[0] > 240 && p[1] > 240 && p[2] > 240 && p[3] == 255
            }
        }));
        request.ticket = 2;
        request.frame = 15;
        request.asset = None;
        bridge.request(request.clone()).unwrap();
        let gap = wait(&bridge, 2).image.unwrap();
        let black = pollster::block_on(bridge.backend.read_texture_rgba(
            &gap.output().texture,
            128,
            64,
        ))
        .unwrap();
        assert!(black.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        assert_eq!(
            pollster::block_on(
                bridge
                    .backend
                    .read_texture_rgba(&image.output().texture, 128, 64)
            )
            .unwrap(),
            pixels
        );
        drop((image, gap));
        request.ticket = 3;
        bridge.request(request).unwrap();
        assert!(wait(&bridge, 3).image.is_ok());
        bridge.shutdown();
    }
    #[test]
    fn obsolete_seek_is_never_published_as_newer_request_and_offline_is_an_error() {
        crate::media::configure_runtime().unwrap();
        let (mut bridge, _configuration, _) = PreviewBridge::new().unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/oq-media/tests/source-time.mp4");
        let mut asset = crate::media::probe(&path).unwrap();
        let mut request = PreviewRequest {
            ticket: 1,
            revision: 7,
            frame: 0,
            source_ns: 1_200_000_000,
            width: 64,
            height: 64,
            asset: Some(asset.clone()),
            transform: ClipTransform::default(),
        };
        bridge.request(request.clone()).unwrap();
        request.ticket = 2;
        request.source_ns = 100_000_000;
        request.frame = 3;
        bridge.request(request.clone()).unwrap();
        let selected = wait(&bridge, 2);
        assert_eq!((selected.revision, selected.frame), (7, 3));
        assert_eq!(selected.actual_pts_ns, Some(100_000_000));
        assert_eq!(selected.cpu_copy_bytes, 0);
        drop(selected);
        asset.online = false;
        request.ticket = 3;
        request.asset = Some(asset);
        bridge.request(request).unwrap();
        assert!(wait(&bridge, 3).image.err().unwrap().contains("offline"));
        bridge.shutdown();
    }
}
