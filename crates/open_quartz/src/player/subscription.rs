use std::sync::{Mutex, Weak};

use crate::player::PlaybackSession;
use crate::project::SdkError;

use super::playback::lock_runtime;

pub struct Subscription {
    runtime: Weak<Mutex<PlaybackSession>>,
    subscription_id: String,
    closed: bool,
}

impl Subscription {
    pub(super) fn new(runtime: Weak<Mutex<PlaybackSession>>, subscription_id: String) -> Self {
        Self {
            runtime,
            subscription_id,
            closed: false,
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn close(&mut self) -> Result<(), SdkError> {
        if self.closed {
            return Ok(());
        }
        if let Some(runtime) = self.runtime.upgrade() {
            lock_runtime(&runtime)?.unsubscribe_output(&self.subscription_id)?;
        }
        self.closed = true;
        Ok(())
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "subscription_coverage_tests.rs"]
mod coverage_tests;
