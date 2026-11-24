use crate::document::{new_id, EditorDocument};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const RECOVERY_DELAY: Duration = Duration::from_secs(2);
const SUFFIX: &str = ".recovery.oqvideo.json";

pub struct Recovery {
    directory: Option<PathBuf>,
    path: Option<PathBuf>,
    offered: Option<PathBuf>,
    lease: Option<File>,
    restored: Option<PathBuf>,
    last_write: Instant,
    last_revision: Option<u64>,
    pending: Option<Save>,
}

struct Save {
    done: mpsc::Receiver<Result<(), String>>,
    worker: JoinHandle<()>,
}

impl Recovery {
    pub fn new() -> Self {
        let directory = std::env::var_os("OPEN_QUARTZ_EDITOR_RECOVERY_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join("Library/Application Support/OpenQuartz Studio/Recovery"))
            });
        let path = directory
            .as_ref()
            .map(|dir| dir.join(format!("{}{SUFFIX}", new_id("edit", 0))));
        let offered = directory.as_deref().and_then(latest_snapshot);
        Self {
            directory,
            path,
            offered,
            lease: None,
            restored: None,
            last_write: Instant::now(),
            last_revision: None,
            pending: None,
        }
    }

    pub fn offer(&self) -> Option<&Path> {
        self.offered.as_deref()
    }

    pub fn discard_offer(&mut self) -> Result<(), String> {
        if let Some(path) = &self.offered {
            remove_snapshot(path)?;
        }
        self.offered = self
            .directory
            .as_deref()
            .and_then(latest_snapshot)
            .filter(|path| Some(path) != self.path.as_ref());
        Ok(())
    }

    pub fn mark_restored(&mut self, path: PathBuf) {
        self.reset_current();
        self.offered = None;
        self.restored = Some(path);
    }

    pub fn saved(&mut self) {
        let restored = self.restored.take();
        self.reset_current();
        if let Some(path) = restored {
            let _ = remove_snapshot(&path);
        }
        self.offered = self.directory.as_deref().and_then(latest_snapshot);
    }

    pub fn reset_current(&mut self) {
        if let Some(save) = self.pending.take() {
            let _ = save.worker.join();
        }
        if let Some(path) = &self.path {
            let _ = remove_snapshot(path);
        }
        if self.lease.take().is_some() {
            if let Some(path) = &self.path {
                let _ = fs::remove_file(lock_path(path));
            }
        }
        self.last_revision = None;
        self.last_write = Instant::now();
        self.restored = None;
    }

    pub fn poll(&mut self, document: &EditorDocument) -> Option<Result<(), String>> {
        if !document.is_dirty() {
            if self.last_revision.is_some() {
                self.reset_current();
            }
            return None;
        }
        if let Some(save) = &self.pending {
            let result = match save.done.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => Err("Recovery worker stopped".into()),
            };
            if let Some(save) = self.pending.take() {
                let _ = save.worker.join();
            }
            if result.is_err() {
                self.last_revision = None;
            }
            return Some(result);
        }
        if self.last_revision == Some(document.revision)
            || self.last_write.elapsed() < RECOVERY_DELAY
        {
            return None;
        }
        let path = self.path.clone()?;
        self.last_revision = Some(document.revision);
        self.last_write = Instant::now();
        if self.lease.is_none() {
            match lock_snapshot(&path) {
                Ok(lease) => self.lease = Some(lease),
                Err(error) => {
                    self.last_revision = None;
                    return Some(Err(error));
                }
            }
        }
        let data = document.data.clone();
        let (send, done) = mpsc::channel();
        match std::thread::Builder::new()
            .name("oq-editor-recovery".into())
            .spawn(move || {
                let _ = send.send(EditorDocument::write_recovery(data, &path));
            }) {
            Ok(worker) => self.pending = Some(Save { done, worker }),
            Err(error) => {
                self.last_revision = None;
                return Some(Err(error.to_string()));
            }
        }
        None
    }
}

fn remove_snapshot(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => {
            let _ = fs::remove_file(lock_path(path));
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot discard recovery snapshot: {error}")),
    }
}

fn latest_snapshot(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(SUFFIX))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let path = entry.path();
            if !metadata.is_file() {
                return None;
            }
            let lease = OpenOptions::new()
                .read(true)
                .write(true)
                .open(lock_path(&path))
                .ok();
            if lease.is_some_and(|file| file.try_lock().is_err()) {
                return None;
            }
            Some((metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), path))
        })
        .max()
        .map(|(_, path)| path)
}

fn lock_path(path: &Path) -> PathBuf {
    path.with_extension("lock")
}

fn lock_snapshot(path: &Path) -> Result<File, String> {
    let parent = path.parent().ok_or("Recovery directory unavailable")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Cannot create recovery directory: {error}"))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path(path))
        .map_err(|error| format!("Cannot reserve recovery snapshot: {error}"))?;
    file.try_lock()
        .map_err(|error| format!("Cannot lock recovery snapshot: {error}"))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ProjectData;

    #[test]
    fn a_recovery_copy_is_offered_only_after_the_writing_process_releases_it() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = directory.path().join(format!("pending{SUFFIX}"));
        let owner = lock_snapshot(&snapshot).unwrap();
        EditorDocument::write_recovery(ProjectData::default(), &snapshot).unwrap();
        assert!(latest_snapshot(directory.path()).is_none());
        drop(owner);
        assert_eq!(latest_snapshot(directory.path()), Some(snapshot));
    }
}
