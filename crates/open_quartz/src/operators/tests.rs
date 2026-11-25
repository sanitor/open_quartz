use super::*;
use crate::operators::{ColorCorrectorStage, COLOR_CORRECTOR_FIELDS};

fn close(actual:f32, expected:f32) {
    assert!((actual-expected).abs() <= 2e-5*expected.abs().max(1.0),"{actual} != {expected}");
}
fn managed(config:&ColorCorrectorConfig, pixel:[f32;4])->[f32;4] {
    evaluate_color_corrector(config,ColorDomain::AcesCgLinear,pixel).unwrap()
}

#[test]
fn exposure_preserves_signed_superwhite_and_alpha() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.exposure=2.0;
    let result=managed(&config,[-0.25,0.5,2.0,0.125]);
    for (actual,expected) in result.into_iter().zip([-1.0,2.0,8.0,0.125]) {close(actual,expected);}
}

#[test]
fn primaries_fixed_order_and_signed_gamma_are_numerically_observable() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.exposure=1.0;
    config.primaries.lift=[0.25;3];
    config.primaries.gamma=[2.0;3];
    config.primaries.gain=[2.0;3];
    config.primaries.offset=[0.1;3];
    config.primaries.contrast=2.0;
    config.primaries.pivot=0.5;
    // -0.25*2+.25=-.25, signed sqrt=-.5, gain/offset=-.9,
    // signed contrast at pivot .5 gives -.5*(.9/.5)^2=-1.62.
    let result=managed(&config,[-0.25;4]);
    for actual in &result[..3] {close(*actual,-1.62);}
    assert_eq!(result[3],-0.25);
}

#[test]
fn hdr_zones_select_real_dark_and_bright_scene_ranges_without_clipping() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.enabled=false;
    config.hdr_zones.blacks.exposure=1.0;
    config.hdr_zones.specular.exposure=-1.0;
    let dark=managed(&config,[0.001;4]);
    let bright=managed(&config,[64.0;4]);
    close(dark[0],0.002);
    close(bright[0],32.0);
    assert_eq!(dark[3],0.001);
    assert_eq!(bright[3],64.0);
    // Exactly at 18% gray, default zone transitions have zero black/specular weight.
    close(managed(&config,[0.18,0.18,0.18,1.0])[0],0.18);
}

#[test]
fn each_of_six_zones_is_effective_and_transition_boundaries_are_continuous() {
    for (field,stops) in [("blacks_exposure",-7.0),("shadows_exposure",-3.0),
        ("darks_exposure",-1.0),("lights_exposure",1.0),("highlights_exposure",3.0),("specular_exposure",7.0)] {
        let mut config=ColorCorrectorConfig::default();
        config.primaries.enabled=false;
        config.hdr_zones.falloff=0.25;
        config.set_field(field,1.0).unwrap();
        let value=0.18*2.0_f32.powf(stops);
        close(managed(&config,[value,value,value,0.5])[0],value*2.0);
    }
    let mut config=ColorCorrectorConfig::default();
    config.primaries.enabled=false;
    for field in ["blacks_exposure","darks_exposure","highlights_exposure"] { config.set_field(field,1.0).unwrap(); }
    for boundary in [-4.0,-2.0,0.0,2.0,4.0] {
        let a=0.18*2.0_f32.powf(boundary-0.0001);
        let b=0.18*2.0_f32.powf(boundary+0.0001);
        let low=managed(&config,[a,a,a,1.0])[0];
        let high=managed(&config,[b,b,b,1.0])[0];
        assert!((high-low).abs()<b*0.001);
    }
}

#[test]
fn saturation_is_ap1_luminance_based_and_white_balance_is_real() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.saturation=0.0;
    let gray=managed(&config,[1.0,0.0,0.0,0.4]);
    for actual in &gray[..3] {close(*actual,0.27222872);}
    config.primaries.saturation=1.0;
    config.primaries.white_balance=[0.5,0.0];
    let warmer=managed(&config,[0.18,0.18,0.18,0.4]);
    assert!(warmer[0]>warmer[1] && warmer[1]>warmer[2]);
    close((warmer[0] as f64*0.2722287168+warmer[1] as f64*0.6740817658+warmer[2] as f64*0.0536895174) as f32,0.18);
    config.primaries.white_balance=[0.0,0.5];
    let magenta=managed(&config,[0.18,0.18,0.18,0.4]);
    assert!(magenta[0]>magenta[1] && magenta[2]>magenta[1]);
    close(magenta[0],magenta[2]);
}

#[test]
fn stage_bypass_retains_values_reset_is_independent_and_alias_is_exact() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.exposure=4.0;
    config.set_stage_enabled(ColorCorrectorStage::Primaries,false);
    let pixel=[-0.75,1.25,3.5,0.33];
    assert_eq!(managed(&config,pixel),pixel);
    let lowered=lower_color_corrector(&config,ColorDomain::LegacySrgb).unwrap();
    assert!(lowered.shader_source.is_none() && lowered.parameters.is_empty());
    assert_eq!(lowered.output_domain,ColorDomain::LegacySrgb);
    assert_eq!(lowered.alias.unwrap().input_port,"source");
    assert_eq!(config.primaries.exposure,4.0);
    config.reset_stage(ColorCorrectorStage::Primaries);
    assert!(!config.primaries.enabled);
    assert_eq!(config.primaries.exposure,0.0);
}

