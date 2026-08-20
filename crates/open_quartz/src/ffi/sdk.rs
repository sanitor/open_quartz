use std::cell::RefCell;
use std::rc::Rc;

use crate::{
    Environment, Graph, GraphCommand, NodeFactoryRequest, OpenQuartz, Player, Project,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = OpenQuartz))]
pub struct OpenQuartzBinding {
    inner: OpenQuartz,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_class = OpenQuartz))]
impl OpenQuartzBinding {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn new() -> Self {
        Self {
            inner: OpenQuartz::new(Environment::headless()),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = createProject))]
    pub fn create_project(&self, name: &str) -> ProjectBinding {
        ProjectBinding {
            inner: Rc::new(RefCell::new(self.inner.create_project(name))),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = openProject))]
    pub fn open_project(&self, project_json: &str) -> Result<ProjectBinding, String> {
        let project = self
            .inner
            .open_project_json(project_json)
            .map_err(|error| error.to_json())?;
        Ok(ProjectBinding {
            inner: Rc::new(RefCell::new(project)),
        })
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = normalizeProject))]
    pub fn normalize_project(&self, project_json: &str) -> Result<String, String> {
        self.inner
            .normalize_project_json(project_json)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = screenSaverExportProject))]
    pub fn screen_saver_export_project(
        &self,
        project_json: &str,
        renderer_node_id: &str,
    ) -> Result<String, String> {
        self.inner
            .screen_saver_export_project_json(project_json, renderer_node_id)
            .map_err(|error| error.to_json())
    }
}

impl Default for OpenQuartzBinding {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = Project))]
pub struct ProjectBinding {
    inner: Rc<RefCell<Project>>,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_class = Project))]
impl ProjectBinding {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(getter))]
    pub fn name(&self) -> String {
        self.inner.borrow().name().to_owned()
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(setter))]
    pub fn set_name(&mut self, name: String) {
        self.inner.borrow_mut().set_name(name);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = toJSON))]
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(&self.inner.borrow().to_file())
            .map_err(|error| format!("Cannot encode project: {error}"))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = graph))]
    pub fn graph(&self) -> GraphBinding {
        GraphBinding {
            project: self.inner.clone(),
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = screenSaverGraph))]
    pub fn screen_saver_graph(
        &self,
        renderer_node_id: &str,
        width: u32,
        height: u32,
    ) -> Result<String, String> {
        let graph = self
            .inner
            .borrow()
            .screen_saver_graph(renderer_node_id, width, height)
            .map_err(|error| error.to_json())?;
        serde_json::to_string(&graph).map_err(|error| format!("Cannot encode graph: {error}"))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = createPlayer))]
    pub fn create_player(&self) -> Result<PlayerBinding, String> {
        OpenQuartz::new(Environment::headless())
            .player(&self.inner.borrow().graph_snapshot())
            .with_resources(self.inner.borrow().resources())
            .build()
            .map(|inner| PlayerBinding { inner })
            .map_err(|error| error.to_json())
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = Graph))]
pub struct GraphBinding {
    project: Rc<RefCell<Project>>,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_class = Graph))]
