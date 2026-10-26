// Current-pose spatial bounds, shared by colour and solar views. Never uses camera
// visibility and never supplies shadow caster omission.
struct Instance {
    r0:vec4<f32>, r1:vec4<f32>, r2:vec4<f32>, sphere:vec4<f32>,
    first_cluster:u32, cluster_count:u32, scale:f32, flags:u32,
}
struct SpatialGroup {sphere:vec4<f32>,span:vec4<u32>}
struct SpatialGroups {header:vec4<u32>,groups:array<SpatialGroup>}
@group(0) @binding(0) var<storage,read> instances:array<Instance>;
@group(0) @binding(1) var<storage,read> wind:array<vec4<f32>>;
@group(0) @binding(2) var<storage,read> owners:array<u32>;
@group(0) @binding(3) var<storage,read_write> spatial:SpatialGroups;
var<workgroup> lower:array<vec3<f32>,64>;
var<workgroup> upper:array<vec3<f32>,64>;

fn enclosing_sphere(lo:vec3<f32>,hi:vec3<f32>)->vec4<f32> {
    if any(lo>hi){return vec4<f32>(0.0,0.0,0.0,-1.0);}
    let center=(lo+hi)*0.5;
    let radius=length((hi-lo)*0.5);
    // Enclose floating-point reduction/center rounding as well as all source
    // spheres. Very large/invalid members force conservative group retention.
    let pad=(max(max(abs(center.x),abs(center.y)),abs(center.z))+radius)*1e-5+0.001;
    return vec4<f32>(center,radius+pad);
}
@compute @workgroup_size(64)
fn refit_leaves(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    let group_id=wg.x+wg.y*65535u;
    if group_id>=spatial.header.x{return;}
    let span=spatial.groups[group_id].span;
    var lo=vec3<f32>(1e30);var hi=vec3<f32>(-1e30);
    if li<span.y {
        let id=owners[span.x+li];let inst=instances[id];
        if inst.cluster_count!=0u {
            let radius=inst.sphere.w+wind[id].z;
            if all(abs(inst.sphere)<vec4<f32>(1e19)) && radius>=0.0 && radius<1e19 {
                lo=inst.sphere.xyz-vec3<f32>(radius);hi=inst.sphere.xyz+vec3<f32>(radius);
            }else{lo=vec3<f32>(-1e20);hi=vec3<f32>(1e20);}
        }
    }
    lower[li]=lo;upper[li]=hi;workgroupBarrier();
    for(var stride=32u;stride>0u;stride/=2u){
        if li<stride {lower[li]=min(lower[li],lower[li+stride]);upper[li]=max(upper[li],upper[li+stride]);}
        workgroupBarrier();
    }
    if li==0u {spatial.groups[group_id].sphere=enclosing_sphere(lower[0],upper[0]);}
}
@compute @workgroup_size(64)
fn refit_parents(@builtin(global_invocation_id) gid:vec3<u32>) {
    let id=gid.x+gid.y*65535u*64u;
    if id>=spatial.header.y{return;}
    let index=spatial.header.x+id;let span=spatial.groups[index].span;
    var lo=vec3<f32>(1e30);var hi=vec3<f32>(-1e30);
    for(var i=0u;i<span.y;i++) {let sphere=spatial.groups[span.x+i].sphere;
        if sphere.w>=0.0 {lo=min(lo,sphere.xyz-vec3<f32>(sphere.w));hi=max(hi,sphere.xyz+vec3<f32>(sphere.w));}}
    spatial.groups[index].sphere=enclosing_sphere(lo,hi);
}
