// Current-pose source quadrature -> disjoint world optical interiors. There
// is no per-fragment root traversal. Integer linked lists avoid float atomics;
// malformed/over-budget cells invalidate their entire brick, never truncate it.
struct BrickParams {
    control:vec4<u32>, // generation, bricks, roots, padded sample jobs
    memory:vec4<u32>, // cell count, links base, area base, max list length
    physical:vec4<f32>, // root spacing, conservative support, max samples, roots/brick
    ranges:array<vec4<u32>,3>,
};
struct BrickGroup {roots:vec4<i32>,work:vec4<u32>};
struct BrickSourceSample {
    position_area:vec4<f32>,anchor_branch:vec4<f32>,
    geometric_normal_roughness:vec4<f32>,shading_normal:vec4<f32>,albedo:vec4<f32>,
};
struct BrickRoot {plant:MeadowPlant,valid:vec4<u32>};
struct BrickWorldSample {position:vec3<f32>,area:f32,geometric:vec3<f32>,roughness:f32,normal:vec3<f32>,albedo:vec3<f32>,valid:bool};
@group(2) @binding(0) var<uniform> brick_params:BrickParams;
@group(2) @binding(1) var<storage,read> brick_groups:array<BrickGroup>;
@group(2) @binding(2) var<storage,read> brick_source:array<BrickSourceSample>;
@group(2) @binding(3) var<storage,read> brick_levels:array<VolumeLevel>;
@group(2) @binding(4) var<storage,read_write> brick_cells:array<VolumeCell>;
@group(2) @binding(5) var<storage,read_write> brick_status:array<atomic<u32>>;
@group(2) @binding(6) var<storage,read_write> brick_scratch:array<atomic<u32>>;
@group(2) @binding(7) var<storage,read_write> brick_roots_buffer:array<BrickRoot>;

