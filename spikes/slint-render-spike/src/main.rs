use std::sync::Arc;
use std::time::{Duration, Instant};

use open_quartz_execution::engine::{ExecutionEngine, FrameInputs};
use open_quartz_execution::gpu::{GpuBackend, GpuExecutor};
use open_quartz_schema::{Edge, ProjectNode};
use serde_json::json;
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

const OUTPUT_WIDTH: u32 = 640;
const OUTPUT_HEIGHT: u32 = 360;

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

    let backend = Arc::new(GpuBackend::from_shared(
        Arc::new(device.clone()),
        Arc::new(queue.clone()),
    ));
    let mut executor = GpuExecutor::new(backend);
    let mut engine = build_engine();

    slint::BackendSelector::new()
        .require_wgpu_30(WGPUConfiguration::Manual {
            instance,
            adapter,
            device,
            queue,
        })
        .select()?;
    eprintln!("slint: wgpu 30 backend selected with manual device/queue");

    let window = SpikeWindow::new()?;
    window.set_status("open_quartz_execution output: starting".into());

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

            let inputs = FrameInputs {
                time: elapsed.as_secs_f64(),
                delta: 1.0 / 60.0,
                frame: frames + 1,
                date: [2026.0, 10.0, 25.0, 0.0],
                mouse: [0.0; 4],
                resolution: [OUTPUT_WIDTH as f32, OUTPUT_HEIGHT as f32, 1.0],
                video_nodes: Vec::new(),
            };
            let work = engine.run_frame(&inputs);
            if let Err(error) = executor.execute(engine.plan(), &work) {
                eprintln!("FAIL: execution failed: {error}");
                let _ = slint::quit_event_loop();
                return;
            }

            let Some(output) = executor.output_handle("renderer") else {
                eprintln!("FAIL: renderer produced no GPU output");
                let _ = slint::quit_event_loop();
                return;
            };
            let dimensions = format!(
                "{}x{} {:?}",
                output.width, output.height, output.format
            );

            match slint::Image::try_from((*output.texture).clone()) {
                Ok(image) => {
                    if first_import {
                        eprintln!(
                            "PASS: imported open_quartz_execution output {dimensions} into slint::Image"
                        );
                        first_import = false;
                    }
                    if let Some(window) = weak.upgrade() {
                        window.set_frame(image);
                        window.set_status(
                            format!(
                                "execution output {dimensions} | frame {frames} | {:.0} ms",
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

fn build_engine() -> ExecutionEngine {
    let nodes: Vec<ProjectNode> = serde_json::from_value(json!([
        {
            "id": "time",
            "type": "input",
            "position": { "x": 0.0, "y": 0.0 },
            "data": {
                "type": "input",
                "label": "Time",
                "shaderCode": "",
                "inputs": [],
                "outputs": [
                    {
                        "id": "time_out",
                        "label": "value",
                        "dataType": "float",
                        "direction": "output"
                    }
                ],
                "uniforms": {},
                "inputMode": "system",
                "inputDataType": "float",
                "systemSource": "time"
            }
        },
        {
            "id": "pulse",
            "type": "shader",
            "position": { "x": 1.0, "y": 0.0 },
            "data": {
                "type": "shader",
                "label": "Pulse",
                "shaderCode": "@group(0) @binding(0) var<uniform> time: f32;\n\n@fragment\nfn main(@location(0) uv: vec2f) -> @location(0) vec4f {\n    let p = (uv - vec2f(0.5, 0.5)) * 2.0;\n    let radius = length(p);\n    let angle = atan2(p.y, p.x);\n    let wave = 0.5 + 0.5 * sin(radius * 24.0 - time * 3.0);\n    let rays = 0.5 + 0.5 * sin(angle * 8.0 + time);\n    let vignette = 1.0 - smoothstep(0.25, 1.25, radius);\n    return vec4f(wave * vignette, rays * vignette, (1.0 - wave * rays) * vignette, 1.0);\n}\n",
                "inputs": [
                    {
                        "id": "time_in",
                        "label": "time",
                        "dataType": "float",
                        "direction": "input"
                    }
                ],
                "outputs": [
                    {
                        "id": "pulse_out",
                        "label": "output",
                        "dataType": "sampler2D",
                        "direction": "output"
                    }
                ],
                "uniforms": {},
                "autoSize": false,
                "width": OUTPUT_WIDTH,
                "height": OUTPUT_HEIGHT
            }
        },
        {
            "id": "renderer",
            "type": "renderer",
            "position": { "x": 2.0, "y": 0.0 },
            "data": {
                "type": "renderer",
                "label": "Renderer",
                "shaderCode": "",
                "inputs": [
                    {
                        "id": "renderer_in",
                        "label": "inputImage",
                        "dataType": "sampler2D",
                        "direction": "input"
                    }
                ],
                "outputs": [],
                "uniforms": {}
            }
        }
    ]))
    .expect("spike graph nodes must deserialize");
    let edges: Vec<Edge> = serde_json::from_value(json!([
        {
            "id": "time_to_pulse",
            "source": "time",
            "sourceHandle": "time_out",
            "target": "pulse",
            "targetHandle": "time_in"
        },
        {
            "id": "pulse_to_renderer",
            "source": "pulse",
            "sourceHandle": "pulse_out",
            "target": "renderer",
            "targetHandle": "renderer_in"
        }
    ]))
    .expect("spike graph edges must deserialize");
    ExecutionEngine::prepare(nodes, edges)
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
