// Fuse exact channel-to-space(3), byte-domain chroma reconstruction, quantization,
// and the reference's final nearest resize. No intermediate image leaves the GPU.
@group(0) @binding(0) var<storage, read> convolution: array<f32>;
@group(0) @binding(1) var<storage, read> planes: array<f32>;
@group(0) @binding(2) var output: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    if (id.x >= size.x || id.y >= size.y) { return; }
    let model = vec2<u32>(id.x * 672u / size.x, id.y * 672u / size.y);
    let pixel = model / 3u;
    let channel = (model.y % 3u) * 3u + model.x % 3u;
    let index = pixel.y * 224u + pixel.x;
    let y = convolution[channel * 50176u + index];
    let cb = planes[50176u + index] - 0.5;
    let cr = planes[100352u + index] - 0.5;
    let rgb = vec3<f32>(y + 1.402 * cr, y - 0.344 * cb - 0.714 * cr, y + 1.772 * cb);
    let quantized = floor(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0 + vec3<f32>(0.5)) / 255.0;
    textureStore(output, vec2<i32>(id.xy), vec4<f32>(quantized, 1.0));
}
