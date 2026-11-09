// Planar NCHW FP32 cross-correlation; ONNX weights are contiguous OIHW.
struct ConvParams {
    input_channels: u32,
    output_channels: u32,
    kernel_height: u32,
    kernel_width: u32,
    pad_top: u32,
    pad_left: u32,
    relu: u32,
    reserved: u32,
}
@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;
@group(0) @binding(2) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<storage, read> bias: array<f32>;
@group(0) @binding(4) var<uniform> params: ConvParams;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 224u || id.y >= 224u || id.z >= params.output_channels) { return; }
    var value = bias[id.z];
    for (var channel = 0u; channel < params.input_channels; channel += 1u) {
        for (var ky = 0u; ky < params.kernel_height; ky += 1u) {
            let y = i32(id.y) + i32(ky) - i32(params.pad_top);
            if (y < 0 || y >= 224) { continue; }
            for (var kx = 0u; kx < params.kernel_width; kx += 1u) {
                let x = i32(id.x) + i32(kx) - i32(params.pad_left);
                if (x < 0 || x >= 224) { continue; }
                let input_index = channel * 50176u + u32(y) * 224u + u32(x);
                let weight_index = ((id.z * params.input_channels + channel) * params.kernel_height + ky) * params.kernel_width + kx;
                value += input[input_index] * weights[weight_index];
            }
        }
    }
    if (params.relu != 0u) { value = max(value, 0.0); }
    output[id.z * 50176u + id.y * 224u + id.x] = value;
}
