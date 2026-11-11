use super::*;
use crate::evaluation::{FrameInputs, GraphEvaluator};
use crate::outputs::{FrameAlphaMode, FrameColorSpace};
use crate::Graph;
use serde_json::json;
use std::sync::atomic::AtomicUsize;

fn identity(node: &str) -> FrameSnapshotIdentity {
    FrameSnapshotIdentity {
        output_node_id: node.into(),
        graph_revision: 7,
        evaluation_stamp: None,
        content_stamp: None,
        color_space: FrameColorSpace::Srgb,
        alpha_mode: FrameAlphaMode::Opaque,
    }
}

async fn render_device() -> RenderDevice {
    RenderDevice::new()
        .await
        .expect("real hardware GPU is required")
        .0
}

fn pool(device: &RenderDevice, frames: usize, bytes: u64) -> StableFramePool {
    StableFramePool::new(
        device.backend(),
        StableFrameBudget {
            max_frames: frames,
            max_bytes: bytes,
        },
    )
    .unwrap()
}

async fn pixels(device: &RenderDevice, frame: &StableFrame) -> Vec<u8> {
    frame.ready.wait(Duration::from_secs(5)).unwrap();
    device
        .backend()
        .read_texture_rgba(
            &frame.output().texture,
            frame.image.width,
            frame.image.height,
        )
        .await
        .unwrap()
}

fn drain(device: &RenderDevice) {
    // Last producer lease drops may register a consumer callback for the next poll.
    for _ in 0..3 {
        device
            .device()
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(5)),
            })
            .unwrap();
    }
}

#[test]
fn snapshots_keep_pixels_and_provenance_across_source_changes_and_pool_saturation() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let pool = pool(&device, 2, 32);
        device
            .upload_rgba("source", &[255, 0, 0, 255].repeat(4), 2, 2)
            .unwrap();
        let red = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        let another_consumer = red.clone().into_output();
        assert_eq!(red.identity.graph_revision, 7);
        assert_eq!(red.image.color_space, FrameColorSpace::Srgb);
        device
            .upload_rgba("source", &[0, 0, 255, 255].repeat(4), 2, 2)
            .unwrap();
        let blue = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        assert_eq!(pixels(&device, &red).await, [255, 0, 0, 255].repeat(4));
        assert_eq!(pixels(&device, &blue).await, [0, 0, 255, 255].repeat(4));
        assert!(matches!(
            pool.try_capture(&device, "source", identity("source")),
            Err(FrameCaptureError::WouldBlock)
        ));
        let original_slot = pool.slots.lock().unwrap()[0].texture.clone();
        drop(red);
        drain(&device);
        assert!(
            matches!(
                pool.try_capture(&device, "source", identity("source")),
                Err(FrameCaptureError::WouldBlock)
            ),
            "a second consumer still owns the red slot"
        );
        // A real consumer GPU use must be submitted before its lease is dropped.
        let target = device
            .backend()
            .create_target(2, 2, TextureFormat::Rgba8Unorm);
        let mut commands = device.device().create_command_encoder(&Default::default());
        commands.copy_texture_to_texture(
            another_consumer.texture.as_image_copy(),
            target.texture.as_image_copy(),
            target.texture.size(),
        );
        device.backend().queue.submit([commands.finish()]);
        drop(another_consumer);
        drain(&device);
        assert_eq!(
            device.backend().read_target_rgba(&target).await.unwrap(),
            [255, 0, 0, 255].repeat(4)
        );
        let next = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        assert_eq!(
            pool.slots.lock().unwrap()[0].texture.as_ref(),
            original_slot.as_ref(),
            "released compatible backing allocation is reused"
        );
        assert_eq!(pixels(&device, &next).await, [0, 0, 255, 255].repeat(4));
        assert_eq!(pool.slots.lock().unwrap().len(), 2);
    });
}

