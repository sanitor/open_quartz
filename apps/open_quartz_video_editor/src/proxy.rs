use crate::document::{fingerprint_until, new_id, MediaAsset, ProjectData};
use oq_media::Session;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, SystemTime};

const CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Default)]
pub struct Proxies {
    pub enabled: bool,
    entries: HashMap<String, CacheEntry>,
    pending: Option<Pending>,
    batch: Option<Batch>,
}

struct Batch {
    remaining: VecDeque<MediaAsset>,
    total: usize,
    inspected: usize,
    succeeded: usize,
    first_error: Option<String>,
}

impl Batch {
    fn failed(&mut self, error: String) {
        if self.first_error.is_none() {
            self.first_error = Some(error);
        }
    }

    fn progress(&self) -> String {
        format!("Preparing proxy {} of {}", self.inspected + 1, self.total)
    }
}

struct CacheEntry {
    path: PathBuf,
    original: String,
    _lease: File,
}

struct Pending {
    id: String,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<CacheEntry, String>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Proxies {
    pub fn building(&self) -> bool {
        self.pending.is_some()
    }

    pub fn start(&mut self, asset: &MediaAsset) -> Result<(), String> {
        if self.building() {
            return Err("A proxy is already being prepared".into());
        }
        if !asset.online || !asset.has_video() {
            return Err("Select an online video asset".into());
        }
        let directory = proxy_directory().ok_or("Proxy cache directory is unavailable")?;
        let asset = asset.clone();
        let id = asset.id.clone();
        let (sender, result) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancelled = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("oq-proxy".into())
            .spawn(move || {
                let _ = sender.send(build(&directory, &asset, &cancelled));
            })
            .map_err(|error| error.to_string())?;
        self.pending = Some(Pending {
            id,
            cancel,
            result,
            worker: Some(worker),
        });
        Ok(())
    }

    pub fn start_batch(&mut self, assets: &[MediaAsset]) -> Result<String, String> {
        if self.building() {
            return Err("A proxy is already being prepared".into());
        }
        let mut remaining: VecDeque<_> = assets
            .iter()
            .filter(|asset| asset.online && asset.has_video())
            .cloned()
            .collect();
        let total = remaining.len();
        let first = remaining
            .pop_front()
            .ok_or("There are no online video assets")?;
        self.start(&first)?;
        let batch = Batch {
            remaining,
            total,
            inspected: 0,
            succeeded: 0,
            first_error: None,
        };
        let status = batch.progress();
        self.batch = Some(batch);
        Ok(status)
    }

    pub fn cancel(&self) {
        if let Some(job) = &self.pending {
            job.cancel.store(true, Ordering::Release);
        }
    }

    pub fn poll(&mut self) -> Option<String> {
        let job = self.pending.as_ref()?;
        let result = match job.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("Proxy worker stopped".into()),
        };
        let id = job.id.clone();
        let cancelled = job.cancel.load(Ordering::Acquire);
        self.pending = None;
        let result = if cancelled {
            Err("Proxy cancelled".into())
        } else {
            result.map(|entry| {
                self.entries.insert(id, entry);
            })
        };
        let Some(mut batch) = self.batch.take() else {
            return Some(match result {
                Ok(()) => "Proxy ready".into(),
                Err(error) if cancelled => error,
                Err(error) => format!("Proxy unavailable · {error}"),
            });
        };
        if cancelled {
            return Some(format!(
                "Proxy batch cancelled · {} of {} ready",
                batch.succeeded, batch.total
            ));
        }
        batch.inspected += 1;
        match result {
            Ok(()) => batch.succeeded += 1,
            Err(error) => batch.failed(error),
        }
        while let Some(asset) = batch.remaining.pop_front() {
            match self.start(&asset) {
                Ok(()) => {
                    let status = batch.progress();
                    self.batch = Some(batch);
                    return Some(status);
                }
                Err(error) => {
                    batch.inspected += 1;
                    batch.failed(error);
                }
            }
        }
        let detail = batch
            .first_error
            .map_or(String::new(), |error| format!(" · {error}"));
        Some(format!(
            "Proxy batch complete · {} of {} ready{detail}",
            batch.succeeded, batch.total
        ))
    }

