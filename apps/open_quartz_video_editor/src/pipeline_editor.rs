use super::*;
use crate::clip_pipeline::{ClipPipeline, SOURCE_ID};
use crate::graph_clipboard::GraphClipboard;
use crate::graph_view_model::{port_at, GraphViewModel};
use crate::inspector_forms::{FormKind, NodeFormsDraft};
use crate::corrector_editor::{CorrectorAction, CorrectorDraft};
use crate::uniform_literals::{effective_uniform_value, format_uniform_literal, parse_uniform_literal, uniform_shape};
use open_quartz::{Graph, GraphCommand, NodeId, PortId, PortKey, Position, Project, ProjectFile, ProjectNode, NodeType};
use std::collections::{BTreeMap, BTreeSet};

#[path = "pipeline_corrector.rs"]
mod pipeline_corrector;
pub(super) enum PipelineCommand {
    Workspace(bool), Select(String, bool), Move(String, f32, f32),
    Port(String, String, bool), Drop(String, String, bool, f32, f32, f32),
    Edge(String), Add(i32), Action(String), Label(String), Shader(String),
    Size(i32, i32, bool), Uniform(String, String), Form(String, String), ApplyForm(String),
    Corrector(String, String, String, String),
}

struct Draft {
    baseline: ProjectNode,
    label: String,
    code: String,
    uniforms: BTreeMap<String, String>,
    forms: NodeFormsDraft,
    width: i32,
    height: i32,
    auto_size: bool,
}
impl Draft {
    fn new(node: &ProjectNode) -> Self {
        Self {
            baseline: node.clone(), label: node.data.label.clone(), code: node.data.shader_code.clone(),
            uniforms: node.data.inputs.iter().filter(|p| uniform_shape(p.data_type).is_some())
                .map(|p| (p.label.clone(), format_uniform_literal(&effective_uniform_value(node,p)))).collect(),
            forms: NodeFormsDraft::from_data(&node.data),
            width: node.data.width.unwrap_or(512) as i32, height: node.data.height.unwrap_or(512) as i32,
            auto_size: node.data.auto_size.unwrap_or(true),
        }
    }
    fn dirty(&self) -> bool {
        self.label != self.baseline.data.label || self.code != self.baseline.data.shader_code
            || self.width != self.baseline.data.width.unwrap_or(512) as i32
            || self.height != self.baseline.data.height.unwrap_or(512) as i32
            || self.auto_size != self.baseline.data.auto_size.unwrap_or(true)
            || self.uniforms.iter().any(|(name,value)| self.baseline.data.inputs.iter().find(|p| &p.label==name)
                .is_some_and(|p| *value != format_uniform_literal(&effective_uniform_value(&self.baseline,p))))
            || self.forms != NodeFormsDraft::from_data(&self.baseline.data)
    }
}
struct Candidate {
    clip: String,
    revision: u64,
    pipeline: ClipPipeline,
    selection: BTreeSet<String>,
    validation: mpsc::Receiver<Result<(), String>>,
}
struct CorrectorState {
    clip: String,
    node: String,
    revision: u64,
    draft: CorrectorDraft,
    preview: Option<ClipPipeline>,
}
#[derive(Default)]
pub(super) struct PipelineEditor {
    clip: Option<String>,
    pub(super) monitor: Option<(String, String)>,
    selected: BTreeSet<String>,
    edge: Option<String>,
    port: Option<(String,String,bool)>,
    clipboard: Option<GraphClipboard>,
    corrector_clipboard: Option<ProjectNode>,
    drafts: HashMap<(String,String), Draft>,
    candidate: Option<Candidate>,
    pub(super) error: String,
    effects: Vec<&'static crate::shader_catalog::ShaderPreset>,
    revision: Option<u64>,
    corrector: Option<CorrectorState>,
}
impl PipelineEditor {
    pub(super) fn reset(&mut self) {
        let effects=std::mem::take(&mut self.effects);
        *self=Self { effects, ..Self::default() };
    }
}
fn as_project(graph: Graph) -> Project {
    Project::from_file(ProjectFile { version: "0.4.0".into(), name: "Clip pipeline".into(), created_at: String::new(), updated_at: String::new(), graph })
}
fn sdk_error(error: open_quartz::SdkError) -> String { error.to_json() }

