// 32 output channels × 32 adjacent pixels per workgroup, reduced in K=32 tiles.
// Each lane accumulates a 4-channel × 4-pixel register tile. Input/weight tiles
// are loaded once and shared instead of refetching each convolution operand.
// INPUT_CHANNELS, OUTPUT_CHANNELS, KERNEL, PADDING and RELU are preparation-time
// constants from the validated ONNX layer; tensors and accumulation remain FP32.
@group(0) @binding(0) var<storage, read> input: array<f32>;
@group(0) @binding(1) var<storage, read_write> output: array<f32>;
@group(0) @binding(2) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<storage, read> bias: array<f32>;

const TILE_K: u32 = 32u;
const TILE_VECTORS: u32 = TILE_K * 8u;
var<workgroup> input_tile: array<vec4f, TILE_VECTORS>;
var<workgroup> weight_tile: array<vec4f, TILE_VECTORS>;
const REDUCTION: u32 = INPUT_CHANNELS * KERNEL * KERNEL;

fn input_value(channel: u32, y: i32, x: i32) -> f32 {
    if (x < 0 || x >= 224 || y < 0 || y >= 224) { return 0.0; }
    return input[channel * 50176u + u32(y) * 224u + u32(x)];
}

fn weight_value(channel: u32, k: u32) -> f32 {
    if (channel >= OUTPUT_CHANNELS || k >= REDUCTION) { return 0.0; }
    return weights[channel * REDUCTION + k];
}

fn bias_value(channel: u32) -> f32 {
    if (channel >= OUTPUT_CHANNELS) { return 0.0; }
    return bias[channel];
}

@compute @workgroup_size(8, 8)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_id) local: vec3u) {
    let lane = local.y * 8u + local.x;
    let first_channel = group.z * 32u + local.y * 4u;
    let first_pixel = group.x * 32u + local.x * 4u;
    var value0 = vec4f(bias_value(first_channel));
    var value1 = vec4f(bias_value(first_channel + 1u));
    var value2 = vec4f(bias_value(first_channel + 2u));
    var value3 = vec4f(bias_value(first_channel + 3u));

    for (var block = 0u; block < REDUCTION; block += TILE_K) {
        for (var element = lane; element < TILE_VECTORS; element += 64u) {
            let k = block + element / 8u;
            let column = element % 8u;
            let channel = group.z * 32u + column * 4u;
            weight_tile[element] = vec4f(
                weight_value(channel, k), weight_value(channel + 1u, k),
                weight_value(channel + 2u, k), weight_value(channel + 3u, k));
            var pixels = vec4f(0.0);
            if (k < REDUCTION) {
                let input_channel = k / (KERNEL * KERNEL);
                let spatial = k % (KERNEL * KERNEL);
                let y = i32(group.y) + i32(spatial / KERNEL) - i32(PADDING);
                let x = i32(group.x * 32u + column * 4u) + i32(spatial % KERNEL) - i32(PADDING);
                pixels = vec4f(input_value(input_channel, y, x), input_value(input_channel, y, x + 1),
                    input_value(input_channel, y, x + 2), input_value(input_channel, y, x + 3));
            }
            input_tile[element] = pixels;
        }
        workgroupBarrier();
        for (var k = 0u; k < TILE_K; k += 1u) {
            let pixels = input_tile[k * 8u + local.x];
            let coefficients = weight_tile[k * 8u + local.y];
            value0 += pixels * coefficients.x;
            value1 += pixels * coefficients.y;
            value2 += pixels * coefficients.z;
            value3 += pixels * coefficients.w;
        }
        // Every reader must finish before any lane writes the next K tile.
        workgroupBarrier();
    }
    if (RELU) {
        value0 = max(value0, vec4f(0.0));
        value1 = max(value1, vec4f(0.0));
        value2 = max(value2, vec4f(0.0));
        value3 = max(value3, vec4f(0.0));
    }
    for (var pixel = 0u; pixel < 4u; pixel += 1u) {
        let position = group.y * 224u + first_pixel + pixel;
        if (first_channel < OUTPUT_CHANNELS) { output[first_channel * 50176u + position] = value0[pixel]; }
        if (first_channel + 1u < OUTPUT_CHANNELS) { output[(first_channel + 1u) * 50176u + position] = value1[pixel]; }
        if (first_channel + 2u < OUTPUT_CHANNELS) { output[(first_channel + 2u) * 50176u + position] = value2[pixel]; }
        if (first_channel + 3u < OUTPUT_CHANNELS) { output[(first_channel + 3u) * 50176u + position] = value3[pixel]; }
    }
}