#[test]
fn byte_budget_resize_and_reset_preserve_outstanding_frames() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let pool = pool(&device, 4, 20);
        device
            .upload_rgba("source", &[10, 20, 30, 255], 1, 1)
            .unwrap();
        let small = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        device
            .upload_rgba("source", &[40, 50, 60, 255].repeat(4), 2, 2)
            .unwrap();
        let big = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        assert!(
            matches!(
                pool.try_capture(&device, "source", identity("source")),
                Err(FrameCaptureError::WouldBlock)
            ),
            "bytes, not frame count, limit admission"
        );
        let budget_bytes: u64 = pool.slots.lock().unwrap().iter().map(|s| s.bytes).sum();
        assert_eq!(budget_bytes, 20);
        device.reset().unwrap();
        assert_eq!(pixels(&device, &small).await, [10, 20, 30, 255]);
        assert_eq!(pixels(&device, &big).await, [40, 50, 60, 255].repeat(4));
        drop(small);
        drop(big);
        drain(&device);
        device
            .upload_rgba("source", &[1, 2, 3, 255].repeat(5), 5, 1)
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let resized = loop {
            match pool.try_capture(&device, "source", identity("source")) {
                Ok(frame) => break frame,
                Err(FrameCaptureError::WouldBlock) if std::time::Instant::now() < deadline => {
                    drain(&device)
                }
                Err(error) => panic!("released byte budget did not become reusable: {error}"),
            }
        };
        assert_eq!(pixels(&device, &resized).await, [1, 2, 3, 255].repeat(5));
        assert_eq!(
            pool.slots
                .lock()
                .unwrap()
                .iter()
                .map(|s| s.bytes)
                .sum::<u64>(),
            20
        );
        device
            .upload_rgba("source", &[1, 2, 3, 255].repeat(6), 6, 1)
            .unwrap();
        assert!(matches!(
            pool.try_capture(&device, "source", identity("source")),
            Err(FrameCaptureError::Capacity)
        ));
        assert_eq!(pixels(&device, &resized).await, [1, 2, 3, 255].repeat(5));
    });
}

#[test]
fn dropping_pending_frame_retains_source_until_producer_completion() {
    struct Owner(Arc<AtomicUsize>);
    impl Drop for Owner {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    pollster::block_on(async {
        let mut device = render_device().await;
        let pool = pool(&device, 1, 64);
        let drops = Arc::new(AtomicUsize::new(0));
        let source = device
            .backend()
            .create_texture(1, 1, TextureFormat::Rgba8Unorm);
        device
            .backend()
            .upload_rgba(&source, &[11, 22, 33, 255])
            .unwrap();
        device
            .register_external_texture(
                "source",
                GpuOutputHandle {
                    texture: source.texture,
                    view: source.view,
                    sampler: source.sampler,
                    width: 1,
                    height: 1,
                    format: source.format,
                    lease: Some(Arc::new(Owner(drops.clone()))),
                },
            )
            .unwrap();
        let frame = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        let ready = frame.ready.clone();
        device.remove_texture("source");
        // Cancellation is simply dropping the consumer; producer still owns the source.
        drop(frame);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(pool.slots.lock().unwrap()[0].busy.load(Ordering::Acquire));
        ready.wait(Duration::from_secs(5)).unwrap();
        drain(&device);
        assert!(ready.is_ready());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        pool.poll_releases().unwrap();
        assert!(!pool.slots.lock().unwrap()[0].busy.load(Ordering::Acquire));
    });
}

#[test]
fn readiness_or_wait_does_not_release_a_consumer_and_pool_drop_does_not_destroy_pixels() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let pool = pool(&device, 1, 16);
        device
            .upload_rgba("source", &[123, 45, 67, 255], 1, 1)
            .unwrap();
        let frame = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        // Zero timeout is intentionally permitted to return either outcome depending
        // on actual device progress. Neither outcome permits recycling this lease.
        let _ = frame.ready.wait(Duration::ZERO);
        assert!(matches!(
            pool.try_capture(&device, "source", identity("source")),
            Err(FrameCaptureError::WouldBlock)
        ));
        frame.ready.wait(Duration::from_secs(5)).unwrap();
        frame.ready.wait(Duration::ZERO).unwrap();
        assert!(matches!(
            pool.try_capture(&device, "source", identity("source")),
            Err(FrameCaptureError::WouldBlock)
        ));
        drop(pool);
        device.reset().unwrap();
        assert_eq!(pixels(&device, &frame).await, [123, 45, 67, 255]);
    });
}