impl Controller {
    pub(super) fn connect_pipeline(this: &Rc<RefCell<Self>>, window: &EditorWindow) {
        macro_rules! bind {
            ($method:ident, |$($arg:ident),*| $command:expr) => {{
                let sender=this.borrow().ui_send.clone();
                window.$method(move |$($arg),*| { let _=sender.send(UiCommand::Pipeline($command)); });
            }};
        }
        bind!(on_nodes_workspace_changed, |open| PipelineCommand::Workspace(open));
        bind!(on_pipeline_node_selected, |id,add| PipelineCommand::Select(id.to_string(),add));
        bind!(on_pipeline_node_moved, |id,x,y| PipelineCommand::Move(id.to_string(),x,y));
        bind!(on_pipeline_port_selected, |id,port,out| PipelineCommand::Port(id.to_string(),port.to_string(),out));
        bind!(on_pipeline_port_dropped, |id,port,out,x,y,slop| PipelineCommand::Drop(id.to_string(),port.to_string(),out,x,y,slop));
        bind!(on_pipeline_edge_selected, |id| PipelineCommand::Edge(id.to_string()));
        bind!(on_pipeline_add_effect, |index| PipelineCommand::Add(index));
        bind!(on_pipeline_action, |action| PipelineCommand::Action(action.to_string()));
        bind!(on_pipeline_label_draft_changed, |value| PipelineCommand::Label(value.to_string()));
        bind!(on_pipeline_shader_draft_changed, |value| PipelineCommand::Shader(value.to_string()));
        bind!(on_pipeline_output_size_draft_changed, |w,h,auto| PipelineCommand::Size(w,h,auto));
        bind!(on_pipeline_uniform_draft_changed, |name,value| PipelineCommand::Uniform(name.to_string(),value.to_string()));
        bind!(on_pipeline_form_draft_changed, |key,value| PipelineCommand::Form(key.to_string(),value.to_string()));
        bind!(on_pipeline_apply_form, |key| PipelineCommand::ApplyForm(key.to_string()));
        bind!(on_pipeline_corrector_event, |target,action,key,value| PipelineCommand::Corrector(target.to_string(),action.to_string(),key.to_string(),value.to_string()));
        let mut this=this.borrow_mut();
        this.pipeline_editor.effects=crate::shader_catalog::presets().iter().filter(|preset|matches!(preset.category,"COLOR"|"FILTER"|"DISTORTION"|"BLEND")).collect::<Vec<_>>();
        this.pipeline_editor.effects.sort_by_key(|preset|if preset.category=="COLOR"{0}else{1});
        let mut groups=Vec::new();
        for category in ["CUSTOM","COLOR","FILTER","DISTORTION","BLEND"] {
            let mut effects:Vec<crate::PipelineEffect>=if category=="CUSTOM" { vec![crate::PipelineEffect { label:"Custom Shader".into(), index:-1, glyph:"ƒx".into() }] }
                else { this.pipeline_editor.effects.iter().enumerate().filter(|(_,preset)|preset.category==category).map(|(index,preset)|crate::PipelineEffect {
                    label:preset.label.to_owned().into(), index:index as i32,
                    glyph:match category { "COLOR"=>"C", "FILTER"=>"F", "DISTORTION"=>"D", "BLEND"=>"B", _=>"□" }.into(),
                }).collect() };
            if category=="COLOR" { effects.insert(0,crate::PipelineEffect { label:"Color Corrector".into(),index:-3,glyph:"C".into() }); }
            let rows=effects.chunks(2).map(|pair|crate::PipelineEffectRow {
                first:pair[0].clone(),
                second:pair.get(1).cloned().unwrap_or_else(||crate::PipelineEffect { label:"".into(), index:-2, glyph:"".into() }),
            }).collect::<Vec<_>>();
            if !rows.is_empty() { groups.push(crate::PipelineGroup { label:category.into(), rows:ModelRc::new(VecModel::from(rows)) }); }
        }
        window.set_pipeline_effect_groups(ModelRc::new(VecModel::from(groups)));
    }

