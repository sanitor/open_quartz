use crate::execution::{FrameEvaluationInput, FrameExecutionSession};
use crate::evaluation::EngineState;
use crate::outputs::{FrameStamp, PlaybackCapabilities};
use crate::project::{SdkError, SdkErrorCode};
use serde::{Deserialize, Serialize};
use super::{ClockState, CompositionClock};
#[cfg(test)]
use crate::execution::session::decode_engine_error;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFrameInput {
    pub now_ns: u64,
    pub date: [f32; 4],
    pub mouse: [f32; 4],
    pub resolution: [f32; 3],
}

/// Playback adds a single clock to the same clock-free graph/output owner.
pub struct PlaybackSession {
    frame: FrameExecutionSession,
    clock: CompositionClock,
}

impl std::ops::Deref for PlaybackSession {
    type Target = FrameExecutionSession;
    fn deref(&self) -> &Self::Target { &self.frame }
}
impl std::ops::DerefMut for PlaybackSession {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.frame }
}

impl PlaybackSession {
    pub fn new(capabilities: PlaybackCapabilities) -> Self {
        Self { frame: FrameExecutionSession::new(capabilities), clock: CompositionClock::new(16_666_667) }
    }
    pub fn new_native(capabilities: PlaybackCapabilities) -> Self {
        Self { frame: FrameExecutionSession::new_native(capabilities), clock: CompositionClock::new(16_666_667) }
    }
    pub fn play(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Ready {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession can only play from the ready state",
            ));
        }
        self.clock.start(now_ns);
        Ok(())
    }

    pub fn pause(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Running {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession can only pause while running",
            ));
        }
        self.clock
            .pause(now_ns)
            .expect("running session owns a running clock");
        self.frame.pause()
    }

    pub fn resume(&mut self, now_ns: u64) -> Result<(), SdkError> {
        if self.state() != EngineState::Paused {
            return Err(SdkError::new(
                SdkErrorCode::InvalidState,
                "PlaybackSession can only resume while paused",
            ));
        }
        self.clock
            .resume(now_ns)
            .expect("paused session owns a paused clock");
        self.frame.resume()
    }

    pub fn advance(&mut self, input: &SessionFrameInput) -> Result<ClockState, SdkError> {
        let mut candidate_clock = self.clock.clone();
        let clock = candidate_clock.tick(input.now_ns)?;
        let stamp = FrameStamp { epoch: clock.epoch, frame: clock.frame,
            timeline_ns: clock.timeline_ns, deadline_ns: clock.next_deadline_ns };
        let result = self.frame.evaluate(&FrameEvaluationInput {
            stamp: stamp.clone(), previous_timeline_ns: clock.previous_timeline_ns,
            date: input.date, mouse: input.mouse, resolution: input.resolution,
        });
        // Output preflight errors do not advance either state. Backend dispatch
        // happens after graph commit and cannot roll back GPU/backend effects.
        if self.frame.last_evaluation() == Some(&stamp) { self.clock = candidate_clock; }
        result?;
        Ok(clock)
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.frame.stop()?;
        self.clock.stop();
        Ok(())
    }

    pub fn dispose(&mut self) -> Result<(), SdkError> {
        self.frame.dispose()?;
        self.clock.stop();
        Ok(())
    }

}

#[cfg(test)]
#[path = "session_coverage_tests.rs"]
mod coverage_tests;
