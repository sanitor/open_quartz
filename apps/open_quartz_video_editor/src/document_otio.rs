use super::audio::{AudioClip, AudioPlacement, AudioTrack, SAMPLE_RATE};
use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;

const MAX_BYTES: u64 = 32 * 1024 * 1024;

fn rate(fps: Rational) -> f64 {
    f64::from(fps.numerator) / f64::from(fps.denominator)
}

fn time(value: f64, rate: f64) -> Value {
    json!({"OTIO_SCHEMA":"RationalTime.1", "value":value, "rate":rate})
}

fn range(start: Value, duration: Value) -> Value {
    json!({"OTIO_SCHEMA":"TimeRange.1", "start_time":start, "duration":duration})
}

fn gap(duration: u64, rate: f64) -> Value {
    json!({"OTIO_SCHEMA":"Gap.1", "name":"", "metadata":{}, "effects":[], "markers":[],
        "source_range":range(time(0.0,rate), time(duration as f64,rate))})
}

fn file_url(asset: &MediaAsset) -> Result<String, String> {
    #[cfg(not(windows))]
    let path = Path::new(&asset.path);
    #[cfg(windows)]
    let path = Path::new(asset.path.strip_prefix(r"\\?\").unwrap_or(&asset.path));
    url::Url::from_file_path(path)
        .map(|url| url.to_string())
        .map_err(|_| format!("Cannot encode local asset path: {}", asset.name))
}

fn clip(asset: &MediaAsset, source_range: Value, metadata: Value) -> Result<Value, String> {
    let reference = json!({"OTIO_SCHEMA":"ExternalReference.1", "name":asset.name,
        "target_url":file_url(asset)?, "available_range":null,
        "metadata":{"openquartz_asset":asset}});
    Ok(
        json!({"OTIO_SCHEMA":"Clip.2", "name":asset.name, "metadata":metadata, "effects":[],
        "markers":[], "enabled":true, "source_range":source_range,
        "media_references":{"DEFAULT_MEDIA":reference}, "active_media_reference_key":"DEFAULT_MEDIA"}),
    )
}

fn track(name: &str, kind: &str, children: Vec<Value>, enabled: bool) -> Value {
    json!({"OTIO_SCHEMA":"Track.1", "name":name, "kind":kind, "metadata":{}, "effects":[],
        "markers":[], "enabled":enabled, "source_range":null, "children":children})
}

pub(crate) fn export(data: &ProjectData, revision: u64, path: &Path) -> Result<String, String> {
    let fps = rate(data.fps);
    let mut tracks = Vec::new();
    let mut losses = BTreeSet::new();
    if !data.sequence_grade.is_identity() || data.tracks.iter().any(|track| !track.grade.is_identity()) {
        losses.insert("track and timeline grades");
    }
    let assets: HashMap<_, _> = data
        .assets
        .iter()
        .map(|asset| (asset.id.as_str(), asset))
        .collect();
    for lane in data.tracks.iter().rev() {
        let mut children = Vec::new();
        let mut end = 0;
        if !lane.dissolves.is_empty() || lane.blend != BlendMode::Normal {
            losses.insert("transitions and track blending");
        }
        for item in &lane.clips {
            if item.title.is_some() {
                losses.insert("titles and subtitles");
                continue;
            }
            let asset = assets
                .get(item.asset_id.as_str())
                .ok_or("OTIO asset is missing")?;
            if asset.sequence_source().is_some() {
                losses.insert("nested sequences");
                continue;
            }
            if item.speed_percent != normal_speed() {
                return Err("OTIO export requires constant 100% speed. Replace retimed clips before exporting.".into());
            }
            if item.pipeline != ClipPipeline::default()
                || item.transform != ClipTransform::default()
                || item.mask.is_some()
                || !item.crop.is_empty()
                || !item.opacity_keys.is_empty()
                || !item.effect_keys.is_empty()
                || !item.crop_keys.is_empty()
                || !item.spatial_keys.is_empty()
            {
                losses.insert("clip effects, grades, masks and transforms");
            }
            if item.timeline_start > end {
                children.push(gap(item.timeline_start - end, fps));
            }
            let ticks = item.source_in_pts as f64
                + item.source_subtick_numerator as f64 / item.source_subtick_denominator as f64;
            let clock = rate(asset.time_base).recip();
            children.push(clip(
                asset,
                range(time(ticks, clock), time(item.duration as f64, fps)),
                json!({"openquartz_clip_id": item.id}),
            )?);
            end = item.timeline_end()?;
        }
        if !children.is_empty() {
            tracks.push(track(&lane.name, "Video", children, lane.visible));
        }
    }
    for lane in &data.audio_tracks {
        let mut children = Vec::new();
        let mut end = 0;
        let audio_rate = SAMPLE_RATE as f64;
        if lane.gain != 1.0 || lane.solo {
            losses.insert("audio mixing");
        }
        let mut timed: Vec<_> = lane
            .clips
            .iter()
            .map(|item| Ok((item.interval(data)?, item)))
            .collect::<Result<_, String>>()?;
        timed.sort_by_key(|(interval, _)| interval.start);
        for (interval, item) in timed {
            if item.gain != 1.0
                || item.fade_in != 0
                || item.fade_out != 0
                || lane.crossfade_samples(&item.id) != 0
            {
                losses.insert("audio mixing");
            }
            if interval.speed_percent != normal_speed() {
                return Err("OTIO export requires 100% audio speed".into());
            }
            if interval.start < end {
                return Err(
                    "OTIO export requires audio clips in timeline order without overlaps".into(),
                );
            }
            if interval.start > end {
                children.push(gap(interval.start - end, audio_rate));
            }
            let asset = assets
                .get(item.asset_id.as_str())
                .ok_or("OTIO audio asset is missing")?;
            let link = match &item.placement {
                AudioPlacement::Linked { video_id } => Some(video_id.as_str()),
                AudioPlacement::Independent { .. } => None,
            };
            children.push(clip(
                asset,
                range(
                    time(interval.source as f64, audio_rate),
                    time((interval.end - interval.start) as f64, audio_rate),
                ),
                json!({"openquartz_link":link}),
            )?);
            end = interval.end;
        }
        if !children.is_empty() {
            tracks.push(track(&lane.name, "Audio", children, !lane.muted));
        }
    }
    let markers: Vec<_> = data
        .markers
        .iter()
        .map(|marker| {
            json!({"OTIO_SCHEMA":"Marker.2", "name":marker.name, "color":"RED", "metadata":{},
            "marked_range":range(time(marker.frame as f64,fps), time(0.0,fps))})
        })
        .collect();
    let timeline = json!({"OTIO_SCHEMA":"Timeline.1","name":data.sequence_name,"global_start_time":time(0.0,fps),
        "metadata":{"openquartz":{"fps":data.fps,"width":data.width,"height":data.height}},
        "tracks":{"OTIO_SCHEMA":"Stack.1","name":"Tracks","source_range":null,"children":tracks,
            "metadata":{},"effects":[],"markers":markers,"enabled":true}});
    let bytes = serde_json::to_vec_pretty(&timeline).map_err(|err| err.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("OTIO timeline exceeds the 32 MiB exchange limit".into());
    }
    write_project_atomic(path, &bytes, revision, false)?;
    Ok(if losses.is_empty() {
        "OTIO timeline exported".into()
    } else {
        format!(
            "OTIO timeline exported · Omits {}",
            losses.into_iter().collect::<Vec<_>>().join(", ")
        )
    })
}

fn values<'a>(node: &'a Value, field: &str) -> Result<&'a [Value], String> {
    node[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("OTIO {field} array is missing"))
}

