use super::*;

impl Controller {
    pub(super) fn discard_pipeline_corrector(&mut self) {
        if self.pipeline_editor.corrector.take().is_some_and(|state| state.preview.is_some()) {
            self.requested = None;
            self.request_preview();
        }
    }

    pub(in crate::controller) fn pipeline_preview_for(&self, clip_id: &str) -> Option<ClipPipeline> {
        let draft = self.pipeline_editor.corrector.as_ref().and_then(|state| {
            (state.clip == clip_id && state.revision == self.document.revision
                && self.selected_clip.as_deref() == Some(clip_id)
                && self.pipeline_editor.selected.len() == 1
                && self.pipeline_editor.selected.contains(&state.node))
                .then(|| state.preview.clone()).flatten()
        });
        if !self.playing && self.window.upgrade().is_some_and(|window| window.get_nodes_open()) {
            if let Some((clip, node)) = &self.pipeline_editor.monitor {
                if clip == clip_id {
                    return draft.as_ref().unwrap_or(&self.document.clip(clip_id)?.pipeline).monitor_tap(node).ok();
                }
            }
        }
        draft
    }

    pub(super) fn publish_pipeline_corrector(&mut self, node: Option<&ProjectNode>) {
        let current = node.filter(|node| node.node_type == NodeType::ColorCorrector
            && self.pipeline_editor.selected.len() == 1)
            .and_then(|node| node.data.color_corrector.as_ref().map(|config| (node, config)));
        let matches = self.pipeline_editor.corrector.as_ref().is_some_and(|state| {
            current.is_some_and(|(node, _)| node.id == state.node)
                && self.pipeline_editor.clip.as_ref() == Some(&state.clip)
                && self.selected_clip.as_ref() == Some(&state.clip)
                && self.document.revision == state.revision
        });
        if !matches {
            self.discard_pipeline_corrector();
            if let (Some((node, config)), Some(clip)) = (current, self.pipeline_editor.clip.clone()) {
                let host = format!("video:{clip}");
                self.pipeline_editor.corrector = Some(CorrectorState {
                    clip, node: node.id.clone(), revision: self.document.revision,
                    draft: CorrectorDraft::new(host, node.id.clone(), self.document.revision, config.clone()),
                    preview: None,
                });
            }
        }
        let Some(window) = self.window.upgrade() else { return; };
        window.set_pipeline_is_corrector(self.pipeline_editor.corrector.is_some());
        window.set_pipeline_corrector_error(self.pipeline_editor.error.clone().into());
        window.set_pipeline_corrector_domain("Scene-linear ACEScg grading · SDR preview (not HDR)".into());
        if let Some(state) = &self.pipeline_editor.corrector {
            window.set_pipeline_corrector_target(state.draft.token().into());
            window.set_pipeline_corrector_fields(state.draft.fields());
            window.set_pipeline_corrector_stages(state.draft.stages());
            window.set_pipeline_corrector_enabled(state.draft.config().enabled);
        } else {
            window.set_pipeline_corrector_target("".into());
            window.set_pipeline_corrector_fields(ModelRc::default());
            window.set_pipeline_corrector_stages(ModelRc::default());
            window.set_pipeline_corrector_enabled(false);
        }
    }

    pub(super) fn pipeline_corrector_event(&mut self, target: &str, action: &str, key: &str, value: &str) {
        let Some(state) = self.pipeline_editor.corrector.as_ref() else { return; };
        if state.draft.token() != target || state.revision != self.document.revision
            || self.pipeline_editor.clip.as_ref() != Some(&state.clip)
            || self.selected_clip.as_ref() != Some(&state.clip)
            || self.pipeline_editor.selected.len() != 1
            || !self.pipeline_editor.selected.contains(&state.node)
        {
            return;
        }
        self.pause();
        self.refresh_time();
        let result = self.pipeline_editor.corrector.as_mut().unwrap().draft.event(action, key, value);
        let event = match result {
            Ok(event) => event,
            Err(error) => { self.pipeline_error(error); return; }
        };
        match event {
            CorrectorAction::None => {}
            CorrectorAction::Cancel => {
                if let Some(state) = self.pipeline_editor.corrector.as_mut() { state.preview = None; }
                self.pipeline_editor.error.clear();
                self.requested = None;
                self.request_preview();
            }
            event @ (CorrectorAction::Preview(_) | CorrectorAction::Commit(_)) => {
                let (config, commit) = match event {
                    CorrectorAction::Preview(config) => (config, false),
                    CorrectorAction::Commit(config) => (config, true),
                    _ => unreachable!(),
                };
                let Some(state) = self.pipeline_editor.corrector.as_ref() else { return; };
                let Some(clip) = self.document.clip(&state.clip) else { return; };
                let Some(node) = clip.pipeline.graph().nodes.iter().find(|node| node.id == state.node) else { return; };
                let mut data = node.data.clone();
                data.color_corrector = Some(config);
                let mut pipeline = clip.pipeline.clone();
                if let Err(error) = pipeline.apply_command(GraphCommand::UpdateNodeData {
                    node_id: NodeId::new(&state.node), data,
                }) {
                    self.pipeline_error(error);
                    return;
                }
                self.pipeline_editor.error.clear();
                if commit {
                    self.prepare_pipeline(pipeline, self.pipeline_editor.selected.clone());
                } else {
                    self.pipeline_editor.corrector.as_mut().unwrap().preview = Some(pipeline);
                    self.requested = None;
                    self.request_preview();
                }
            }
        }
        self.publish_pipeline_inspector();
    }
}
