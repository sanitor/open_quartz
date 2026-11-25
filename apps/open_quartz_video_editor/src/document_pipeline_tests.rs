use super::*;
use crate::clip_pipeline::{OUTPUT_ID, SOURCE_ID};
use open_quartz::project::{GraphCommand, NodeId, Position};

const INVERT: &str = "@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f { let color = textureSample(image, imageSampler, uv); return vec4f(vec3f(1.0) - color.rgb, color.a); }";

fn document() -> EditorDocument {
    let mut data = ProjectData::default();
    data.id = "pipeline-project".into();
    data.assets.push(MediaAsset {
        id: "asset-1".into(), name: "movie.mp4".into(), path: "/offline/movie.mp4".into(),
        fingerprint: "offline-fingerprint".into(), still: false, size_bytes: 1, width: 64, height: 64,
        duration_pts: Some(90), start_pts: Some(9000),
        time_base: Rational { numerator: 1, denominator: 30 },
        fps: Some(Rational { numerator: 30, denominator: 1 }), has_audio: false, audio: None, online: false,
    });
    data.tracks[0].clips.push(Clip {
        id: "clip-1".into(), title: None, asset_id: "asset-1".into(), timeline_start: 0, duration: 60,
        speed_percent: normal_speed(), source_span_remainder: 0,
        source_in_pts: 9000, source_subtick_numerator: 1, source_subtick_denominator: 3,
        transform: ClipTransform::default(), opacity_keys: Vec::new(), mask: None, crop: Crop::default(), crop_keys: Vec::new(), spatial_keys: Vec::new(), pipeline: ClipPipeline::default(), effect_keys: Vec::new(),
    });
    EditorDocument::new(data).unwrap()
}

fn document_with_media(directory: &Path) -> EditorDocument {
    let path = directory.join("movie.mp4");
    fs::write(&path, b"pipeline persistence fixture").unwrap();
    let mut data = document().data;
    let asset = &mut data.assets[0];
    asset.path = path.canonicalize().unwrap().to_string_lossy().into_owned();
    asset.fingerprint = fingerprint(&path).unwrap();
    asset.size_bytes = fs::metadata(&path).unwrap().len();
    asset.online = true;
    EditorDocument::new(data).unwrap()
}

fn serialized(document: &EditorDocument) -> serde_json::Value {
    serde_json::to_value(ProjectFile { schema_version: SCHEMA_VERSION, data: document.data.clone() }).unwrap()
}

