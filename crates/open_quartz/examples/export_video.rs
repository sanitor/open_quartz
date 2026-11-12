//! Hardware-only, headless video-effect export using the shared frame pipeline.
//! Usage: INPUT OUTPUT START END FPS_NUM FPS_DEN [--invert] [--overwrite]
#[cfg(target_os = "macos")]
mod native {
    use open_quartz::execution::{ExportConfig, ExportJob, ExportPhase, FramePipeline};
    use open_quartz::media::native::{NativeVideoConfig, NativeVideoSourceKind};
    use open_quartz::outputs::PlaybackCapabilities;
    use open_quartz::rendering::{InputInterpretation, RenderDevice};
    use oq_media::{EncoderConfig, RationalTimeBase};
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::Duration;

    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        pollster::block_on(run())
    }
    async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        if args.len() < 6
            || args[6..]
                .iter()
                .any(|arg| arg != "--invert" && arg != "--overwrite")
        {
            return Err("usage: cargo run -p open_quartz --features native-runtime,native-video --example export_video -- INPUT OUTPUT START END FPS_NUM FPS_DEN [--invert] [--overwrite]".into());
        }
        let start: u64 = args[2].parse()?;
        let end: u64 = args[3].parse()?;
        let numerator: i32 = args[4].parse()?;
        let denominator: i32 = args[5].parse()?;
        let invert = args.iter().any(|arg| arg == "--invert");
        let code = if invert {
            "@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {let c=textureSample(image,imageSampler,uv);return vec4f(vec3f(1.0)-c.rgb,c.a);}"
        } else {
            "@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f {return textureSample(image,imageSampler,uv);}"
        };
        let graph = serde_json::from_value(json!({"nodes":[
        {"id":"video","type":"input","position":{"x":0,"y":0},"data":{"type":"input","label":"video","shaderCode":"","inputMode":"video","inputDataType":"sampler2D","inputs":[],"outputs":[{"id":"out","label":"image","dataType":"sampler2D","direction":"output"}],"uniforms":{}}},
        {"id":"effect","type":"shader","position":{"x":0,"y":0},"data":{"type":"shader","label":"effect","shaderCode":code,"inputs":[{"id":"in","label":"image","dataType":"sampler2D","direction":"input"}],"outputs":[{"id":"out","label":"fragColor","dataType":"vec4","direction":"output"}],"uniforms":{},"autoSize":true,"width":64,"height":64}}
    ],"edges":[{"id":"a","source":"video","sourceHandle":"out","target":"effect","targetHandle":"in"}]}))?;
        let (device, info) = RenderDevice::new().await?;
        let mut pipeline =
            FramePipeline::from_render_device(device, PlaybackCapabilities { data_paths: vec![] });
        pipeline.set_graph_snapshot(&graph)?;
        let source = pipeline.attach_video(
            "video",
            NativeVideoConfig {
                kind: NativeVideoSourceKind::File,
                source: args[0].clone(),
                looping: false,
                playback_rate: 1.0,
            },
        )?;
        let mut job = ExportJob::new(
            pipeline,
            ExportConfig {
                output: PathBuf::from(&args[1]),
                output_node_id: "effect".into(),
                frames: start..end,
                encoder: EncoderConfig {
                    width: source.width,
                    height: source.height,
                    time_base: RationalTimeBase {
                        numerator: denominator,
                        denominator: numerator,
                    },
                    bitrate: 8_000_000,
                    queue_capacity: 3,
                    max_b_frames: 0,
                    overwrite: args.iter().any(|arg| arg == "--overwrite"),
                },
                input_interpretation: InputInterpretation::SdrSrgbRgba8Black,
                date: [0.0; 4],
                mouse: [0.0; 4],
            },
        )?;
        let owner = std::thread::current();
        job.set_waker(std::sync::Arc::new(move || owner.unpark()))?;
        loop {
            let state = job.poll();
            if state.is_terminal() {
                if state.phase != ExportPhase::Completed {
                    return Err(state
                        .error
                        .as_deref()
                        .unwrap_or("export cancelled")
                        .to_owned()
                        .into());
                }
                println!(
                    "{} / {} · {} frames · {:?}",
                    info.adapter_name, info.backend, state.accepted_frames, state.report
                );
                return Ok(());
            }
            std::thread::park_timeout(Duration::from_millis(2));
        }
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    native::main()
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Hardware H.264 MP4 export currently requires macOS VideoToolbox");
    std::process::exit(1);
}
