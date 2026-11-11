struct Detection { bbox:vec4f, score:f32, class_id:u32, index:u32, reserved:u32 }
struct Detections { header:vec4u, records:array<Detection> }
struct VertexOut { @builtin(position) position:vec4f }
@group(0) @binding(0) var<storage,read> detections:Detections;
@group(0) @binding(1) var<uniform> dimensions:vec2u;
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->VertexOut {
    var out:VertexOut;out.position=vec4f(2.0,2.0,0.0,1.0);
    if(instance>=detections.header.x){return out;}
    // Detections stay normalized; raster bounds match pipeline.rs::draw_box
    // after the CPU image task converts xyxy to source-image pixels.
    let b=detections.records[instance].bbox;
    let size=vec2f(dimensions);
    let last=max(size-vec2f(1.0),vec2f(0.0));
    let low=clamp(floor(b.xy*size),vec2f(0.0),last);
    let high=clamp(ceil(b.zw*size),vec2f(0.0),last);
    let edge=vertex/6u; let c=array<u32,6>(0u,1u,2u,2u,1u,3u)[vertex%6u];
    // Each CPU edge writes two inclusive one-pixel lines, saturating the
    // second line independently. This also draws outward for zero-sized boxes.
    // Collapse reversed inclusive ranges instead of rasterizing a flipped quad.
    var start:vec2f;
    var end:vec2f;
    if(edge==0u){
        start=low;
        end=vec2f(max(high.x+1.0,low.x),min(low.y+2.0,size.y));
    } else if(edge==1u){
        start=vec2f(low.x,max(high.y-1.0,0.0));
        end=vec2f(max(high.x+1.0,low.x),high.y+1.0);
    } else if(edge==2u){
        start=low;
        end=vec2f(min(low.x+2.0,size.x),max(high.y+1.0,low.y));
    } else {
        start=vec2f(max(high.x-1.0,0.0),low.y);
        end=vec2f(high.x+1.0,max(high.y+1.0,low.y));
    }
    let pixel=vec2f(select(start.x,end.x,(c&1u)!=0u),select(start.y,end.y,(c&2u)!=0u));
    out.position=vec4f(pixel.x/size.x*2.0-1.0,1.0-pixel.y/size.y*2.0,0.0,1.0);return out;
}
@fragment fn fs()->@location(0) vec4f{return vec4f(1.0,59.0/255.0,48.0/255.0,1.0);}