fn write_project(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

#[test]
fn added_and_split_clips_have_independent_pipeline_values_and_exact_source_time() {
    let mut document = document();
    let mut pipeline = document.clip("clip-1").unwrap().pipeline.clone();
    let node = pipeline.add_serial(SOURCE_ID, INVERT, "Invert").unwrap();
    document.set_clip_pipeline("clip-1", pipeline.clone(), document.revision).unwrap();
    let second = document.add_clip("asset-1").unwrap();
    assert_eq!(document.clip(&second).unwrap().pipeline, ClipPipeline::default());
    assert_eq!(document.clip("clip-1").unwrap().pipeline, pipeline);
    let right = document.split("clip-1", 15).unwrap();
    assert_eq!(document.clip(&right).unwrap().pipeline, pipeline);
    assert_eq!(document.clip("clip-1").unwrap().pipeline, pipeline);
    let right_clip = document.clip(&right).unwrap();
    assert_eq!((right_clip.source_in_pts, right_clip.source_subtick_numerator,
        right_clip.source_subtick_denominator), (9015, 1, 3));
    let before = document.data.clone();
    let mut edited = document.clip(&right).unwrap().pipeline.clone();
    edited.remove_nodes(&[node]).unwrap();
    document.set_clip_pipeline(&right, edited, document.revision).unwrap();
    assert_eq!(document.clip("clip-1").unwrap().pipeline, pipeline);
    assert_eq!(document.clip(&second).unwrap().pipeline, ClipPipeline::default());
    let expected_right = before.clips().find(|clip| clip.id == right).unwrap();
    let mut actual_right = document.clip(&right).unwrap().clone();
    actual_right.pipeline = expected_right.pipeline.clone();
    assert_eq!(&actual_right, expected_right);
}

#[test]
fn pipeline_commit_undo_redo_noop_stale_and_invalid_commands_share_document_history() {
    let mut document = document();
    let initial = document.data.clone();
    let mut changed = document.clip("clip-1").unwrap().pipeline.clone();
    changed.add_serial(SOURCE_ID, INVERT, "Invert").unwrap();
    document.set_clip_pipeline("clip-1", changed.clone(), 0).unwrap();
    assert_eq!(document.revision, 1);
    assert!(document.is_dirty());
    assert!(document.undo());
    assert_eq!(document.data, initial);
    assert!(!document.is_dirty());
    assert!(document.can_redo());
    let revision = document.revision;
    document.set_clip_pipeline("clip-1", ClipPipeline::default(), revision).unwrap();
    assert_eq!(document.revision, revision);
    assert!(document.can_redo());
    assert!(document.set_clip_pipeline("clip-1", changed.clone(), 0).is_err());
    assert!(document.set_clip_pipeline("missing", changed.clone(), revision).is_err());
    let mut invalid = serde_json::to_value(&changed).unwrap();
    invalid["graph"]["nodes"].as_array_mut().unwrap().retain(|node| node["id"] != OUTPUT_ID);
    let invalid: ClipPipeline = serde_json::from_value(invalid).unwrap();
    assert!(document.set_clip_pipeline("clip-1", invalid, revision).is_err());
    assert_eq!(document.revision, revision);
    assert_eq!(document.data, initial);
    assert!(document.can_redo());
    assert!(document.redo());
    assert_eq!(document.clip("clip-1").unwrap().pipeline, changed);
    assert!(document.is_dirty());
    assert!(document.undo());
    document.rename("A timeline command").unwrap();
    assert!(!document.can_redo());
}

#[test]
fn schema_one_migrates_to_explicit_pipeline_in_default_track() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.oqvideo.json");
    let original = document_with_media(dir.path());
    let mut legacy = serialized(&original);
    legacy["schema_version"] = serde_json::json!(1);
    legacy["data"]["schema_version"] = serde_json::json!(1);
    legacy["data"]["clips"] = legacy["data"]["tracks"][0]["clips"].clone();
    legacy["data"].as_object_mut().unwrap().remove("tracks");
    legacy["data"]["clips"][0].as_object_mut().unwrap().remove("pipeline");
    write_project(&path, &legacy);
    let mut migrated = EditorDocument::open(path.clone()).unwrap();
    assert_eq!(migrated.data, original.data);
    assert_eq!(migrated.data.schema_version, SCHEMA_VERSION);
    assert_eq!(migrated.clip("clip-1").unwrap().pipeline, ClipPipeline::default());
    assert!(!migrated.is_dirty());
    assert!(!migrated.can_undo());
    migrated.save().unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["schema_version"], SCHEMA_VERSION);
    assert_eq!(saved["data"]["schema_version"], SCHEMA_VERSION);
    assert!(saved["data"]["tracks"][0]["clips"][0].get("pipeline").is_some());
    assert_eq!(EditorDocument::open(path.clone()).unwrap().data, migrated.data);
    let object = legacy["data"]["clips"][0].as_object_mut().unwrap();
    object.remove("source_subtick_numerator");
    object.remove("source_subtick_denominator");
    write_project(&path, &legacy);
    let old_fractionless = EditorDocument::open(path).unwrap();
    let clip = old_fractionless.clip("clip-1").unwrap();
    assert_eq!((clip.source_in_pts, clip.source_subtick_numerator, clip.source_subtick_denominator), (9000, 0, 1));
}

