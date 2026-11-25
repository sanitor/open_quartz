use crate::operators::CameraLog;
use oq_media::{ColorInfo, ColorMatrix, ColorPrimaries, TransferFunction};

pub(super) struct Conversion {
    format: u8,
    matrix: u8,
    full: bool,
    transfer: u8,
    primaries: u8,
    managed: bool,
    camera: Option<CameraLog>,
}

impl Conversion {
    pub(super) fn new(format: u8, color: ColorInfo, managed: bool, camera: Option<CameraLog>) -> Self {
        let matrix = match color.matrix { ColorMatrix::Bt601 => 0, ColorMatrix::Bt709 => 1, ColorMatrix::Bt2020 => 2 };
        let transfer = match color.transfer {
            TransferFunction::Bt709 => 0,
            TransferFunction::Srgb => 1,
            TransferFunction::Pq => 2,
            TransferFunction::Hlg => 3,
        };
        let primaries = match color.primaries {
            ColorPrimaries::Bt709 => 0,
            ColorPrimaries::Bt470Bg => 1,
            ColorPrimaries::Bt2020 => 2,
        };
        Self { format, matrix, full: color.full_range, transfer, primaries, managed, camera }
    }

    pub(super) fn key(&self) -> u16 {
        u16::from(((self.format * 6 + self.matrix * 2 + u8::from(self.full)) * 4 + self.transfer) * 3 + self.primaries)
            + if self.managed { 216 } else { 0 }
            + self.camera.map_or(0, |profile| profile.index() as u16 * 432)
    }

    pub(super) fn create(&self, device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
        let Self { format, matrix, full, transfer, primaries, managed, camera } = *self;
        create_pipeline(device, format, matrix, full, transfer, primaries, managed, camera)
    }
}

