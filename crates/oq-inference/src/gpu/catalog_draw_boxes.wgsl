struct Detection { bbox:vec4f, score:f32, class_id:u32, index:u32, reserved:u32 }
struct Detections { header:vec4u, records:array<Detection> }
struct VertexOut { @builtin(position) position:vec4f }
@group(0) @binding(0) var<storage,read> detections:Detections;
@group(0) @binding(1) var<uniform> dimensions:vec2u;
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->VertexOut {
    var out:VertexOut;out.position=vec4f(2.0,2.0,0.0,1.0);
    if(instance>=detections.header.x){return out;}
    let b=detections.records[instance].bbox;
    let edge=vertex/6u; let c=array<u32,6>(0u,1u,2u,2u,1u,3u)[vertex%6u];
    let left=floor(b.x*f32(dimensions.x));let top=floor(b.y*f32(dimensions.y));
    let right=ceil(b.z*f32(dimensions.x));let bottom=ceil(b.w*f32(dimensions.y));
    var pixel:vec2f;
    if(edge==0u){pixel=vec2f(select(left,right+1.0,(c&1u)!=0u),select(top,top+2.0,(c&2u)!=0u));}
    else if(edge==1u){pixel=vec2f(select(left,right+1.0,(c&1u)!=0u),select(bottom-1.0,bottom+1.0,(c&2u)!=0u));}
    else if(edge==2u){pixel=vec2f(select(left,left+2.0,(c&1u)!=0u),select(top,bottom+1.0,(c&2u)!=0u));}
    else {pixel=vec2f(select(right-1.0,right+1.0,(c&1u)!=0u),select(top,bottom+1.0,(c&2u)!=0u));}
    out.position=vec4f(pixel.x/f32(dimensions.x)*2.0-1.0,1.0-pixel.y/f32(dimensions.y)*2.0,0.0,1.0);return out;
}
@fragment fn fs()->@location(0) vec4f{return vec4f(1.0,59.0/255.0,48.0/255.0,1.0);}