    fn selected_pipeline_node(&self) -> Option<ProjectNode> {
        let id=self.pipeline_editor.selected.iter().next()?;
        self.document.clip(self.pipeline_editor.clip.as_deref()?)?.pipeline.graph().nodes.iter().find(|n| &n.id==id).cloned()
    }
    fn pipeline_draft(&mut self) -> Option<&mut Draft> {
        let node=self.selected_pipeline_node()?;
        let key=(self.pipeline_editor.clip.clone()?, node.id.clone());
        let draft=self.pipeline_editor.drafts.entry(key).or_insert_with(|| Draft::new(&node));
        if draft.baseline != node { *draft=Draft::new(&node); }
        Some(draft)
    }
    pub(super) fn pipeline_has_drafts(&self) -> bool { self.pipeline_editor.candidate.is_some() || self.pipeline_editor.drafts.values().any(Draft::dirty) || self.pipeline_editor.corrector.as_ref().is_some_and(|state|state.draft.dirty()) }
    fn pipeline_error(&mut self, error: String) {
        self.pipeline_editor.error=error.clone();
        self.status(error);
        self.publish_pipeline_inspector();
    }
    pub(super) fn pipeline_command(&mut self, command: PipelineCommand) {
        if let PipelineCommand::Workspace(open)=command {
            self.extra_selection.clear();
            if !open {
                self.discard_pipeline_corrector();
                if self.pipeline_editor.monitor.take().is_some() {
                    self.requested = None;
                    self.request_preview();
                }
            }
            if open {
                let id=self.selected_clip.clone().filter(|id|self.document.clip(id).is_some_and(|clip|clip.title.is_none()))
                    .or_else(||self.document.data.active_clips(self.playhead).find(|(_, clip)| clip.title.is_none()).map(|(_, clip)| clip.id.clone()));
                if let Some(id)=id { self.selected_clip=Some(id.clone());
                    if let Some(clip)=self.document.clip(&id) {
                        if self.playhead<clip.timeline_start||self.playhead>=clip.timeline_end().unwrap_or(0){self.playhead=clip.timeline_start;}
                        self.selected_asset=Some(clip.asset_id.clone());
                    }
                    self.asset_preview=false;self.pause();self.requested=None;self.request_preview();
                }
            }
            self.refresh();return;
        }
        let read_only = match &command {
            PipelineCommand::Select(..) | PipelineCommand::Edge(..) => true,
            PipelineCommand::Action(action) => matches!(action.as_str(),
                "copy" | "select-all" | "discard-drafts" | "cancel-connection" | "show-selected-node" | "show-program") || action.starts_with("toggle-preview:"),
            _ => false,
        };
        if !read_only {
            if let Some(track) = self.pipeline_editor.clip.as_deref()
                .and_then(|id| self.document.clip_track(id)).filter(|track| track.locked) {
                self.pipeline_error(format!("{} is locked", track.name));
                return;
            }
        }
        if self.pipeline_editor.candidate.is_some() {
            self.pipeline_error("Preparing pipeline; wait for completion before another edit".into());return;
        }
        match command {
            PipelineCommand::Select(id,add) => {
                self.discard_pipeline_corrector();
                self.pause();
                if !add {self.pipeline_editor.selected.clear();}
                if !self.pipeline_editor.selected.insert(id.clone())&&add {self.pipeline_editor.selected.remove(&id);}
                self.pipeline_editor.edge=None;self.publish_pipeline();
            }
            PipelineCommand::Move(id,x,y) => {
                let Some(clip_id)=self.pipeline_editor.clip.clone() else{return;};
                let Some(clip)=self.document.clip(&clip_id)else{return;};
                let mut pipeline=clip.pipeline.clone();
                let result=pipeline.apply_command(GraphCommand::SetNodePosition {node_id:NodeId::new(id),position:Position{x:x as f64,y:y as f64}})
                    .and_then(|()|self.document.set_clip_pipeline(&clip_id,pipeline,self.document.revision));
                if let Err(e)=result{self.pipeline_error(e);}else{self.refresh();}
            }
            PipelineCommand::Edge(id)=>{self.pipeline_editor.edge=Some(id);self.pipeline_editor.port=None;self.publish_pipeline();}
            PipelineCommand::Port(id,port,out)=>{ self.pipeline_port(id,port,out); }
            PipelineCommand::Drop(id,port,out,x,y,slop)=>{
                if let Some(window)=self.window.upgrade(){
                    if let Some((target,tp,to))=port_at(&window.get_pipeline_nodes(),x,y,slop){
                        self.pipeline_editor.port=Some((id,port,out));self.pipeline_port(target,tp,to);
                    }else{self.pipeline_editor.port=None;self.publish_pipeline();}
                }
            }
            PipelineCommand::Add(index)=>{
                let Some(id)=self.pipeline_editor.clip.clone()else{return;};
                let Some(clip)=self.document.clip(&id)else{return;};
                let mut pipeline=clip.pipeline.clone();
                let after=self.pipeline_editor.selected.iter().next().map(String::as_str).unwrap_or(SOURCE_ID);
                let result=if index==-3 { pipeline.add_corrector_serial(after) } else {
                    let (code,label)=if index==-1 {("@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f{return textureSample(image,imageSampler,uv);}","Custom Shader")}
                        else if let Some(effect)=self.pipeline_editor.effects.get(index as usize){(effect.code,effect.label)}else{return;};
                    pipeline.add_serial(after,code,label)
                };
                match result {
                    Ok(node)=>self.prepare_pipeline(pipeline,BTreeSet::from([node])),Err(e)=>self.pipeline_error(e),
                }
            }
            PipelineCommand::Action(action)=>self.pipeline_action(&action),
            PipelineCommand::Label(value)=>{if let Some(d)=self.pipeline_draft(){d.label=value;}self.publish_pipeline_inspector();}
            PipelineCommand::Shader(value)=>{if let Some(d)=self.pipeline_draft(){d.code=value;}self.publish_pipeline_inspector();}
            PipelineCommand::Size(w,h,auto)=>{if let Some(d)=self.pipeline_draft(){d.width=w;d.height=h;d.auto_size=auto;}self.publish_pipeline_inspector();}
            PipelineCommand::Uniform(name,value)=>{if let Some(d)=self.pipeline_draft(){d.uniforms.insert(name,value);}self.publish_pipeline_inspector();}
            PipelineCommand::Form(key,value)=>{if let Some(d)=self.pipeline_draft(){d.forms.change(&key,&value);}self.publish_pipeline_inspector();}
            PipelineCommand::ApplyForm(key)=>self.apply_pipeline_inspector(&format!("form:{key}")),
            PipelineCommand::Corrector(target,action,key,value)=>self.pipeline_corrector_event(&target,&action,&key,&value),
            PipelineCommand::Workspace(_)=>{}
        }
    }
    fn pipeline_port(&mut self,id:String,port:String,out:bool){
        let endpoint=(id,port,out);
        let Some(first)=self.pipeline_editor.port.take()else{self.pipeline_editor.port=Some(endpoint);self.publish_pipeline();return;};
        if first.2==endpoint.2 {self.pipeline_editor.port=Some(endpoint);self.publish_pipeline();return;}
        let (source,target)=if first.2{(first,endpoint)}else{(endpoint,first)};
        let Some(clip)=self.pipeline_editor.clip.as_deref().and_then(|id|self.document.clip(id))else{return;};
        let mut graph=clip.pipeline.graph().clone();
        let mut disconnect=graph.edges.iter().filter(|e|e.target==target.0&&e.target_handle==target.1).map(|e|e.id.clone()).collect::<BTreeSet<_>>();
        if let Some(edge)=&self.pipeline_editor.edge{disconnect.insert(edge.clone());}
        let result=graph.edit(&mut|edit|{
            for edge in &disconnect{edit.disconnect(edge)?;}
            edit.connect(PortKey::new(NodeId::new(&source.0),PortId::new(&source.1)),PortKey::new(NodeId::new(&target.0),PortId::new(&target.1)))
        }).map_err(sdk_error);
        match result.and_then(|_|{let mut p=clip.pipeline.clone();p.replace_graph(graph)?;Ok(p)}){
            Ok(p)=>{self.pipeline_editor.edge=None;self.prepare_pipeline(p,self.pipeline_editor.selected.clone());},
            Err(e)=>self.pipeline_error(e),
        }
        self.publish_pipeline();
    }
    fn pipeline_action(&mut self, action:&str){
        if action == "show-program" {
            self.pipeline_editor.monitor = None;
            self.refresh_pipeline_monitor();
            return;
        }

        if action.starts_with("apply-"){self.apply_pipeline_inspector(action);return;}
        let Some(id)=self.pipeline_editor.clip.clone()else{return;};
        let Some(clip)=self.document.clip(&id)else{return;};
        let mut pipeline=clip.pipeline.clone();
        match action {
            "cancel-connection"=>{self.pipeline_editor.port=None;self.pipeline_editor.edge=None;self.publish_pipeline();}
            "select-all"=>{self.pipeline_editor.selected=pipeline.graph().nodes.iter().filter(|n|!ClipPipeline::is_protected(&n.id)).map(|n|n.id.clone()).collect();self.publish_pipeline();}
            "discard-drafts"=>{self.discard_pipeline_corrector();let clip=self.pipeline_editor.clip.clone().unwrap_or_default();let selected=self.pipeline_editor.selected.clone();self.pipeline_editor.drafts.retain(|(c,n),_|c!=&clip||!selected.contains(n));self.pipeline_editor.error.clear();self.publish_pipeline_inspector();}
            "remove-nodes"=>{let ids=self.pipeline_editor.selected.iter().cloned().collect::<Vec<_>>();match pipeline.remove_nodes(&ids){Ok(())=>self.prepare_pipeline(pipeline,BTreeSet::new()),Err(e)=>self.pipeline_error(e)}}
            "delete-edge"=>{if let Some(edge)=self.pipeline_editor.edge.clone(){match pipeline.apply_command(GraphCommand::Disconnect{edge_id:edge}){Ok(())=>self.prepare_pipeline(pipeline,self.pipeline_editor.selected.clone()),Err(e)=>self.pipeline_error(e)}}}
            "reconnect-edge"=>{
                if let Some(edge)=self.pipeline_editor.edge.as_ref().and_then(|id|pipeline.graph().edges.iter().find(|e|&e.id==id)){
                    self.pipeline_editor.port=Some((edge.source.clone(),edge.source_handle.clone(),true));self.publish_pipeline();
                }
            }
            "copy"=>{
                let selected:BTreeSet<String>=self.pipeline_editor.selected.iter().filter(|id|!ClipPipeline::is_protected(id)).cloned().collect();
                self.pipeline_editor.corrector_clipboard=if selected.len()==1 {
                    pipeline.graph().nodes.iter().find(|node|selected.contains(&node.id)&&node.node_type==NodeType::ColorCorrector).cloned()
                }else{None};
                self.pipeline_editor.clipboard=GraphClipboard::capture(&as_project(pipeline.graph().clone()),&selected);self.publish_pipeline();
            }
            "paste"=>{
                if let Some(original)=self.pipeline_editor.corrector_clipboard.clone(){
                    let after=self.pipeline_editor.selected.iter().next().map(String::as_str).unwrap_or(SOURCE_ID);
                    let result=pipeline.add_corrector_serial(after).and_then(|id|{
                        let mut data=pipeline.graph().nodes.iter().find(|node|node.id==id).expect("inserted corrector").data.clone();
                        data.color_corrector=original.data.color_corrector;
                        data.label=original.data.label;
                        pipeline.apply_command(GraphCommand::UpdateNodeData{node_id:NodeId::new(&id),data})?;
                        Ok(id)
                    });
                    match result{Ok(id)=>self.prepare_pipeline(pipeline,BTreeSet::from([id])),Err(error)=>self.pipeline_error(error)}
                    return;
                }
                if let Some(clipboard)=self.pipeline_editor.clipboard.as_mut(){
                    let mut project=as_project(pipeline.graph().clone());
                    match clipboard.paste(&mut project).map_err(sdk_error).and_then(|selected|{pipeline.replace_graph(project.graph_snapshot())?;Ok(selected)}){
                        Ok(selected)=>self.prepare_pipeline(pipeline,selected),Err(e)=>self.pipeline_error(e),
                    }
                }
            }
            action if action == "show-selected-node" || action.starts_with("toggle-preview:") => {
                let node = action.strip_prefix("toggle-preview:").map(str::to_owned)
                    .or_else(|| (self.pipeline_editor.selected.len() == 1).then(|| self.pipeline_editor.selected.first().cloned()).flatten());
                let Some(node) = node else { self.pipeline_error("Select one image node".into()); return; };
                if let Err(error) = pipeline.monitor_tap(&node) { self.pipeline_error(error); return; }
                self.pipeline_editor.monitor = Some((id, node));
                self.refresh_pipeline_monitor();
            }
            _=>{}
        }
    }
    fn refresh_pipeline_monitor(&mut self) {
        self.pause();
        self.requested = None;
        self.publish_pipeline();
        self.request_preview();
    }