fn brick_error(group:u32,flag:u32) {atomicOr(&brick_status[group*4u+2u],flag);}
fn brick_job(id:vec3<u32>)->u32 {return id.x+id.y*262144u;}
fn brick_count(level:VolumeLevel)->u32 {return level.dims_offset.x*level.dims_offset.y*level.dims_offset.z;}
fn brick_coord(index:u32,dims:vec3<u32>)->vec3<u32> {
    return vec3<u32>(index%dims.x,(index/dims.x)%dims.y,index/(dims.x*dims.y));
}
fn brick_index(coord:vec3<u32>,level:VolumeLevel)->u32 {
    return level.dims_offset.w+coord.x+coord.y*level.dims_offset.x+coord.z*level.dims_offset.x*level.dims_offset.y;
}
fn brick_source_world(job:u32)->BrickWorldSample {
    var result:BrickWorldSample;
    let max_samples=u32(brick_params.physical.z);let root_index=job/max_samples;
    let root=brick_roots_buffer[root_index];if root.valid.x==0u {return result;}
    let plant=root.plant;let variant=plant.road_variant_tint.z;
    if variant>=3u {return result;}
    let range=brick_params.ranges[variant];let local=job%max_samples;
    if local>=range.y {return result;}
    let source=brick_source[range.x+local];let anchor=source.anchor_branch.xyz;
    let base=meadow_yaw(anchor,plant.yaw.xy);
    if !meadow_road_anchor_allowed(plant.road_variant_tint.x,bitcast<f32>(plant.road_variant_tint.y),
        base.xz,plant.root_scale.xz+base.xz,meadow.grid.z) {return result;}
    let branch=u32(source.anchor_branch.w);
    if branch>=4u {return result;}
    let pose=meadow_deform_source(source.position_area.xyz,source.shading_normal.xyz,
        source.geometric_normal_roughness.xyz,anchor,plant.root_scale,plant.yaw,
        plant.branches[branch],meadow.previous.yz,false);
    result.position=pose.world;result.area=source.position_area.w*pose.area_scale;
    result.geometric=pose.geometric_normal;result.normal=pose.normal;
    result.roughness=source.geometric_normal_roughness.w;
    result.albedo=source.albedo.rgb*meadow_unpack_tint(plant.road_variant_tint.w)*meadow.edits.rgb;
    result.valid=true;return result;
}
@compute @workgroup_size(64)
fn brick_clear(@builtin(global_invocation_id) id:vec3<u32>) {
    let index=brick_job(id);
    if index<brick_params.memory.x {
        atomicStore(&brick_scratch[index],0u);
        atomicStore(&brick_scratch[brick_params.memory.z+index],0u);
        var empty:VolumeCell;brick_cells[index]=empty;
    }
    if index<brick_params.control.y {
        atomicStore(&brick_status[index*4u],brick_params.control.x);
        atomicStore(&brick_status[index*4u+1u],0u);
        atomicStore(&brick_status[index*4u+2u],0u);
        atomicStore(&brick_status[index*4u+3u],0u);
    }
}
@compute @workgroup_size(64)
fn brick_roots(@builtin(global_invocation_id) id:vec3<u32>) {
    let index=brick_job(id);if index>=brick_params.control.z {return;}
    let per_group=u32(brick_params.physical.w);let group=index/per_group;
    var root:BrickRoot;
    if abs(meadow.field.w-brick_params.physical.x)>1e-6 {
        brick_error(group,32u);brick_roots_buffer[index]=root;return;
    }
    // Covers even roots OUTSIDE the enumerated halo. Rigid blade rotation
    // preserves root-relative length; vertical terrain shear leaves X/Z
    // unchanged. The accepted source growth field is bounded by 1.3.
    let global_support=meadow.support.x+meadow.support.y*max(meadow.shape.x,0.0)*1.3;
    if !(global_support<=brick_params.physical.y) {
        brick_error(group,1u);brick_roots_buffer[index]=root;return;
    }
    let spec=brick_groups[group];let local=index%per_group;
    let cell=spec.roots.xy+vec2<i32>(i32(local%u32(spec.roots.z)),i32(local/u32(spec.roots.z)));
    let candidate=meadow_generate_cell(cell,true);
    if candidate.valid {
        root.plant=candidate.plant;root.valid.x=1u;
    }
    brick_roots_buffer[index]=root;
}
@compute @workgroup_size(64)
fn brick_bin(@builtin(global_invocation_id) id:vec3<u32>) {
    let job=brick_job(id);if job>=brick_params.control.w {return;}
    let root=job/u32(brick_params.physical.z);let group=root/u32(brick_params.physical.w);
    if atomicLoad(&brick_status[group*4u+2u])!=0u {return;}
    let sample=brick_source_world(job);if !sample.valid {return;}
    if !all(abs(sample.position)<vec3<f32>(1e15)) || !(sample.area>=0.0 && sample.area<1e10)
        || !all(abs(sample.normal)<=vec3<f32>(1.01)) || !all(abs(sample.geometric)<=vec3<f32>(1.01))
        || !all(abs(sample.albedo)<vec3<f32>(1e10)) || !(sample.roughness>=0.0 && sample.roughness<=1.0) {
        brick_error(group,8u);return;
    }
    let level=brick_levels[group*4u];let dims=level.dims_offset.xyz;
    let q=(sample.position-level.origin_cell.xyz)/level.origin_cell.w;
    // Unique ownership: neighbouring halo roots contribute only here if
    // their current sample belongs to this half-open horizontal interior.
    let owner_key=vec2<i32>(floor(sample.position.xz/(brick_params.physical.x*8.0)));
    let spec=brick_groups[group];
    let interior_root=spec.roots.xy+vec2<i32>((spec.roots.z-8)/2);
    if any(owner_key!=interior_root/8) {return;}
    if q.y<0.0 || q.y>=f32(dims.y) {brick_error(group,2u);return;}
    // Global integer ownership above is unique even if adjacent f32 bounds
    // differ by an ulp. Clamp only those XZ boundary-rounding coordinates.
    let coord=vec3<u32>(clamp(floor(q),vec3<f32>(0.0),vec3<f32>(dims-vec3<u32>(1u))));
    let cell_index=brick_index(coord,level);
    let previous=atomicExchange(&brick_scratch[cell_index],job+1u);
    atomicStore(&brick_scratch[brick_params.memory.y+job],previous);
    atomicAdd(&brick_status[group*4u+3u],1u);
}
fn brick_direction(index:u32)->vec3<f32> {
    let directions=array<vec3<f32>,13>(vec3<f32>(1,0,0),vec3<f32>(0,1,0),vec3<f32>(0,0,1),
        vec3<f32>(1,1,0),vec3<f32>(1,-1,0),vec3<f32>(1,0,1),vec3<f32>(1,0,-1),
        vec3<f32>(0,1,1),vec3<f32>(0,1,-1),vec3<f32>(1,1,1),vec3<f32>(1,1,-1),
        vec3<f32>(1,-1,1),vec3<f32>(1,-1,-1));
    return normalize(directions[index]);
}
fn brick_finish(moments:VolumeCell,area:f32)->VolumeCell {
    var result=moments;
    if area<=0.0 {return result;}
    result.albedo_rough/=area;
    for(var lobe=0u;lobe<6u;lobe++) {
        let mass=result.normals[lobe].w;
        if mass>0.0 {result.normals[lobe]=vec4<f32>(result.normals[lobe].xyz/mass,mass/area);}
    }
    result.extinction[3].y=1.0;return result;
}
@compute @workgroup_size(64)
fn brick_reduce(@builtin(global_invocation_id) id:vec3<u32>) {
    let group=id.y;if group>=brick_params.control.y {return;}
    let level=brick_levels[group*4u];if id.x>=brick_count(level) {return;}
    if atomicLoad(&brick_status[group*4u+2u])!=0u {return;}
    let cell_index=level.dims_offset.w+id.x;var link=atomicLoad(&brick_scratch[cell_index]);
    var moments:VolumeCell;var area=0.0;var count=0u;
    let volume=level.origin_cell.w*level.origin_cell.w*level.origin_cell.w;
    loop {
        if link==0u {break;}
        if count>=brick_params.memory.w || link>brick_params.control.w {brick_error(group,4u);return;}
        let job=link-1u;let sample=brick_source_world(job);let a=sample.area;
        area+=a;moments.albedo_rough+=vec4<f32>(sample.albedo,sample.roughness)*a;
        for(var axis=0u;axis<13u;axis++) {
            moments.extinction[axis/4u][axis%4u]+=a*abs(dot(sample.geometric,brick_direction(axis)))/volume;
        }
        // Thin, two-sided source: actual conditional normals in both signed
        // hemispheres. Squared-component partition sums exactly to area.
        for(var axis=0u;axis<3u;axis++) {
            let mass=a*sample.normal[axis]*sample.normal[axis]*0.5;
            let positive=select(-sample.normal,sample.normal,sample.normal[axis]>=0.0);
            moments.normals[axis*2u]+=vec4<f32>(positive*mass,mass);
            moments.normals[axis*2u+1u]+=vec4<f32>(-positive*mass,mass);
        }
        link=atomicLoad(&brick_scratch[brick_params.memory.y+job]);count++;
    }
    brick_cells[cell_index]=brick_finish(moments,area);
    atomicStore(&brick_scratch[brick_params.memory.z+cell_index],bitcast<u32>(area));
}
fn brick_reduce_mip(id:vec3<u32>,mip:u32) {
    let group=id.y;if group>=brick_params.control.y {return;}
    let level=brick_levels[group*4u+mip];if id.x>=brick_count(level) {return;}
    if atomicLoad(&brick_status[group*4u+2u])!=0u {return;}
    let child_level=brick_levels[group*4u+mip-1u];let base=brick_coord(id.x,level.dims_offset.xyz)*2u;
    var moments:VolumeCell;var area=0.0;
    for(var child=0u;child<8u;child++) {
        let coord=base+vec3<u32>(child&1u,(child>>1u)&1u,(child>>2u)&1u);
        let source=brick_index(coord,child_level);let cell=brick_cells[source];
        let a=bitcast<f32>(atomicLoad(&brick_scratch[brick_params.memory.z+source]));
        area+=a;moments.albedo_rough+=cell.albedo_rough*a;
        for(var axis=0u;axis<13u;axis++) {moments.extinction[axis/4u][axis%4u]+=cell.extinction[axis/4u][axis%4u]*0.125;}
        for(var lobe=0u;lobe<6u;lobe++) {
            let mass=cell.normals[lobe].w*a;
            moments.normals[lobe]+=vec4<f32>(cell.normals[lobe].xyz*mass,mass);
        }
    }
    let cell_index=level.dims_offset.w+id.x;brick_cells[cell_index]=brick_finish(moments,area);
    atomicStore(&brick_scratch[brick_params.memory.z+cell_index],bitcast<u32>(area));
}
@compute @workgroup_size(64) fn brick_mip1(@builtin(global_invocation_id) id:vec3<u32>) {brick_reduce_mip(id,1u);}
@compute @workgroup_size(64) fn brick_mip2(@builtin(global_invocation_id) id:vec3<u32>) {brick_reduce_mip(id,2u);}
@compute @workgroup_size(64) fn brick_mip3(@builtin(global_invocation_id) id:vec3<u32>) {brick_reduce_mip(id,3u);}
@compute @workgroup_size(64)
fn brick_publish(@builtin(global_invocation_id) id:vec3<u32>) {
    let group=brick_job(id);if group>=brick_params.control.y {return;}
    if atomicLoad(&brick_status[group*4u+2u])==0u {
        atomicStore(&brick_status[group*4u+1u],brick_params.control.x);
    }
}
