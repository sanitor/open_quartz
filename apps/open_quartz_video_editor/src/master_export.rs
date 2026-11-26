use super::*;
use open_quartz::execution::{FrameAdmission, FrameDemand, FrameEvaluationInput, FrameOutcome};
use open_quartz::outputs::FrameStamp;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

struct Encoder(Child);

impl Drop for Encoder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run(
    render: RenderDevice,
    project: ProjectData,
    revision: u64,
    path: &Path,
    frames: Range<u64>,
    _options: ExportOptions,
    cancel: &Arc<AtomicBool>,
    progress: &Mutex<(String, Option<Phase>)>,
) -> Result<bool, String> {
    let io = |error: std::io::Error| error.to_string();
    if path.exists() {
        return Err("Output already exists".into());
    }
    let parent = path.parent().ok_or("Choose an output directory")?;
    let temp = tempfile::tempdir_in(parent).map_err(io)?;
    let movie = tempfile::NamedTempFile::new_in(temp.path())
        .map_err(io)?
        .into_temp_path();
    let audio_path = temp.path().join("mix.f32");
    let log = temp.path().join("ffmpeg.log");
    let fps = project.fps;
    let width = project.width;
    let height = project.height;
    if width % 2 != 0 {
        return Err("ProRes 422 HQ requires an even sequence width".into());
    }
    let total = frames.end - frames.start;
    let cadence = Cadence::new(frames.start, fps, fps);
    let ids = video_source_ids(&project, frames.end, &cadence, total, cancel)?;
    if let Some(asset) = project
        .assets
        .iter()
        .find(|asset| ids.contains(&asset.id) && !asset.online)
    {
        return Err(format!("Media offline: {}. Use Relink.", asset.name));
    }
    let samples = fps.frame_to_sample(frames.start)?..fps.frame_to_sample(frames.end)?;
    let mut command = Command::new(open_quartz::media::native::find_ffmpeg()?);
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-video_size",
        &format!("{width}x{height}"),
        "-framerate",
        &format!("{}/{}", fps.numerator, fps.denominator),
        "-i",
        "pipe:0",
    ]);
    if let Some(mut audio) =
        crate::audio_mix::AudioMix::new(&project, cancel.clone(), Some(&samples))?
    {
        let mut output = BufWriter::new(File::create(&audio_path).map_err(io)?);
        let mut block = [[0.0f32; 2]; 1024];
        let mut start = samples.start;
        while start < samples.end && !cancel.load(Ordering::Acquire) {
            let count = (samples.end - start).min(block.len() as u64) as usize;
            audio.render(start, &mut block[..count])?;
            for sample in &block[..count] {
                for channel in sample {
                    output.write_all(&channel.to_le_bytes()).map_err(io)?;
                }
            }
            start += count as u64;
        }
        output.flush().map_err(io)?;
        command
            .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(&audio_path);
    }
    if cancel.load(Ordering::Acquire) {
        return Ok(false);
    }
    command
        .args([
            "-c:v",
            "prores_ks",
            "-profile:v",
            "3",
            "-pix_fmt",
            "yuv422p10le",
            "-vf",
            "scale=out_color_matrix=bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "iec61966-2-1",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
            "-c:a",
            "pcm_s24le",
            "-f",
            "mov",
            "-y",
        ])
        .arg(&*movie)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(File::create(&log).map_err(io)?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut encoder = Encoder(command.spawn().map_err(io)?);
    let mut input = encoder
        .0
        .stdin
        .take()
        .ok_or("FFmpeg input pipe unavailable")?;
    let mut sequence = Sequence {
        project,
        revision,
        end: frames.end,
        cadence,
        hdr: false,
        configured: None,
        request: None,
    };
    let mut pipeline =
        FramePipeline::from_render_device(render, PlaybackCapabilities { data_paths: vec![] });
    let rendered = (|| {
        let mut region_end = 0;
        let mut epoch = 0;
        for frame in 0..total {
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            if frame == region_end {
                region_end = sequence.prepare_region(&mut pipeline, frame)?;
                epoch += 1;
            }
            let (timeline_ns, source_times) = sequence.source_times(frame)?;
            let mut demand = Some(FrameDemand {
                request_id: frame + 1,
                graph_revision: pipeline.revision(),
                output_node_id: OUTPUT_ID.into(),
                sources: vec![],
                input: FrameEvaluationInput {
                    stamp: FrameStamp {
                        epoch,
                        frame: frame + 1,
                        timeline_ns,
                        deadline_ns: 0,
                    },
                    previous_timeline_ns: timeline_ns,
                    date: [0.0; 4],
                    mouse: [0.0; 4],
                    resolution: [width as f32, height as f32, 1.0],
                },
            });
            let started = Instant::now();
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Ok(false);
                }
                if let Some(next) = demand.take() {
                    match pipeline.submit_window_with_source_times(next, &source_times) {
                        FrameAdmission::Accepted => {}
                        FrameAdmission::WouldBlock(next) => demand = Some(next),
                        FrameAdmission::Rejected { error, .. } => return Err(error),
                    }
                }
                pipeline.progress();
                if let Some(completion) = pipeline.poll_completion() {
                    let output = match completion.outcome {
                        FrameOutcome::Submitted(output) => output,
                        FrameOutcome::Failed(error) => return Err(error),
                        FrameOutcome::Cancelled => return Err("Master frame was cancelled".into()),
                    };
                    let pixels = pollster::block_on(pipeline.backend().read_texture_rgba(
                        &output.output().texture,
                        width,
                        height,
                    ))?;
                    input.write_all(&pixels).map_err(|error| {
                        let details = std::fs::read_to_string(&log).unwrap_or_default();
                        format!("FFmpeg stopped accepting master frames: {error} {details}")
                    })?;
                    break;
                }
                if started.elapsed() > Duration::from_secs(30) {
                    return Err("Timed out rendering a master frame".into());
                }
                thread::park_timeout(Duration::from_millis(2));
            }
            progress.lock().unwrap().0 =
                format!("Exporting ProRes · {} / {total} frames", frame + 1);
        }
        Ok(true)
    })();
    drop(input);
    let disposed = pipeline.dispose();
    if !rendered? {
        return Ok(false);
    }
    disposed?;
    progress.lock().unwrap().0 = "Finalizing ProRes…".into();
    while !cancel.load(Ordering::Acquire) {
        if let Some(status) = encoder.0.try_wait().map_err(io)? {
            if !status.success() {
                let details = std::fs::read_to_string(&log).unwrap_or_default();
                return Err(format!("FFmpeg master export failed: {details}"));
            }
            OpenOptions::new()
                .write(true)
                .open(&*movie)
                .and_then(|file| file.sync_all())
                .map_err(|error| format!("Cannot flush the MOV master: {error}"))?;
            if cancel.load(Ordering::Acquire) {
                return Ok(false);
            }
            movie
                .persist_noclobber(path)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        thread::park_timeout(Duration::from_millis(10));
    }
    Ok(false)
}
