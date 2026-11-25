use super::StableFrame;
use oq_media::EncoderSurface;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::Instant,
};

#[derive(Clone)]
pub struct ConversionReady(pub(super) Arc<Completion>);
pub(super) struct Completion {
    pub(super) ready: AtomicBool,
    pub(super) completed_at: OnceLock<Instant>,
}
impl ConversionReady {
    /// Observes actual queue completion; does not submit or wait for GPU work.
    pub fn is_ready(&self) -> bool {
        self.0.ready.load(Ordering::Acquire)
    }
    /// Host callback observation, present only when tracing was enabled.
    pub fn completed_at(&self) -> Option<Instant> {
        self.0.completed_at.get().copied()
    }
}
pub struct ConvertedFrame {
    pub surface: EncoderSurface,
    pub ready: ConversionReady,
}
pub enum ConversionAdmission {
    Accepted(ConvertedFrame),
    WouldBlock(StableFrame),
}
