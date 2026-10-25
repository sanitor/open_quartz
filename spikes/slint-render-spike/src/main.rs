use std::time::{Duration, Instant};

use slint::wgpu_30::{wgpu, WGPUConfiguration};

slint::slint! {
    export component SpikeWindow inherits Window {
        title: "Slint + wgpu 30 shared texture spike";
        width: 720px;
        height: 480px;
        background: #0b0d10;

        in property <image> frame;
        in property <string> status;

        VerticalLayout {
            padding: 12px;
            spacing: 8px;

            Text {
                text: root.status;
                color: #e8edf2;
                font-size: 13px;
            }

            Image {
                source: root.frame;
                image-fit: contain;
                horizontal-stretch: 1;
                vertical-stretch: 1;
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seconds = parse_args()?;

    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    #[cfg(windows)]
    {
        instance_descriptor.backends = wgpu::Backends::DX12;
    }
    #[cfg(not(windows))]
    {
        instance_descriptor.backends = wgpu::Backends::PRIMARY;
    }
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))?;
    let adapter_info = adapter.get_info();
    eprintln!(
        "adapter: {} | backend={:?} | type={:?}",
        adapter_info.name, adapter_info.backend, adapter_info.device_type
    );

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("slint-spike-device"),
        ..Default::default()
    }))?;

    let size = wgpu::Extent3d {
        width: 640,
        height: 360,
        depth_or_array_layers: 1,
    };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("spike-output"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    slint::BackendSelector::new()
        .require_wgpu_30(WGPUConfiguration::Manual {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        })
        .select()?;
    eprintln!("slint: wgpu 30 backend selected with manual device/queue");

    let window = SpikeWindow::new()?;
    window.set_status(format!("shared wgpu 30 texture: starting | format={format:?}").into());

    let weak = window.as_weak();
    let start = Instant::now();
    let mut last_log = start;
    let mut frames = 0_u64;
    let mut first_import = true;

    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(16),
        move || {
            let elapsed = start.elapsed();
            if seconds > 0.0 && elapsed.as_secs_f64() >= seconds {
                eprintln!(
                    "spike: completed {frames} frames in {:.2}s",
                    elapsed.as_secs_f64()
                );
                let _ = slint::quit_event_loop();
                return;
            }

            let pixels = make_frame(size.width, size.height, elapsed.as_secs_f32());
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.width * 4),
                    rows_per_image: Some(size.height),
                },
                size,
            );

            match slint::Image::try_from(texture.clone()) {
                Ok(image) => {
                    if first_import {
                        eprintln!("PASS: imported wgpu texture into slint::Image");
                        first_import = false;
                    }
                    if let Some(window) = weak.upgrade() {
                        window.set_frame(image);
                        window.set_status(
                            format!(
                                "shared wgpu 30 texture OK | frame {frames} | {:.0} ms",
                                elapsed.as_secs_f64() * 1000.0
                            )
                            .into(),
                        );
                    }
                }
                Err(error) => {
                    eprintln!("FAIL: Image::try_from failed: {error:?}");
                    let _ = slint::quit_event_loop();
                    return;
                }
            }

            frames += 1;
            if last_log.elapsed() >= Duration::from_secs(1) {
                let fps = frames as f64 / elapsed.as_secs_f64().max(f64::EPSILON);
                eprintln!(
                    "spike: frames={frames} elapsed={:.2}s fps={fps:.1}",
                    elapsed.as_secs_f64()
                );
                last_log = Instant::now();
            }
        },
    );

    window.run()?;
    Ok(())
}

fn parse_args() -> Result<f64, String> {
    let mut seconds = 0.0;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                let value = args.next().ok_or("--seconds requires a value")?;
                seconds = value
                    .parse::<f64>()
                    .map_err(|error| format!("invalid --seconds value {value:?}: {error}"))?;
            }
            "-h" | "--help" => {
                println!("Usage: slint-render-spike [--seconds N]");
                println!(
                    "  N > 0 auto-closes after N seconds; default 0 runs until the window closes."
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(seconds)
}

fn make_frame(width: u32, height: u32, phase: f32) -> Vec<u8> {
    let mut pixels = vec![0_u8; (width * height * 4) as usize];
    let bar_x = ((phase * 140.0) as u32) % width;
    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            let in_bar = x.abs_diff(bar_x) < width / 24;
            pixels[offset] = if in_bar { 255 } else { (x * 255 / width) as u8 };
            pixels[offset + 1] = (y * 255 / height) as u8;
            pixels[offset + 2] = (((x + y) / 2 + (phase * 60.0) as u32) % 256) as u8;
            pixels[offset + 3] = 255;
        }
    }
    pixels
}
