use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MaskShape {
    #[default]
    Rectangle,
    Ellipse,
    Polygon,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Polygon {
    count: usize,
    vertices: [[f32; 2]; 12],
}

impl Polygon {
    fn parse(input: &str) -> Result<Self, String> {
        let mut polygon = Self { count: 0, vertices: [[0.0; 2]; 12] };
        for entry in input.split_whitespace() {
            let (x, y) = entry.split_once(',').ok_or("Enter polygon vertices as x,y pairs")?;
            let parse = |value: &str| value.parse::<f32>().map(|value| value / 100.0)
                .map_err(|_| "Enter polygon vertex coordinates from 0 to 100".to_owned());
            *polygon.vertices.get_mut(polygon.count).ok_or("Use at most 12 polygon vertices")? = [parse(x)?, parse(y)?];
            polygon.count += 1;
        }
        polygon.validate()?;
        Ok(polygon)
    }

    fn validate(&self) -> Result<(), String> {
        if !(3..=12).contains(&self.count)
            || self.points().iter().flatten().any(|value| !(0.0..=1.0).contains(value))
            || self.edges().any(|(a, b)| a == b)
        {
            return Err("Use 3 to 12 distinct neighboring vertices between 0 and 100".into());
        }
        Ok(())
    }

    fn points(&self) -> &[[f32; 2]] { &self.vertices[..self.count] }

    pub fn edges(&self) -> impl Iterator<Item = ([f32; 2], [f32; 2])> + '_ {
        self.points().iter().copied().zip(self.points().iter().copied().cycle().skip(1))
    }

    pub fn text(&self) -> String {
        self.points().iter().map(|[x, y]| format!("{:.3},{:.3}", x * 100.0, y * 100.0))
            .collect::<Vec<_>>().join(" ")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Mask {
    pub shape: MaskShape,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub feather: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polygon: Option<Polygon>,
}

impl Default for Mask {
    fn default() -> Self {
        Self { shape: MaskShape::Rectangle, x: 0.5, y: 0.5, width: 0.75, height: 0.75, feather: 0.0, polygon: None }
    }
}

impl Mask {
    pub(super) fn validate(self) -> Result<(), String> {
        if [self.x, self.y, self.feather].iter().any(|value| !(0.0..=1.0).contains(value))
            || [self.width, self.height].iter().any(|value| !(0.01..=1.0).contains(value))
        {
            return Err("Mask position and feather must be 0–100%; width and height must be 1–100%".into());
        }
        if self.shape == MaskShape::Polygon {
            self.polygon.ok_or("Polygon mask needs vertices")?.validate()?;
        }
        Ok(())
    }
}

pub const MAX_TRACKED_FRAMES: u64 = 240;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MaskPosition {
    pub source_ns: u64,
    pub x: f32,
    pub y: f32,
    #[serde(default = "identity_scale", skip_serializing_if = "is_identity_scale")]
    pub scale: f32,
    #[serde(default, skip_serializing_if = "is_zero_rotation")]
    pub rotation: f32,
}

fn identity_scale() -> f32 { 1.0 }
fn is_identity_scale(value: &f32) -> bool { *value == 1.0 }
fn is_zero_rotation(value: &f32) -> bool { *value == 0.0 }

pub(super) fn validate_tracking(clip: &Clip) -> Result<(), String> {
    let points = &clip.mask_tracking;
    if points.is_empty() { return Ok(()); }
    if clip.mask.is_none() || points.len() == 1 || points.len() as u64 > MAX_TRACKED_FRAMES
        || points.windows(2).any(|pair| pair[0].source_ns >= pair[1].source_ns)
        || points.iter().any(|point| !(0.0..=1.0).contains(&point.x) || !(0.0..=1.0).contains(&point.y)
            || !(0.5..=2.0).contains(&point.scale) || !(-360.0..=360.0).contains(&point.rotation)) {
        return Err("Invalid tracked mask coordinates or source times".into());
    }
    Ok(())
}

impl Clip {
    pub fn mask_center_at(&self, source_ns: u64) -> Option<[f32; 2]> {
        let mask = self.mask?;
        let index = self.mask_tracking.partition_point(|point| point.source_ns <= source_ns);
        match (index.checked_sub(1).and_then(|index| self.mask_tracking.get(index)), self.mask_tracking.get(index)) {
            (Some(left), Some(right)) => {
                let blend = (source_ns - left.source_ns) as f64 / (right.source_ns - left.source_ns) as f64;
                Some([left.x + (right.x - left.x) * blend as f32, left.y + (right.y - left.y) * blend as f32])
            }
            (Some(point), None) | (None, Some(point)) => Some([point.x, point.y]),
            (None, None) => Some([mask.x, mask.y]),
        }
    }
}

impl EditorDocument {
    pub fn correct_mask(&mut self, id: &str, frame: u64, field: &str, input: &str) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &data.tracks[track].clips[index];
            let time = data.clip_source_time(clip, frame)?;
            let first = clip.mask_tracking.first().ok_or("Track the mask before correcting drift")?;
            let last = clip.mask_tracking.last().unwrap();
            if !(first.source_ns..=last.source_ns).contains(&time) {
                return Err("Move the playhead inside the tracked mask range".into());
            }
            let value = input.trim().parse::<f32>().map_err(|_| "Enter a mask percentage")? / 100.0;
            if !(0.0..=1.0).contains(&value) { return Err("Enter a mask percentage from 0 to 100".into()); }
            let [x, y] = clip.mask_center_at(time).ok_or("Choose a mask shape first")?;
            let mut points = clip.mask_tracking.clone();
            let limit = points.len() as u64 >= MAX_TRACKED_FRAMES;
            let mut time = time;
            if let Err(index) = points.binary_search_by_key(&time, |point| point.source_ns) {
                if limit {
                    time = [points[index - 1].source_ns, points[index].source_ns].into_iter()
                        .min_by_key(|point| point.abs_diff(time)).unwrap();
                } else {
                    let left = points[index - 1];
                    let right = points[index];
                    let t = (time - left.source_ns) as f32 / (right.source_ns - left.source_ns) as f32;
                    points.insert(index, MaskPosition { source_ns: time, x, y,
                        scale: left.scale + t * (right.scale - left.scale),
                        rotation: left.rotation + t * (right.rotation - left.rotation) });
                }
            }
            let center = clip.mask_center_at(time).ok_or("Choose a mask shape first")?;
            let delta = value - center[usize::from(field == "y")];
            for point in &mut points {
                let t = (1.0 - point.source_ns.abs_diff(time) as f32 / 400_000_000.0).max(0.0);
                let coordinate = if field == "x" { &mut point.x } else { &mut point.y };
                *coordinate = (*coordinate + delta * t * t * (3.0 - 2.0 * t)).clamp(0.0, 1.0);
            }
            data.tracks[track].clips[index].mask_tracking = points;
            Ok(())
        })
    }

    pub fn set_mask_tracking(&mut self, id: &str, revision: u64, points: Vec<MaskPosition>) -> Result<(), String> {
        self.transact_at(revision, |data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[track].clips[index];
            clip.mask_tracking = points;
            validate_tracking(clip)
        })
    }

    pub fn edit_mask(&mut self, id: &str, field: &str, input: &str) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[track].clips[index];
            let mut mask = clip.mask.unwrap_or_default();
            if field == "shape" {
                mask.shape = match input {
                    "0" => { clip.mask = None; clip.mask_tracking.clear(); return Ok(()); }
                    "1" => MaskShape::Rectangle,
                    "2" => MaskShape::Ellipse,
                    "3" => {
                        if mask.polygon.is_none() { mask.polygon = Some(Polygon::parse("50,0 100,100 0,100")?); }
                        MaskShape::Polygon
                    }
                    _ => return Err("Unknown mask shape".into()),
                };
            } else if field == "polygon" && mask.shape == MaskShape::Polygon {
                mask.polygon = Some(Polygon::parse(input)?);
            } else {
                if clip.mask.is_none() { return Err("Choose a mask shape first".into()); }
                let value = input.trim().parse::<f32>().map_err(|_| "Enter a mask percentage")? / 100.0;
                match field {
                    "x" => { mask.x = value; clip.mask_tracking.clear(); }
                    "y" => { mask.y = value; clip.mask_tracking.clear(); }
                    "width" => mask.width = value,
                    "height" => mask.height = value,
                    "feather" => mask.feather = value,
                    _ => return Err("Unknown mask property".into()),
                }
            }
            mask.validate()?;
            clip.mask = Some(mask);
            Ok(())
        })
    }
}
