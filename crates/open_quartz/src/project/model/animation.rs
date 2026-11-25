use serde::{Deserialize, Serialize};

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


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UniformAnimationKey {
    pub time: f64,
    pub value: f32,
    pub curve: Curve,
}

pub(crate) fn sample_uniform(keys: &[UniformAnimationKey], time: f64) -> Option<f32> {
    let right = keys.partition_point(|key| key.time <= time);
    match (right.checked_sub(1).and_then(|index| keys.get(index)), keys.get(right)) {
        (Some(left), Some(right)) => {
            let t = left.curve.apply((time - left.time) / (right.time - left.time));
            Some((f64::from(left.value) + (f64::from(right.value) - f64::from(left.value)) * t) as f32)
        }
        (Some(key), None) | (None, Some(key)) => Some(key.value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f64, value: f32, curve: Curve) -> UniformAnimationKey {
        UniformAnimationKey { time, value, curve }
    }

    #[test]
    fn uniforms_clamp_to_keys_and_use_the_outgoing_curve() {
        assert_eq!(sample_uniform(&[], 0.0), None);
        assert_eq!(sample_uniform(&[key(1.0, 6.0, Curve::Hold)], -5.0), Some(6.0));
        let mut keys = [key(1.0, 2.0, Curve::Linear), key(3.0, 10.0, Curve::Hold)];
        assert_eq!(sample_uniform(&keys, 0.0), Some(2.0));
        assert_eq!(sample_uniform(&keys, 1.5), Some(4.0));
        keys[0].curve = Curve::Smooth;
        assert_eq!(sample_uniform(&keys, 1.5), Some(3.25));
        keys[0].curve = Curve::Hold;
        assert_eq!(sample_uniform(&keys, 2.99), Some(2.0));
        assert_eq!(sample_uniform(&keys, 3.0), Some(10.0));
        assert_eq!(sample_uniform(&keys, 4.0), Some(10.0));
        keys[0].curve = Curve::Bezier { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };
        assert!((sample_uniform(&keys, 1.5).unwrap() - 4.0).abs() < 0.001);
    }
}
