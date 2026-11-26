use crate::audio_mix::AudioMix;
use crate::document::{ProjectData, SAMPLE_RATE};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::ops::Range;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{atomic::AtomicBool, Arc};

pub struct Settings {
    pub target: f64,
    pub ceiling: f64,
    pub stems: bool,
}

impl Settings {
    pub fn parse(target: &str, ceiling: &str, stems: bool) -> Result<Self, String> {
        let target = target
            .trim()
            .parse()
            .map_err(|_| "Enter a numeric LUFS target")?;
        let ceiling = ceiling
            .trim()
            .parse()
            .map_err(|_| "Enter a numeric true peak ceiling")?;
        if !(-36.0..=-10.0).contains(&target) || !(-12.0..=-0.1).contains(&ceiling) {
            return Err("Target must be −36 to −10 LUFS and ceiling −12 to −0.1 dBTP".into());
        }
        Ok(Self {
            target,
            ceiling,
            stems,
        })
    }
}

#[derive(Serialize)]
struct Level {
    integrated_lufs: Option<f64>,
    true_peak_dbtp: Option<f64>,
}

fn command() -> Result<Command, String> {
    let mut command = Command::new(open_quartz::media::native::find_ffmpeg()?);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.args(["-nostdin", "-hide_banner", "-v", "info"]);
    Ok(command)
}

fn input(command: &mut Command, path: &Path) {
    command
        .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
        .arg(path);
}

fn run(command: &mut Command) -> Result<String, String> {
    let result = command
        .stdout(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    let log = String::from_utf8_lossy(&result.stderr);
    if !result.status.success() {
        return Err(format!("FFmpeg audio delivery failed: {log}"));
    }
    Ok(log.into_owned())
}

fn measure(path: &Path, raw: bool) -> Result<Level, String> {
    let mut command = command()?;
    if raw {
        input(&mut command, path);
    } else {
        command.arg("-i").arg(path);
    }
    let log = run(command.args([
        "-af",
        "loudnorm=print_format=json",
        "-c:a",
        "pcm_s24le",
        "-f",
        "null",
        "-",
    ]))?;
    let start = log.rfind('{').ok_or("FFmpeg did not report loudness")?;
    let end = log
        .rfind('}')
        .filter(|end| *end > start)
        .ok_or("FFmpeg loudness report is incomplete")?;
    let stats: serde_json::Value =
        serde_json::from_str(&log[start..=end]).map_err(|error| error.to_string())?;
    let metric = |key: &str| {
        stats[key]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
    };
    Ok(Level {
        integrated_lufs: metric("input_i"),
        true_peak_dbtp: metric("input_tp"),
    })
}

fn mix_raw(project: &ProjectData, range: &Range<u64>, path: &Path) -> Result<(), String> {
    let mut mixer = AudioMix::new(project, Arc::new(AtomicBool::new(false)), Some(range))?;
    let mut writer = BufWriter::new(File::create(path).map_err(|error| error.to_string())?);
    let mut samples = [[0.0; 2]; 1024];
    let mut at = range.start;
    while at < range.end {
        let count = (range.end - at).min(samples.len() as u64) as usize;
        let block = &mut samples[..count];
        if let Some(mixer) = &mut mixer {
            mixer.render_unclipped(at, block)?;
        }
        for sample in block.iter().flatten() {
            writer
                .write_all(&sample.to_le_bytes())
                .map_err(|error| error.to_string())?;
        }
        at += count as u64;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn write_wav(raw: &Path, path: &Path, gain: f64) -> Result<(), String> {
    let mut command = command()?;
    input(&mut command, raw);
    run(command
        .args([
            "-af",
            &format!("volume={gain:.4}dB"),
            "-c:a",
            "pcm_s24le",
            "-rf64",
            "auto",
            "-n",
        ])
        .arg(path))?;
    Ok(())
}

pub fn export(
    project: ProjectData,
    frames: Range<u64>,
    path: &Path,
    settings: Settings,
) -> Result<String, String> {
    let io = |error: std::io::Error| error.to_string();
    if path.exists() {
        return Err("Audio delivery folder already exists".into());
    }
    let parent = path
        .parent()
        .ok_or("Choose a destination inside an existing directory")?;
    let staging = tempfile::tempdir_in(parent).map_err(io)?;
    let raw = staging.path().join("mix.f32");
    let mix = staging.path().join("mix.wav");
    let range =
        project.fps.frame_to_sample(frames.start)?..project.fps.frame_to_sample(frames.end)?;
    mix_raw(&project, &range, &raw)?;
    let input = measure(&raw, true)?;
    let requested = input
        .integrated_lufs
        .map_or(0.0, |level| settings.target - level);
    // Avoid limiting or clipping transients. Reserve a small margin for quantization and analysis rounding.
    let gain = input
        .true_peak_dbtp
        .map_or(0.0, |peak| requested.min(settings.ceiling - peak - 0.02));
    write_wav(&raw, &mix, gain)?;
    let output = measure(&mix, false)?;
    let mut files = vec!["mix.wav".to_owned()];
    let mut stems = Vec::new();
    if settings.stems {
        let mut stem = project.clone();
        for track in &mut stem.tracks {
            track.visible = false;
        } // Nested sequence audio is part of the mix.
        for (index, track) in project
            .audio_tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| !track.clips.is_empty())
        {
            let mut lane = track.clone();
            lane.muted = false;
            lane.solo = false;
            stem.audio_tracks = vec![lane];
            mix_raw(&stem, &range, &raw)?;
            let file = format!("track-{:02}.wav", index + 1);
            write_wav(&raw, &staging.path().join(&file), 0.0)?;
            stems.push(serde_json::json!({"file": file, "track": track.name}));
            files.push(file);
        }
    }
    let report = serde_json::json!({
        "sample_rate": SAMPLE_RATE, "start_frame": frames.start, "end_frame_exclusive": frames.end,
        "samples": range.end - range.start, "target_lufs": settings.target, "ceiling_dbtp": settings.ceiling,
        "input": input, "applied_gain_db": gain, "output": output,
        "peak_limited": gain < requested - 0.01, "stems": stems,
    });
    files.push("loudness.json".into());
    fs::write(
        staging.path().join("loudness.json"),
        serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?,
    )
    .map_err(io)?;
    fs::create_dir(path).map_err(io)?;
    let mut published = Vec::new();
    for file in files {
        let destination = path.join(&file);
        if let Err(error) = fs::hard_link(staging.path().join(file), &destination) {
            for entry in published {
                let _ = fs::remove_file(entry);
            }
            let _ = fs::remove_dir(path);
            return Err(error.to_string());
        }
        published.push(destination);
    }
    Ok(format!("Audio delivery complete · {}", path.display()))
}
