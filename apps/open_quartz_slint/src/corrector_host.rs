use super::*;

impl AppController {
    pub(super) fn publish_corrector(&mut self) {
        let Some(window) = self.window.upgrade() else { return; };
        let node = (self.selected_node_ids.len() == 1).then(|| self.project.graph().nodes().iter()
            .find(|node| self.selected_node_ids.contains(&node.id))).flatten();
        let Some(node) = node.filter(|node| node.node_type == NodeType::ColorCorrector) else {
            self.corrector_draft = None;
            self.corrector_revision = None;
            window.set_inspector_is_corrector(false);
            window.set_inspector_corrector_target("".into());
            return;
        };
        let revision = self.project.graph_revision();
        if self.corrector_revision != Some(revision) || window.get_inspector_node_id().as_str() != node.id {
            self.corrector_draft = node.data.color_corrector.clone().map(|config| CorrectorDraft::new("graph".into(), node.id.clone(), revision.into(), config));
            self.corrector_revision = Some(revision);
        }
        window.set_inspector_is_corrector(true);
        window.set_inspector_corrector_domain("Legacy sRGB inputs are converted to scene-linear AP1; desktop preview is SDR sRGB. No HDR monitor output.".into());
        if let Some(draft) = &self.corrector_draft {
            window.set_inspector_corrector_target(draft.token().into());
            window.set_inspector_corrector_fields(draft.fields());
            window.set_inspector_corrector_stages(draft.stages());
            window.set_inspector_corrector_enabled(draft.config().enabled);
            window.set_inspector_corrector_error(draft.error.as_str().into());
        }
    }

    pub(super) fn corrector_event(&mut self, target: &str, action: &str, key: &str, value: &str) {
        if self.busy || self.corrector_commit.is_some() || self.selected_node_ids.len() != 1
            || self.corrector_revision != Some(self.project.graph_revision()) { return; }
        let Some(draft) = self.corrector_draft.as_mut().filter(|draft| draft.token() == target) else { return; };
        let event = draft.event(action, key, value);
        match event {
            Ok(CorrectorAction::None) => {},
            Ok(CorrectorAction::Preview(config)) => {
                if let Err(error) = self.submit_corrector_graph(Some(config)) { self.corrector_failure(error); }
            }
            Ok(CorrectorAction::Cancel) => {
                if let Err(error) = self.submit_corrector_graph(None) { self.corrector_failure(error); }
            }
            Ok(CorrectorAction::Commit(config)) => {
                let id = self.selected_node_ids.iter().next().unwrap().clone();
                self.corrector_commit = Some((id, self.project.graph_revision(), config.clone()));
                match self.submit_corrector_graph(Some(config)) {
                    Ok(()) if self.renderer.is_none() => self.finish_corrector_commit(),
                    Ok(()) => self.set_edit_status("Preparing Color Corrector; previous accepted frame retained…"),
                    Err(error) => { self.corrector_commit = None; self.corrector_failure(error); }
                }
            }
            Err(error) => self.set_edit_status(&error),
        }
        self.publish_corrector();
        self.publish_inspector_dirty();
    }

    // Draft snapshots never enter Project/history or saved JSON. The same worker
    // prepares previews and final commits, and reports acceptance before history.
    fn submit_corrector_graph(&mut self, config: Option<open_quartz::operators::ColorCorrectorConfig>) -> Result<(), String> {
        let mut graph = self.project.graph_snapshot();
        if let Some(config) = config {
            let id = self.selected_node_ids.iter().next().ok_or("No Corrector selected")?;
            let node = graph.nodes.iter_mut().find(|node| &node.id == id).ok_or("Corrector target was removed")?;
            node.data.color_corrector = Some(config);
        }
        let images = self.image_assets.iter().map(|(id, asset)| (id.clone(), crate::render_bridge::ImageUpload {
            key: asset.key, width: asset.width, height: asset.height, rgba: asset.rgba.clone(),
        })).chain(self.framebuffer_assets.iter().map(|(id, asset)| (id.clone(), crate::render_bridge::ImageUpload {
            key: asset.key, width: asset.width, height: asset.height, rgba: asset.rgba.clone(),
        }))).collect();
        let playing = self.window.upgrade().is_some_and(|window| window.get_playback_state() == "playing");
        if let Some(renderer) = &mut self.renderer {
            self.corrector_request = Some(renderer.set_preview_graph(graph, images, playing)?);
        }
        Ok(())
    }
    pub(super) fn corrector_graph_result(&mut self, generation: u64, result: Result<(), String>) {
        if self.corrector_request != Some(generation) { return; }
        self.corrector_request = None;
        match result {
            Ok(()) => self.finish_corrector_commit(),
            Err(error) => { self.corrector_commit = None; self.corrector_failure(error); }
        }
    }
    fn finish_corrector_commit(&mut self) {
        let Some((id, revision, config)) = self.corrector_commit.take() else { return; };
        if self.project.graph_revision() != revision || !self.selected_node_ids.contains(&id) { return; }
        let Some(node) = self.project.graph().nodes().iter().find(|node| node.id == id) else { return; };
        let mut data = node.data.clone();
        data.color_corrector = Some(config);
        match Arc::make_mut(&mut self.project).apply_graph_command(GraphCommand::UpdateNodeData { node_id: NodeId::new(id), data }, revision) {
            Ok(_) => {
                self.undo_edits.push(EditKind::Content); self.redo_edits.clear();
                self.corrector_draft = None; self.corrector_revision = None;
                self.publish_graph_projection();
                self.set_edit_status("Color Corrector applied; one history entry.");
            }
            Err(error) => self.corrector_failure(error.to_json()),
        }
    }
    fn corrector_failure(&mut self, error: String) {
        if let Some(draft) = &mut self.corrector_draft { draft.error = error.clone(); }
        self.set_edit_status(&format!("Corrector rejected; accepted document and frame retained: {error}"));
        self.publish_corrector();
    }
    pub(super) fn discard_corrector(&mut self) {
        // Retire the final intent before restoring pixels. Its queued acceptance
        // must never promote a discarded candidate into the document/history.
        self.corrector_commit = None;
        self.corrector_request = None;
        if self.corrector_draft.as_ref().is_some_and(CorrectorDraft::dirty) {
            let _ = self.submit_corrector_graph(None);
        }
        self.corrector_draft = None; self.corrector_revision = None;
    }
}
