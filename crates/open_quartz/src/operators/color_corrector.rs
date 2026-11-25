use serde::{Deserialize, Serialize};
use super::OperatorError;

/// Straight-alpha, linear AP1 working space. Legacy bytes are explicitly decoded at a boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorDomain { LegacySrgb, AcesCgLinear }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorCorrectorStage { Primaries, HdrZones, RgbMixer, Curves }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrimariesConfig {
    pub enabled: bool,
    /// Relative temperature and tint in [-1, 1], not a camera-specific Kelvin IDT.
    pub white_balance: [f32; 2],
    pub exposure: f32,
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    pub offset: [f32; 3],
    pub contrast: f32,
    pub pivot: f32,
    pub saturation: f32,
    pub color_boost: f32,
}
impl Default for PrimariesConfig {
    fn default() -> Self {
        Self { enabled: true, white_balance: [0.0; 2], exposure: 0.0, lift: [0.0; 3],
            gamma: [1.0; 3], gain: [1.0; 3], offset: [0.0; 3], contrast: 1.0,
            pivot: 0.18, saturation: 1.0, color_boost: 0.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HdrZone { pub exposure: f32, pub saturation: f32 }
impl Default for HdrZone {
    fn default() -> Self { Self { exposure: 0.0, saturation: 1.0 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HdrZonesConfig {
    pub enabled: bool,
    pub blacks: HdrZone,
    pub shadows: HdrZone,
    pub darks: HdrZone,
    pub lights: HdrZone,
    pub highlights: HdrZone,
    pub specular: HdrZone,
    /// Zone transition centers in stops relative to scene-linear 18% gray.
    pub shadow_pivot: f32,
    pub highlight_pivot: f32,
    /// Outer zone extension in stops, independent of transition softness.
    pub range: f32,
    pub falloff: f32,
}
impl Default for HdrZonesConfig {
    fn default() -> Self {
        Self { enabled: true, blacks: HdrZone::default(), shadows: HdrZone::default(), darks: HdrZone::default(),
            lights: HdrZone::default(), highlights: HdrZone::default(), specular: HdrZone::default(),
            shadow_pivot: -2.0, highlight_pivot: 2.0, range: 2.0, falloff: 2.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RgbMixerConfig {
    pub enabled: bool,
    pub red: [f32; 3],
    pub green: [f32; 3],
    pub blue: [f32; 3],
}

impl Default for RgbMixerConfig {
    fn default() -> Self {
        Self { enabled: true, red: [1.0, 0.0, 0.0], green: [0.0, 1.0, 0.0], blue: [0.0, 0.0, 1.0] }
    }
}

impl RgbMixerConfig {
    pub(super) fn is_default(&self) -> bool { *self == Self::default() }

    pub(super) fn is_identity(&self) -> bool { !self.enabled || self.is_default() }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CurveConfig {
    pub enabled: bool,
    pub master: [f32; 4],
    pub red: [f32; 4],
    pub green: [f32; 4],
    pub blue: [f32; 4],
}

impl Default for CurveConfig {
    fn default() -> Self {
        Self { enabled: true, master: [0.0; 4], red: [0.0; 4], green: [0.0; 4], blue: [0.0; 4] }
    }
}

impl CurveConfig {
    pub(super) fn is_default(&self) -> bool { *self == Self::default() }

    pub(super) fn is_identity(&self) -> bool { !self.enabled || self.is_default() }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ColorCorrectorWire {
    schema_version: u32,
    algorithm_version: u32,
    enabled: bool,
    primaries: PrimariesConfig,
    hdr_zones: HdrZonesConfig,
    #[serde(default)]
    rgb_mixer: RgbMixerConfig,
    #[serde(default)]
    curves: CurveConfig,
}

/// Version 1 order: WB → exposure → lift/gamma/gain/offset → contrast → HDR zones
/// → saturation/color boost. RGB is signed and unclipped; alpha is copied unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "ColorCorrectorWire")]
pub struct ColorCorrectorConfig {
    pub schema_version: u32,
    pub algorithm_version: u32,
    pub enabled: bool,
    pub primaries: PrimariesConfig,
    pub hdr_zones: HdrZonesConfig,
    #[serde(skip_serializing_if = "RgbMixerConfig::is_default")]
    pub rgb_mixer: RgbMixerConfig,
    #[serde(skip_serializing_if = "CurveConfig::is_default")]
    pub curves: CurveConfig,
}
impl Default for ColorCorrectorConfig {
    fn default() -> Self {
        Self { schema_version: 1, algorithm_version: 1, enabled: true,
            primaries: PrimariesConfig::default(), hdr_zones: HdrZonesConfig::default(), rgb_mixer: RgbMixerConfig::default(), curves: CurveConfig::default() }
    }
}
impl TryFrom<ColorCorrectorWire> for ColorCorrectorConfig {
    type Error = OperatorError;
    fn try_from(w: ColorCorrectorWire) -> Result<Self, Self::Error> {
        let config = Self { schema_version: w.schema_version, algorithm_version: w.algorithm_version,
            enabled: w.enabled, primaries: w.primaries, hdr_zones: w.hdr_zones, rgb_mixer: w.rgb_mixer, curves: w.curves };
        config.validate()?;
        Ok(config)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ColorCorrectorField {
    pub key: &'static str,
    pub label: &'static str,
    pub stage: ColorCorrectorStage,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    get: fn(&ColorCorrectorConfig) -> f32,
    set: fn(&mut ColorCorrectorConfig, f32),
}
macro_rules! field {
    ($key:literal,$label:literal,$stage:ident,$min:expr,$max:expr,$default:expr,$($member:ident).+) => {
        ColorCorrectorField { key:$key,label:$label,stage:ColorCorrectorStage::$stage,min:$min,max:$max,default:$default,
            get: |c| c.$($member).+, set: |c,v| c.$($member).+ = v }
    };
    ($key:literal,$label:literal,$stage:ident,$min:expr,$max:expr,$default:expr,$($member:ident).+[$index:expr]) => {
        ColorCorrectorField { key:$key,label:$label,stage:ColorCorrectorStage::$stage,min:$min,max:$max,default:$default,
            get: |c| c.$($member).+[$index], set: |c,v| c.$($member).+[$index] = v }
    };
}

pub const COLOR_CORRECTOR_FIELDS: &[ColorCorrectorField] = &[
    field!("temperature","Temperature",Primaries,-1.0,1.0,0.0,primaries.white_balance[0]),
    field!("tint","Tint",Primaries,-1.0,1.0,0.0,primaries.white_balance[1]),
    field!("exposure","Exposure (stops)",Primaries,-16.0,16.0,0.0,primaries.exposure),
    field!("lift_r","Lift R",Primaries,-2.0,2.0,0.0,primaries.lift[0]),
    field!("lift_g","Lift G",Primaries,-2.0,2.0,0.0,primaries.lift[1]),
    field!("lift_b","Lift B",Primaries,-2.0,2.0,0.0,primaries.lift[2]),
    field!("gamma_r","Gamma R",Primaries,0.1,4.0,1.0,primaries.gamma[0]),
    field!("gamma_g","Gamma G",Primaries,0.1,4.0,1.0,primaries.gamma[1]),
    field!("gamma_b","Gamma B",Primaries,0.1,4.0,1.0,primaries.gamma[2]),
    field!("gain_r","Gain R",Primaries,0.0,8.0,1.0,primaries.gain[0]),
    field!("gain_g","Gain G",Primaries,0.0,8.0,1.0,primaries.gain[1]),
    field!("gain_b","Gain B",Primaries,0.0,8.0,1.0,primaries.gain[2]),
    field!("offset_r","Offset R",Primaries,-4.0,4.0,0.0,primaries.offset[0]),
    field!("offset_g","Offset G",Primaries,-4.0,4.0,0.0,primaries.offset[1]),
    field!("offset_b","Offset B",Primaries,-4.0,4.0,0.0,primaries.offset[2]),
    field!("contrast","Contrast",Primaries,0.1,4.0,1.0,primaries.contrast),
    field!("pivot","Contrast pivot",Primaries,0.001,4.0,0.18,primaries.pivot),
    field!("saturation","Saturation",Primaries,0.0,4.0,1.0,primaries.saturation),
    field!("color_boost","Color boost",Primaries,0.0,2.0,0.0,primaries.color_boost),
    field!("blacks_exposure","Black exposure",HdrZones,-16.0,16.0,0.0,hdr_zones.blacks.exposure),
    field!("blacks_saturation","Black saturation",HdrZones,0.0,4.0,1.0,hdr_zones.blacks.saturation),
    field!("shadows_exposure","Shadows exposure",HdrZones,-16.0,16.0,0.0,hdr_zones.shadows.exposure),
    field!("shadows_saturation","Shadows saturation",HdrZones,0.0,4.0,1.0,hdr_zones.shadows.saturation),
    field!("darks_exposure","Dark exposure",HdrZones,-16.0,16.0,0.0,hdr_zones.darks.exposure),
    field!("darks_saturation","Dark saturation",HdrZones,0.0,4.0,1.0,hdr_zones.darks.saturation),
    field!("lights_exposure","Light exposure",HdrZones,-16.0,16.0,0.0,hdr_zones.lights.exposure),
    field!("lights_saturation","Light saturation",HdrZones,0.0,4.0,1.0,hdr_zones.lights.saturation),
    field!("highlights_exposure","Highlights exposure",HdrZones,-16.0,16.0,0.0,hdr_zones.highlights.exposure),
    field!("highlights_saturation","Highlights saturation",HdrZones,0.0,4.0,1.0,hdr_zones.highlights.saturation),
    field!("specular_exposure","Specular exposure",HdrZones,-16.0,16.0,0.0,hdr_zones.specular.exposure),
    field!("specular_saturation","Specular saturation",HdrZones,0.0,4.0,1.0,hdr_zones.specular.saturation),
    field!("shadow_pivot","Shadow center (stops)",HdrZones,-12.0,12.0,-2.0,hdr_zones.shadow_pivot),
    field!("highlight_pivot","Highlight center (stops)",HdrZones,-12.0,12.0,2.0,hdr_zones.highlight_pivot),
    field!("zone_range","Outer zone range (stops)",HdrZones,0.1,8.0,2.0,hdr_zones.range),
    field!("falloff","Zone falloff (stops)",HdrZones,0.1,8.0,2.0,hdr_zones.falloff),
    field!("mix_red_red","Red output from red",RgbMixer,-4.0,4.0,1.0,rgb_mixer.red[0]),
    field!("mix_red_green","Red output from green",RgbMixer,-4.0,4.0,0.0,rgb_mixer.red[1]),
    field!("mix_red_blue","Red output from blue",RgbMixer,-4.0,4.0,0.0,rgb_mixer.red[2]),
    field!("mix_green_red","Green output from red",RgbMixer,-4.0,4.0,0.0,rgb_mixer.green[0]),
    field!("mix_green_green","Green output from green",RgbMixer,-4.0,4.0,1.0,rgb_mixer.green[1]),
    field!("mix_green_blue","Green output from blue",RgbMixer,-4.0,4.0,0.0,rgb_mixer.green[2]),
    field!("mix_blue_red","Blue output from red",RgbMixer,-4.0,4.0,0.0,rgb_mixer.blue[0]),
    field!("mix_blue_green","Blue output from green",RgbMixer,-4.0,4.0,0.0,rgb_mixer.blue[1]),
    field!("mix_blue_blue","Blue output from blue",RgbMixer,-4.0,4.0,1.0,rgb_mixer.blue[2]),
    field!("curve_master_shadows","Shadows (-4 EV)",Curves,-4.0,4.0,0.0,curves.master[0]),
    field!("curve_master_low","Low mids (-2 EV)",Curves,-4.0,4.0,0.0,curves.master[1]),
    field!("curve_master_middle","Mid grey (+0 EV)",Curves,-4.0,4.0,0.0,curves.master[2]),
    field!("curve_master_highlights","Highlights (+3 EV)",Curves,-4.0,4.0,0.0,curves.master[3]),
    field!("curve_red_shadows","Shadows (-4 EV)",Curves,-4.0,4.0,0.0,curves.red[0]),
    field!("curve_red_low","Low mids (-2 EV)",Curves,-4.0,4.0,0.0,curves.red[1]),
    field!("curve_red_middle","Mid grey (+0 EV)",Curves,-4.0,4.0,0.0,curves.red[2]),
    field!("curve_red_highlights","Highlights (+3 EV)",Curves,-4.0,4.0,0.0,curves.red[3]),
    field!("curve_green_shadows","Shadows (-4 EV)",Curves,-4.0,4.0,0.0,curves.green[0]),
    field!("curve_green_low","Low mids (-2 EV)",Curves,-4.0,4.0,0.0,curves.green[1]),
    field!("curve_green_middle","Mid grey (+0 EV)",Curves,-4.0,4.0,0.0,curves.green[2]),
    field!("curve_green_highlights","Highlights (+3 EV)",Curves,-4.0,4.0,0.0,curves.green[3]),
    field!("curve_blue_shadows","Shadows (-4 EV)",Curves,-4.0,4.0,0.0,curves.blue[0]),
    field!("curve_blue_low","Low mids (-2 EV)",Curves,-4.0,4.0,0.0,curves.blue[1]),
    field!("curve_blue_middle","Mid grey (+0 EV)",Curves,-4.0,4.0,0.0,curves.blue[2]),
    field!("curve_blue_highlights","Highlights (+3 EV)",Curves,-4.0,4.0,0.0,curves.blue[3]),
];

impl ColorCorrectorConfig {
    pub fn validate(&self) -> Result<(), OperatorError> {
        if self.schema_version != 1 || self.algorithm_version != 1 {
            return Err(OperatorError("Unsupported ColorCorrector schema or algorithm version".into()));
        }
        for field in COLOR_CORRECTOR_FIELDS {
            let value = (field.get)(self);
            if !value.is_finite() || value < field.min || value > field.max {
                return Err(OperatorError(format!("{} must be finite and in [{}, {}]",field.key,field.min,field.max)));
            }
        }
        if self.hdr_zones.shadow_pivot >= self.hdr_zones.highlight_pivot {
            return Err(OperatorError("Shadow center must be below highlight center".into()));
        }
        Ok(())
    }
    pub fn field_value(&self, key: &str) -> Option<f32> {
        COLOR_CORRECTOR_FIELDS.iter().find(|f| f.key == key).map(|f| (f.get)(self))
    }
    pub fn set_field(&mut self, key: &str, value: f32) -> Result<(), OperatorError> {
        let field = COLOR_CORRECTOR_FIELDS.iter().find(|f| f.key == key)
            .ok_or_else(|| OperatorError(format!("Unknown ColorCorrector field {key}")))?;
        let previous = (field.get)(self);
        (field.set)(self, value);
        if let Err(error) = self.validate() { (field.set)(self, previous); return Err(error); }
        Ok(())
    }
    pub fn stage_enabled(&self, stage: ColorCorrectorStage) -> bool {
        match stage { ColorCorrectorStage::Primaries => self.primaries.enabled, ColorCorrectorStage::HdrZones => self.hdr_zones.enabled, ColorCorrectorStage::RgbMixer => self.rgb_mixer.enabled, ColorCorrectorStage::Curves => self.curves.enabled }
    }
    pub fn set_stage_enabled(&mut self, stage: ColorCorrectorStage, enabled: bool) {
        match stage { ColorCorrectorStage::Primaries => self.primaries.enabled = enabled, ColorCorrectorStage::HdrZones => self.hdr_zones.enabled = enabled, ColorCorrectorStage::RgbMixer => self.rgb_mixer.enabled = enabled, ColorCorrectorStage::Curves => self.curves.enabled = enabled }
    }
    /// Reset parameters, preserving the independent bypass state.
    pub fn reset_stage(&mut self, stage: ColorCorrectorStage) {
        for field in COLOR_CORRECTOR_FIELDS.iter().filter(|f| f.stage == stage) { (field.set)(self, field.default); }
    }
    pub fn is_identity(&self) -> bool {
        !self.enabled || ((!self.primaries.enabled || {
            let p = &self.primaries;
            p.white_balance == [0.0;2] && p.exposure == 0.0 && p.lift == [0.0;3]
                && p.gamma == [1.0;3] && p.gain == [1.0;3] && p.offset == [0.0;3]
                && p.contrast == 1.0 && p.saturation == 1.0 && p.color_boost == 0.0
        }) && (!self.hdr_zones.enabled || {
            let z = &self.hdr_zones;
            [&z.blacks,&z.shadows,&z.darks,&z.lights,&z.highlights,&z.specular].iter().all(|z| z.exposure == 0.0 && z.saturation == 1.0)
        }) && self.rgb_mixer.is_identity() && self.curves.is_identity())
    }
}
