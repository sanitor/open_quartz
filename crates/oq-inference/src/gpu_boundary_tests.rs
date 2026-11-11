use super::*;

#[test]
fn invalid_models_and_insufficient_requested_limits_fail_before_inference() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("hardware Metal adapter");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        for invalid in 0..4 {
            let mut model = identity_model();
            match invalid {
                0 => model.plan.scale = 2,
                1 => model.layers[0].stride = [2, 2],
                2 => {
                    model.layers[1].weights.pop();
                }
                _ => model.layers[3].bias[0] = f32::NAN,
            }
            assert!(matches!(
                GpuImageSession::prepare(device.clone(), queue.clone(), model),
                Err(GpuError::UnsupportedModel(_))
            ));
        }
        let limits = wgpu::Limits {
            max_storage_buffer_binding_size: 16_384,
            ..Default::default()
        };
        let (limited_device, limited_queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(matches!(
            GpuImageSession::prepare(
                Arc::new(limited_device),
                Arc::new(limited_queue),
                identity_model()
            ),
            Err(GpuError::DeviceLimit(_))
        ));
        // A rejected preparation does not poison the real shared device.
        let mut healthy =
            GpuImageSession::prepare(device.clone(), queue.clone(), identity_model()).unwrap();
        let source = texture(
            &device,
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        generate(&device, &queue, &source, "green");
        let output = healthy.submit_texture(&source, 1, 1).unwrap();
        assert_color(&readback(&device, &queue, &output), [0, 255, 0, 255]);
        assert_counters(&healthy, 1);
    });
}

#[test]
fn rotating_input_cache_eviction_preserves_retained_binding_and_rejects_foreign_session_device() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("hardware Metal adapter");
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let (device, queue) = (Arc::new(device), Arc::new(queue));
        let mut session =
            GpuImageSession::prepare(device.clone(), queue.clone(), identity_model()).unwrap();
        let sources: Vec<_> = (0..=INPUT_CACHE_CAPACITY)
            .map(|_| {
                texture(
                    &device,
                    1,
                    2,
                    wgpu::TextureFormat::Rgba8Unorm,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                )
            })
            .collect();
        generate(&device, &queue, &sources[0], "red");
        let retained = session.bind_input(&sources[0], 1, 2).unwrap();
        for (index, source) in sources.iter().enumerate() {
            let color = if index % 2 == 0 { "red" } else { "blue" };
            generate(&device, &queue, source, color);
            let output = session.submit_texture(source, 1, 2).unwrap();
            assert_eq!((output.width(), output.height()), (3, 6));
            assert_color(
                &readback(&device, &queue, &output),
                if index % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                },
            );
        }
        let (foreign_device, foreign_queue) =
            adapter.request_device(&Default::default()).await.unwrap();
        let mut foreign = GpuImageSession::prepare(
            Arc::new(foreign_device),
            Arc::new(foreign_queue),
            identity_model(),
        )
        .unwrap();
        assert!(matches!(
            foreign.submit(&retained),
            Err(GpuError::WrongDevice)
        ));
        assert_counters(&foreign, 0);
        let oversize = device.limits().max_texture_dimension_2d / 3 + 1;
        let too_wide = texture(
            &device,
            oversize,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        assert!(matches!(
            session.submit_texture(&too_wide, oversize, 1),
            Err(GpuError::DeviceLimit(_))
        ));
        assert_counters(&session, INPUT_CACHE_CAPACITY as u64 + 1);
        // Even after eviction and failed submissions, the original bound input remains usable.
        let output = session.submit(&retained).unwrap();
        assert_color(&readback(&device, &queue, &output), [255, 0, 0, 255]);
        assert_counters(&session, INPUT_CACHE_CAPACITY as u64 + 2);
    });
}
