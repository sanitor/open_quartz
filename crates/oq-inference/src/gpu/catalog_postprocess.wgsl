struct Params { geometry:vec4u, result:vec4u, controls:vec4f }
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var<storage,read> tensor:array<f32>;
@group(0) @binding(2) var<uniform> params:Params;
@group(0) @binding(3) var<storage,read> auxiliary:array<u32>;
@group(0) @binding(4) var output:texture_storage_2d<rgba8unorm,write>;
const PALETTE=array<vec3u,19>(vec3u(128,64,128),vec3u(244,35,232),vec3u(70,70,70),vec3u(102,102,156),vec3u(190,153,153),vec3u(153,153,153),vec3u(250,170,30),vec3u(220,220,0),vec3u(107,142,35),vec3u(152,251,152),vec3u(70,130,180),vec3u(220,20,60),vec3u(255,0,0),vec3u(0,0,142),vec3u(0,0,70),vec3u(0,60,100),vec3u(0,80,100),vec3u(0,0,230),vec3u(119,11,32));
fn byte(value:f32)->f32{return floor(clamp(value,0.0,1.0)*255.0+0.5)/255.0;}
@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) id:vec3u) {
    let size=textureDimensions(output);if(any(id.xy>=size)){return;}
    let kind=u32(params.controls.z);let original=params.geometry.xy;let raw=params.result.zw;
    var rgba=textureLoad(source,vec2i(id.xy*original/size),0);
    if(kind==0u){
        let p=id.xy*raw/size;let plane=raw.x*raw.y;let index=p.y*raw.x+p.x;
        rgba=vec4f(byte(tensor[index]),byte(tensor[plane+index]),byte(tensor[2u*plane+index]),1.0);
    }else if(kind==1u){
        // Reference masks sample from the model-sized RGB image before resizing
        // back to source dimensions, rather than retaining arbitrary source detail.
        let p=id.xy*raw/size;let image_size=params.geometry.zw;
        let ingress=p*image_size/raw;let coord=ingress*original/image_size;
        rgba=textureLoad(source,vec2i(coord),0);
        rgba.a=byte(tensor[p.y*raw.x+p.x]);
    }else if(kind==2u){
        let p=id.xy*raw/size;
        let minimum=bitcast<f32>(auxiliary[0]);let maximum=bitcast<f32>(auxiliary[1]);
        let gray=byte((tensor[p.y*raw.x+p.x]-minimum)/max(maximum-minimum,1.1920929e-7));
        rgba=vec4f(gray,gray,gray,1.0);
    }else if(kind==3u){
        let input=params.geometry.zw;let scale=min(f32(input.x)/f32(original.x),f32(input.y)/f32(original.y));
        let pad=(vec2f(input)-vec2f(original)*scale)*0.5;
        let crop=vec2u(floor(pad*vec2f(raw)/vec2f(input)+0.5));
        let crop_size=max(vec2u(1),vec2u(floor(vec2f(original)*scale*vec2f(raw)/vec2f(input)+0.5)));
        let p=min(crop+id.xy*crop_size/size,raw-vec2u(1));
        let category=u32(tensor[p.y*raw.x+p.x]);var color=vec3u(0);
        if(category<19u){color=PALETTE[category];}
        rgba=vec4f(floor((rgba.rgb*255.0+vec3f(color))*0.5)/255.0,rgba.a);
    }else if(kind==4u){
        rgba=textureLoad(source,vec2i(id.xy),0);
    }
    textureStore(output,vec2i(id.xy),rgba);
}