fn seconds(node: &Value) -> Result<f64, String> {
    let rate = node["rate"]
        .as_f64()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .ok_or("OTIO time rate is invalid")?;
    node["value"]
        .as_f64()
        .map(|value| value / rate)
        .filter(|value| value.is_finite())
        .ok_or_else(|| "OTIO time value is invalid".into())
}

fn units(value: f64, rate: f64) -> Result<u64, String> {
    let value = (value * rate).round();
    if !value.is_finite() || value < 0.0 || value >= u64::MAX as f64 {
        return Err("OTIO timeline time exceeds supported range".into());
    }
    Ok(value as u64)
}

fn reference(node: &Value) -> Result<&Value, String> {
    let r = match node["OTIO_SCHEMA"].as_str() {
        Some("Clip.1") => &node["media_reference"],
        Some("Clip.2") => {
            &node["media_references"][node["active_media_reference_key"]
                .as_str()
                .unwrap_or("DEFAULT_MEDIA")]
        }
        _ => {
            return Err(
                "OTIO contains an unsupported item. Only clips and gaps can be imported.".into(),
            )
        }
    };
    if r["OTIO_SCHEMA"] != "ExternalReference.1" {
        return Err("OTIO clip must reference a local media file".into());
    }
    Ok(r)
}

fn load_asset(
    reference: &Value,
    parent: &Path,
    data: &mut ProjectData,
    seen: &mut HashMap<PathBuf, usize>,
) -> Result<usize, String> {
    let target = reference["target_url"]
        .as_str()
        .ok_or("OTIO media path is missing")?;
    let path = match url::Url::parse(target) {
        Ok(url) if url.scheme() == "file" => {
            url.to_file_path().map_err(|_| "Invalid OTIO file URL")?
        }
        _ if Path::new(target).is_absolute() => PathBuf::from(target),
        Err(_) => parent.join(target),
        _ => return Err("Only local OTIO media paths are supported".into()),
    };
    if let Some(index) = seen.get(&path) {
        return Ok(*index);
    }
    let recorded =
        serde_json::from_value::<MediaAsset>(reference["metadata"]["openquartz_asset"].clone())
            .ok();
    let asset = if path.is_file() {
        let probed = crate::media::probe(&path)?;
        if recorded
            .as_ref()
            .is_some_and(|asset| asset.fingerprint != probed.fingerprint)
        {
            return Err(format!(
                "OTIO media fingerprint mismatch: {}",
                path.display()
            ));
        }
        probed
    } else {
        let mut asset =
            recorded.ok_or_else(|| format!("OTIO media is missing: {}", path.display()))?;
        asset.path = path.to_string_lossy().into_owned();
        asset.online = false;
        asset
    };
    let index = data.assets.len();
    let mut asset = asset;
    asset.id = format!("otio-asset-{index}");
    data.assets.push(asset);
    seen.insert(path, index);
    Ok(index)
}

