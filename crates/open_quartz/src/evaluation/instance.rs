use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;


use crate::evaluation::{ExecutionCommand, GraphEvaluator, FrameInputs};
use crate::compiler::{ExecutionPlan};
use crate::project::model::{Edge, Graph, ProjectNode};

use crate::evaluation::event::{EngineEvent, EngineState};
use crate::project::model::{SdkError, SdkErrorCode};


#[derive(Clone)]
pub struct GraphInstance {
    inner: Option<GraphEvaluator>,
    graph: Option<Arc<Graph>>,
    revision: u32,
    node_generations: HashMap<String, u32>,
    video_nodes: Vec<String>,
    pending_commands: Vec<ExecutionCommand>,
    last_frame: Option<u64>,
    events: VecDeque<EngineEvent>,
    state: EngineState,
    external_video_textures: bool,
}

impl GraphInstance {
    pub fn new() -> Self {
        new_engine(true)
    }

    pub fn set_graph_json(&mut self, graph_json: &str) -> Result<u32, String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        let graph: Graph = serde_json::from_str(graph_json).map_err(|error| {
            SdkError::new(SdkErrorCode::InvalidGraph, "Invalid graph JSON")
                .with_details(error.to_string())
                .to_json()
        })?;
        self.set_graph(graph).map_err(|error| error.to_json())
    }

    pub fn mark_dirty(&mut self, node_id: &str) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        if !self.node_generations.contains_key(node_id) {
            return Err(
                SdkError::new(SdkErrorCode::UnknownNode, "Unknown graph node")
                    .for_node(node_id)
                    .to_json(),
            );
        }
        self.prepared_mut()?.mark_dirty(node_id);
        Ok(())
    }

    pub fn mark_dependents_dirty(&mut self, node_id: &str) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        if !self.node_generations.contains_key(node_id) {
            return Err(
                SdkError::new(SdkErrorCode::UnknownNode, "Unknown graph node")
                    .for_node(node_id)
                    .to_json(),
            );
        }
        self.prepared_mut()?.mark_dependents_dirty(node_id);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn run_frame(
        &mut self,
        time: f64,
        delta: f64,
        frame: u64,
        date: &[f32],
        mouse: &[f32],
        resolution: &[f32],
    ) -> Result<(), String> {
        self.ensure_can_run().map_err(|error| error.to_json())?;
        let frame_inputs = FrameInputs {
            time,
            delta,
            frame,
            date: fixed_frame_array::<4>("date", date)?,
            mouse: fixed_frame_array::<4>("mouse", mouse)?,
            resolution: fixed_frame_array::<3>("resolution", resolution)?,
            video_nodes: Vec::new(),
        };
        let (inner, video_nodes) = (&mut self.inner, &self.video_nodes);
        let engine = inner.as_mut().ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::NotPrepared,
                "GraphInstance must receive a graph before this operation",
            )
            .to_json()
        })?;
        for node_id in video_nodes {
            engine.mark_dirty(node_id);
        }
        let result = engine.run_frame(&frame_inputs);
        let command_count = result.commands.len() as u32;
        let dirty_node_count = result.dirty_nodes.len() as u32;
        self.pending_commands = result.commands;
        self.last_frame = Some(frame);
        self.transition(EngineState::Running);
        self.push_frame_planned(frame, command_count, dirty_node_count);
        Ok(())
    }

    pub fn set_video_nodes_json(&mut self, node_ids_json: &str) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        let node_ids: Vec<String> = serde_json::from_str(node_ids_json).map_err(|error| {
            SdkError::new(
                SdkErrorCode::InvalidResource,
                "Invalid video node configuration",
            )
            .with_details(error.to_string())
            .to_json()
        })?;
        self.set_video_nodes(&node_ids)
    }

    pub fn set_video_nodes(&mut self, node_ids: &[String]) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        if let Some(node_id) = node_ids
            .iter()
            .find(|node_id| !self.node_generations.contains_key(*node_id))
        {
            return Err(
                SdkError::new(SdkErrorCode::UnknownNode, "Unknown video node")
                    .for_node(node_id)
                    .to_json(),
            );
        }
        self.video_nodes = node_ids.to_vec();
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        match self.state {
            EngineState::Ready | EngineState::Running | EngineState::Paused => {
                self.transition(EngineState::Paused);
                Ok(())
            }
            _ => Err(self.invalid_state("pause")),
        }
    }

    pub fn resume(&mut self) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        if self.state != EngineState::Paused {
            return Err(self.invalid_state("resume"));
        }
        self.transition(EngineState::Running);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        if self.inner.is_none() {
            return Err(self.invalid_state("stop"));
        }
        self.pending_commands.clear();
        self.last_frame = None;
        self.transition(EngineState::Stopped);
        Ok(())
    }

    pub fn node_generation(&self, node_id: &str) -> Result<u32, String> {
        self.ensure_active().map_err(|error| error.to_json())?;
        self.node_generations.get(node_id).copied().ok_or_else(|| {
            SdkError::new(SdkErrorCode::UnknownNode, "Unknown graph node")
                .for_node(node_id)
                .to_json()
        })
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn last_frame(&self) -> Option<u64> {
        self.last_frame
    }

    pub fn pending_command_count(&self) -> u32 {
        self.pending_commands.len() as u32
    }

    pub(crate) fn reset_temporal_state(&mut self) -> Result<(), SdkError> {
        self.ensure_active()?;
        let graph = self.graph.as_ref().ok_or_else(|| SdkError::new(
            SdkErrorCode::NotPrepared, "GraphInstance must receive a graph before reset"))?;
        self.inner = Some(GraphEvaluator::prepare_with_options(
            graph.nodes.clone(), graph.edges.clone(), self.external_video_textures));
        self.pending_commands.clear();
        self.last_frame = None;
        Ok(())
    }

    pub fn engine_state(&self) -> String {
        engine_state_name(self.state).to_owned()
    }

    pub(crate) fn state_value(&self) -> EngineState {
        self.state
    }

    pub fn drain_events_json(&mut self) -> String {
        serde_json::to_string(&self.drain_events()).expect("EngineEvent is always JSON serializable")
    }

    pub fn drain_events(&mut self) -> Vec<EngineEvent> {
        self.events.drain(..).collect()
    }

    pub fn dispose(&mut self) {
        if self.state == EngineState::Disposed {
            return;
        }
        self.inner = None;
        self.graph = None;
        self.node_generations.clear();
        self.video_nodes.clear();
        self.pending_commands.clear();
        self.last_frame = None;
        self.events.clear();
        self.state = EngineState::Disposed;
        self.events.push_back(EngineEvent::State {
            state: EngineState::Disposed,
        });
    }

    fn prepared_mut(&mut self) -> Result<&mut GraphEvaluator, String> {
        self.inner.as_mut().ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::NotPrepared,
                "GraphInstance must receive a graph before this operation",
            )
            .to_json()
        })
    }

    fn ensure_active(&self) -> Result<(), SdkError> {
        if self.state == EngineState::Disposed {
            Err(SdkError::new(
                SdkErrorCode::Disposed,
                "GraphInstance has been disposed",
            ))
        } else {
            Ok(())
        }
    }

    fn ensure_can_run(&self) -> Result<(), SdkError> {
        self.ensure_active()?;
        match self.state {
            EngineState::Ready | EngineState::Running => Ok(()),
            EngineState::Empty => Err(SdkError::new(
                SdkErrorCode::NotPrepared,
                "GraphInstance must receive a graph before runFrame",
            )),
            _ => Err(SdkError::new(
                SdkErrorCode::InvalidState,
                format!(
                    "Cannot runFrame while engine is {}",
                    engine_state_name(self.state)
                ),
            )),
        }
    }

    fn invalid_state(&self, operation: &str) -> String {
        SdkError::new(
            SdkErrorCode::InvalidState,
            format!(
                "Cannot {operation} while engine is {}",
                engine_state_name(self.state)
            ),
        )
        .to_json()
    }

    fn push_frame_planned(&mut self, frame: u64, command_count: u32, dirty_node_count: u32) {
        let event = EngineEvent::FramePlanned {
            frame,
            revision: self.revision,
            command_count,
            dirty_node_count,
        };
        if self
            .events
            .back()
            .is_some_and(|current| matches!(current, EngineEvent::FramePlanned { .. }))
        {
            *self.events.back_mut().expect("frame event exists") = event;
        } else {
            self.events.push_back(event);
        }
    }

    fn transition(&mut self, state: EngineState) {
        if self.state == state {
            return;
        }
        self.state = state;
        self.events.push_back(EngineEvent::State { state });
    }
}

