//! RGB interpretation and GPU plane shaders for hardware delivery.
use super::TextureFormat;
use crate::outputs::{FrameAlphaMode, FrameColorSpace};
#[cfg(windows)]
use oq_media::d3d12_encoder::Profile as EncoderProfile;
#[cfg(target_os = "macos")]
use oq_media::EncoderProfile;

/// Explicit host declaration, not a guess made from untagged shader output.
/// RGB is sRGB encoded with BT.709 primaries; output is limited-range BT.709.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputInterpretation {
    /// Straight alpha; decode sRGB, composite over black in linear light.
    SdrSrgbRgba8Black,
    /// Encoded RGB is premultiplied; unpremultiply before decoding sRGB.
    SdrSrgbRgba8PremultipliedBlack,
    SdrSrgbRgba8OpaqueBlack,
    Hdr10AcesCgRgba16Float,
    HlgAcesCgRgba16Float,
}
impl InputInterpretation {
    pub fn texture_format(self) -> TextureFormat {
        match self {
            Self::Hdr10AcesCgRgba16Float | Self::HlgAcesCgRgba16Float => TextureFormat::Rgba16Float,
            _ => TextureFormat::Rgba8Unorm,
        }
    }

    pub fn encoder_profile(self) -> EncoderProfile {
        match self {
            Self::Hdr10AcesCgRgba16Float => EncoderProfile::Hdr10,
            Self::HlgAcesCgRgba16Float => EncoderProfile::Hlg,
            _ => EncoderProfile::Sdr,
        }
    }

    pub(super) fn color_space(self) -> FrameColorSpace {
        match self {
            Self::Hdr10AcesCgRgba16Float | Self::HlgAcesCgRgba16Float => {
                FrameColorSpace::AcesCgLinear
            }
            _ => FrameColorSpace::Srgb,
        }
    }

    pub(super) fn alpha(self) -> FrameAlphaMode {
        match self {
            Self::SdrSrgbRgba8Black | Self::Hdr10AcesCgRgba16Float | Self::HlgAcesCgRgba16Float => {
                FrameAlphaMode::Straight
            }
            Self::SdrSrgbRgba8PremultipliedBlack => FrameAlphaMode::Premultiplied,
            Self::SdrSrgbRgba8OpaqueBlack => FrameAlphaMode::Opaque,
        }
    }
    fn shader_alpha(self) -> u32 {
        match self {
            Self::SdrSrgbRgba8Black | Self::Hdr10AcesCgRgba16Float | Self::HlgAcesCgRgba16Float => {
                0
            }
            Self::SdrSrgbRgba8PremultipliedBlack => 1,
            Self::SdrSrgbRgba8OpaqueBlack => 2,
        }
    }
}

pub(super) fn plane_formats(profile: EncoderProfile) -> [wgpu::TextureFormat; 2] {
    use wgpu::TextureFormat::*;
    match profile {
        EncoderProfile::Sdr => [R8Unorm, Rg8Unorm],
        EncoderProfile::Hdr10 | EncoderProfile::Hlg => [R16Unorm, Rg16Unorm],
    }
}

