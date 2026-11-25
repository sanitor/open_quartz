use serde::{Deserialize, Serialize};

pub const MAX_KEYS: usize = 256;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Curve {
    #[default]
    Linear,
    Smooth,
    Hold,
    Bezier { x1: f32, y1: f32, x2: f32, y2: f32 },
}

impl Curve {
    pub fn is_linear(&self) -> bool {
        *self == Self::Linear
    }

    pub fn apply(self, unit: f64) -> f64 {
        match self {
            Self::Linear => unit,
            Self::Smooth => unit * unit * (3.0 - 2.0 * unit),
            Self::Hold => 0.0,
            Self::Bezier { x1, y1, x2, y2 } => {
                if unit <= 0.0 || unit >= 1.0 { return unit; }
                let (mut low, mut high) = (0.0, 1.0);
                for _ in 0..16 {
                    let middle = (low + high) / 2.0;
                    if cubic(middle, f64::from(x1), f64::from(x2)) < unit { low = middle; }
                    else { high = middle; }
                }
                cubic((low + high) / 2.0, f64::from(y1), f64::from(y2))
            }
        }
    }

    pub fn wgsl(self) -> String {
        match self {
            Self::Linear => "t".into(),
            Self::Smooth => "t * t * (3.0 - 2.0 * t)".into(),
            Self::Hold => "0.0".into(),
            Self::Bezier { x1, y1, x2, y2 } => format!(
                "bezier_progress(t, vec4f({x1:.9e}, {y1:.9e}, {x2:.9e}, {y2:.9e}))"
            ),
        }
    }

    pub fn valid(self) -> bool {
        match self {
            Self::Bezier { x1, y1, x2, y2 } =>
                x1 <= x2 && [x1, y1, x2, y2].into_iter().all(|value| (0.0..=1.0).contains(&value)),
            _ => true,
        }
    }
}

fn cubic(t: f64, first: f64, second: f64) -> f64 {
    let remainder = 1.0 - t;
    3.0 * remainder * remainder * t * first + 3.0 * remainder * t * t * second + t * t * t
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe<Time, Value> {
    pub time: Time,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Curve::is_linear")]
    pub curve: Curve,
}

pub fn sample<Time: Copy + Into<i128>, Value: Copy>(
    keys: &[Keyframe<Time, Value>],
    time: i128,
    fallback: Value,
    mix: impl Fn(Value, Value, f64) -> Value,
) -> Value {
    let right = keys.partition_point(|key| key.time.into() <= time);
    match (
        right.checked_sub(1).and_then(|index| keys.get(index)),
        keys.get(right),
    ) {
        (Some(left), Some(right)) => {
            let position = time - left.time.into();
            let duration = right.time.into() - left.time.into();
            mix(
                left.value,
                right.value,
                left.curve.apply(position as f64 / duration as f64),
            )
        }
        (Some(key), None) | (None, Some(key)) => key.value,
        (None, None) => fallback,
    }
}

pub fn mix_scalar(left: f32, right: f32, t: f64) -> f32 {
    (f64::from(left) + (f64::from(right) - f64::from(left)) * t) as f32
}

pub fn set_key<Time: Copy + Ord, Value>(
    keys: &mut Vec<Keyframe<Time, Value>>,
    time: Time,
    value: Value,
) -> Result<(), String> {
    match keys.binary_search_by_key(&time, |key| key.time) {
        Ok(index) => keys[index].value = value,
        Err(index) if keys.len() < MAX_KEYS => {
            keys.insert(
                index,
                Keyframe {
                    time,
                    value,
                    curve: Curve::Linear,
                },
            );
        }
        Err(_) => return Err(format!("A property supports up to {MAX_KEYS} keyframes")),
    }
    Ok(())
}

pub fn validate<Time: Ord, Value>(
    keys: &[Keyframe<Time, Value>],
    valid: impl Fn(&Value) -> bool,
) -> bool {
    keys.len() <= MAX_KEYS
        && keys.iter().all(|key| valid(&key.value) && key.curve.valid())
        && keys.windows(2).all(|pair| pair[0].time < pair[1].time)
}
