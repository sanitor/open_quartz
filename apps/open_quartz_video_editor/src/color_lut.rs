use std::fmt::Write;
use std::io::Read;
use std::path::Path;

pub const TEMPLATE: &str = "oq-clip-cube-v1";
const MAX_FILE_BYTES: u64 = 6 * 1024 * 1024;

pub struct Cube {
    size: usize,
    minimum: [f32; 3],
    maximum: [f32; 3],
    samples: Vec<[f32; 3]>,
}

impl Cube {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut text = String::new();
        let file = std::fs::File::open(path).map_err(|error| format!("Cannot open LUT: {error}"))?;
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_FILE_BYTES {
            return Err("CUBE file exceeds 6 MiB".into());
        }
        file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)
            .map_err(|error| format!("Cannot read LUT: {error}"))?;
        if text.len() as u64 > MAX_FILE_BYTES { return Err("CUBE file exceeds 6 MiB".into()); }
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut size: Option<usize> = None;
        let mut minimum = None;
        let mut maximum = None;
        let mut samples = Vec::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let mut words = line.split('#').next().unwrap_or_default().split_ascii_whitespace();
            let Some(first) = words.next() else { continue; };
            match first {
                "TITLE" if samples.is_empty() => {}
                "LUT_3D_SIZE" if size.is_none() && samples.is_empty() => {
                    let value = words.next().and_then(|word| word.parse::<usize>().ok())
                        .filter(|size| (2..=33).contains(size))
                        .ok_or("CUBE requires LUT_3D_SIZE from 2 through 33")?;
                    if words.next().is_some() { return Err("Unexpected value on LUT_3D_SIZE line".into()); }
                    size = Some(value);
                    samples.reserve(value.pow(3));
                }
                "DOMAIN_MIN" if minimum.is_none() && samples.is_empty() => minimum = Some(triplet(words)?),
                "DOMAIN_MAX" if maximum.is_none() && samples.is_empty() => maximum = Some(triplet(words)?),
                _ => {
                    let size = size.ok_or("Declare LUT_3D_SIZE before the CUBE samples")?;
                    if samples.len() >= size.pow(3) { return Err("CUBE contains too many samples".into()); }
                    samples.push(triplet(std::iter::once(first).chain(words))?);
                }
            }
        }
        let size = size.ok_or("CUBE has no LUT_3D_SIZE")?;
        let minimum = minimum.unwrap_or([0.0; 3]);
        let maximum = maximum.unwrap_or([1.0; 3]);
        if (0..3).any(|index| maximum[index] - minimum[index] < 0.000001) {
            return Err("CUBE domain bounds must be ordered and separated".into());
        }
        if samples.len() != size.pow(3) { return Err("CUBE sample count does not match LUT_3D_SIZE".into()); }
        Ok(Self { size, minimum, maximum, samples })
    }

    pub fn shader(&self) -> String {
        let mut code = format!(r#"
// OpenQuartz embedded ACEScg CUBE
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var imageSampler: sampler;
const cube: array<vec3f, {}> = array(
"#, self.samples.len());
        for rgb in &self.samples {
            writeln!(code, "vec3f({:.9e}, {:.9e}, {:.9e}),", rgb[0], rgb[1], rgb[2]).unwrap();
        }
        let size = self.size;
        let last = size - 1;
        let [r0, g0, b0] = self.minimum;
        let [r1, g1, b1] = self.maximum;
        write!(code, r#");
fn lookup(p: vec3u) -> vec3f {{ return cube[p.x + {size}u * (p.y + {size}u * p.z)]; }}
@fragment fn main(@location(0) uv: vec2f) -> @location(0) vec4f {{
    let source = textureSample(image, imageSampler, uv);
    let low = vec3f({r0:.9e}, {g0:.9e}, {b0:.9e});
    let high = vec3f({r1:.9e}, {g1:.9e}, {b1:.9e});
    let index = clamp((source.rgb - low) / (high - low), vec3f(0.0), vec3f(1.0)) * {last}.0;
    let a = vec3u(index);
    let b = min(a + vec3u(1u), vec3u({last}u));
    let t = fract(index);
    let near0 = mix(lookup(a), lookup(vec3u(b.x, a.y, a.z)), t.x);
    let near1 = mix(lookup(vec3u(a.x, b.y, a.z)), lookup(vec3u(b.x, b.y, a.z)), t.x);
    let far0 = mix(lookup(vec3u(a.x, a.y, b.z)), lookup(vec3u(b.x, a.y, b.z)), t.x);
    let far1 = mix(lookup(vec3u(a.x, b.y, b.z)), lookup(b), t.x);
    return vec4f(mix(mix(near0, near1, t.y), mix(far0, far1, t.y), t.z), source.a);
}}
"#).unwrap();
        code
    }
}

fn triplet<'a>(mut words: impl Iterator<Item = &'a str>) -> Result<[f32; 3], String> {
    let mut values = [0.0; 3];
    for value in &mut values {
        *value = words.next().and_then(|word| word.parse::<f32>().ok())
            .filter(|value| value.is_finite() && value.abs() <= 65504.0)
            .ok_or("Invalid CUBE numeric triple")?;
    }
    if words.next().is_some() { return Err("CUBE samples and domains must have exactly three values".into()); }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_checks_domain_count_and_numbers_and_generates_valid_wgsl() {
        let row = "0 0 0\n";
        let fixture = format!("TITLE \"Test\"\nLUT_3D_SIZE 2\nDOMAIN_MAX 2 2 2\n{}", row.repeat(8));
        let cube = Cube::parse(&fixture).unwrap();
        let code = cube.shader();
        let module = naga::front::wgsl::parse_str(&code).unwrap_or_else(|error| panic!("{}", error.emit_to_string(&code)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module).unwrap();
        assert!(Cube::parse(&format!("{fixture}{row}")).is_err());
        assert!(Cube::parse(&fixture.replace("2 2 2", "0 2 2")).is_err());
        assert!(Cube::parse(&fixture.replace("0 0 0", "NaN 0 0")).is_err());
        assert!(Cube::parse(&fixture.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE 64")).is_err());
    }
}