    fn apply_pipeline_inspector(&mut self,action:&str){
        let Some(node)=self.selected_pipeline_node()else{return;};
        if ClipPipeline::is_protected(&node.id){self.pipeline_error("Clip source and output are managed by the timeline".into());return;}
        let Some(draft)=self.pipeline_draft()else{return;};
        let mut updated=node.clone();
        let result=(||->Result<(),String>{
            match action {
                "apply-label"=>{if draft.label.trim().is_empty(){return Err("Node label cannot be empty".into());}updated.data.label=draft.label.clone();}
                "apply-shader"=>{}
                "apply-uniforms"=>{
                    for port in &node.data.inputs {
                        if let Some(literal)=draft.uniforms.get(&port.label){updated.data.uniforms.insert(port.label.clone(),parse_uniform_literal(port.data_type,literal)?);}
                    }
                }
                "apply-output-size"=>{if draft.width<=0||draft.height<=0{return Err("Output dimensions must be positive".into());}updated.data.width=Some(draft.width as u32);updated.data.height=Some(draft.height as u32);updated.data.auto_size=Some(draft.auto_size);}
                key if key.starts_with("form:")=>{let kind=FormKind::from_key(&key[5..]).ok_or("Unknown parameter group")?;updated.data=draft.forms.updated_data(&node.data,kind)?;}
                _=>return Err("Unknown inspector action".into()),
            }Ok(())
        })();
        if let Err(e)=result{self.pipeline_error(e);return;}
        let code=draft.code.clone();
        let Some(clip)=self.pipeline_editor.clip.as_deref().and_then(|id|self.document.clip(id))else{return;};
        let mut pipeline=clip.pipeline.clone();
        let command=if action=="apply-shader"{GraphCommand::UpdateShaderCode{node_id:NodeId::new(&node.id),shader_code:code}}else{GraphCommand::UpdateNode{node:updated}};
        match pipeline.apply_command(command){Ok(())=>{
            if action=="apply-label"{
                let id=clip.id.clone();match self.document.set_clip_pipeline(&id,pipeline,self.document.revision){Ok(())=>{self.pipeline_editor.drafts.remove(&(id,node.id));self.refresh();},Err(e)=>self.pipeline_error(e)}
            }else{self.prepare_pipeline(pipeline,self.pipeline_editor.selected.clone());}
        },Err(e)=>self.pipeline_error(e)}
    }
    fn prepare_pipeline(&mut self,pipeline:ClipPipeline,selection:BTreeSet<String>){
        let Some(clip)=self.pipeline_editor.clip.clone()else{return;};
        self.pause();self.pipeline_editor.error.clear();
        match self.preview.validate_pipeline(pipeline.clone(),self.document.data.width,self.document.data.height){
            Ok(validation)=>{self.pipeline_editor.candidate=Some(Candidate{clip,revision:self.document.revision,pipeline,selection,validation});self.publish_pipeline_inspector();},
            Err(e)=>self.pipeline_error(e),
        }
    }
    pub(super) fn poll_pipeline(&mut self){
        let result=match self.pipeline_editor.candidate.as_ref().map(|c|c.validation.try_recv()){
            Some(Ok(result))=>Some(result),Some(Err(mpsc::TryRecvError::Disconnected))=>Some(Err("Pipeline worker disconnected".into())),_=>None,
        };
        if let Some(result)=result{
            let candidate=self.pipeline_editor.candidate.take().expect("validated candidate");
            if self.pipeline_editor.clip.as_ref()!=Some(&candidate.clip)||candidate.revision!=self.document.revision{
                self.pipeline_error("Pipeline edit cancelled because the project or selected clip changed".into());return;
            }
            match result.and_then(|()|self.document.set_clip_pipeline(&candidate.clip,candidate.pipeline,candidate.revision)){
                Ok(())=>{
                    self.pipeline_editor.selected=candidate.selection;
                    self.pipeline_editor.drafts.retain(|(clip,_),_|clip!=&candidate.clip);
                    self.requested=None;self.changed();
                }
                Err(error)=>self.pipeline_error(error),
            }
        }
        if self.window.upgrade().is_some_and(|w|w.get_nodes_open()) && self.playing{
            let clip=self.document.data.active_clips(self.playhead).find(|(_, clip)| clip.title.is_none()).map(|(_, clip)| clip.id.clone());
            if clip!=self.pipeline_editor.clip{self.selected_clip=clip;self.publish_pipeline();}
        }
    }
    pub(super) fn publish_pipeline(&mut self){
        let Some(window)=self.window.upgrade()else{return;};
        if !window.get_nodes_open(){return;}
        let active=self.selected_clip.clone().filter(|id|self.document.clip(id).is_some_and(|clip|clip.title.is_none()));
        if active!=self.pipeline_editor.clip{
            self.pipeline_editor.monitor = None;
            self.pipeline_editor.clip=active.clone();self.pipeline_editor.selected.clear();self.pipeline_editor.edge=None;self.pipeline_editor.port=None;self.pipeline_editor.error.clear();
        }
        let mut graph=active.as_deref().and_then(|id|self.document.clip(id)).map(|clip|clip.pipeline.graph().clone()).unwrap_or_default();
        if self.pipeline_editor.monitor.as_ref().is_some_and(|(_, id)| !graph.nodes.iter().any(|node| node.id == *id)) {
            self.pipeline_editor.monitor = None;
        }
        let monitor_label = self.pipeline_editor.monitor.as_ref()
            .and_then(|(_, id)| graph.nodes.iter().find(|node| node.id == *id))
            .map(|node| node.data.label.as_str());
        window.set_pipeline_monitor_label(monitor_label.unwrap_or_default().into());
        self.pipeline_editor.selected.retain(|id|graph.nodes.iter().any(|n|&n.id==id));
        for node in &mut graph.nodes { node.data.extra.insert("expanded".into(),serde_json::json!(false)); }
        let projection=GraphViewModel::from_graph(&graph,&self.pipeline_editor.selected);
        window.set_pipeline_nodes(projection.nodes);window.set_pipeline_edges(projection.edges);
        window.set_pipeline_content_width(projection.content_width);window.set_pipeline_content_height(projection.content_height);
        window.set_pipeline_origin_x(projection.origin_x);window.set_pipeline_origin_y(projection.origin_y);
        window.set_pipeline_selected_edge_id(self.pipeline_editor.edge.as_deref().unwrap_or_default().into());
        window.set_pipeline_connection_active(self.pipeline_editor.port.is_some());
        window.set_pipeline_pending_node_id(self.pipeline_editor.port.as_ref().map(|p|p.0.as_str()).unwrap_or_default().into());
        window.set_pipeline_pending_port_id(self.pipeline_editor.port.as_ref().map(|p|p.1.as_str()).unwrap_or_default().into());
        window.set_pipeline_can_copy(self.pipeline_editor.selected.iter().any(|id|!ClipPipeline::is_protected(id)));
        window.set_pipeline_can_paste(self.pipeline_editor.clipboard.is_some()&&active.is_some());
        window.set_pipeline_selection_count(self.pipeline_editor.selected.len() as i32);
        self.pipeline_editor.revision=Some(self.document.revision);
        self.publish_pipeline_inspector();
    }
    fn publish_pipeline_inspector(&mut self){
        let Some(window)=self.window.upgrade()else{return;};
        window.set_pipeline_busy(self.pipeline_editor.candidate.is_some()||self.busy);
        window.set_pipeline_error(self.pipeline_editor.error.clone().into());
        let node=self.selected_pipeline_node();
        let editable=node.as_ref().is_some_and(|n|!ClipPipeline::is_protected(&n.id));
        window.set_pipeline_has_selection(node.is_some());
        window.set_pipeline_label_read_only(!editable);window.set_pipeline_shader_read_only(!editable);window.set_pipeline_can_delete(editable);
        window.set_pipeline_selection_message(if self.pipeline_editor.clip.is_none(){"Select a timeline clip to edit its pipeline"}else{"Select a node to edit its parameters"}.into());
        window.set_pipeline_node_id(node.as_ref().map(|n|n.id.as_str()).unwrap_or_default().into());
        window.set_pipeline_node_type(node.as_ref().map(|n|format!("{:?}",n.node_type)).unwrap_or_default().into());
        window.set_pipeline_is_shader(editable&&node.as_ref().is_some_and(|n|n.node_type==NodeType::Shader));
        window.set_pipeline_supports_output_size(editable&&node.as_ref().is_some_and(|n|matches!(n.node_type,NodeType::Shader|NodeType::Constant)));
        let graph=self.pipeline_editor.clip.as_deref().and_then(|id|self.document.clip(id)).map(|c|c.pipeline.graph().clone()).unwrap_or_default();
        self.publish_pipeline_corrector(node.as_ref());
        let Some(node)=node else {
            window.set_pipeline_inspector_dirty(false);return;
        };
        let error=self.pipeline_editor.error.clone();
        let corrector_dirty=self.pipeline_editor.corrector.as_ref().is_some_and(|state|state.draft.dirty());
        let Some(draft)=self.pipeline_draft()else{return;};
        if window.get_pipeline_node_label()!=draft.label.as_str(){window.set_pipeline_node_label(draft.label.clone().into());}
        if window.get_pipeline_shader_code()!=draft.code.as_str(){window.set_pipeline_shader_code(draft.code.clone().into());}
        window.set_pipeline_auto_size(draft.auto_size);window.set_pipeline_output_width(draft.width);window.set_pipeline_output_height(draft.height);
        window.set_pipeline_label_dirty(draft.label!=node.data.label);window.set_pipeline_shader_dirty(draft.code!=node.data.shader_code);
        window.set_pipeline_output_size_dirty(draft.width!=node.data.width.unwrap_or(512) as i32||draft.height!=node.data.height.unwrap_or(512) as i32||draft.auto_size!=node.data.auto_size.unwrap_or(true));
        let uniforms=node.data.inputs.iter().filter(|p|uniform_shape(p.data_type).is_some()).map(|port|{
            let literal=draft.uniforms.get(&port.label).cloned().unwrap_or_else(||format_uniform_literal(&effective_uniform_value(&node,port)));
            let connected=graph.edges.iter().any(|e|e.target==node.id&&e.target_handle==port.id);
            let error=if connected{String::new()}else{parse_uniform_literal(port.data_type,&literal).err().unwrap_or_default()};
            crate::InspectorUniform{name:port.label.clone().into(),data_type:format!("{:?}",port.data_type).into(),value:literal.into(),connected,valid:error.is_empty(),error:error.into()}
        }).collect::<Vec<_>>();
        window.set_pipeline_uniforms_valid(uniforms.iter().all(|u|u.valid));
        window.set_pipeline_uniform_dirty(draft.uniforms.iter().any(|(name,value)|node.data.inputs.iter().find(|p|&p.label==name).is_some_and(|p|*value!=format_uniform_literal(&effective_uniform_value(&node,p)))));
        window.set_pipeline_inspector_dirty(editable&&(draft.dirty()||corrector_dirty));
        window.set_pipeline_validation_message(error.into());
        window.set_pipeline_ports(ModelRc::new(VecModel::from(node.data.inputs.iter().chain(&node.data.outputs).map(|port|crate::InspectorPort{label:port.label.clone().into(),data_type:format!("{:?}",port.data_type).into(),output:port.direction==open_quartz::PortDirection::Output}).collect::<Vec<_>>())));
        // Keep form field identities stable while a text field owns focus.
        let old=window.get_pipeline_uniforms();
        if let Some(model)=old.as_any().downcast_ref::<VecModel<crate::InspectorUniform>>(){
            if model.row_count()==uniforms.len()&&(0..uniforms.len()).all(|i|model.row_data(i).is_some_and(|old|old.name==uniforms[i].name)){
                for(index,row)in uniforms.into_iter().enumerate(){if model.row_data(index).as_ref()!=Some(&row){model.set_row_data(index,row);}}
            }else{model.set_vec(uniforms);}
        }else{window.set_pipeline_uniforms(ModelRc::new(VecModel::from(uniforms)));}
        let forms=if editable{draft.forms.groups(&node.data)}else{Vec::new()};
        let old=window.get_pipeline_forms();
        let same=(0..old.row_count()).all(|i|old.row_data(i).zip(forms.get(i)).is_some_and(|(a,b)|{
            a.key==b.key&&a.dirty==b.dirty&&a.error==b.error&&a.fields.row_count()==b.fields.row_count()
                &&(0..a.fields.row_count()).all(|j|a.fields.row_data(j)==b.fields.row_data(j))
        }))&&old.row_count()==forms.len();
        if !same{window.set_pipeline_forms(ModelRc::new(VecModel::from(forms)));}
    }
}
