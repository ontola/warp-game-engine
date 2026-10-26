// Receivers use actual current instance/wind buffers, independently of caster
// material flags, camera Hi-Z, fog and shadow LOD. Every dispatch is ordered.
struct DemandInstance {
    r0:vec4<f32>,r1:vec4<f32>,r2:vec4<f32>,sphere:vec4<f32>,
    first_cluster:u32,cluster_count:u32,scale:f32,flags:u32,
}
// Exact GpuCluster80B layout. Every resident LOD/material is a receiver:
// selecting a camera LOD here could lose glass or a different rendered cut.
struct DemandCluster {
    cull:vec4<f32>,parent:vec4<f32>,own:vec4<f32>,
    parent_error:f32,own_error:f32,first_tri:u32,tri_count:u32,
    first_vert:u32,material:u32,depth:u32,pad:u32,
}
struct DemandCamera {planes:array<vec4<f32>,6>,flags:vec4<u32>}
struct DemandMap {vp:mat4x4<f32>,info:vec4<u32>,offsets:array<vec4<u32>,2>,reserved:vec4<u32>}
struct DemandConfig {header:vec4<u32>,control:vec4<u32>,cameras:array<DemandCamera,4>,maps:array<DemandMap,5>}
struct DemandExtra {lower:vec3<f32>,map_mask:u32,upper:vec3<f32>,reserved:u32}
@group(0) @binding(0) var<uniform> demand_config:DemandConfig;
@group(0) @binding(1) var<storage,read> demand_instances:array<DemandInstance>;
@group(0) @binding(2) var<storage,read> demand_wind:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read> demand_extra:array<DemandExtra>;
@group(0) @binding(4) var<storage,read_write> demand_words:array<atomic<u32>>;
@group(0) @binding(5) var<storage,read> demand_clusters:array<DemandCluster>;

