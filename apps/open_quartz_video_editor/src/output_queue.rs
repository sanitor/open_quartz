use crate::{delivery::ExportOptions, document::ProjectData};
use std::{fs::{self, File, OpenOptions}, ops::Range, path::{Path, PathBuf}};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ExportSpec {
    pub project: ProjectData,
    pub revision: u64,
    pub path: PathBuf,
    pub frames: Range<u64>,
    pub options: ExportOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Running,
    Complete,
    Failed,
    Cancelled,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Rendering",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
}

pub struct Job {
    pub spec: ExportSpec,
    pub phase: Phase,
    pub status: String,
}

#[derive(Default)]
pub struct OutputQueue {
    pub jobs: Vec<Job>,
    pub running: bool,
    pub active: Option<usize>,
    storage: Option<Result<Storage, String>>,
}

struct Storage {
    path: PathBuf,
    _lock: File,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot<T> {
    version: u32,
    jobs: Vec<T>,
}

impl OutputQueue {
    pub fn open() -> Self {
        let directory = std::env::var_os("OPEN_QUARTZ_RENDER_QUEUE_DIR").map(PathBuf::from)
            .or_else(|| {
                let root = if cfg!(windows) {
                    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
                } else {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
                };
                root.map(|root| root.join("OpenQuartz Studio/Render Queue"))
            });
        let loaded = directory.ok_or_else(|| "No render queue data directory is available".to_owned())
            .and_then(|directory| Self::load(&directory));
        match loaded {
            Ok(queue) => queue,
            Err(error) => Self { storage: Some(Err(error)), ..Self::default() },
        }
    }

    fn load(directory: &Path) -> Result<Self, String> {
        fs::create_dir_all(directory).map_err(|error| format!("Cannot open render queue directory: {error}"))?;
        let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .open(directory.join("queue.lock")).map_err(|error| format!("Cannot open render queue lock: {error}"))?;
        lock.try_lock().map_err(|error| format!("Cannot lock render queue: {error}"))?;
        let path = directory.join("queue.json");
        let specs = match fs::read(&path) {
            Ok(bytes) => {
                let snapshot: Snapshot<ExportSpec> = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("Cannot decode render queue: {error}"))?;
                if snapshot.version != 1 { return Err("Unsupported render queue version".into()); }
                snapshot.jobs
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(format!("Cannot read render queue: {error}")),
        };
        let mut queue = Self::default();
        for spec in specs {
            crate::document::validate(&spec.project)?;
            if !spec.path.is_absolute() || spec.frames.is_empty() {
                return Err("Render queue contains an invalid destination or frame range".into());
            }
            if queue.jobs.iter().any(|job| job.spec.path == spec.path) {
                return Err("Render queue contains duplicate output paths".into());
            }
            queue.jobs.push(Job { spec, phase: Phase::Pending, status: String::new() });
        }
        queue.storage = Some(Ok(Storage { path, _lock: lock }));
        Ok(queue)
    }

    pub fn storage_error(&self) -> Option<&str> {
        self.storage.as_ref().and_then(|storage| storage.as_ref().err()).map(String::as_str)
    }

    fn persist(&self) -> Result<(), String> {
        let Some(storage) = &self.storage else { return Ok(()); };
        let storage = storage.as_ref().map_err(Clone::clone)?;
        let jobs = self.jobs.iter().filter(|job| matches!(job.phase, Phase::Pending | Phase::Running))
            .map(|job| &job.spec).collect();
        let bytes = serde_json::to_vec(&Snapshot { version: 1, jobs }).map_err(|error| error.to_string())?;
        crate::document::write_project_atomic(&storage.path, &bytes, 0, true)
            .map_err(|error| format!("Cannot save render queue: {error}"))
    }

