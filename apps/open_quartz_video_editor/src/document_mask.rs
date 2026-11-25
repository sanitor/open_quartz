use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MaskShape {
    #[default]
    Rectangle,
    Ellipse,
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
}

impl Default for Mask {
    fn default() -> Self {
        Self { shape: MaskShape::Rectangle, x: 0.5, y: 0.5, width: 0.75, height: 0.75, feather: 0.0 }
    }
}

impl Mask {
    pub(super) fn validate(self) -> Result<(), String> {
        if [self.x, self.y, self.feather].iter().any(|value| !(0.0..=1.0).contains(value))
            || [self.width, self.height].iter().any(|value| !(0.01..=1.0).contains(value))
        {
            return Err("Mask position and feather must be 0–100%; width and height must be 1–100%".into());
        }
        Ok(())
    }
}

impl EditorDocument {
    pub fn edit_mask(&mut self, id: &str, field: &str, input: &str) -> Result<(), String> {
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[track].clips[index];
            let mut mask = clip.mask.unwrap_or_default();
            if field == "shape" {
                mask.shape = match input {
                    "0" => { clip.mask = None; return Ok(()); }
                    "1" => MaskShape::Rectangle,
                    "2" => MaskShape::Ellipse,
                    _ => return Err("Unknown mask shape".into()),
                };
            } else {
                if clip.mask.is_none() { return Err("Choose a mask shape first".into()); }
                let value = input.trim().parse::<f32>().map_err(|_| "Enter a mask percentage")? / 100.0;
                match field {
                    "x" => mask.x = value,
                    "y" => mask.y = value,
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