@compute @workgroup_size(64)
fn demand_clear(@builtin(global_invocation_id) gid:vec3<u32>) {
    let i=gid.x;if i>=arrayLength(&demand_words) {return;}
    var value=0u;
    if i==0u {value=1u;}
    if i==1u {value=demand_config.header.x;}
    if i==3u {value=demand_config.control.y;}
    if i==4u {value=demand_config.header.y;}
    if i==5u {value=demand_config.header.w;}
    if i==8u {value=demand_config.control.x;}
    if i>=16u && i<176u {
        let map=(i-16u)/32u;let word=(i-16u)%32u;let m=demand_config.maps[map];
        if word<16u {value=bitcast<u32>(m.vp[word/4u][word%4u]);}
        else if word<20u {value=m.info[word-16u];}
        else if word<28u {value=m.offsets[(word-20u)/4u][(word-20u)%4u];}
    }
    atomicStore(&demand_words[i],value);
}
fn demand_maps_complete(map_mask:u32)->bool {
    for(var map=0u;map<demand_config.header.y;map++) {
        if (map_mask&(1u<<map))==0u {continue;}
        let tiles=demand_config.maps[map].info.y;
        if atomicLoad(&demand_words[16u+map*32u+28u])!=tiles*tiles {return false;}
    }
    return true;
}
fn demand_camera_maps()->u32 {
    // All possible camera maps, not just the coarse owner's current mask:
    // a refined cluster may reach another view's plane conservatively.
    var mask=0u;
    for(var camera=0u;camera<demand_config.header.z;camera++) {mask|=demand_config.cameras[camera].flags.x;}
    return mask;
}
fn demand_mark_tile(map:u32,tile:u32) {
    // The count can lag writes, but never leads them: only the unique first
    // request increments it. Dispatch ordering makes all writes visible before
    // reduction/publication; no same-dispatch mask read relies on the count.
    if atomicOr(&demand_words[tile],1u)==0u {atomicAdd(&demand_words[16u+map*32u+28u],1u);}
}
fn demand_request(center:vec3<f32>,extent:vec3<f32>,map_mask:u32) {
    for(var map=0u;map<demand_config.header.y;map++) {
        if (map_mask&(1u<<map))==0u {continue;}
        let m=demand_config.maps[map];
        if atomicLoad(&demand_words[16u+map*32u+28u])==m.info.y*m.info.y {continue;}
        let rect=demand_project(m.vp,center,extent,m.info.x,4.0);
        if rect.kind==2u {atomicOr(&demand_words[3],1u);continue;}
        if rect.kind==0u {continue;}
        atomicAdd(&demand_words[16u+map*32u+29u],1u);
        for(var y=rect.first.y;y<=rect.last.y;y++) {
            if atomicLoad(&demand_words[16u+map*32u+28u])==m.info.y*m.info.y {break;}
            for(var x=rect.first.x;x<=rect.last.x;x++) {demand_mark_tile(map,m.offsets[0].x+y*m.info.y+x);}
        }
    }
}
fn demand_camera_mask(center:vec3<f32>,radius:f32)->u32 {
    // Camera plane arithmetic must not discard a touching sphere due to f32
    // roundoff. Caster policy is deliberately absent (NO_SHADOW may receive).
    let plane_pad=(dot(abs(center),vec3<f32>(1.0))+radius+1.0)*0.000003814697265625;
    var mask=0u;
    for(var camera=0u;camera<demand_config.header.z;camera++) {
        var visible=true;
        for(var p=0u;p<6u;p++) {
            let plane=demand_config.cameras[camera].planes[p];
            if dot(plane.xyz,center)+plane.w < -radius-plane_pad {visible=false;}
        }
        if visible {mask|=demand_config.cameras[camera].flags.x;}
    }
    return mask;
}
fn demand_count_cluster(word:u32) {
    // Capture totals use two words, independent of repeated mesh instances.
    if atomicAdd(&demand_words[word],1u)==0xffffffffu {atomicAdd(&demand_words[word+1u],1u);}
}
@compute @workgroup_size(64)
fn demand_owners(@builtin(global_invocation_id) gid:vec3<u32>) {
    let id=gid.x+gid.y*65535u*64u;
    if id>=demand_config.header.w {return;}
    if id>=arrayLength(&demand_instances) || id>=arrayLength(&demand_wind) {atomicOr(&demand_words[3],1u);return;}
    let camera_maps=demand_camera_maps();
    if demand_maps_complete(camera_maps) {return;}
    let inst=demand_instances[id];if inst.cluster_count==0u {return;}
    atomicAdd(&demand_words[6],1u);
    let wind=demand_wind[id].z;let radius=inst.sphere.w+wind;
    if !all(abs(inst.sphere)<vec4<f32>(1e18)) || !(inst.sphere.w>=0.0) ||
        !(wind>=0.0 && wind<1e18) || !(radius>=0.0 && radius<1e18) {atomicOr(&demand_words[3],1u);return;}
    let mask=demand_camera_mask(inst.sphere.xyz,radius);
    if mask==0u {return;}
    atomicAdd(&demand_words[7],1u);
    // Terrain provenance selects request precision only, never render policy.
    // In particular the whole road is one enormous owner; projecting its
    // sphere requests all maps even when only a small road section is visible.
    if ((inst.flags>>24u)&7u)!=1u {
        atomicAdd(&demand_words[9],1u);
        demand_request(inst.sphere.xyz,vec3<f32>(radius),mask);return;
    }
    atomicAdd(&demand_words[10],1u);
    let count=arrayLength(&demand_clusters);
    if inst.first_cluster>=count || inst.cluster_count>count-inst.first_cluster ||
        !all(abs(inst.r0)<vec4<f32>(1e18)) || !all(abs(inst.r1)<vec4<f32>(1e18)) ||
        !all(abs(inst.r2)<vec4<f32>(1e18)) || !(inst.scale>=0.0 && inst.scale<1e18) {
        atomicOr(&demand_words[3],1u);return;
    }
    // No loop budget or LOD/material predicate. Stop only when every possible
    // camera map is already full; the remaining geometry cannot add demand.
    // Invalid visited source data requests all maps instead of truncating.
    for(var local=0u;local<inst.cluster_count;local++) {
        if demand_maps_complete(camera_maps) {break;}
        demand_count_cluster(11u);
        let bound=demand_clusters[inst.first_cluster+local].cull;
        let p=vec4<f32>(bound.xyz,1.0);
        let center=vec3<f32>(dot(inst.r0,p),dot(inst.r1,p),dot(inst.r2,p));
        let r=bound.w*inst.scale+wind;
        if !all(abs(bound)<vec4<f32>(1e18)) || !(bound.w>=0.0) ||
            !all(abs(center)<vec3<f32>(1e18)) || !(r>=0.0 && r<1e18) {
            atomicOr(&demand_words[3],1u);return;
        }
        let cluster_mask=demand_camera_mask(center,r);
        if cluster_mask==0u {continue;}
        demand_count_cluster(13u);
        demand_request(center,vec3<f32>(r),cluster_mask);
    }
}
@compute @workgroup_size(64)
fn demand_extras(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.x>=demand_config.control.x {return;}
    let b=demand_extra[gid.x];demand_request((b.lower+b.upper)*0.5,(b.upper-b.lower)*0.5,b.map_mask);
}
@compute @workgroup_size(64)
fn demand_fallback(@builtin(global_invocation_id) gid:vec3<u32>) {
    if gid.y>=demand_config.header.y || atomicLoad(&demand_words[3])==0u {return;}
    let m=demand_config.maps[gid.y];if gid.x>=m.info.y*m.info.y {return;}
    demand_mark_tile(gid.y,m.offsets[0].x+gid.x);
}
fn demand_reduce(gid:vec3<u32>,level:u32) {
    if gid.y>=demand_config.header.y {return;}
    let m=demand_config.maps[gid.y];if level>=m.info.z {return;}
    let dim=m.info.y>>level;if gid.x>=dim*dim {return;}
    let input=m.offsets[(level-1u)/4u][(level-1u)%4u];
    let output=m.offsets[level/4u][level%4u];
    let p=vec2<u32>(gid.x%dim,gid.x/dim)*2u;let first=input+p.y*dim*2u+p.x;
    let value=atomicLoad(&demand_words[first])|atomicLoad(&demand_words[first+1u])|
        atomicLoad(&demand_words[first+dim*2u])|atomicLoad(&demand_words[first+dim*2u+1u]);
    atomicStore(&demand_words[output+gid.x],value);
}
@compute @workgroup_size(64) fn demand_reduce_1(@builtin(global_invocation_id) id:vec3<u32>) {demand_reduce(id,1u);}
@compute @workgroup_size(64) fn demand_reduce_2(@builtin(global_invocation_id) id:vec3<u32>) {demand_reduce(id,2u);}
@compute @workgroup_size(64) fn demand_reduce_3(@builtin(global_invocation_id) id:vec3<u32>) {demand_reduce(id,3u);}
@compute @workgroup_size(64) fn demand_reduce_4(@builtin(global_invocation_id) id:vec3<u32>) {demand_reduce(id,4u);}
@compute @workgroup_size(64) fn demand_reduce_5(@builtin(global_invocation_id) id:vec3<u32>) {demand_reduce(id,5u);}
@compute @workgroup_size(64) fn demand_reduce_6(@builtin(global_invocation_id) id:vec3<u32>) {demand_reduce(id,6u);}
@compute @workgroup_size(1)
fn demand_publish() {atomicStore(&demand_words[2],demand_config.header.x);}
