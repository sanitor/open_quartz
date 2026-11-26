@group(0) @binding(0) var program: texture_2d<f32>;
@group(0) @binding(1) var program_sampler: sampler;
@group(0) @binding(2) var<uniform> display: vec4f;

struct Vertex {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    return Vertex(vec4f(uv * vec2f(2.0, -2.0) + vec2f(-1.0, 1.0), 0.0, 1.0), uv);
}

@fragment fn fragment(@location(0) uv: vec2f) -> @location(0) vec4f {
    let pixel = textureSample(program, program_sampler, uv);
    let luma = max(dot(pixel.rgb, vec3f(0.2722287, 0.6740818, 0.0536895)), 0.0);
    if (display.z > 0.0) {
        let span = max(display.x - 1.0, 0.01);
        let mapped = min(luma, 1.0) + span * (1.0 - exp(-max(luma - 1.0, 0.0) / span));
        let linear = cc_srgb_decode(cc_ap1_to_srgb(pixel.rgb));
        return vec4f(linear * (mapped / max(luma, 1e-6)) * display.y, pixel.a);
    }
    let encoded = cc_ap1_to_srgb(pixel.rgb / (1.0 + luma));
    return vec4f(select(encoded, cc_srgb_decode(encoded), display.w > 0.0), pixel.a);
}