#[test]
fn legacy_input_is_decoded_not_interpreted_as_linear_ap1() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.exposure=1.0;
    let legacy=evaluate_color_corrector(&config,ColorDomain::LegacySrgb,[0.5,0.5,0.5,0.2]).unwrap();
    close(legacy[0],0.4280823);
    close(legacy[1],0.4280823);
    close(legacy[2],0.4280823);
    close(managed(&config,[0.5,0.5,0.5,0.2])[0],1.0);
    assert_eq!(legacy[3],0.2);
}

#[test]
fn updates_separate_uniform_values_from_stage_and_domain_structure() {
    let mut config=ColorCorrectorConfig::default();
    config.primaries.exposure=1.0;
    let mut edited=config.clone();
    edited.primaries.exposure=2.0;
    assert_eq!(config.classify_update(&edited,ColorDomain::LegacySrgb).unwrap(),OperatorUpdate::Parameters);
    let a=lower_color_corrector(&config,ColorDomain::LegacySrgb).unwrap();
    let b=lower_color_corrector(&edited,ColorDomain::LegacySrgb).unwrap();
    assert_eq!(a.shader_source,b.shader_source);
    assert_ne!(a.parameters,b.parameters);
    edited.hdr_zones.enabled=false;
    assert_eq!(config.classify_update(&edited,ColorDomain::LegacySrgb).unwrap(),OperatorUpdate::Structural);
    assert_ne!(config.structure_key(ColorDomain::LegacySrgb).unwrap(),config.structure_key(ColorDomain::AcesCgLinear).unwrap());
    assert_eq!(config.classify_update(&ColorCorrectorConfig::default(),ColorDomain::LegacySrgb).unwrap(),OperatorUpdate::Structural);
}

#[test]
fn generated_variants_are_valid_wgsl_and_disabled_stages_have_no_bindings() {
    for domain in [ColorDomain::LegacySrgb,ColorDomain::AcesCgLinear] {
        for (primaries,zones) in [(true,false),(false,true),(true,true)] {
            let mut config=ColorCorrectorConfig::default();
            config.primaries.enabled=primaries;
            config.hdr_zones.enabled=zones;
            config.primaries.exposure=1.0;
            config.hdr_zones.shadows.exposure=1.0;
            let lowered=lower_color_corrector(&config,domain).unwrap();
            assert_eq!(lowered.parameters.contains_key("cc_primary"),primaries);
            assert_eq!(lowered.parameters.contains_key("cc_zones_a"),zones);
            let source=lowered.shader_source.unwrap();
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|error|panic!("{}",error.emit_to_string(&source)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::empty()).validate(&module).unwrap();
        }
    }
}

#[test]
fn malformed_configs_are_rejected_including_disabled_stages_and_unknown_versions() {
    for key in ["schemaVersion","algorithmVersion"] {
        let mut json=serde_json::to_value(ColorCorrectorConfig::default()).unwrap();
        json[key]=serde_json::json!(99);
        assert!(serde_json::from_value::<ColorCorrectorConfig>(json).is_err());
    }
    let mut unknown=serde_json::to_value(ColorCorrectorConfig::default()).unwrap();
    unknown["curves"]=serde_json::json!({"enabled":true});
    assert!(serde_json::from_value::<ColorCorrectorConfig>(unknown).is_err());
    let mut unknown_stage=serde_json::to_value(ColorCorrectorConfig::default()).unwrap();
    unknown_stage["primaries"]["secret"]=serde_json::json!(0);
    assert!(serde_json::from_value::<ColorCorrectorConfig>(unknown_stage).is_err());
    let mut config=ColorCorrectorConfig::default();
    config.enabled=false;
    config.primaries.gamma[0]=0.0;
    assert!(config.validate().is_err());
    config.primaries.gamma[0]=1.0;
    config.hdr_zones.shadow_pivot=config.hdr_zones.highlight_pivot;
    assert!(config.validate().is_err());
    config=ColorCorrectorConfig::default();
    assert!(config.set_field("exposure",f32::NAN).is_err());
    assert_eq!(config.primaries.exposure,0.0);
    assert!(config.set_field("does_not_exist",0.0).is_err());
    for descriptor in COLOR_CORRECTOR_FIELDS {
        assert_eq!(config.field_value(descriptor.key),Some(descriptor.default));
    }
}

#[test]
fn rgb_mixer_preserves_signed_headroom_and_alpha() {
    let mut config = ColorCorrectorConfig::default();
    config.rgb_mixer.red = [0.0, -2.0, 1.0];
    config.rgb_mixer.green = [1.5, 0.0, 0.0];
    let pixel = [2.0, -0.25, -1.0, 0.5];
    assert_eq!(managed(&config, pixel), [-0.5, 3.0, -1.0, 0.5]);
    let lowered = lower_color_corrector(&config, ColorDomain::AcesCgLinear).unwrap();
    assert_eq!(lowered.parameters["cc_mix"], [0.0, 1.5, 0.0, -2.0, 0.0, 0.0, 1.0, 0.0, 1.0]);
    config.set_stage_enabled(ColorCorrectorStage::RgbMixer, false);
    assert_eq!(managed(&config, pixel), pixel);
    config.reset_stage(ColorCorrectorStage::RgbMixer);
    config.set_stage_enabled(ColorCorrectorStage::RgbMixer, true);
    assert!(config.is_identity());
}
