@group(0) @binding(0) var<storage, read> depth:array<f32>;
@group(0) @binding(1) var<storage, read_write> ranges:array<vec2f>;
var<workgroup> values:array<vec2f,256>;
fn fold(lane:u32) {
    workgroupBarrier();
    for(var stride=128u;stride>0u;stride/=2u) {
        if(lane<stride) {values[lane]=vec2f(min(values[lane].x,values[lane+stride].x),max(values[lane].y,values[lane+stride].y));}
        workgroupBarrier();
    }
}
@compute @workgroup_size(256)
fn partial(@builtin(local_invocation_index) lane:u32,@builtin(workgroup_id) group:vec3u) {
    let index=group.x*256u+lane;
    var value=vec2f(3.402823e38,-3.402823e38);
    if(index<arrayLength(&depth)) {value=vec2f(depth[index]);}
    values[lane]=value;
    fold(lane);
    if(lane==0u) {ranges[group.x]=values[0];}
}
@compute @workgroup_size(256)
fn finish(@builtin(local_invocation_index) lane:u32) {
    values[lane]=vec2f(depth[lane*2u],depth[lane*2u+1u]);
    fold(lane);
    if(lane==0u) {ranges[0]=values[0];}
}
