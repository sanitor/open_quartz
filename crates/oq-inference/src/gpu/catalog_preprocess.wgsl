struct Params { geometry: vec4u, result: vec4u, controls: vec4f }
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> image: array<f32>;
@group(0) @binding(2) var<uniform> params: Params;
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) p:vec3u) {
    let size=params.geometry.zw;
    if (any(p.xy>=size)) {return;}
    let original=params.geometry.xy;
    let kind=u32(params.controls.z);
    var rgb=vec3f(114.0/255.0);
    if (kind==3u || kind==4u) {
        let scale=min(f32(size.x)/f32(original.x),f32(size.y)/f32(original.y));
        let resized=vec2u(floor(vec2f(original)*scale+vec2f(0.5)));
        let pad=vec2u(floor(vec2f(size-resized)*0.5+vec2f(0.5)));
        if (all(p.xy>=pad) && all(p.xy<pad+resized)) {
            let coord=vec2i(clamp(floor((vec2f(p.xy-pad)+vec2f(0.5))/scale),vec2f(0.0),vec2f(original-vec2u(1))));
            rgb=textureLoad(source,coord,0).rgb;
        }
    } else {
        rgb=textureLoad(source,vec2i(p.xy*original/size),0).rgb;
    }
    let plane=size.x*size.y;
    let index=p.y*size.x+p.x;
    image[index]=rgb.r; image[plane+index]=rgb.g; image[plane*2u+index]=rgb.b;
}
