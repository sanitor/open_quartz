use std::borrow::Cow;

const MARKER: &str = "// OpenQuartz embedded ACEScg CUBE\n";
const DECLARATION: &str = "const cube: array<vec3f, ";

struct EmbeddedCube<'a> {
    header: &'a str,
    samples: &'a str,
    suffix: &'a str,
    count: usize,
}

impl<'a> EmbeddedCube<'a> {
    fn parse(source: &'a str) -> Option<Self> {
        let (_, tagged) = source.split_once(MARKER)?;
        let (_, declaration) = tagged.split_once(DECLARATION)?;
        let header = &source[..source.len() - declaration.len() - DECLARATION.len()];
        let (count, samples) = declaration.split_once("> = array(\n")?;
        let count = count
            .parse::<usize>()
            .ok()
            .filter(|n| (8..=33_usize.pow(3)).contains(n))?;
        let (samples, suffix) = samples.split_once(");\n")?;
        Some(Self {
            header,
            samples,
            suffix,
            count,
        })
    }

    fn source(&self, binding: &str, space: &str) -> String {
        format!(
            "{}{binding}var<{space}> cube: array<vec3f, {}>;\n{}",
            self.header, self.count, self.suffix
        )
    }

    fn bytes(&self) -> Option<Vec<u8>> {
        let mut bytes = Vec::with_capacity(self.count * 16);
        for line in self.samples.lines().take(self.count + 1) {
            let rgb = line.strip_prefix("vec3f(")?.strip_suffix("),")?;
            let mut channels = rgb.split(',');
            for _ in 0..3 {
                let channel = channels
                    .next()?
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|value| value.is_finite())?;
                bytes.extend_from_slice(&channel.to_le_bytes());
            }
            if channels.next().is_some() {
                return None;
            }
            bytes.extend_from_slice(&[0; 4]);
        }
        (bytes.len() == self.count * 16).then_some(bytes)
    }
}

/// Inspect CUBE shader ports and instructions without building literal-array
/// expressions for every sample. DX12 binds the checked payload at render time.
pub fn shader_validation_source(source: &str) -> Cow<'_, str> {
    if cfg!(windows) {
        if let Some(cube) = EmbeddedCube::parse(source) {
            if cube.bytes().is_some() {
                return Cow::Owned(cube.source("", "private"));
            }
        }
    }
    Cow::Borrowed(source)
}

#[cfg(feature = "gpu")]
pub(crate) fn embedded_cube_buffer(source: &str, binding: u32) -> Option<(String, Vec<u8>)> {
    let cube = EmbeddedCube::parse(source)?;
    let bytes = cube.bytes()?;
    let qualifier = format!("@group(0) @binding({binding}) ");
    Some((cube.source(&qualifier, "storage, read"), bytes))
}