    fn entry(&self, asset: &MediaAsset) -> Option<&CacheEntry> {
        let entry = self.entries.get(&asset.id)?;
        (self.enabled && asset.online && entry.original == asset.fingerprint).then_some(entry)
    }

    pub fn apply(&self, asset: &mut MediaAsset) {
        if let Some(entry) = self.entry(asset) {
            asset.path = entry.path.to_string_lossy().into_owned();
        }
    }

    pub fn project(&self, project: &ProjectData) -> ProjectData {
        let mut project = project.clone();
        if self.enabled {
            project
                .assets
                .iter_mut()
                .for_each(|asset| self.apply(asset));
        }
        project
    }
}

fn proxy_directory() -> Option<PathBuf> {
    std::env::var_os("OPEN_QUARTZ_PROXY_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library/Caches/OpenQuartz Studio/Proxies"))
        })
}

fn lock(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| error.to_string())
}

fn build(
    directory: &Path,
    asset: &MediaAsset,
    cancel: &Arc<AtomicBool>,
) -> Result<CacheEntry, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let journal = lock(&directory.join(".proxy-cache.lock"))?;
    journal.try_lock().map_err(|_| "The proxy cache is busy")?;
    let original = Path::new(&asset.path);
    let fingerprint = |path: &Path| fingerprint_until(path, || cancel.load(Ordering::Acquire));
    if fingerprint(original)? != asset.fingerprint {
        return Err("Original media changed since import".into());
    }
    let key = format!(
        "{:x}",
        Sha256::digest(format!("oq-preview-360p-v2:{}", asset.fingerprint).as_bytes())
    );
    let path = directory.join(format!("{key}.mp4"));
    let meta = path.with_extension("sha256");
    let lease = lock(&path.with_extension("lock"))?;
    if let Ok(hash) = fs::read_to_string(&meta) {
        if fingerprint(&path).is_ok_and(|current| current == hash) {
            lease
                .try_lock_shared()
                .map_err(|_| "The cached proxy is busy")?;
            if let Ok(file) = File::open(&path) {
                let _ = file.set_modified(SystemTime::now());
            }
            return Ok(CacheEntry {
                path,
                original: asset.fingerprint.clone(),
                _lease: lease,
            });
        }
    }
    lease.try_lock().map_err(|_| "The cached proxy is in use")?;
    let temp = directory.join(format!(".{}.mp4", new_id("building", 0)));
    let result: Result<(), String> = (|| {
        transcode(asset, &temp, cancel)?;
        match_frames(original, &temp, cancel)?;
        if fingerprint(original)? != asset.fingerprint {
            return Err("Original media changed during proxy preparation".into());
        }
        let bytes = fs::metadata(&temp)
            .map_err(|error| error.to_string())?
            .len();
        make_room(directory, &path, bytes)?;
        let hash = fingerprint(&temp)?;
        if cancel.load(Ordering::Acquire) {
            return Err("Proxy cancelled".into());
        }
        fs::rename(&temp, &path).map_err(|error| error.to_string())?;
        fs::write(&meta, hash).map_err(|error| error.to_string())?;
        Ok(())
    })();
    let _ = fs::remove_file(&temp);
    result?;
    lease.unlock().map_err(|error| error.to_string())?;
    lease
        .try_lock_shared()
        .map_err(|_| "The new proxy is busy")?;
    Ok(CacheEntry {
        path,
        original: asset.fingerprint.clone(),
        _lease: lease,
    })
}

fn transcode(asset: &MediaAsset, destination: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let encoder = crate::media::proxy_encoder_path()?;
    let scale = (640.0 / f64::from(asset.width))
        .min(360.0 / f64::from(asset.height))
        .min(1.0);
    let even = |value: u32| (f64::from(value) * scale / 2.0).round().max(1.0) as u32 * 2;
    let filter = format!(
        "scale={}:{}:out_color_matrix=bt709:out_range=tv",
        even(asset.width),
        even(asset.height)
    );
    let mut child = Command::new(encoder)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-copyts",
            "-start_at_zero",
            "-i",
        ])
        .arg(&asset.path)
        .args([
            "-map",
            "0:v:0",
            "-an",
            "-vf",
            &filter,
            "-fps_mode",
            "passthrough",
            "-enc_time_base",
        ])
        .arg(format!(
            "{}/{}",
            asset.time_base.numerator, asset.time_base.denominator
        ))
        .args([
            "-c:v",
            "h264_videotoolbox",
            "-allow_sw",
            "0",
            "-b:v",
            "1400k",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-video_track_timescale",
        ])
        .arg(asset.time_base.denominator.to_string())
        .args(["-fs", &CACHE_BYTES.to_string(), "-n"])
        .arg(destination)
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Proxy cancelled".into());
        }
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(format!("Proxy encoder exited with {status}")),
            None => std::thread::park_timeout(Duration::from_millis(20)),
        }
    }
}

