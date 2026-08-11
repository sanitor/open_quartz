use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockState {
    pub epoch: u64,
    pub timeline_ns: u64,
    pub previous_timeline_ns: u64,
    pub frame: u64,
    pub next_deadline_ns: u64,
}

#[derive(Clone, Debug)]
pub struct CompositionClock {
    epoch: u64,
    accumulated_active_ns: u64,
    running_since_ns: Option<u64>,
    previous_timeline_ns: u64,
    frame: u64,
    next_deadline_ns: Option<u64>,
    period_ns: u64,
}

impl CompositionClock {
    pub fn new(period_ns: u64) -> Self {
        Self {
            epoch: 0,
            accumulated_active_ns: 0,
            running_since_ns: None,
            previous_timeline_ns: 0,
            frame: 0,
            next_deadline_ns: None,
            period_ns: period_ns.max(1),
        }
    }

    pub fn start(&mut self, now_ns: u64) {
        self.epoch = self.epoch.saturating_add(1);
        self.accumulated_active_ns = 0;
        self.running_since_ns = Some(now_ns);
        self.previous_timeline_ns = 0;
        self.frame = 0;
        self.next_deadline_ns = Some(now_ns);
    }

    pub fn pause(&mut self, now_ns: u64) -> Result<(), SdkError> {
        let Some(running_since) = self.running_since_ns.take() else {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Clock is not running",
            ));
        };
        self.accumulated_active_ns = self
            .accumulated_active_ns
            .saturating_add(now_ns.saturating_sub(running_since));
        self.next_deadline_ns = None;
        Ok(())
    }

    pub fn resume(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.running_since_ns.is_some() {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Clock is already running",
            ));
        }
        self.running_since_ns = Some(now_ns);
        self.next_deadline_ns = Some(now_ns);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.epoch = self.epoch.saturating_add(1);
        self.accumulated_active_ns = 0;
        self.running_since_ns = None;
        self.previous_timeline_ns = 0;
        self.frame = 0;
        self.next_deadline_ns = None;
    }

    pub fn tick(&mut self, now_ns: u64) -> Result<ClockState, SdkError> {
        let Some(running_since) = self.running_since_ns else {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "Clock is not running",
            ));
        };
        let timeline_ns = self
            .accumulated_active_ns
            .saturating_add(now_ns.saturating_sub(running_since));
        let previous_timeline_ns = self.previous_timeline_ns;
        let deadline_ns = self.next_deadline_ns.unwrap_or(now_ns);
        self.previous_timeline_ns = timeline_ns;
        self.frame = self.frame.saturating_add(1);
        self.next_deadline_ns = Some(deadline_ns.saturating_add(self.period_ns));
        Ok(ClockState {
            epoch: self.epoch,
            timeline_ns,
            previous_timeline_ns,
            frame: self.frame,
            next_deadline_ns: deadline_ns,
        })
    }

    pub fn state(&self) -> ClockState {
        ClockState {
            epoch: self.epoch,
            timeline_ns: self.accumulated_active_ns,
            previous_timeline_ns: self.previous_timeline_ns,
            frame: self.frame,
            next_deadline_ns: self.next_deadline_ns.unwrap_or(0),
        }
    }
}
