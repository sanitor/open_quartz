use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::evaluation::event::EngineState;
use crate::player::{PlaybackSession, SessionFrameInput};
use crate::project::{Graph, GraphChange, SdkError, SdkErrorCode};
use crate::resources::{Resource, ResourceCatalog};

use super::output::build_outputs;
use super::{Environment, Output};

pub struct PlayerBuilder {
    environment: Environment,
    graph: Graph,
    resources: Vec<Resource>,
}

impl PlayerBuilder {
    pub(super) fn new(environment: Environment, graph: Graph) -> Self {
        Self {
            environment,
            graph,
            resources: Vec::new(),
        }
    }

    pub fn with_resources(mut self, resources: &ResourceCatalog) -> Self {
        self.resources = resources.iter().cloned().collect();
        self
    }

    pub fn build(self) -> Result<Player, SdkError> {
        self.graph.validate()?;
        let mut runtime = PlaybackSession::new(self.environment.capabilities());
        runtime
            .set_graph(&self.graph)
            .expect("validated Graph should initialize the PlaybackSession");
        let runtime = Arc::new(Mutex::new(runtime));
        let subscription_counter = Arc::new(AtomicU64::new(1));
        let outputs = build_outputs(&self.graph, &runtime, &subscription_counter);
        Ok(Player {
            runtime,
            outputs,
            resources: self.resources,
            subscription_counter,
            now_ns: 0,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Empty,
    Ready,
    Playing,
    Paused,
    Stopped,
    Closed,
}

pub struct Player {
    runtime: Arc<Mutex<PlaybackSession>>,
    outputs: Vec<Output>,
    resources: Vec<Resource>,
    subscription_counter: Arc<AtomicU64>,
    now_ns: u64,
}

impl Player {
    pub fn state(&self) -> PlayerState {
        let Ok(runtime) = self.runtime.lock() else {
            return PlayerState::Closed;
        };
        map_player_state(runtime.state())
    }

    pub fn play(&mut self) -> Result<(), SdkError> {
        let start = self.next_now();
        let first_frame = self.next_now();
        let mut runtime = self.open_runtime()?;
        runtime.play(start)?;
        runtime
            .advance(&SessionFrameInput {
                now_ns: first_frame,
                date: [0.0; 4],
                mouse: [0.0; 4],
                resolution: [1.0, 1.0, 1.0],
            })
            .expect("a newly playing PlaybackSession should accept its first frame");
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), SdkError> {
        let now = self.next_now();
        self.open_runtime()?.pause(now)
    }

    pub fn resume(&mut self) -> Result<(), SdkError> {
        let now = self.next_now();
        self.open_runtime()?.resume(now)
    }

    pub fn stop(&mut self) -> Result<(), SdkError> {
        self.open_runtime()?.stop()
    }

    pub fn apply_graph(&mut self, graph: &Graph, _change: &GraphChange) -> Result<u32, SdkError> {
        graph.validate()?;
        let revision = self
            .open_runtime()?
            .set_graph(graph)
            .expect("validated Graph should update the PlaybackSession");
        self.outputs = build_outputs(graph, &self.runtime, &self.subscription_counter);
        Ok(revision)
    }

    pub fn close(&mut self) -> Result<(), SdkError> {
        let mut runtime = lock_runtime(&self.runtime)?;
        if runtime.state() == EngineState::Disposed {
            return Ok(());
        }
        runtime.dispose()
    }

    pub fn graph_revision(&self) -> u32 {
        lock_runtime(&self.runtime)
            .map(|runtime| runtime.revision())
            .unwrap_or_default()
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    fn open_runtime(&self) -> Result<MutexGuard<'_, PlaybackSession>, SdkError> {
        let runtime = lock_runtime(&self.runtime)?;
        if runtime.state() == EngineState::Disposed {
            return Err(SdkError::new(
                SdkErrorCode::Disposed,
                "Player has been closed",
            ));
        }
        Ok(runtime)
    }

    fn next_now(&mut self) -> u64 {
        self.now_ns = self.now_ns.saturating_add(1);
        self.now_ns
    }
}

fn map_player_state(state: EngineState) -> PlayerState {
    match state {
        EngineState::Empty => PlayerState::Empty,
        EngineState::Ready => PlayerState::Ready,
        EngineState::Running => PlayerState::Playing,
        EngineState::Paused => PlayerState::Paused,
        EngineState::Stopped => PlayerState::Stopped,
        EngineState::Disposed => PlayerState::Closed,
    }
}

pub(super) fn lock_runtime(runtime: &Arc<Mutex<PlaybackSession>>) -> Result<MutexGuard<'_, PlaybackSession>, SdkError> {
    runtime.lock().map_err(|_| {
        SdkError::new(
            SdkErrorCode::InvalidState,
            "Player runtime lock is poisoned",
        )
    })
}

#[cfg(test)]
#[path = "playback_coverage_tests.rs"]
mod coverage_tests;