fn match_frames(original: &Path, proxy: &Path, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    let mut original =
        Session::open_with_cancel(original, cancel.clone()).map_err(|error| error.to_string())?;
    let mut proxy =
        Session::open_with_cancel(proxy, cancel.clone()).map_err(|error| error.to_string())?;
    let duration = |session: &Session| {
        session
            .source_info()
            .duration_ns()
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Video duration is unknown".to_owned())
    };
    let original_duration = duration(&original)?;
    let proxy_duration = duration(&proxy)?;
    let mut last = None;
    loop {
        let source = original.next_frame().map_err(|error| error.to_string())?;
        let derived = proxy.next_frame().map_err(|error| error.to_string())?;
        match (source, derived) {
            (None, None) => break,
            (Some(source), Some(derived)) => {
                let pts = source
                    .frame
                    .pts_ns()
                    .ok_or("Unknown source frame timestamp")?;
                if derived.frame.pts_ns() != Some(pts) {
                    return Err("Proxy presentation timestamps differ from the original".into());
                }
                last = Some((pts, source, derived));
            }
            _ => return Err("Proxy frame count differs from the original".into()),
        }
    }
    let (last_pts, source, derived) = last.ok_or("Video has no presentation frames")?;
    // The project keeps the original edit bounds. A new MP4 may report a
    // different container duration after reordering the same presentation frames.
    let tail_end = |frame: oq_media::FrameLease, duration| {
        frame
            .frame
            .presentation_end_ns()
            .map(|end| {
                end.unwrap_or(last_pts)
                    .min(duration)
                    .min(original_duration)
                    .max(last_pts)
            })
            .map_err(|error| error.to_string())
    };
    if tail_end(source, original_duration)? != tail_end(derived, proxy_duration)? {
        return Err("Proxy does not preserve the original tail frame coverage".into());
    }
    Ok(())
}

fn make_room(directory: &Path, target: &Path, incoming: u64) -> Result<(), String> {
    let cache_error = |error: std::io::Error| error.to_string();
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(cache_error)?
        .filter_map(Result::ok)
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path == target
            || !name.strip_suffix(".mp4").is_some_and(|stem| {
                stem.len() == 64 && stem.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            continue;
        }
        let metadata = entry.metadata().map_err(cache_error)?;
        files.push((
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            path,
            metadata.len(),
        ));
    }
    files.sort();
    let mut total = files
        .iter()
        .fold(incoming, |sum, entry| sum.saturating_add(entry.2));
    for (_, path, size) in files {
        if total <= CACHE_BYTES {
            return Ok(());
        }
        let lease = lock(&path.with_extension("lock"))?;
        if lease.try_lock().is_err() {
            continue;
        }
        fs::remove_file(&path).map_err(cache_error)?;
        let _ = fs::remove_file(path.with_extension("sha256"));
        total = total.saturating_sub(size);
    }
    if total > CACHE_BYTES {
        Err("Proxy cache has no evictable space".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_quota_evicts_unleased_entries_and_keeps_active_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let active = dir.path().join(format!("{}.mp4", "a".repeat(64)));
        let disposable = dir.path().join(format!("{}.mp4", "b".repeat(64)));
        let target = dir.path().join(format!("{}.mp4", "c".repeat(64)));
        for path in [&active, &disposable] {
            File::create(path)
                .unwrap()
                .set_len(CACHE_BYTES / 2)
                .unwrap();
        }
        let lease = lock(&active.with_extension("lock")).unwrap();
        lease.try_lock_shared().unwrap();
        make_room(dir.path(), &target, 128).unwrap();
        assert!(active.is_file());
        assert!(!disposable.exists());
    }
}