pub(crate) fn import(path: &Path) -> Result<(EditorDocument, String), String> {
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|err| format!("Cannot read OTIO timeline: {err}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("OTIO timeline exceeds 32 MiB".into());
    }
    let root: Value =
        serde_json::from_slice(&bytes).map_err(|err| format!("Invalid OTIO JSON: {err}"))?;
    if root["OTIO_SCHEMA"] != "Timeline.1" || root["tracks"]["OTIO_SCHEMA"] != "Stack.1" {
        return Err("Expected an OTIO Timeline.1 and Stack.1".into());
    }
    let meta = &root["metadata"]["openquartz"];
    let mut data = ProjectData::default();
    if let Some(name) = root["name"].as_str().filter(|name| !name.trim().is_empty()) {
        data.sequence_name = name.chars().take(64).collect();
        data.name = data.sequence_name.clone();
    }
    let raw_rate = root["global_start_time"]["rate"].as_f64().unwrap_or(30.0);
    data.fps = serde_json::from_value(meta["fps"].clone()).unwrap_or_else(|_| {
        [24_000, 30_000, 60_000, 120_000]
            .into_iter()
            .find(|&n| (raw_rate - f64::from(n) / 1001.0).abs() < 0.0001)
            .map_or(
                Rational {
                    numerator: raw_rate.round() as i32,
                    denominator: 1,
                },
                |numerator| Rational {
                    numerator,
                    denominator: 1001,
                },
            )
    });
    Rational::new(data.fps.numerator, data.fps.denominator)?;
    data.width = meta["width"]
        .as_u64()
        .and_then(|value| value.try_into().ok())
        .unwrap_or(data.width);
    data.height = meta["height"]
        .as_u64()
        .and_then(|value| value.try_into().ok())
        .unwrap_or(data.height);
    data.tracks.clear();
    let parent = path.parent().ok_or("OTIO path must be absolute")?;
    let mut seen = HashMap::new();
    let mut links = HashMap::new();
    let mut losses = false;
    let tracks = values(&root["tracks"], "children")?;
    if !root["tracks"]["source_range"].is_null()
        || root["tracks"]["enabled"] == false
        || tracks
            .iter()
            .any(|lane| lane["kind"] != "Video" && lane["kind"] != "Audio")
    {
        return Err("OTIO requires an enabled, untrimmed stack of video and audio tracks".into());
    }
    for kind in ["Video", "Audio"] {
        for (index, lane) in tracks
            .iter()
            .enumerate()
            .filter(|(_, lane)| lane["kind"] == kind)
        {
            if lane["OTIO_SCHEMA"] != "Track.1" || !lane["source_range"].is_null() {
                return Err("Trimmed or nested OTIO tracks are unsupported".into());
            }
            let name = lane["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(kind)
                .to_owned();
            let enabled = lane["enabled"].as_bool().unwrap_or(true);
            let mut video = VideoTrack::new(format!("otio-track-{index}"), name.clone());
            video.visible = enabled;
            let mut audio = AudioTrack {
                id: format!("otio-audio-{index}"),
                name,
                muted: !enabled,
                solo: false,
                locked: false,
                sync_ripple: false,
                gain: 1.0,
                clips: Vec::new(),
                crossfades: Vec::new(),
            };
            let clock = if kind == "Video" {
                rate(data.fps)
            } else {
                SAMPLE_RATE as f64
            };
            let mut cursor = 0.0;
            for (item_index, item) in values(lane, "children")?.iter().enumerate() {
                let length = seconds(&item["source_range"]["duration"])?;
                if length < 0.0 {
                    return Err("Negative OTIO clip duration".into());
                }
                let start = units(cursor, clock)?;
                cursor += length;
                let duration = units(cursor, clock)? - start;
                if item["OTIO_SCHEMA"] == "Gap.1" || item["enabled"] == false {
                    continue;
                }
                if duration == 0 {
                    return Err("OTIO clip spans no timeline samples".into());
                }
                losses |= item["effects"]
                    .as_array()
                    .is_some_and(|effects| !effects.is_empty());
                let src = seconds(&item["source_range"]["start_time"])?;
                let reference = reference(item)?;
                let asset = load_asset(reference, parent, &mut data, &mut seen)?;
                let asset = &data.assets[asset];
                let id = format!("otio-clip-{index}-{item_index}");
                if kind == "Video" {
                    let ticks = src / rate(asset.time_base);
                    let whole = signed_time(ticks.floor())?;
                    let fraction = ((ticks - ticks.floor()) * 1_000_000.0).round() as u64;
                    let mut video_clip = Clip {
                        id: id.clone(),
                        asset_id: asset.id.clone(),
                        title: None,
                        timeline_start: start,
                        duration,
                        speed_percent: normal_speed(),
                        source_span_remainder: 0,
                        source_in_pts: whole
                            .checked_add((fraction / 1_000_000) as i64)
                            .ok_or("OTIO source time overflow")?,
                        source_subtick_numerator: fraction % 1_000_000,
                        source_subtick_denominator: 1_000_000,
                        transform: ClipTransform::default(),
                        opacity_keys: Vec::new(),
                        mask: None,
                        mask_tracking: Vec::new(),
                        crop: Crop::default(),
                        crop_keys: Vec::new(),
                        spatial_keys: Vec::new(),
                        pipeline: ClipPipeline::default(),
                        effect_keys: Vec::new(),
                        grade_versions: None,
                    };
                    if asset.still {
                        video_clip.source_in_pts = 0;
                        video_clip.source_subtick_numerator = 0;
                    }
                    video.clips.push(video_clip);
                    if let Some(key) = item["metadata"]["openquartz_clip_id"].as_str() {
                        links.insert(key.to_owned(), id);
                    }
                } else {
                    let source = signed_time((src * clock).round())?;
                    let placement = item["metadata"]["openquartz_link"]
                        .as_str()
                        .and_then(|key| links.get(key))
                        .and_then(|video_id| {
                            let candidate = AudioPlacement::Linked {
                                video_id: video_id.clone(),
                            };
                            let check = AudioClip {
                                id: id.clone(),
                                asset_id: asset.id.clone(),
                                gain: 1.0,
                                fade_in: 0,
                                fade_out: 0,
                                speed_percent: normal_speed(),
                                placement: candidate.clone(),
                            };
                            check
                                .interval(&data)
                                .ok()
                                .filter(|range| {
                                    range.start == start
                                        && range.end - start == duration
                                        && range.source == source
                                })
                                .map(|_| candidate)
                        })
                        .unwrap_or(AudioPlacement::Independent {
                            start,
                            duration,
                            source,
                        });
                    audio.clips.push(AudioClip {
                        id,
                        asset_id: asset.id.clone(),
                        gain: 1.0,
                        fade_in: 0,
                        fade_out: 0,
                        speed_percent: normal_speed(),
                        placement,
                    });
                }
            }
            if kind == "Video" {
                data.tracks.insert(0, video);
            } else {
                data.audio_tracks.push(audio);
            }
        }
    }
    if data.tracks.is_empty() {
        data.tracks
            .push(VideoTrack::new(DEFAULT_TRACK.into(), "Video 1".into()));
    }
    if let Some(markers) = root["tracks"]["markers"].as_array() {
        for (index, marker) in markers.iter().enumerate() {
            let name = marker["name"]
                .as_str()
                .unwrap_or("Marker")
                .chars()
                .take(64)
                .collect();
            let frame = units(
                seconds(&marker["marked_range"]["start_time"])?,
                rate(data.fps),
            )?;
            data.markers.push(Marker {
                id: format!("otio-marker-{index}"),
                name,
                frame,
            });
        }
        data.markers.sort_by_key(|marker| marker.frame);
        data.markers.dedup_by_key(|marker| marker.frame);
    }
    validate(&data)?;
    let mut doc = EditorDocument::new(data)?;
    doc.saved_data = ProjectData::default();
    Ok((
        doc,
        if losses {
            "OTIO timeline imported · Omits unsupported clip effects"
        } else {
            "OTIO timeline imported"
        }
        .into(),
    ))
}

fn signed_time(value: f64) -> Result<i64, String> {
    if !value.is_finite() || value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return Err("OTIO source time exceeds supported range".into());
    }
    Ok(value as i64)
}