impl GraphInstance {
    pub(crate) fn set_graph(&mut self, graph: Graph) -> Result<u32, SdkError> {
        self.ensure_active()?;
        let changed_nodes = changed_node_ids(self.graph.as_deref(), &graph);
        let removed_nodes = removed_node_ids(self.graph.as_deref(), &graph);
        let changed_node_set = changed_nodes.iter().cloned().collect::<HashSet<_>>();

        let nodes = graph.nodes.clone();
        let edges = graph.edges.clone();
        match self.inner.as_mut() {
            Some(engine) => {
                engine.replace_graph_preserving_state(nodes, edges, &changed_node_set);
            }
            None => {
                self.inner = Some(GraphEvaluator::prepare_with_options(
                    nodes,
                    edges,
                    self.external_video_textures,
                ));
            }
        }

        let released = removed_nodes
            .iter()
            .filter_map(|node_id| {
                self.node_generations
                    .remove(node_id)
                    .map(|generation| (node_id.clone(), generation))
            })
            .collect::<Vec<_>>();
        let invalidated = changed_nodes
            .iter()
            .map(|node_id| {
                let generation = self
                    .node_generations
                    .entry(node_id.clone())
                    .and_modify(|generation| *generation = generation.saturating_add(1))
                    .or_insert(1);
                (node_id.clone(), *generation)
            })
            .collect::<Vec<_>>();

        self.graph = Some(Arc::new(graph));
        self.revision = self.revision.saturating_add(1);
        self.pending_commands.clear();
        self.events.push_back(EngineEvent::GraphReady {
            revision: self.revision,
        });
        for (node_id, generation) in released {
            self.events.push_back(EngineEvent::ResourceReleased {
                node_id,
                generation,
            });
        }
        for (node_id, generation) in invalidated {
            self.events.push_back(EngineEvent::ResourceInvalidated {
                node_id,
                generation,
            });
        }
        self.transition(EngineState::Ready);
        Ok(self.revision)
    }
    pub fn new_native() -> Self {
        new_engine(false)
    }

