// GPU-only postprocessing for the unmodified YOLOv8 FLOAT [1, 84, 8400]
// output. All three entry points use the same explicit bind-group layout.
struct Params {
    geometry: vec4u,
    result: vec4u,
    controls: vec4f,
}

struct SortStep {
    sequence: u32,
    stride: u32,
    reserved0: u32,
    reserved1: u32,
}

struct Detection {
    bbox: vec4f,
    score: f32,
    class_id: u32,
    index: u32,
    reserved: u32,
}

struct Detections {
    header: vec4u,
    records: array<Detection>,
}

@group(0) @binding(0) var<storage, read> raw: array<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> detections: Detections;
@group(0) @binding(3) var<uniform> step: SortStep;

const BOX_COUNT: u32 = 8400u;
const CAPACITY: u32 = 16384u;

fn finite(value: f32) -> bool {
    return (bitcast<u32>(value) & 0x7f800000u) != 0x7f800000u;
}

fn finite_box(value: vec4f) -> bool {
    return all((bitcast<vec4u>(value) & vec4u(0x7f800000u)) != vec4u(0x7f800000u));
}

@compute @workgroup_size(64)
fn decode(@builtin(global_invocation_id) invocation: vec3u) {
    let index = invocation.x;
    if index >= CAPACITY {
        return;
    }
    if index == 0u {
        detections.header = vec4u(0u);
    }
    // Every record is initialized, including the bitonic power-of-two padding.
    detections.records[index] = Detection(vec4f(0.0), -1.0, 0u, index, 0u);
    if index >= BOX_COUNT || any(params.geometry == vec4u(0u)) {
        return;
    }

    let xywh = vec4f(raw[index], raw[BOX_COUNT + index],
        raw[2u * BOX_COUNT + index], raw[3u * BOX_COUNT + index]);
    if !finite_box(xywh) {
        return;
    }
    var score = 0.0;
    var class_id = 0u;
    for (var class_index = 0u; class_index < 80u; class_index += 1u) {
        let candidate = raw[(4u + class_index) * BOX_COUNT + index];
        // Native output must be finite. Discard malformed candidates explicitly
        // rather than letting NaNs violate the sort's total ordering.
        if !finite(candidate) {
            return;
        }
        if candidate > score {
            score = candidate;
            class_id = class_index;
        }
    }
    if score < params.controls.x {
        return;
    }

    let source = vec2f(params.geometry.xy);
    let network = vec2f(params.geometry.zw);
    let scale = min(network.x / source.x, network.y / source.y);
    // Rust f32::round is half-away-from-zero, unlike WGSL round's permitted
    // half-to-even behavior. Avoid adding .5 before floor: that addition can
    // itself round a value just below a half-integer up to the next integer.
    let scaled = source * scale;
    let resized = floor(scaled) + select(vec2f(0.0), vec2f(1.0), fract(scaled) >= vec2f(0.5));
    let padding = (network - resized) / 2.0;
    let inverse = vec2f(1.0) / (source * scale);
    let low = (xywh.xy - xywh.zw / 2.0 - padding) * inverse;
    let high = (xywh.xy + xywh.zw / 2.0 - padding) * inverse;
    let bbox = clamp(vec4f(low, high), vec4f(0.0), vec4f(1.0));
    if !finite_box(bbox) {
        return;
    }
    detections.records[index] = Detection(bbox, score, class_id, index, 0u);
}

// The original index breaks all score ties, matching the CPU's stable sort.
fn before(a: Detection, b: Detection) -> bool {
    return a.score > b.score || (a.score == b.score && a.index < b.index);
}

@compute @workgroup_size(64)
fn sort(@builtin(global_invocation_id) invocation: vec3u) {
    let pair = invocation.x;
    if pair >= CAPACITY / 2u {
        return;
    }
    // One invocation owns each disjoint pair and writes both records. No
    // invocation reads a record concurrently written by another in this pass.
    let first = (pair / step.stride) * (2u * step.stride) + pair % step.stride;
    let second = first + step.stride;
    let a = detections.records[first];
    let b = detections.records[second];
    let descending = (first & step.sequence) == 0u;
    if (descending && before(b, a)) || (!descending && before(a, b)) {
        detections.records[first] = b;
        detections.records[second] = a;
    }
}

fn iou(a: vec4f, b: vec4f) -> f32 {
    let low = max(a.xy, b.xy);
    let high = min(a.zw, b.zw);
    let overlap = max(high - low, vec2f(0.0));
    let intersection = overlap.x * overlap.y;
    let area_a = (a.z - a.x) * (a.w - a.y);
    let area_b = (b.z - b.x) * (b.w - b.y);
    let union_area = area_a + area_b - intersection;
    if union_area <= 0.0 {
        return 0.0;
    }
    return intersection / union_area;
}

@compute @workgroup_size(64)
fn nms(@builtin(global_invocation_id) invocation: vec3u) {
    if invocation.x != 0u {
        return;
    }
    var count = 0u;
    // Testing each sorted candidate against every already-kept box is exactly
    // the CPU's greedy suppression, without a top-K or fixed detection limit.
    // count <= index, so in-place compaction cannot overwrite unread records.
    for (var index = 0u; index < BOX_COUNT; index += 1u) {
        let candidate = detections.records[index];
        if candidate.score < 0.0 {
            break;
        }
        var suppressed = false;
        for (var kept = 0u; kept < count; kept += 1u) {
            if iou(detections.records[kept].bbox, candidate.bbox) > params.controls.y {
                suppressed = true;
                break;
            }
        }
        if !suppressed {
            detections.records[count] = candidate;
            count += 1u;
        }
    }
    detections.header = vec4u(count, 0u, 0u, 0u);
}