impl GraphBinding {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(getter))]
    pub fn revision(&self) -> u32 {
        self.project.borrow().graph_revision()
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = snapshotJSON))]
    pub fn snapshot_json(&self) -> Result<String, String> {
        serde_json::to_string(&self.project.borrow().graph_snapshot())
            .map_err(|error| format!("Cannot encode graph: {error}"))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = initialize))]
    pub fn initialize(&self, graph_json: &str) -> Result<(), String> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|error| format!("Cannot decode graph: {error}"))?;
        self.project
            .borrow_mut()
            .initialize_graph(graph)
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = replace))]
    pub fn replace(&self, graph_json: &str, expected_revision: u32) -> Result<String, String> {
        let graph: Graph = serde_json::from_str(graph_json)
            .map_err(|error| format!("Cannot decode graph: {error}"))?;
        let change = self
            .project
            .borrow_mut()
            .replace_graph(graph, expected_revision)
            .map_err(|error| error.to_json())?;
        Ok(serde_json::json!({
            "revision": self.revision(),
            "changedNodes": change.changed_nodes().iter().map(|id| id.as_str()).collect::<Vec<_>>()
        }).to_string())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = rollback))]
    pub fn rollback(&self, expected_revision: u32) -> Result<String, String> {
        let change = self
            .project
            .borrow_mut()
            .rollback_graph(expected_revision)
            .map_err(|error| error.to_json())?;
        Ok(change_json(self.revision(), &change))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = redo))]
    pub fn redo(&self, expected_revision: u32) -> Result<String, String> {
        let change = self
            .project
            .borrow_mut()
            .redo_graph(expected_revision)
            .map_err(|error| error.to_json())?;
        Ok(change_json(self.revision(), &change))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = apply))]
    pub fn apply(&self, command_json: &str, expected_revision: u32) -> Result<String, String> {
        let command: GraphCommand = serde_json::from_str(command_json)
            .map_err(|error| format!("Cannot decode graph command: {error}"))?;
        let change = self
            .project
            .borrow_mut()
            .apply_graph_command(command, expected_revision)
            .map_err(|error| error.to_json())?;
        Ok(change_json(self.revision(), &change))
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = canConnect))]
    pub fn can_connect(
        &self,
        source_node_id: &str,
        source_port_id: &str,
        target_node_id: &str,
        target_port_id: &str,
    ) -> Result<(), String> {
        let command = GraphCommand::Connect {
            source: crate::PortKey::new(
                crate::NodeId::new(source_node_id),
                crate::PortId::new(source_port_id),
            ),
            target: crate::PortKey::new(
                crate::NodeId::new(target_node_id),
                crate::PortId::new(target_port_id),
            ),
        };
        self.project
            .borrow()
            .graph_snapshot()
            .clone()
            .apply_command(command)
            .map(|_| ())
            .map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = createNode))]
    pub fn create_node(
        &self,
        factory_json: &str,
        expected_revision: u32,
    ) -> Result<String, String> {
        let request: NodeFactoryRequest = serde_json::from_str(factory_json)
            .map_err(|error| format!("Cannot decode node factory request: {error}"))?;
        let (node, change) = self
            .project
            .borrow_mut()
            .create_graph_node(request, expected_revision)
            .map_err(|error| error.to_json())?;
        Ok(serde_json::json!({
            "revision": self.revision(),
            "node": node,
            "changedNodes": change.changed_nodes().iter().map(|id| id.as_str()).collect::<Vec<_>>()
        }).to_string())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = nodeJSON))]
    pub fn node_json(&self, node_id: &str) -> Option<String> {
        self.project
            .borrow()
            .graph_snapshot()
            .nodes()
            .iter()
            .find(|node| node.id == node_id)
            .and_then(|node| serde_json::to_string(node).ok())
    }
}

fn change_json(revision: u32, change: &crate::GraphChange) -> String {
    serde_json::json!({
        "revision": revision,
        "changedNodes": change.changed_nodes().iter().map(|id| id.as_str()).collect::<Vec<_>>()
    })
    .to_string()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = Player))]
pub struct PlayerBinding {
    inner: Player,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_class = Player))]
impl PlayerBinding {
    pub fn play(&mut self) -> Result<(), String> {
        self.inner.play().map_err(|error| error.to_json())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        self.inner.pause().map_err(|error| error.to_json())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        self.inner.resume().map_err(|error| error.to_json())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        self.inner.stop().map_err(|error| error.to_json())
    }

    pub fn close(&mut self) -> Result<(), String> {
        self.inner.close().map_err(|error| error.to_json())
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(getter, js_name = graphRevision))]
    pub fn graph_revision(&self) -> u32 {
        self.inner.graph_revision()
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(js_name = outputsJSON))]
    pub fn outputs_json(&self) -> String {
        serde_json::to_string(
            &self
                .inner
                .outputs()
                .iter()
                .map(|output| output.key())
                .collect::<Vec<_>>(),
        )
        .expect("Output keys are serializable")
    }
}
