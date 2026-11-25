use open_quartz::operators::{ColorCorrectorConfig, ColorCorrectorStage as Stage, COLOR_CORRECTOR_FIELDS};
use slint::{Model, ModelRc, VecModel};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::atomic::{AtomicU64, Ordering}};
static NEXT_DRAFT: AtomicU64 = AtomicU64::new(1);

/// A host-owned, non-persistent transaction. The target token travels with every
/// queued UI intent, so a late pointer release cannot edit a different selection.
#[derive(Clone)]
pub struct CorrectorDraft {
    token: String,
    baseline: ColorCorrectorConfig,
    config: ColorCorrectorConfig,
    gesture: Option<ColorCorrectorConfig>,
    pub generation: u64,
    pub error: String,
    fields: Rc<VecModel<crate::CorrectorField>>,
    stages: Rc<VecModel<crate::CorrectorStage>>,
    text: RefCell<BTreeMap<String, String>>,
}

pub enum CorrectorAction {
    None,
    Preview(ColorCorrectorConfig),
    Commit(ColorCorrectorConfig),
    Cancel,
}

impl CorrectorDraft {
    pub fn new(host: String, node_id: String, revision: u64, config: ColorCorrectorConfig) -> Self {
        Self { token: format!("{host}/{node_id}/{revision}/{}", NEXT_DRAFT.fetch_add(1, Ordering::Relaxed)), baseline: config.clone(), config,
            gesture: None, generation: 0, error: String::new(), fields: Rc::new(VecModel::default()),
            stages: Rc::new(VecModel::default()), text: RefCell::new(BTreeMap::new()) }
    }
    pub fn token(&self) -> String { self.token.clone() }
    pub fn config(&self) -> &ColorCorrectorConfig { &self.config }
    pub fn dirty(&self) -> bool { self.config != self.baseline || !self.error.is_empty() }
    pub fn fields(&self) -> ModelRc<crate::CorrectorField> {
        let values = COLOR_CORRECTOR_FIELDS.iter().map(|field| crate::CorrectorField {
            key: field.key.into(), label: field.label.into(), stage: stage_key(field.stage).into(),
            tool: field_tool(field.key, field.stage).into(),
            value: self.config.field_value(field.key).unwrap_or(field.default),
            text: self.text.borrow().get(field.key).cloned().unwrap_or_else(|| format!("{:.4}", self.config.field_value(field.key).unwrap_or(field.default))).into(),
            minimum: field.min, maximum: field.max,
            wheel_x: self.wheel_position(field.key.trim_end_matches("_r")).0,
            wheel_y: self.wheel_position(field.key.trim_end_matches("_r")).1,
        }).collect::<Vec<_>>();
        update_model(&self.fields, values);
        ModelRc::from(self.fields.clone())
    }
    pub fn wheel_position(&self, prefix: &str) -> (f32, f32) {
        let r = self.config.field_value(&format!("{prefix}_r")).unwrap_or(0.0);
        let g = self.config.field_value(&format!("{prefix}_g")).unwrap_or(0.0);
        let b = self.config.field_value(&format!("{prefix}_b")).unwrap_or(0.0);
        ((r - (g + b) * 0.5) / 0.75, (g - b) / 0.8660254)
    }
    pub fn stages(&self) -> ModelRc<crate::CorrectorStage> {
        let values = [Stage::Primaries, Stage::HdrZones, Stage::RgbMixer].into_iter().map(|stage| crate::CorrectorStage {
            key: stage_key(stage).into(), label: match stage { Stage::Primaries => "Primaries", Stage::HdrZones => "HDR Zones", Stage::RgbMixer => "RGB Mixer" }.into(),
            active: self.config.stage_enabled(stage),
        }).collect::<Vec<_>>();
        update_model(&self.stages, values);
        ModelRc::from(self.stages.clone())
    }
    pub fn event(&mut self, action: &str, key: &str, value: &str) -> Result<CorrectorAction, String> {
        let result = self.apply_event(action, key, value);
        self.error = result.as_ref().err().cloned().unwrap_or_default();
        result
    }
    fn apply_event(&mut self, action: &str, key: &str, value: &str) -> Result<CorrectorAction, String> {
        if action == "cancel" {
            self.config = self.baseline.clone(); self.gesture = None;
            self.text.borrow_mut().clear();
            self.generation = self.generation.wrapping_add(1);
            return Ok(CorrectorAction::Cancel);
        }
        if action == "begin" {
            if self.gesture.is_none() { self.gesture = Some(self.config.clone()); }
            return Ok(CorrectorAction::None);
        }
        if action == "commit" && key.is_empty() && !self.error.is_empty() { return Err(self.error.clone()); }
        let mut candidate = self.config.clone();
        match action {
            "preview" => { self.text.borrow_mut().insert(key.into(), value.into()); }
            _ => { self.text.borrow_mut().clear(); }
        }
        match action {
            "preview" | "commit" if !key.is_empty() => {
                let number: f32 = value.trim().parse().map_err(|_| format!("{key}: enter a finite number"))?;
                candidate.set_field(key, number).map_err(|e| e.to_string())?;
            }
            "wheel" => {
                let (x, y) = value.split_once(',').ok_or("Invalid wheel coordinates")?;
                let x: f32 = x.parse().map_err(|_| "Invalid wheel x")?;
                let y: f32 = y.parse().map_err(|_| "Invalid wheel y")?;
                if !x.is_finite() || !y.is_finite() { return Err("Wheel coordinates must be finite".into()); }
                let radius = x.hypot(y).max(1.0);
                let (x, y) = (x / radius, y / radius);
                let source = self.gesture.as_ref().unwrap_or(&self.baseline);
                let keys = [format!("{key}_r"), format!("{key}_g"), format!("{key}_b")];
                let average = keys.iter().map(|k| source.field_value(k).ok_or("Unknown wheel")).collect::<Result<Vec<_>, _>>()?.iter().sum::<f32>() / 3.0;
                for (k, delta) in keys.iter().zip([x * 0.5, (-x * 0.5 + y * 0.8660254) * 0.5, (-x * 0.5 - y * 0.8660254) * 0.5]) {
                    candidate.set_field(k, average + delta).map_err(|e| e.to_string())?;
                }
            }
            "commit" => {},
            "enable" => candidate.enabled = parse_bool(value)?,
            "stage-enable" => candidate.set_stage_enabled(parse_stage(key)?, parse_bool(value)?),
            "reset" if key == "all" => {
                candidate.reset_stage(Stage::Primaries);
                candidate.reset_stage(Stage::HdrZones);
                candidate.reset_stage(Stage::RgbMixer);
            }
            "reset" => candidate.reset_stage(parse_stage(key)?),
            _ => return Err(format!("Unknown Corrector action: {action}")),
        }
        candidate.validate().map_err(|e| e.to_string())?;
        self.config = candidate;
        self.generation = self.generation.wrapping_add(1);
        if action == "preview" || action == "wheel" { return Ok(CorrectorAction::Preview(self.config.clone())); }
        self.gesture = None;
        if self.config != self.baseline { Ok(CorrectorAction::Commit(self.config.clone())) }
        else { Ok(CorrectorAction::Cancel) }
    }
}
fn update_model<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() { model.set_vec(rows); }
    else { for (index, value) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&value) { model.set_row_data(index, value); }
    } }
}
fn field_tool(key: &str, stage: Stage) -> &str {
    match stage {
        Stage::Primaries => key.strip_suffix("_r").or_else(|| key.strip_suffix("_g")).or_else(|| key.strip_suffix("_b")).unwrap_or("basic"),
        Stage::RgbMixer => "matrix",
        Stage::HdrZones => key.strip_suffix("_exposure").or_else(|| key.strip_suffix("_saturation")).unwrap_or("ranges"),
    }
}
fn stage_key(stage: Stage) -> &'static str { match stage { Stage::Primaries => "primaries", Stage::HdrZones => "hdr_zones", Stage::RgbMixer => "rgb_mixer" } }
fn parse_stage(key: &str) -> Result<Stage, String> { match key { "primaries" => Ok(Stage::Primaries), "hdr_zones" => Ok(Stage::HdrZones), "rgb_mixer" => Ok(Stage::RgbMixer), _ => Err(format!("Unknown Corrector stage: {key}")) } }
fn parse_bool(value: &str) -> Result<bool, String> { match value { "true" => Ok(true), "false" => Ok(false), _ => Err("Expected true or false".into()) } }