pub(super) fn create_pipelines(
    device: &wgpu::Device,
    interpretation: InputInterpretation,
) -> (
    InputInterpretation,
    wgpu::BindGroupLayout,
    [wgpu::RenderPipeline; 2],
) {
    let profile = interpretation.encoder_profile();
    let source = SHADER
        .replace("ALPHA_MODE", &interpretation.shader_alpha().to_string())
        .replace("HDR_MODE", if profile.is_hdr() { "true" } else { "false" })
        .replace(
            "HLG_MODE",
            if profile == EncoderProfile::Hlg {
                "true"
            } else {
                "false"
            },
        );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("encoder-nv12-srgb-to-709"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("encoder-nv12-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("encoder-nv12-pipeline-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let [y, uv] = plane_formats(profile);
    let pipelines = [("fs_y", y), ("fs_uv", uv)].map(|(entry, format)| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("encoder-nv12-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
    });
    (interpretation, layout, pipelines)
}

const SHADER: &str = r#"
@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    return vec4f(f32(i / 2u) * 4.0 - 1.0, f32(i % 2u) * 4.0 - 1.0, 0.0, 1.0);
}
fn pq_encode(nits: vec3f) -> vec3f {
    let v = pow(clamp(nits, vec3f(0.0), vec3f(10000.0)) / 10000.0, vec3f(2610.0 / 16384.0));
    return pow((0.8359375 + 18.8515625 * v) / (1.0 + 18.6875 * v), vec3f(2523.0 / 32.0));
}
fn hlg_encode(display: vec3f) -> vec3f {
    let luminance = dot(display, vec3f(0.2627, 0.6780, 0.0593));
    let scene = clamp(display * pow(max(luminance, 0.00000001), (1.0 - 1.2) / 1.2), vec3f(0.0), vec3f(1.0));
    let low = sqrt(3.0 * scene);
    let high = 0.17883277 * log(max(12.0 * scene - 0.28466892, vec3f(0.00000001))) + 0.55991073;
    return select(low, high, scene > vec3f(1.0 / 12.0));
}
fn rgb_signal(p: vec2i) -> vec3f {
    let sample = textureLoad(source, p, 0);
    if HDR_MODE {
        let c = sample.rgb * sample.a;
        let rgb2020 = vec3f(dot(c, vec3f(1.0258247, -0.0200532, -0.0057715)),
            dot(c, vec3f(-0.0022344, 1.0045865, -0.0023521)),
            dot(c, vec3f(-0.0050134, -0.0252901, 1.0303035)));
        if HLG_MODE { return hlg_encode(clamp(rgb2020 * (203.0 / 1000.0), vec3f(0.0), vec3f(1.0))); }
        return pq_encode(min(rgb2020 * 203.0, vec3f(1000.0)));
    }
    var encoded = sample.rgb;
    var alpha = sample.a;
    if (ALPHA_MODE == 1u) {
        encoded = select(vec3f(0.0), clamp(encoded / max(alpha, 0.000001), vec3f(0.0), vec3f(1.0)), alpha > 0.0);
    }
    if (ALPHA_MODE == 2u) { alpha = 1.0; }
    let linear = select(pow((encoded + vec3f(0.055)) / 1.055, vec3f(2.4)), encoded / 12.92, encoded <= vec3f(0.04045)) * alpha;
    return select(1.099 * pow(linear, vec3f(0.45)) - vec3f(0.099), 4.5 * linear, linear < vec3f(0.018));
}
fn ycbcr(rgb: vec3f) -> vec3f {
    if HDR_MODE {
        let y = dot(rgb, vec3f(0.2627, 0.6780, 0.0593));
        return vec3f(y, (rgb.b - y) / 1.8814, (rgb.r - y) / 1.4746);
    }
    let y = dot(rgb, vec3f(0.2126, 0.7152, 0.0722));
    return vec3f(y, (rgb.b - y) / 1.8556, (rgb.r - y) / 1.5748);
}
@fragment fn fs_y(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let y = ycbcr(rgb_signal(vec2i(p.xy))).x;
    if HDR_MODE { return vec4f(round(clamp(64.0 + 876.0 * y, 64.0, 940.0)) * (64.0 / 65535.0), 0.0, 0.0, 1.0); }
    return vec4f(floor(clamp(16.0 + 219.0 * y, 16.0, 235.0) + 0.5) / 255.0, 0.0, 0.0, 1.0);
}
@fragment fn fs_uv(@builtin(position) p: vec4f) -> @location(0) vec4f {
    let q = vec2i(p.xy) * 2;
    let rgb = (rgb_signal(q) + rgb_signal(q + vec2i(1,0)) + rgb_signal(q + vec2i(0,1)) + rgb_signal(q + vec2i(1,1))) * 0.25;
    let uv = ycbcr(rgb).yz;
    if HDR_MODE {
        let codes = round(clamp(vec2f(512.0) + 896.0 * uv, vec2f(64.0), vec2f(960.0)));
        return vec4f(codes * (64.0 / 65535.0), 0.0, 1.0);
    }
    let codes = floor(clamp(vec2f(128.0) + 224.0 * uv, vec2f(16.0), vec2f(240.0)) + vec2f(0.5));
    return vec4f(codes / 255.0, 0.0, 1.0);
}
"#;
