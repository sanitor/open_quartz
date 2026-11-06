pub const FULLSCREEN_VERT_WITH_UV: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4f,
    @location(0) v_uv: vec2f,
}

@vertex
fn main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    var output: VertexOutput;
    output.position = vec4f(x, y, 0.0, 1.0);
    output.v_uv = vec2f((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return output;
}
"#;

pub const BLIT_FRAG: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
    return textureSample(tex, samp, v_uv);
}
"#;

pub mod compiler;
pub mod parser;

pub use compiler::{
    compile_shader, validate_shader, CompilePort, CompileRequest, CompiledShader,
    WgslCompilationError,
};
pub use parser::{parse_shader, ParsedShader};
