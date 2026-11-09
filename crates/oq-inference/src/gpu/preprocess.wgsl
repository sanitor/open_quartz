// Byte-domain BT.601 preprocessing, identical to the host nearest-neighbor reference.
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> planes: array<f32>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 224u || id.y >= 224u) { return; }
    let size = textureDimensions(image);
    let source = vec2<u32>(id.x * size.x / 224u, id.y * size.y / 224u);
    let rgb = textureLoad(image, vec2<i32>(source), 0).rgb;
    let index = id.y * 224u + id.x;
    planes[index] = 0.299 * rgb.r + 0.587 * rgb.g + 0.114 * rgb.b;
    planes[50176u + index] = -0.169 * rgb.r - 0.331 * rgb.g + 0.5 * rgb.b + 0.5;
    planes[100352u + index] = 0.5 * rgb.r - 0.419 * rgb.g - 0.081 * rgb.b + 0.5;
}