    /// Native adapters consume the plan without serializing it across FFI.
    pub fn execution_plan(&self) -> Option<&ExecutionPlan> {
        self.inner.as_ref().map(GraphEvaluator::plan)
    }

    /// GPU commands stay inside Rust; native adapters borrow the current frame batch.
    pub fn pending_commands(&self) -> &[ExecutionCommand] {
        &self.pending_commands
    }

    pub fn drain_commands(&mut self) -> Vec<ExecutionCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    pub fn execute_gpu(
        &self,
        facade: &mut dyn crate::evaluation::GpuFacade,
        commands: &[ExecutionCommand],
    ) -> Result<(), SdkError> {
        let plan = self.execution_plan().ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::NotPrepared,
                "GraphInstance must receive a graph before GPU execution",
            )
        })?;
        facade.execute(plan, commands)
    }
}

fn new_engine(external_video_textures: bool) -> GraphInstance {
    GraphInstance {
        inner: None,
        graph: None,
        revision: 0,
        node_generations: HashMap::new(),
        video_nodes: Vec::new(),
        pending_commands: Vec::new(),
        last_frame: None,
        events: VecDeque::new(),
        state: EngineState::Empty,
        external_video_textures,
    }
}

fn fixed_frame_array<const N: usize>(name: &str, values: &[f32]) -> Result<[f32; N], String> {
    values.try_into().map_err(|_| {
        SdkError::new(
            SdkErrorCode::InvalidFrame,
            format!("Frame {name} must contain exactly {N} values"),
        )
        .to_json()
    })
}

fn changed_node_ids(previous: Option<&Graph>, next: &Graph) -> Vec<String> {
    next.nodes
        .iter()
        .filter(|node| node_changed(previous, next, node))
        .map(|node| node.id.clone())
        .collect()
}

fn node_changed(previous: Option<&Graph>, next: &Graph, node: &ProjectNode) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    let Some(old_node) = previous.nodes.iter().find(|old| old.id == node.id) else {
        return true;
    };
    old_node.node_type != node.node_type
        || old_node.data != node.data
        || incoming_edges(previous, &node.id) != incoming_edges(next, &node.id)
}

fn removed_node_ids(previous: Option<&Graph>, next: &Graph) -> Vec<String> {
    previous
        .into_iter()
        .flat_map(|graph| &graph.nodes)
        .filter(|old| !next.nodes.iter().any(|node| node.id == old.id))
        .map(|node| node.id.clone())
        .collect()
}

fn incoming_edges<'a>(graph: &'a Graph, node_id: &str) -> HashSet<(&'a str, &'a str, &'a str)> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.target == node_id)
        .map(edge_signature)
        .collect()
}

fn edge_signature(edge: &Edge) -> (&str, &str, &str) {
    (
        edge.source.as_str(),
        edge.source_handle.as_str(),
        edge.target_handle.as_str(),
    )
}

fn engine_state_name(state: EngineState) -> &'static str {
    match state {
        EngineState::Empty => "empty",
        EngineState::Ready => "ready",
        EngineState::Running => "running",
        EngineState::Paused => "paused",
        EngineState::Stopped => "stopped",
        EngineState::Disposed => "disposed",
    }
}

impl Default for GraphInstance {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "instance_coverage_tests.rs"]
mod coverage_tests;