#[test]
fn invalid_sources_and_wrong_devices_do_not_consume_capacity() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let other = render_device().await;
        let pool = pool(&device, 1, 64);
        assert!(matches!(
            pool.try_capture(&other, "missing", identity("missing")),
            Err(FrameCaptureError::WrongDevice)
        ));
        assert!(matches!(
            pool.try_capture(&device, "missing", identity("missing")),
            Err(FrameCaptureError::InvalidSource(_))
        ));
        device.upload_rgba("source", &[4, 5, 6, 255], 1, 1).unwrap();
        assert!(matches!(
            pool.try_capture(&device, "source", identity("another")),
            Err(FrameCaptureError::InvalidSource(_))
        ));
        for bad in [0, 2] {
            let mut handle = device.output_handle("source").unwrap();
            handle.width = bad;
            assert!(matches!(
                pool.capture(handle, identity("source")),
                Err(FrameCaptureError::InvalidSource(_))
            ));
        }
        let mut mismatched = device.output_handle("source").unwrap();
        mismatched.format = TextureFormat::Rgba32Float;
        assert!(matches!(
            pool.capture(mismatched, identity("source")),
            Err(FrameCaptureError::InvalidSource(_))
        ));
        let texture = Arc::new(device.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("not-copyable"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }));
        let mut no_copy = device.output_handle("source").unwrap();
        no_copy.view = Arc::new(texture.create_view(&Default::default()));
        no_copy.texture = texture;
        assert!(matches!(
            pool.capture(no_copy, identity("source")),
            Err(FrameCaptureError::InvalidSource(_))
        ));
        assert!(pool.slots.lock().unwrap().is_empty());
        let valid = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        assert_eq!(pixels(&device, &valid).await, [4, 5, 6, 255]);
        for limits in [
            StableFrameBudget {
                max_frames: 0,
                max_bytes: 64,
            },
            StableFrameBudget {
                max_frames: 1,
                max_bytes: 0,
            },
        ] {
            assert!(matches!(
                StableFramePool::new(device.backend(), limits),
                Err(FrameCaptureError::InvalidSource(_))
            ));
        }
    });
}

#[test]
fn float_snapshots_keep_format_and_exact_values() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let target = device
            .backend()
            .create_target(1, 1, TextureFormat::Rgba32Float);
        let mut encoder = device.device().create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 2.0,
                            g: 0.5,
                            b: 0.25,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        device.backend().queue.submit([encoder.finish()]);
        device
            .register_external_texture(
                "float",
                GpuOutputHandle {
                    texture: target.texture,
                    view: target.view,
                    sampler: Arc::new(device.device().create_sampler(&Default::default())),
                    width: 1,
                    height: 1,
                    format: TextureFormat::Rgba32Float,
                    lease: None,
                },
            )
            .unwrap();
        let pool = pool(&device, 1, 16);
        let frame = pool
            .try_capture(&device, "float", identity("float"))
            .unwrap();
        frame.ready.wait(Duration::from_secs(5)).unwrap();
        assert_eq!(frame.output().format, TextureFormat::Rgba32Float);
        let read = device.device().create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.device().create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            frame.output().texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        device.backend().queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        drain(&device);
        rx.recv().unwrap().unwrap();
        let mapped = read.slice(..).get_mapped_range().unwrap();
        let values: Vec<f32> = mapped[..16]
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(values, [2.0, 0.5, 0.25, 1.0]);
        drop(mapped);
        read.unmap();
    });
}

#[test]
fn explicit_time_two_source_graph_frames_stay_stable_during_later_execution() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let graph: Graph = serde_json::from_value(json!({"nodes":[
            {"id":"a","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"A","shaderCode":"","inputDataType":"sampler2D","inputMode":"image","inputs":[],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
            {"id":"b","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"B","shaderCode":"","inputDataType":"sampler2D","inputMode":"image","inputs":[],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
            {"id":"mix","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"Mix","shaderCode":"@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {return mix(textureSample(a,aSampler,uv),textureSample(b,bSampler,uv),iTime);}","inputs":[{"id":"a","label":"a","dataType":"sampler2D","direction":"input"},{"id":"b","label":"b","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{},"autoSize":false,"width":2,"height":2}}
        ],"edges":[{"id":"a","source":"a","sourceHandle":"out","target":"mix","targetHandle":"a"},{"id":"b","source":"b","sourceHandle":"out","target":"mix","targetHandle":"b"}]})).unwrap();
        let mut evaluator = GraphEvaluator::prepare(graph.nodes, graph.edges);
        device
            .upload_rgba("a", &[255, 0, 0, 255].repeat(4), 2, 2)
            .unwrap();
        device
            .upload_rgba("b", &[0, 0, 255, 255].repeat(4), 2, 2)
            .unwrap();
        let pool = pool(&device, 3, 48);
        let mut held = Vec::new();
        for (n, time) in [0.0, 0.5, 1.0].into_iter().enumerate() {
            let result = evaluator.run_frame(&FrameInputs {
                time,
                delta: 0.5,
                frame: n as u64 + 1,
                date: [0.0; 4],
                mouse: [0.0; 4],
                resolution: [2.0, 2.0, 1.0],
                video_nodes: vec![],
            });
            device.execute(evaluator.plan(), &result.commands).unwrap();
            held.push(pool.try_capture(&device, "mix", identity("mix")).unwrap());
        }
        for (frame, expected) in
            held.iter()
                .zip([[255, 0, 0, 255], [128, 0, 128, 255], [0, 0, 255, 255]])
        {
            let got = pixels(&device, frame).await;
            assert!(
                got.chunks_exact(4).all(|p| p
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| (*a as i16 - b as i16).abs() <= 1)),
                "{got:?} expected {expected:?}"
            );
        }
    });
}