#[test]
fn schema_two_migrates_without_changing_legacy_transform_or_effect() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-effects.oqvideo.json");
    let mut original = document_with_media(dir.path());
    let mut pipeline = ClipPipeline::default();
    pipeline.add_serial("video",
        "@fragment fn main(@location(0) uv:vec2f)->@location(0) vec4f { return textureSample(image,imageSampler,uv); }",
        "Legacy effect").unwrap();
    original.set_clip_pipeline("clip-1", pipeline, original.revision).unwrap();
    let transform = ClipTransform { scale: 0.75, x: -0.2, y: 0.3, exposure: -1.0, ..Default::default() };
    original.set_transform("clip-1", transform, original.revision).unwrap();
    let mut legacy = serialized(&original);
    legacy["schema_version"] = serde_json::json!(2);
    legacy["data"]["schema_version"] = serde_json::json!(2);
    let old_data = legacy["data"].as_object_mut().unwrap();
    let mut clips = old_data.remove("tracks").unwrap()[0]["clips"].clone();
    let old_transform = clips[0]["transform"].as_object_mut().unwrap();
    old_transform.remove("rotation");
    old_transform.remove("opacity");
    old_data.insert("clips".into(), clips);
    write_project(&path, &legacy);
    let migrated = EditorDocument::open(path).unwrap();
    assert_eq!(migrated.data, original.data);
    assert_eq!(migrated.clip_track("clip-1").unwrap().id, DEFAULT_TRACK);
}

#[test]
fn schema_loading_rejects_missing_null_invalid_and_undeclared_pipelines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid.oqvideo.json");
    let original = serialized(&document());
    let mut variants = Vec::new();
    let mut missing = original.clone();
    missing["data"]["tracks"][0]["clips"][0].as_object_mut().unwrap().remove("pipeline");
    variants.push(missing);
    let mut null = original.clone();
    null["data"]["tracks"][0]["clips"][0]["pipeline"] = serde_json::Value::Null;
    variants.push(null);
    let mut invalid = original.clone();
    invalid["data"]["tracks"][0]["clips"][0]["pipeline"]["graph"]["edges"][0]["sourceHandle"] = "missing".into();
    variants.push(invalid);
    let mut conflicting = original.clone();
    conflicting["schema_version"] = serde_json::json!(1);
    variants.push(conflicting);
    let mut not_legacy = original.clone();
    not_legacy["schema_version"] = serde_json::json!(1);
    not_legacy["data"]["schema_version"] = serde_json::json!(1);
    variants.push(not_legacy);
    let mut unknown = original.clone();
    unknown["data"]["tracks"][0]["clips"][0]["future_effects"] = serde_json::json!([]);
    variants.push(unknown);
    let mut unsupported = original;
    unsupported["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
    unsupported["data"]["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
    variants.push(unsupported);
    for value in variants {
        write_project(&path, &value);
        assert!(EditorDocument::open(path.clone()).is_err(), "accepted {value}");
    }
}

#[test]
fn pipeline_shader_uniform_and_layout_survive_save_reopen_and_saved_state_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipeline.oqvideo.json");
    let mut document = document_with_media(dir.path());
    let mut pipeline = document.clip("clip-1").unwrap().pipeline.clone();
    let code = INVERT.replace("color.a);", "color.a) * gain;");
    let shader = pipeline.add_serial(SOURCE_ID, &code, "Invert with gain").unwrap();
    pipeline.apply_command(GraphCommand::SetNodePosition {
        node_id: NodeId::new(&shader), position: Position { x: 123.0, y: 456.0 },
    }).unwrap();
    let mut node = pipeline.graph().nodes.iter().find(|node| node.id == shader).unwrap().clone();
    node.data.uniforms.insert("gain".into(), serde_json::json!(0.75));
    pipeline.apply_command(GraphCommand::UpdateNode { node }).unwrap();
    document.set_clip_pipeline("clip-1", pipeline.clone(), document.revision).unwrap();
    document.save_as(path.clone()).unwrap();
    assert!(!document.is_dirty());
    let reopened = EditorDocument::open(path).unwrap();
    assert_eq!(reopened.data, document.data);
    assert_eq!(reopened.clip("clip-1").unwrap().pipeline, pipeline);
    assert!(!reopened.is_dirty());
    assert!(document.undo());
    assert!(document.is_dirty());
    assert!(document.redo());
    assert!(!document.is_dirty());
}