    pub fn add(&mut self, mut spec: ExportSpec) -> Result<(), String> {
        let directory = spec.path.parent().ok_or("Delivery folder is missing")?
            .canonicalize().map_err(|error| format!("Cannot open delivery folder · {error}"))?;
        let name = spec.path.file_name().ok_or("Delivery filename is missing")?;
        spec.path = directory.join(name);
        if self.jobs.iter().any(|job| job.spec.path == spec.path &&
            matches!(job.phase, Phase::Pending | Phase::Running))
        {
            return Err("A queued delivery already uses that output path".into());
        }
        self.jobs.push(Job { spec, phase: Phase::Pending, status: String::new() });
        if let Err(error) = self.persist() {
            self.jobs.pop();
            return Err(error);
        }
        Ok(())
    }

    pub fn has_pending(&self) -> bool {
        self.jobs.iter().any(|job| job.phase == Phase::Pending)
    }

    pub fn next(&mut self) -> Option<ExportSpec> {
        if !self.running || self.active.is_some() { return None; }
        let Some(index) = self.jobs.iter().position(|job| job.phase == Phase::Pending) else {
            self.running = false;
            return None;
        };
        self.active = Some(index);
        let job = &mut self.jobs[index];
        job.phase = Phase::Running;
        job.status.clear();
        Some(job.spec.clone())
    }

    pub fn update_active(&mut self, status: &str, outcome: Option<Phase>) -> Result<(), String> {
        let Some(index) = self.active else { return Ok(()); };
        let job = &mut self.jobs[index];
        if job.status != status { job.status = status.to_owned(); }
        if let Some(phase) = outcome {
            job.phase = phase;
            self.active = None;
            if let Err(error) = self.persist() {
                self.running = false;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn cancel_pending(&mut self, index: usize) -> Result<(), String> {
        if let Some(job) = self.jobs.get_mut(index).filter(|job| job.phase == Phase::Pending) {
            job.phase = Phase::Cancelled;
            if let Err(error) = self.persist() {
                self.jobs[index].phase = Phase::Pending;
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn retry(&mut self, index: usize) -> Result<(), String> {
        let Some(job) = self.jobs.get(index).filter(|job| matches!(job.phase, Phase::Failed | Phase::Cancelled)) else {
            return Ok(());
        };
        if self.jobs.iter().any(|other| other.spec.path == job.spec.path
            && matches!(other.phase, Phase::Pending | Phase::Running))
        {
            return Err("A queued delivery already uses that output path".into());
        }
        let previous = self.jobs[index].phase;
        self.jobs[index].phase = Phase::Pending;
        if let Err(error) = self.persist() {
            self.jobs[index].phase = previous;
            return Err(error);
        }
        self.jobs[index].status.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::ExportColor;

    #[test]
    fn restart_restores_frozen_running_spec_without_starting_or_touching_output() {
        let folder = tempfile::tempdir().unwrap();
        let destination = folder.path().join("delivery.mp4");
        let mut queue = OutputQueue::load(folder.path()).unwrap();
        queue.add(ExportSpec {
            project: ProjectData { name: "Frozen cut".into(), ..ProjectData::default() },
            revision: 12,
            path: destination.clone(),
            frames: 4..10,
            options: ExportOptions { resolution: 2, frame_rate: 0, quality: 0, color: ExportColor::Hlg },
        }).unwrap();
        assert!(OutputQueue::load(folder.path()).is_err());
        queue.running = true;
        assert!(queue.next().is_some());
        drop(queue);
        let mut queue = OutputQueue::load(folder.path()).unwrap();
        assert!(queue.next().is_none());
        assert!(queue.has_pending());
        let job = &queue.jobs[0];
        assert_eq!(job.spec.project.name, "Frozen cut");
        assert_eq!(job.spec.revision, 12);
        assert_eq!(job.spec.frames, 4..10);
        assert!(job.spec.options.color == ExportColor::Hlg);
        assert!(!destination.exists());
        queue.cancel_pending(0).unwrap();
        drop(queue);
        assert!(!OutputQueue::load(folder.path()).unwrap().has_pending());
    }
}