#[cfg(feature = "native-runtime")]
#[test]
fn native_player_stable_preview_backpressure_and_graph_replacement() {
    pollster::block_on(async {
        let (mut player, _) = crate::NativePlayer::new().await.unwrap();
        let graph: Graph = serde_json::from_value(json!({"nodes":[
            {"id":"color","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"Numbered frame","shaderCode":"@fragment fn main()->@location(0) vec4f {return vec4f(iFrame / 255.0,0.0,1.0,1.0);}","inputs":[],"outputs":[{"id":"out","label":"output","dataType":"sampler2D","direction":"output"}],"uniforms":{},"autoSize":false,"width":2,"height":2}}
        ],"edges":[]})).unwrap();
        let first_revision = player.set_graph_snapshot(&graph).unwrap();
        player.start_playback().unwrap();
        let backend = player.backend();
        let mut held = Vec::new();
        for _ in 0..3 {
            player.render_next().unwrap();
            held.push(player.try_stable_preview("color").unwrap().unwrap());
        }
        player.render_next().unwrap();
        assert!(player.try_stable_preview("color").unwrap().is_none());
        for (n, frame) in held.iter().enumerate() {
            frame.ready.wait(Duration::from_secs(5)).unwrap();
            assert_eq!(frame.identity.graph_revision, first_revision);
            let evaluated = frame.identity.evaluation_stamp.as_ref().expect("accepted frame stamp");
            let content = frame.identity.content_stamp.as_ref().expect("composition identity");
            assert_eq!(content.timeline_ns, evaluated.timeline_ns);
            assert_eq!(content.epoch, evaluated.epoch);
            assert!(content.media_pts_ns.is_none(), "source PTS is not inferred from the playback clock");
            let got = backend
                .read_texture_rgba(&frame.output().texture, 2, 2)
                .await
                .unwrap();
            assert!(got.chunks_exact(4).all(|px| px == [n as u8, 0, 255, 255]));
        }
        let mut replacement = graph.clone();
        replacement.nodes[0].data.shader_code =
            "@fragment fn main()->@location(0) vec4f {return vec4f(0.0,1.0,0.0,1.0);}".into();
        let next_revision = player.set_graph_snapshot(&replacement).unwrap();
        player.render_next().unwrap();
        let released = held.remove(0);
        drop(released);
        for _ in 0..3 {
            backend
                .device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(5)),
                })
                .unwrap();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if player.render_next().unwrap().width > 0 { break; }
            assert!(std::time::Instant::now() < deadline, "released pool did not complete the admitted frame");
        }
        let green = player.try_stable_preview("color").unwrap().unwrap();
        assert_eq!(green.identity.graph_revision, next_revision);
        green.ready.wait(Duration::from_secs(5)).unwrap();
        assert_eq!(
            backend
                .read_texture_rgba(&green.output().texture, 2, 2)
                .await
                .unwrap(),
            [0, 255, 0, 255].repeat(4)
        );
        player.dispose().unwrap();
        for (n, frame) in held.iter().enumerate() {
            assert_eq!(
                backend
                    .read_texture_rgba(&frame.output().texture, 2, 2)
                    .await
                    .unwrap(),
                [n as u8 + 1, 0, 255, 255].repeat(4)
            );
        }
    });
}

#[cfg(target_os = "macos")]
#[test]
fn slint_style_texture_clone_keeps_the_pool_slot_until_native_resource_release() {
    pollster::block_on(async {
        let mut device = render_device().await;
        let pool = pool(&device, 1, 16);
        device
            .upload_rgba("source", &[200, 10, 20, 255], 1, 1)
            .unwrap();
        let frame = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        frame.ready.wait(Duration::from_secs(5)).unwrap();
        // Slint Image retains a wgpu::Texture, not the Rust output wrapper.
        let texture = (*frame.output().texture).clone();
        drop(frame);
        drain(&device);
        device
            .upload_rgba("source", &[0, 100, 255, 255], 1, 1)
            .unwrap();
        assert!(matches!(
            pool.try_capture(&device, "source", identity("source")),
            Err(FrameCaptureError::WouldBlock)
        ));
        assert_eq!(
            device
                .backend()
                .read_texture_rgba(&texture, 1, 1)
                .await
                .unwrap(),
            [200, 10, 20, 255]
        );
        drop(texture);
        drain(&device);
        let next = pool
            .try_capture(&device, "source", identity("source"))
            .unwrap();
        assert_eq!(pixels(&device, &next).await, [0, 100, 255, 255]);
    });
}
