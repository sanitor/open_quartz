use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineState {
    Empty,
    Ready,
    Running,
    Paused,
    Stopped,
    Disposed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum EngineEvent {
    State {
        state: EngineState,
    },
    GraphReady {
        revision: u32,
    },
    ResourceInvalidated {
        node_id: String,
        generation: u32,
    },
    ResourceReleased {
        node_id: String,
        generation: u32,
    },
    FramePlanned {
        frame: u64,
        revision: u32,
        command_count: u32,
        dirty_node_count: u32,
    },
}
