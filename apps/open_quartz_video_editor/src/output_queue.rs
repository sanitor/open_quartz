use crate::{delivery::ExportOptions, document::ProjectData};
use std::{ops::Range, path::PathBuf};

#[derive(Clone)]
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
}

impl OutputQueue {
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

    pub fn update_active(&mut self, status: &str, outcome: Option<Phase>) {
        let Some(index) = self.active else { return; };
        let job = &mut self.jobs[index];
        if job.status != status { job.status = status.to_owned(); }
        if let Some(phase) = outcome {
            job.phase = phase;
            self.active = None;
        }
    }

    pub fn cancel_pending(&mut self, index: usize) {
        if let Some(job) = self.jobs.get_mut(index).filter(|job| job.phase == Phase::Pending) {
            job.phase = Phase::Cancelled;
        }
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
        self.jobs[index].phase = Phase::Pending;
        self.jobs[index].status.clear();
        Ok(())
    }
}