fn create_pipeline(device: &wgpu::Device, format: u8, matrix: u8, full: bool, transfer: u8, primaries: u8, managed: bool, camera: Option<CameraLog>) -> (wgpu::BindGroupLayout, wgpu::RenderPipeline) {
    let (kr, kb) = match matrix { 0 => (0.299f64, 0.114f64), 1 => (0.2126, 0.0722), _ => (0.2627, 0.0593) };
    let kg = 1.0 - kr - kb;
    let (scale, y_offset, y_range, c_mid, c_range) = match (format, full) {
        (1, false) => (65535.0 / 64.0, 64.0, 876.0, 512.0, 896.0),
        (1, true) => (65535.0 / 64.0, 0.0, 1023.0, 512.0, 1023.0),
        (_, false) => (255.0, 16.0, 219.0, 128.0, 224.0),
        _ => (255.0, 0.0, 255.0, 128.0, 255.0),
    };
    let conversion = if format == 2 { "return vec4f(output_color(textureLoad(y_tex,p,0).rgb),1.0);".to_owned() } else { format!("let y = (textureLoad(y_tex,p,0).r * {scale} - {y_offset}) / {y_range}; let chroma = (textureLoad(uv_tex,c,0).rg * {scale} - vec2f({c_mid})) / {c_range}; let rgb = vec3f(y + {rv} * chroma.y, y - {gu} * chroma.x - {gv} * chroma.y, y + {bu} * chroma.x); return vec4f(output_color({channel}),1.0);", rv=2.0*(1.0-kr), gu=2.0*kb*(1.0-kb)/kg, gv=2.0*kr*(1.0-kr)/kg, bu=2.0*(1.0-kb), channel=if camera.is_some() || (managed && transfer < 2) { "rgb" } else { "clamp(rgb, vec3f(0.0), vec3f(1.0))" }) };
    let decode_transfer = match transfer {
        1 => {
        "let linear = select(pow(max((v + vec3f(0.055)) / 1.055, vec3f(0.0)), vec3f(2.4)), v / 12.92, v <= vec3f(0.04045));"
        }
        2 => "let u = pow(v, vec3f(32.0 / 2523.0)); let linear = (10000.0 / 203.0) * pow(max(u - 0.8359375, vec3f(0.0)) / (18.8515625 - 18.6875 * u), vec3f(16384.0 / 2610.0));",
        3 => "let scene = select((exp((v - 0.55991073) / 0.17883277) + 0.28466892) / 12.0, v * v / 3.0, v <= vec3f(0.5)); let linear = (1000.0 / 203.0) * scene * pow(dot(scene, vec3f(0.2627, 0.6780, 0.0593)), 0.2);",
        _ => "let linear = select(pow(max((v + vec3f(0.099)) / 1.099, vec3f(0.0)), vec3f(1.0 / 0.45)), v / 4.5, v < vec3f(0.081));",
    };
    // BT.470BG (PAL) and BT.709 share D65. Convert linear RGB using their
    // chromaticities: PAL R(.64,.33) G(.29,.60) B(.15,.06), 709 G(.30,.60).
    let gamut = match primaries {
        1 => "let rgb709 = vec3f(1.044043209 * linear.r - 0.044043209 * linear.g, linear.g, 0.011793378 * linear.g + 0.988206622 * linear.b);",
        2 => "let rgb709 = vec3f(dot(linear, vec3f(1.660491, -0.587641, -0.072850)), dot(linear, vec3f(-0.124550, 1.132900, -0.008349)), dot(linear, vec3f(-0.018151, -0.100579, 1.118730)));",
        _ => "let rgb709 = linear;",
    };
    let display = if transfer >= 2 {
        "let luminance = max(dot(rgb709, vec3f(0.2126, 0.7152, 0.0722)), 0.0); let mapped = rgb709 / (1.0 + luminance);"
    } else { "let mapped = rgb709;" };
    let transfer = if let Some(camera) = camera { camera_transfer(camera, full, managed) }
    else if managed { format!(r#"
        {decode_transfer}
        {gamut}
        return vec3f(dot(rgb709, vec3f(0.613128899, 0.339517308, 0.047353793)),
            dot(rgb709, vec3f(0.070106762, 0.916365764, 0.013527474)),
            dot(rgb709, vec3f(0.020586613, 0.109567717, 0.869845670)));
    "#) } else if transfer == 1 && primaries == 0 { "return v;".to_owned() } else { format!(r#"
        {decode_transfer}
        {gamut}
        {display}
        let encoded = select(1.055 * pow(max(mapped, vec3f(0.0)), vec3f(1.0 / 2.4)) - vec3f(0.055), 12.92 * mapped, mapped <= vec3f(0.0031308));
        return clamp(encoded, vec3f(0.0), vec3f(1.0));
    "#) };
    let source = format!(r#"
struct O {{ @builtin(position) p: vec4f, @location(0) uv: vec2f }}
@vertex fn vs(@builtin(vertex_index) i: u32) -> O {{ var o: O; let x = f32(i / 2u) * 4.0 - 1.0; let y = f32(i % 2u) * 4.0 - 1.0; o.p = vec4f(x,y,0,1); o.uv = vec2f((x+1.0)*0.5,(1.0-y)*0.5); return o; }}
@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var uv_tex: texture_2d<f32>;
fn output_color(v: vec3f) -> vec3f {{ {transfer} }}
@fragment fn fs(@location(0) uv: vec2f) -> @location(0) vec4f {{ let ys = textureDimensions(y_tex); let cs = textureDimensions(uv_tex); let p = vec2i(clamp(uv * vec2f(ys), vec2f(0.0), vec2f(ys - 1u))); let c = vec2i(clamp(uv * vec2f(cs), vec2f(0.0), vec2f(cs - 1u))); {conversion} }}
"#);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("native-video-conversion"), source: wgpu::ShaderSource::Wgsl(source.into()) });
    let entries = [0, 1].map(|binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("native-video-layout"), entries: &entries });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("native-video-pipeline-layout"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("native-video-pipeline"), layout: Some(&pipeline_layout), vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() }, fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs"), targets: &[Some(wgpu::ColorTargetState { format: if managed { wgpu::TextureFormat::Rgba16Float } else { wgpu::TextureFormat::Rgba8Unorm }, blend: None, write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }), primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: None });
    (layout, pipeline)
}

fn camera_transfer(profile: CameraLog, full: bool, managed: bool) -> String {
    // The surface range belongs to YCbCr. S-Log3's equation consumes the
    // recorded code, so restore limited-range RGB codes before the inverse.
    let recorded = if full { "v" } else { "(v * 876.0 + 64.0) / 1023.0" };
    let decode = match profile {
        CameraLog::SonySLog2 => format!(
            "let encoded = ({recorded} * 1023.0 - 64.0) / 876.0; \
             let linear = select((encoded - 0.03000122285) / (3.5388127854 / 0.9), \
             (pow(vec3f(10.0), (encoded - 0.646596) / 0.432699) - 0.037584) * (0.9 * 219.0 / 155.0), \
             encoded >= vec3f(26.0 / 876.0));"
        ),
        _ => format!(
            "let code = {recorded} * 1023.0; \
             let linear = select((code - 95.0) * (0.01125 / (171.2102946929 - 95.0)), \
             0.19 * pow(vec3f(10.0), (code - 420.0) / 261.5) - 0.01, code >= vec3f(171.2102946929));"
        ),
    };
    // Linear Sony primaries → AP1 with CAT02 D65→D60 adaptation.
    let gamut = match profile {
        CameraLog::SonySLog3Cine => "vec3f(dot(linear, vec3f(0.934517052, 0.143641683, -0.078158735)), dot(linear, vec3f(-0.050526660, 1.261609212, -0.211082552)), dot(linear, vec3f(-0.024503019, -0.030671048, 1.055174067)))",
        _ => "vec3f(dot(linear, vec3f(1.089798218, -0.032765047, -0.057033172)), dot(linear, vec3f(-0.031171867, 1.182936689, -0.151764822)), dot(linear, vec3f(-0.003263585, -0.001569851, 1.004833436)))",
    };
    let output = if managed { "return ap1;" } else { r#"
        let rgb = vec3f(dot(ap1, vec3f(1.7048586763, -0.6217160219, -0.0831426544)),
            dot(ap1, vec3f(-0.1300768242, 1.1407357748, -0.0106589506)),
            dot(ap1, vec3f(-0.0239640729, -0.1289755083, 1.1529395812)));
        let luma = max(dot(rgb, vec3f(0.2126, 0.7152, 0.0722)), 0.0);
        let mapped = rgb / (1.0 + luma);
        return clamp(select(1.055 * pow(max(mapped, vec3f(0.0)), vec3f(1.0/2.4)) - vec3f(0.055),
            mapped * 12.92, mapped <= vec3f(0.0031308)), vec3f(0.0), vec3f(1.0));
    "# };
    format!("{decode} let ap1 = {gamut}; {output}")
}
