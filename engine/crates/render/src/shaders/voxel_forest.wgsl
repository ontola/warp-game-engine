// Appended to the shared physical-lighting helpers in shade.wgsl.
// This is fractional surface-density integration, not opaque voxel cubes.
// Cell optics come from foliage_volume.wgsl. These resources, owner poses and
// bounded scheduling remain specific to the forest prototype.
struct VolumeTree {
    inverse: array<vec4<f32>,3>,
    bounds_min: vec4<f32>, bounds_max: vec4<f32>, identity: vec4<u32>,
};
// proxy: x first whole-plant proxy tree (Scene::voxel_proxies trail the
// forest), y its pixel error.
struct VolumeConfig { size_count: vec4<u32>, march: vec4<f32>, dispatch: vec4<u32>, proxy: vec4<f32> };
@group(2) @binding(0) var<storage,read> volume_levels: array<VolumeLevel>;
@group(2) @binding(1) var<storage,read> volume_indices: array<u32>;
@group(2) @binding(2) var<storage,read> volume_cells: array<PackedVolumeCell>;
@group(2) @binding(3) var<storage,read> volume_trees: array<VolumeTree>;
@group(2) @binding(4) var<storage,read_write> volume_tiles: array<atomic<u32>>;
@group(2) @binding(5) var<storage,read_write> volume_stats: array<atomic<u32>>;
@group(2) @binding(6) var volume_output: texture_storage_2d<rgba16float,write>;
@group(2) @binding(7) var<uniform> volume_config: VolumeConfig;
@group(2) @binding(8) var<storage,read_write> volume_jobs: array<atomic<u32>>;
@group(2) @binding(9) var<storage,read> volume_selection_words: array<u32>;
fn volume_selection_at(i:u32)->vec4<u32> { return selection_unpack(volume_selection_words[i]); }
// Packed group selection (aggregate_lod.wgsl writes it): one u32 per group,
// runtime-sized storage table. Bits 0-1 choice, 2-7
// voxel level (63: none), 8-9 sun mode, 16-31 the triangle error as f16.
fn selection_unpack(w:u32)->vec4<u32> {
    let y=(w>>2u)&63u;
    return vec4<u32>(w&3u,select(y,0xffffffffu,y==63u),bitcast<u32>(unpack2x16float(w>>16u).x),(w>>8u)&3u);
}

// VOLUME_MOTION_BEGIN
// Included only in the temporal pipeline; native specialization removes all
// these blocks, including the extra bindings and every moment calculation.
@group(2) @binding(10) var volume_motion_output: texture_storage_2d<rgba32float,write>;
@group(2) @binding(11) var<storage,read> volume_previous_instances: array<Instance>;
// VOLUME_MOTION_END

// Shared immutable bounds/references plus a per-view deduplication mask.
@group(2) @binding(12) var<storage,read> volume_bricks: array<u32>;
// Deduplication words follow the 22 diagnostic words in volume_stats.
// Sharing this allocation keeps the temporal pipeline within 16 storage bindings.
const VOLUME_DRAW_WORD: u32 = 22u;
const VOLUME_MASK_WORD: u32 = 36u;
const VOLUME_BRICK_VISIBILITY: bool = false;
const VOLUME_TILE: u32 = 16u;
const VOLUME_CANDIDATES: u32 = 64u;
const VOLUME_MAX_STEPS: u32 = 4096u;
const VOLUME_ACTIVE: u32 = 16u;
// Host enables the accepted fix by default; env=0 retains this raw baseline.
const VOLUME_EMPTY_CEILING: bool = false;
// Portable minimum workgroup-count limit. Four 8x8 groups cover a 16x16 tile.
const VOLUME_JOB_ROW: u32 = 65535u;

// Per-ray u32 sums are bounded by 4096 steps x4096 volumes x16 cell fetches.
// Frame totals can exceed 2^32, so atomically add low-word contributions with
// an exact carry. No atomics occur inside the sample/march loops.
fn volume_add_total(low_index:u32,value:u32) {
    if value==0u {return;}
    let previous=atomicAdd(&volume_stats[low_index],value);
    if previous>0xffffffffu-value {atomicAdd(&volume_stats[low_index+1u],1u);}
}

// Project each selected volume once. The host clears tile counters before
// this pass; the following pass sees all atomic appends. Overflow is counted
// even after storage fills, so integration can scan the complete volume set.
@compute @workgroup_size(64)
fn voxel_bins(@builtin(global_invocation_id) id:vec3<u32>) {
    let work=id.x+id.y*65535u*64u;
    var i=work;var brick_min=vec3<f32>(0.0);var brick_max=vec3<f32>(0.0);
    if VOLUME_BRICK_VISIBILITY {
        if work>=volume_bricks[0] {return;}
        let reference=volume_bricks[1]+work*4u;i=volume_bricks[reference];
        if volume_selection_at(i).x!=1u || volume_selection_at(i).y!=volume_bricks[reference+1u] {return;}
        let brick=4u+volume_bricks[reference+2u]*8u;
        brick_min=vec3<f32>(bitcast<f32>(volume_bricks[brick]),bitcast<f32>(volume_bricks[brick+1u]),bitcast<f32>(volume_bricks[brick+2u]));
        brick_max=vec3<f32>(bitcast<f32>(volume_bricks[brick+4u]),bitcast<f32>(volume_bricks[brick+5u]),bitcast<f32>(volume_bricks[brick+6u]));
    } else {if i>=volume_config.size_count.w || volume_selection_at(i).x!=1u {return;}}
    let tile_dims=(volume_config.size_count.xy+vec2<u32>(15u))/16u;
    let tree=volume_trees[i];
    var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var in_front=0u;
    for(var corner=0u;corner<8u;corner++) {
        var p=vec3<f32>(
            select(tree.bounds_min.x,tree.bounds_max.x,(corner&1u)!=0u),
            select(tree.bounds_min.y,tree.bounds_max.y,(corner&2u)!=0u),
            select(tree.bounds_min.z,tree.bounds_max.z,(corner&4u)!=0u));
        if VOLUME_BRICK_VISIBILITY {
            let local=vec4<f32>(select(brick_min.x,brick_max.x,(corner&1u)!=0u),
                select(brick_min.y,brick_max.y,(corner&2u)!=0u),
                select(brick_min.z,brick_max.z,(corner&4u)!=0u),1.0);
            let owner=instances[tree.identity.x];
            p=vec3<f32>(dot(owner.r0,local),dot(owner.r1,local),dot(owner.r2,local));
        }
        let clip=view.view_proj*vec4<f32>(p,1.0);
        if clip.w>view.params.y {
            in_front++;
            let pixel=(clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5)*vec2<f32>(volume_config.size_count.xy);
            lo=min(lo,pixel);hi=max(hi,pixel);
        }
    }
    if in_front==0u {return;}
    var first=vec2<u32>(0u);var last=tile_dims-vec2<u32>(1u);
    if in_front==8u {
        // Match the previous inclusive box/tile overlap, including a box
        // exactly on a tile edge and a partially filled final screen tile.
        if any(hi<vec2<f32>(0.0)) || any(lo>vec2<f32>(tile_dims*VOLUME_TILE)) {return;}
        first=vec2<u32>(clamp(ceil(lo/f32(VOLUME_TILE))-1.0,vec2<f32>(0.0),vec2<f32>(last)));
        last=vec2<u32>(clamp(floor(hi/f32(VOLUME_TILE)),vec2<f32>(0.0),vec2<f32>(last)));
        if any(first>last) {return;}
    }
    // Near-plane intersections conservatively bin to the whole screen.
    for(var y=first.y;y<=last.y;y++) {
        for(var x=first.x;x<=last.x;x++) {
            let start=(y*tile_dims.x+x)*(VOLUME_CANDIDATES+1u);
            // Multiple bricks may project to the same tile. Append the source
            // volume exactly once, including overlapping reconstruction halos.
            if VOLUME_BRICK_VISIBILITY {
                let mask=(y*tile_dims.x+x)*((volume_config.size_count.w+31u)/32u)+i/32u;
                let bit=1u<<(i%32u);
                if (atomicOr(&volume_stats[VOLUME_MASK_WORD+mask],bit)&bit)!=0u {continue;}
            }
            let slot=atomicAdd(&volume_tiles[start],1u);
            if slot<VOLUME_CANDIDATES {atomicStore(&volume_tiles[start+1u+slot],i);}
        }
    }
}

// One invocation finalizes each tile. Atomic append arrival order is arbitrary;
// sorting the complete nonoverflow list by source ID restores the previous
// deterministic summation/tie order. Overflow ignores the stored subset.
@compute @workgroup_size(8,8)
fn voxel_tiles(@builtin(global_invocation_id) id:vec3<u32>) {
    let tile_dims=(volume_config.size_count.xy+vec2<u32>(15u))/16u;
    if any(id.xy>=tile_dims) {return;}
    let start=(id.y*tile_dims.x+id.x)*(VOLUME_CANDIDATES+1u);
    let count=atomicLoad(&volume_tiles[start]);
    if count<=VOLUME_CANDIDATES {
        for(var j=1u;j<count;j++) {
            let value=atomicLoad(&volume_tiles[start+1u+j]);var k=j;
            for(var move_count=0u;move_count<VOLUME_CANDIDATES;move_count++) {
                if k==0u {break;}
                let previous=atomicLoad(&volume_tiles[start+k]);
                if previous<=value {break;}
                atomicStore(&volume_tiles[start+1u+k],previous);k--;
            }
            atomicStore(&volume_tiles[start+1u+k],value);
        }
    }
    atomicMax(&volume_stats[4],count);
    if count>VOLUME_CANDIDATES {atomicAdd(&volume_stats[0],1u);}
    if count>0u {
        atomicAdd(&volume_stats[12],1u);
        let extent=min(vec2<u32>(16u),volume_config.size_count.xy-id.xy*16u);
        atomicAdd(&volume_stats[13],extent.x*extent.y);
    }
    if count>0u && volume_config.dispatch.x!=0u {
        let job=atomicAdd(&volume_jobs[3],1u);
        atomicStore(&volume_jobs[4u+job],id.y*tile_dims.x+id.x);
        if VOLUME_BRICK_VISIBILITY {atomicStore(&volume_stats[VOLUME_MASK_WORD+id.y*tile_dims.x+id.x],job+1u);}
        atomicMax(&volume_jobs[0],min(job+1u,VOLUME_JOB_ROW));
        atomicMax(&volume_jobs[1],4u*(job/VOLUME_JOB_ROW+1u));
        atomicStore(&volume_jobs[2],1u);
    }
}

@compute @workgroup_size(8,8)
fn voxel_clear(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=volume_config.size_count.xy) {return;}
    textureStore(volume_output,vec2<i32>(id.xy),vec4<f32>(0.0));
    // VOLUME_MOTION_BEGIN
    textureStore(volume_motion_output,vec2<i32>(id.xy),vec4<f32>(0.0));
    // VOLUME_MOTION_END
}

fn volume_interval(tree:VolumeTree,origin:vec3<f32>,direction:vec3<f32>,limit:f32)->vec2<f32> {
    var enter=0.0;
    var leave=limit;
    for(var axis=0u;axis<3u;axis++) {
        if abs(direction[axis])<1e-8 {
            if origin[axis]<tree.bounds_min[axis] || origin[axis]>tree.bounds_max[axis] {return vec2<f32>(1.0,0.0);}
        } else {
            let a=(tree.bounds_min[axis]-origin[axis])/direction[axis];
            let b=(tree.bounds_max[axis]-origin[axis])/direction[axis];
            enter=max(enter,min(a,b)); leave=min(leave,max(a,b));
        }
    }
    return vec2<f32>(enter,leave);
}

// Resource-dependent adapters deliberately stay out of the shared module.
// 4^3 aggregate-brick directory, or the legacy dense level encoding.
fn volume_cell_address(level:VolumeLevel,c:vec3<u32>)->u32 {
    let base=level.dims_offset.w&0x7fffffffu;
    if (level.dims_offset.w&0x80000000u)==0u {
        return volume_indices[base+c.x+level.dims_offset.x*(c.y+level.dims_offset.y*c.z)];
    }
    let bd=(level.dims_offset.xyz+vec3<u32>(3u))/4u;let b=c/4u;
    let address=volume_indices[base+b.x+bd.x*(b.y+bd.y*b.z)];
    if address==0u {return 0u;}
    let k=c%4u;
    return volume_indices[address-1u+k.x+4u*k.y+16u*k.z];
}
fn volume_sample_level(level:VolumeLevel,local:vec3<f32>,direction:VolumeDirection)->VolumeSample {
    let grid=(local-level.origin_cell.xyz)/level.origin_cell.w-0.5;
    let base=vec3<i32>(floor(grid));
    let fraction=fract(grid);
    var result:VolumeSample;
    for(var corner=0u;corner<8u;corner++) {
        let offset=vec3<i32>(i32(corner&1u),i32((corner>>1u)&1u),i32((corner>>2u)&1u));
        let coord=base+offset;
        if any(coord<vec3<i32>(0)) || any(coord>=vec3<i32>(level.dims_offset.xyz)) {continue;}
        let weight3=select(1.0-fraction,fraction,offset!=vec3<i32>(0));
        let weight=weight3.x*weight3.y*weight3.z;
        let c=vec3<u32>(coord);
        let index=volume_cell_address(level,c);
        if index==0u {continue;}
        let cell=unpack_volume_cell(volume_cells[index-1u]);
        let sigma=volume_sigma(cell,direction)*weight;
        result=volume_accumulate_cell(result,cell,sigma);
    }
    return volume_normalize_sample(result);
}

// Reconstruct extinction and extinction-weighted material moments, not
// opacity or straight colour. An empty coarse cell must not darken a fine
// cell's material, and opposing normal lobes must not cancel to a flat normal.
fn volume_sample_local(tree:VolumeTree,local:vec3<f32>,direction:VolumeDirection,lod:f32)->VolumeSample {
    let lower=u32(floor(lod));
    var a=volume_sample_level(volume_levels[tree.identity.y+lower],local,direction);
    let blend=fract(lod);
    if blend<=0.0 || lower+1u>=tree.identity.z {return a;}
    let b=volume_sample_level(volume_levels[tree.identity.y+lower+1u],local,direction);
    return volume_blend_mips(a,b,blend);
}

// Bounded actual ray intersections, rather than projected tile boxes.
// More intersections use the complete scan path; none are discarded.
struct VolumeInterval { time:vec2<f32>,index:u32,padding:u32 };
struct VolumeRay {
    origin_exit:vec4<f32>,
    direction_scale:vec4<f32>,
    weights:vec3<f32>,identity:u32,
    // End of current reconstruction brick, and whether all its fine support
    // is empty. Cached until the ray crosses that boundary.
    brick:vec2<f32>,
    // x: conservative finest-cell projected diameter * ray distance.
    // y: half finest-cell size divided by this ray's current inverse stretch.
    // z: finest-cell bounding radius / forward ray cosine, for conservative
    // finite-cell perspective support (not just its centre Jacobian).
    lod_step:vec4<f32>,
};
fn volume_projected_cell(tree:VolumeTree,direction:vec3<f32>)->vec2<f32> {
    let owner=instances[tree.identity.x];
    let r0=owner.r0.xyz;let r1=owner.r1.xyz;let r2=owner.r2.xyz;
    let frobenius=sqrt(dot(r0,r0)+dot(r1,r1)+dot(r2,r2));
    let norm_inf=max(dot(abs(r0),vec3<f32>(1.0)),max(dot(abs(r1),vec3<f32>(1.0)),dot(abs(r2),vec3<f32>(1.0))));
    let sums=abs(r0)+abs(r1)+abs(r2);
    let norm_one=max(sums.x,max(sums.y,sums.z));
    let stretch=min(frobenius,sqrt(norm_inf*norm_one));
    // Perspective Jacobian evaluated along this ray. Including the off-axis
    // -ndc*row_w term prevents corners of a wide-FOV view selecting too coarse
    // a mip. Frobenius bounds its largest singular value; sqrt(3) bounds the
    // diameter of one local cubic cell under arbitrary rotation and shear.
    let row_w=vec3<f32>(view.view_proj[0].w,view.view_proj[1].w,view.view_proj[2].w);
    let row_x=vec3<f32>(view.view_proj[0].x,view.view_proj[1].x,view.view_proj[2].x);
    let row_y=vec3<f32>(view.view_proj[0].y,view.view_proj[1].y,view.view_proj[2].y);
    let depth_rate=max(dot(row_w,direction),1e-6);
    let ndc=vec2<f32>(dot(row_x,direction),dot(row_y,direction))/depth_rate;
    let jx=(row_x-ndc.x*row_w)*(0.5*f32(volume_config.size_count.x));
    let jy=(row_y-ndc.y*row_w)*(0.5*f32(volume_config.size_count.y));
    let pixel_scale=sqrt(dot(jx,jx)+dot(jy,jy))/depth_rate;
    let diameter=volume_levels[tree.identity.y].origin_cell.w*1.73205081*stretch;
    return vec2<f32>(diameter*pixel_scale,0.5*diameter/depth_rate);
}
fn volume_ray(index:u32,leave:f32,direction:vec3<f32>)->VolumeRay {
    let tree=volume_trees[index];
    let p=vec4<f32>(view.cam.xyz,1.0);
    let d=vec4<f32>(direction,0.0);
    let local_origin=vec3<f32>(dot(tree.inverse[0],p),dot(tree.inverse[1],p),dot(tree.inverse[2],p));
    let local_direction=vec3<f32>(dot(tree.inverse[0],d),dot(tree.inverse[1],d),dot(tree.inverse[2],d));
    let scale=length(local_direction);
    let angular=volume_direction(local_direction/max(scale,1e-8));
    let identity=(index<<12u)|angular.indices.x|(angular.indices.y<<4u)|(angular.indices.z<<8u);
    var lod_step=vec4<f32>(0.0,volume_levels[tree.identity.y].origin_cell.w*0.5/max(scale,1e-8),0.0,0.0);
    if volume_config.march.w>0.0 {
        let projected=volume_projected_cell(tree,direction);
        lod_step=vec4<f32>(projected.x,volume_levels[tree.identity.y].origin_cell.w*0.5/max(scale,1e-8),projected.y,0.0);
    }
    return VolumeRay(vec4<f32>(local_origin,leave),vec4<f32>(local_direction,scale),
        angular.weights,identity,vec2<f32>(-1.0,0.0),lod_step);
}
fn volume_lod(ray:VolumeRay,t:f32)->f32 {
    let tree=volume_trees[ray.identity>>12u];
    let selected=volume_selection_at(ray.identity>>12u);
    // Aggregate errors were measured for this exact representation. Sampling
    // a different finer mip would not implement the selected candidate.
    if selected.w==1u {return f32(min(tree.identity.z-1u,selected.y));}
    if volume_config.march.w<=0.0 {return 0.0;}
    // Solve target = diameter_px*2^lod / (distance-radius*2^lod).
    // Cells touching the near plane therefore cannot pick a coarse level.
    let denominator=ray.lod_step.x+volume_config.march.w*ray.lod_step.z;
    let footprint=volume_config.march.w*max(t,1e-6)/max(denominator,1e-8);
    let ceiling=min(tree.identity.z-1u,volume_selection_at(ray.identity>>12u).y);
    return clamp(log2(max(footprint,1e-8)),0.0,f32(ceiling));
}
fn volume_ray_step(ray:VolumeRay,t:f32)->f32 {
    if volume_config.march.w<=0.0 && volume_selection_at(ray.identity>>12u).w!=1u {return volume_config.march.x;}
    return ray.lod_step.y*exp2(floor(volume_lod(ray,t)));
}
fn volume_ray_direction(ray:VolumeRay)->VolumeDirection {
    return VolumeDirection(vec3<u32>(ray.identity&15u,(ray.identity>>4u)&15u,(ray.identity>>8u)&15u),ray.weights);
}
fn volume_index(level:VolumeLevel,coord:vec3<i32>)->u32 {
    if any(coord<vec3<i32>(0)) || any(coord>=vec3<i32>(level.dims_offset.xyz)) {return 0u;}
    let c=vec3<u32>(coord);
    return volume_cell_address(level,c);
}
fn volume_empty_brick_level(ray:VolumeRay,t:f32,relative:u32)->vec2<f32> {
    let tree=volume_trees[ray.identity>>12u];
    let fine=volume_levels[tree.identity.y+relative];
    let coarse=volume_levels[tree.identity.y+min(relative+2u,tree.identity.z-1u)];
    let ratio=coarse.origin_cell.w/fine.origin_cell.w;
    let grid=(ray.origin_exit.xyz+ray.direction_scale.xyz*t-fine.origin_cell.xyz)/fine.origin_cell.w-0.5;
    let rate=ray.direction_scale.xyz/fine.origin_cell.w;
    var brick=vec3<i32>(floor(grid/ratio));
    var leave=ray.origin_exit.w;
    for(var axis=0u;axis<3u;axis++) {
        // Exactly on a face, a negative ray enters the previous brick. No
        // epsilon jump: near a face precision failure just disables skipping.
        if rate[axis]<0.0 && grid[axis]==f32(brick[axis])*ratio {brick[axis]--;}
        if rate[axis]!=0.0 {
            let face=f32(brick[axis]+select(0,1,rate[axis]>0.0))*ratio;
            leave=min(leave,t+(face-grid[axis])/rate[axis]);
        }
    }
    if leave<=t {return vec2<f32>(t,0.0);}
    // q=floor(grid/B): the two fine trilinear neighbours on every axis map
    // only to coarse q or q+1. Host validates that each occupied fine cell
    // has an occupied coarse parent before enabling this exact empty test.
    for(var corner=0u;corner<8u;corner++) {
        let offset=vec3<i32>(i32(corner&1u),i32((corner>>1u)&1u),i32((corner>>2u)&1u));
        if volume_index(coarse,brick+offset)!=0u {return vec2<f32>(leave,0.0);}
    }
    return vec2<f32>(leave,1.0);
}
fn volume_empty_brick(ray:VolumeRay,t:f32)->vec2<f32> {
    if volume_selection_at(ray.identity>>12u).w==1u {
        return volume_empty_brick_level(ray,t,u32(volume_lod(ray,t)));
    }
    if volume_config.march.w<=0.0 {return volume_empty_brick_level(ray,t,0u);}
    let tree=volume_trees[ray.identity>>12u];
    let relative=u32(floor(volume_lod(ray,t)));
    let a=volume_empty_brick_level(ray,t,relative);
    // Once reconstruction reaches its selected ceiling, no higher level can
    // contribute anywhere in this brick. An unclamped next-LOD time may then
    // already be behind t, causing needless rechecks and disabling empty skips.
    if VOLUME_EMPTY_CEILING {
        let ceiling=min(tree.identity.z-1u,volume_selection_at(ray.identity>>12u).y);
        if relative>=ceiling {return a;}
    }
    if relative+1u>=tree.identity.z {return a;}
    // Test both mip supports even when the upper weight is currently zero:
    // it can become nonzero before the next brick boundary. Invalidate the
    // cache when the lower mip changes; no skipped interval loses density.
    let b=volume_empty_brick_level(ray,t,relative+1u);
    let next_lod=(ray.lod_step.x/volume_config.march.w+ray.lod_step.z)*exp2(f32(relative+1u));
    return vec2<f32>(min(min(a.x,b.x),next_lod),min(a.y,b.y));
}

fn volume_leaf_headlights(p:vec3<f32>,n:vec3<f32>,v:vec3<f32>,base:vec3<f32>,rough:f32,leaf_fraction:f32)->vec3<f32> {
    var result=vec3<f32>(0.0);
    for(var index=0u;index<incident_light_count(p);index++) {
        // Match source Meadow's two-sided receiver bias and coloured diffuse
        // reflection/transmission. Cone, range and blocker visibility remain
        // authoritative: zero incident irradiance contributes exactly zero.
        let towards=incident_light_position(index)-p;
        let bias=n*select(-1.0,1.0,dot(n,towards)>=0.0);
        let lamp=incident_light_sample(index,p,bias);
        if all(lamp.irradiance==vec3<f32>(0.0)) {continue;}
        result+=lamp.irradiance*volume_thin_leaf_direct(base,n,v,lamp.direction,rough,leaf_fraction);
    }
    return result;
}

fn volume_radiance(sample:VolumeSample,tree:VolumeTree,p:vec3<f32>,v:vec3<f32>,shadow:f32,sky_vis:f32)->vec3<f32> {
    let owner=instances[tree.identity.x];
    // Asset albedo already includes the source material tint.
    var base=sample.albedo;
    // Procedural thin foliage already carries its source colours. A hash of
    // the animated part translation would make its colour shimmer in wind.
    let procedural=tree.identity.w==0xffffffffu;
    if !procedural {base*=0.82+0.3*hash2(vec2<f32>(owner.r0.w,owner.r2.w));}
    let sun_e=light.sun_color.rgb*light.sun_dir.w*shadow;
    let bounce=gi.bounce.rgb*(gi.bounce.w*sky_vis+light.sun_dir.w*max(light.sun_dir.y,0.0)*shadow);
    let rough=clamp(sample.roughness,0.04,1.0);
    var direct=vec3<f32>(0.0);var indirect=vec3<f32>(0.0);var specular=vec3<f32>(0.0);
    for(var k=0u;k<VOLUME_NORMAL_COUNT;k++) {
        let weight=sample.normals[k].w;
        if weight<=0.0 {continue;}
        // Transform the authored normal moment by inverse transpose. Folding
        // the result matches the source two-sided material; the axis only
        // partitions moments and never substitutes a new lighting normal.
        let local_n=sample.normals[k].xyz;
        let world_n=tree.inverse[0].xyz*local_n.x+tree.inverse[1].xyz*local_n.y+tree.inverse[2].xyz*local_n.z;
        if dot(world_n,world_n)<=1e-16 {continue;}
        var n=normalize(world_n);
        if dot(n,v)<0.0 {n=-n;}
        // VOLUME_ANTIPODAL_RESPONSE_BEGIN
        if procedural {
            direct+=volume_leaf_headlights(p,n,v,base,rough,sample.leaf_fraction)*weight;
            // Integrate the authored normal lobes without random normal
            // selection or a new global up-normal at the representation swap.
            let front=sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis);
            let back=sky_irradiance(-n)*sky_vis+bounce*(1.0-sky_vis);
            let refl=reflect(-v,n);
            let env=mix(bounce/PI,environment(refl,rough,bounce),sky_vis);
            let response=thin_foliage_response(base,n,v,rough,sample.leaf_fraction,sun_e,front,back,env);
            direct+=response.direct*weight;indirect+=response.indirect*weight;specular+=response.specular*weight;
            continue;
        }
        direct+=headlight_radiance(p,n,v,base/PI,rough,vec3<f32>(0.01))*weight;
        let ndl=dot(n,light.sun_dir.xyz);
        let ambient=(sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis))*0.85;
        let refl=reflect(-v,n);
        let env=mix(bounce/PI,environment(refl,rough,bounce),sky_vis);
        let back=vec3<f32>(0.175,0.21,0.1)*max(-ndl,0.0);
        direct+=(base/PI*sun_e*max(ndl,0.0)+sun_e*back/PI+sun_e*ggx(n,v,light.sun_dir.xyz,rough,vec3<f32>(0.01)))*weight;
        indirect+=base/PI*ambient*weight;
        specular+=env*env_brdf(vec3<f32>(0.01),rough,max(dot(n,v),1e-4))*0.85*weight;
        // VOLUME_ANTIPODAL_RESPONSE_END
    }
    var rad=apply_distance_fog(direct+indirect+specular,distance(p,view.cam.xyz),-v);
    if light.sky.z>0.5&&light.sky.z<1.5 {rad=vec3<f32>(sky_vis)*6000.0;}
    else if light.sky.z>1.5&&light.sky.z<2.5 {rad=base*6000.0;}
    else if light.sky.z>2.5&&light.sky.z<3.5 {rad=direct;}
    else if light.sky.z>3.5&&light.sky.z<4.5 {rad=indirect;}
    else if light.sky.z>4.5&&light.sky.z<5.5 {rad=specular;}
    return rad;
}

// SHARED_VOLUME_HELPERS_END

// Opt-in global interval stream. It extends volume_jobs, preserving every
// resource binding and all immutable cell/tree ABIs. Header z identifies four
// counters; w is the admitted pixel-slot count. Each slot stores offset/count,
// followed by bounded 16-byte (enter,exit,volume,pad) records. Metadata overflow
// and record overflow always use a complete scan for that pixel.
const VOLUME_INTERVAL_STREAM_WORDS:u32=8388608u;
const VOLUME_STREAM_FALLBACK:u32=0xffffffffu;
struct VolumePixelRay {direction:vec3<f32>,near:f32,limit:f32};
fn volume_pixel_ray(id:vec2<u32>)->VolumePixelRay {
    let ndc=(vec2<f32>(id)+0.5)/vec2<f32>(volume_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let direction=normalize(far.xyz/far.w-view.cam.xyz);
    let near_point=view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let near_distance=distance(view.cam.xyz,near_point.xyz/near_point.w);
    let depth=textureLoad(depth_tex,vec2<i32>(id),0);var limit=1e30;
    if depth>0.0 {let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);limit=distance(view.cam.xyz,hit.xyz/hit.w);}
    return VolumePixelRay(direction,near_distance,limit);
}
fn volume_stream_pool()->u32 {return volume_config.dispatch.z+4u+volume_config.dispatch.w*2u;}
fn volume_stream_capacity()->u32 {return (VOLUME_INTERVAL_STREAM_WORDS-4u-volume_config.dispatch.w*2u)/4u;}
fn volume_stream_read(first:u32,index:u32)->VolumeInterval {
    let at=volume_stream_pool()+(first+index)*4u;
    return VolumeInterval(vec2<f32>(bitcast<f32>(atomicLoad(&volume_jobs[at])),bitcast<f32>(atomicLoad(&volume_jobs[at+1u]))),
        atomicLoad(&volume_jobs[at+2u]),0u);
}
// Compact support visibility. Regions are disjoint in each pose's local
// space, including the trilinear halo; different branches still overlap and
// are integrated jointly. No opaque hit substitutes for fractional density.
fn volume_brick_interval(brick:u32,origin:vec3<f32>,direction:vec3<f32>,near:f32,limit:f32)->vec2<f32> {
    let base=4u+brick*8u;var enter=near;var leave=limit;
    for(var axis=0u;axis<3u;axis++) {
        let lo=bitcast<f32>(volume_bricks[base+axis]);let hi=bitcast<f32>(volume_bricks[base+4u+axis]);
        if abs(direction[axis])<1e-12 {
            // Half-open ownership at lattice boundaries prevents two adjacent
            // regions from integrating the same boundary-parallel ray.
            if origin[axis]<lo || origin[axis]>=hi {return vec2<f32>(1.0,0.0);}
        } else {
            let a=(lo-origin[axis])/direction[axis];let b=(hi-origin[axis])/direction[axis];
            enter=max(enter,min(a,b));leave=min(leave,max(a,b));
        }
    }
    return vec2<f32>(enter,leave);
}
fn volume_stream_write(first:u32,index:u32,value:VolumeInterval) {
    let at=volume_stream_pool()+(first+index)*4u;
    atomicStore(&volume_jobs[at],bitcast<u32>(value.time.x));atomicStore(&volume_jobs[at+1u],bitcast<u32>(value.time.y));
    atomicStore(&volume_jobs[at+2u],value.index);atomicStore(&volume_jobs[at+3u],0u);
}
fn volume_interval_less(a:VolumeInterval,b:VolumeInterval)->bool {
    return a.time.x<b.time.x || (a.time.x==b.time.x && (a.index<b.index || (a.index==b.index && a.time.y<b.time.y)));
}
fn volume_stream_sift(first:u32,start:u32,count:u32) {
    var root=start;
    loop {
        var child=root*2u+1u;if child>=count {break;}
        if child+1u<count && volume_interval_less(volume_stream_read(first,child),volume_stream_read(first,child+1u)) {child++;}
        let value=volume_stream_read(first,root);let larger=volume_stream_read(first,child);
        if !volume_interval_less(value,larger) {break;}
        volume_stream_write(first,root,larger);volume_stream_write(first,child,value);root=child;
    }
}
fn volume_stream_sort(first:u32,count:u32) {
    if count<2u {return;}
    var parent=count/2u;loop {if parent==0u {break;}parent--;volume_stream_sift(first,parent,count);}
    var end=count;loop {if end<=1u {break;}end--;
        let last=volume_stream_read(first,end);volume_stream_write(first,end,volume_stream_read(first,0u));
        volume_stream_write(first,0u,last);volume_stream_sift(first,0u,end);
    }
}
// Bounded reservation, without a globally contended compare-exchange loop.
// Once full, at most one racing reservation per invocation can overshoot:
// <4M catalog references plus 262144*512 gather records, safely below u32.
// Capacity failure flags a complete branch fallback, never an invalid write.
fn volume_stream_reserve(count:u32)->u32 {
    let capacity=volume_stream_capacity();
    if atomicLoad(&volume_jobs[volume_config.dispatch.z])>=capacity {return VOLUME_STREAM_FALLBACK;}
    let first=atomicAdd(&volume_jobs[volume_config.dispatch.z],count);
    if first>capacity || count>capacity-first {return VOLUME_STREAM_FALLBACK;}
    return first;
}
fn volume_brick_rect(brick:u32,tree:VolumeTree)->vec4<i32> {
    let base=4u+brick*8u;let owner=instances[tree.identity.x];
    let low=vec3<f32>(bitcast<f32>(volume_bricks[base]),bitcast<f32>(volume_bricks[base+1u]),bitcast<f32>(volume_bricks[base+2u]));
    let high=vec3<f32>(bitcast<f32>(volume_bricks[base+4u]),bitcast<f32>(volume_bricks[base+5u]),bitcast<f32>(volume_bricks[base+6u]));
    var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var front=0u;
    for(var corner=0u;corner<8u;corner++) {
        let local=vec4<f32>(select(low.x,high.x,(corner&1u)!=0u),select(low.y,high.y,(corner&2u)!=0u),select(low.z,high.z,(corner&4u)!=0u),1.0);
        let p=vec3<f32>(dot(owner.r0,local),dot(owner.r1,local),dot(owner.r2,local));let clip=view.view_proj*vec4<f32>(p,1.0);
        if clip.w>view.params.y {front++;let pixel=(clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5)*vec2<f32>(volume_config.size_count.xy);lo=min(lo,pixel);hi=max(hi,pixel);}
    }
    if front==0u {return vec4<i32>(0);}
    let size=vec2<i32>(volume_config.size_count.xy);
    if front<8u {return vec4<i32>(vec2<i32>(0),size);}
    // Conservative pixel centres, bounded before f32 -> integer conversion.
    let first=vec2<i32>(clamp(floor(lo-0.5),vec2<f32>(0.0),vec2<f32>(size)));
    let last=vec2<i32>(clamp(ceil(hi+0.5),vec2<f32>(0.0),vec2<f32>(size)));
    return vec4<i32>(first,last);
}
@compute @workgroup_size(64)
fn voxel_brick_scatter(@builtin(global_invocation_id) id:vec3<u32>) {
    if !VOLUME_BRICK_VISIBILITY {return;}
    let work=id.x+id.y*65535u*64u;if work>=volume_bricks[0] {return;}
    let reference=volume_bricks[1]+work*4u;let index=volume_bricks[reference];
    if volume_selection_at(index).x!=1u || volume_selection_at(index).y!=volume_bricks[reference+1u] {return;}
    let brick=volume_bricks[reference+2u];let tree=volume_trees[index];let rect=volume_brick_rect(brick,tree);
    let p=vec4<f32>(view.cam.xyz,1.0);let origin=vec3<f32>(dot(tree.inverse[0],p),dot(tree.inverse[1],p),dot(tree.inverse[2],p));
    var tests=0u;
    for(var y=rect.y;y<rect.w;y++) {for(var x=rect.x;x<rect.z;x++) {
        let pixel_id=vec2<u32>(u32(x),u32(y));var slot=u32(y)*volume_config.size_count.x+u32(x);
        if volume_config.dispatch.x!=0u {
            let tile=pixel_id/16u;let job=atomicLoad(&volume_stats[VOLUME_MASK_WORD+tile.y*volume_config.size_count.z+tile.x]);
            if job==0u {continue;}slot=(job-1u)*256u+(pixel_id.y%16u)*16u+pixel_id.x%16u;
        }
        if slot>=volume_config.dispatch.w {continue;}
        let metadata=volume_config.dispatch.z+4u+slot*2u;
        if (atomicLoad(&volume_jobs[metadata+1u])&0x80000000u)!=0u {continue;}
        let pixel=volume_pixel_ray(pixel_id);let d=vec4<f32>(pixel.direction,0.0);
        let direction=vec3<f32>(dot(tree.inverse[0],d),dot(tree.inverse[1],d),dot(tree.inverse[2],d));
        tests++;let interval=volume_brick_interval(brick,origin,direction,pixel.near,pixel.limit);
        if interval.y<=interval.x {continue;}
        let record=volume_stream_reserve(1u);
        if record==VOLUME_STREAM_FALLBACK {atomicOr(&volume_jobs[metadata+1u],0x80000000u);continue;}
        let count=atomicAdd(&volume_jobs[metadata+1u],1u);
        if (count&0x7fffffffu)>=512u {atomicOr(&volume_jobs[metadata+1u],0x80000000u);continue;}
        let previous=atomicExchange(&volume_jobs[metadata],record+1u);
        volume_stream_write(record,0u,VolumeInterval(interval,index,0u));
        atomicStore(&volume_jobs[volume_stream_pool()+record*4u+3u],previous);
    }}
    volume_add_total(20u,tests);
}
fn voxel_brick_intervals_pixel(id:vec2<u32>,slot:u32) {
    if any(id>=volume_config.size_count.xy) || slot>=volume_config.dispatch.w {return;}
    let metadata=volume_config.dispatch.z+4u+slot*2u;let count=atomicLoad(&volume_jobs[metadata+1u]);
    if count==0u {return;}
    if count>512u {atomicStore(&volume_jobs[metadata+1u],VOLUME_STREAM_FALLBACK);return;}
    let first=volume_stream_reserve(count);
    if first==VOLUME_STREAM_FALLBACK {atomicStore(&volume_jobs[metadata+1u],VOLUME_STREAM_FALLBACK);return;}
    var link=atomicLoad(&volume_jobs[metadata]);
    for(var j=0u;j<count;j++) {
        // Broken chains must never produce a partially populated span.
        if link==0u {atomicStore(&volume_jobs[metadata+1u],VOLUME_STREAM_FALLBACK);return;}
        let record=link-1u;let value=volume_stream_read(record,0u);
        link=atomicLoad(&volume_jobs[volume_stream_pool()+record*4u+3u]);volume_stream_write(first,j,value);
    }
    volume_stream_sort(first,count);atomicStore(&volume_jobs[metadata],first);
    atomicAdd(&volume_jobs[volume_config.dispatch.z+2u],count);
}

// No per-ray private lists. A scalar first scan counts exact intersections;
// the second fills a contiguous global span in stable source-ID order.
fn voxel_intervals_pixel(id:vec2<u32>,slot:u32) {
    if VOLUME_BRICK_VISIBILITY {voxel_brick_intervals_pixel(id,slot);return;}
    if any(id>=volume_config.size_count.xy) || slot>=volume_config.dispatch.w {return;}
    let metadata=volume_config.dispatch.z+4u+slot*2u;
    atomicStore(&volume_jobs[metadata],0u);atomicStore(&volume_jobs[metadata+1u],0u);
    let tile=id/16u;let start=(tile.y*volume_config.size_count.z+tile.x)*(VOLUME_CANDIDATES+1u);
    let count=atomicLoad(&volume_tiles[start]);if count==0u {return;}
    let tile_fallback=count>VOLUME_CANDIDATES;
    let candidates=select(count,volume_config.size_count.w,tile_fallback);let pixel_ray=volume_pixel_ray(id);
    var ray_count=0u;var interval_tests=0u;
    for(var j=0u;j<candidates;j++) {
        var index=j;if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
        if volume_selection_at(index).x!=1u {continue;}
        interval_tests++;
        var interval=volume_interval(volume_trees[index],view.cam.xyz,pixel_ray.direction,pixel_ray.limit);
        interval.x=max(interval.x,pixel_ray.near);
        if interval.y>interval.x {ray_count++;}
    }
    if ray_count==0u {volume_add_total(20u,interval_tests);return;}
    // At most262144 admitted slots x4096 volumes: reservation cannot wrap u32.
    let first=atomicAdd(&volume_jobs[volume_config.dispatch.z],ray_count);
    let capacity=volume_stream_capacity();
    if first>capacity || ray_count>capacity-first {
        atomicStore(&volume_jobs[metadata+1u],VOLUME_STREAM_FALLBACK);
        volume_add_total(20u,interval_tests);return;
    }
    var written=0u;
    for(var j=0u;j<candidates;j++) {
        var index=j;if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
        if volume_selection_at(index).x!=1u {continue;}
        interval_tests++;
        var interval=volume_interval(volume_trees[index],view.cam.xyz,pixel_ray.direction,pixel_ray.limit);
        interval.x=max(interval.x,pixel_ray.near);if interval.y<=interval.x {continue;}
        let at=volume_stream_pool()+(first+written)*4u;
        atomicStore(&volume_jobs[at],bitcast<u32>(interval.x));atomicStore(&volume_jobs[at+1u],bitcast<u32>(interval.y));
        atomicStore(&volume_jobs[at+2u],index);atomicStore(&volume_jobs[at+3u],0u);written++;
    }
    atomicStore(&volume_jobs[metadata],first);atomicStore(&volume_jobs[metadata+1u],written);
    atomicAdd(&volume_jobs[volume_config.dispatch.z+2u],written);volume_add_total(20u,interval_tests);
}
fn volume_stream_pixel(group:vec3<u32>,local:vec3<u32>)->vec4<u32> {
    let job=group.x+(group.y/4u)*VOLUME_JOB_ROW;
    if job>=atomicLoad(&volume_jobs[3]) {return vec4<u32>(0u);}
    let tile_index=atomicLoad(&volume_jobs[4u+job]);let tile_width=volume_config.size_count.z;
    let tile=vec2<u32>(tile_index%tile_width,tile_index/tile_width);
    let offset=vec2<u32>(group.y&1u,(group.y>>1u)&1u)*8u+local.xy;
    return vec4<u32>(tile*16u+offset,job*256u+offset.y*16u+offset.x,1u);
}
@compute @workgroup_size(8,8)
fn voxel_intervals(@builtin(global_invocation_id) id:vec3<u32>) {
    voxel_intervals_pixel(id.xy,id.y*volume_config.size_count.x+id.x);
}
@compute @workgroup_size(8,8)
fn voxel_intervals_active(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>) {
    let pixel=volume_stream_pixel(group,local);if pixel.w!=0u {voxel_intervals_pixel(pixel.xy,pixel.z);}
}

// Aggregate selections already specify an exact mip. Computing a perspective
// diameter for them is unnecessary; legacy providers retain their exact LOD.
fn volume_stream_ray(index:u32,leave:f32,direction:vec3<f32>)->VolumeRay {
    let tree=volume_trees[index];let p=vec4<f32>(view.cam.xyz,1.0);let d=vec4<f32>(direction,0.0);
    let local_origin=vec3<f32>(dot(tree.inverse[0],p),dot(tree.inverse[1],p),dot(tree.inverse[2],p));
    let local_direction=vec3<f32>(dot(tree.inverse[0],d),dot(tree.inverse[1],d),dot(tree.inverse[2],d));
    let scale=length(local_direction);let angular=volume_direction(local_direction/max(scale,1e-8));
    let identity=(index<<12u)|angular.indices.x|(angular.indices.y<<4u)|(angular.indices.z<<8u);
    var lod_step=vec4<f32>(0.0,volume_levels[tree.identity.y].origin_cell.w*0.5/max(scale,1e-8),0.0,0.0);
    if volume_config.march.w>0.0 && volume_selection_at(index).w!=1u {
        let projected=volume_projected_cell(tree,direction);lod_step.x=projected.x;lod_step.z=projected.y;
    }
    return VolumeRay(vec4<f32>(local_origin,leave),vec4<f32>(local_direction,scale),angular.weights,identity,vec2<f32>(-1.0,0.0),lod_step);
}
@compute @workgroup_size(8,8)
fn voxel_integrate_global(@builtin(global_invocation_id) id:vec3<u32>) {
    voxel_integrate_global_pixel(id.xy,id.y*volume_config.size_count.x+id.x);
}
@compute @workgroup_size(8,8)
fn voxel_integrate_global_active(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>) {
    let pixel=volume_stream_pixel(group,local);if pixel.w!=0u {voxel_integrate_global_pixel(pixel.xy,pixel.z);}
}
fn voxel_integrate_global_pixel(id:vec2<u32>,slot:u32) {
    if any(id>=volume_config.size_count.xy) {return;}
    let px=vec2<i32>(id);textureStore(volume_output,px,vec4<f32>(0.0));
    // VOLUME_MOTION_BEGIN
    textureStore(volume_motion_output,px,vec4<f32>(0.0));
    // VOLUME_MOTION_END
    let tile=id/16u;let start=(tile.y*volume_config.size_count.z+tile.x)*(VOLUME_CANDIDATES+1u);
    let count=atomicLoad(&volume_tiles[start]);if count==0u {return;}
    let tile_fallback=count>VOLUME_CANDIDATES;let candidates=select(count,volume_config.size_count.w,tile_fallback);
    let pixel_ray=volume_pixel_ray(id);let direction=pixel_ray.direction;let near_distance=pixel_ray.near;let limit=pixel_ray.limit;
    var first=0u;var ray_count=VOLUME_STREAM_FALLBACK;
    if slot<volume_config.dispatch.w {
        let metadata=volume_config.dispatch.z+4u+slot*2u;
        first=atomicLoad(&volume_jobs[metadata]);ray_count=atomicLoad(&volume_jobs[metadata+1u]);
    }
    let streamed=ray_count!=VOLUME_STREAM_FALLBACK;
    var interval_tests=0u;var begin=1e30;var end=0.0;
    if streamed {
        for(var j=0u;j<ray_count;j++) {let interval=volume_stream_read(first,j).time;begin=min(begin,interval.x);end=max(end,interval.y);}
    } else {
        ray_count=0u;
        for(var j=0u;j<candidates;j++) {
            var index=j;if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
            if volume_selection_at(index).x!=1u {continue;}interval_tests++;
            var interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);interval.x=max(interval.x,near_distance);
            if interval.y<=interval.x {continue;}ray_count++;begin=min(begin,interval.x);end=max(end,interval.y);
        }
    }
    atomicMax(&volume_stats[5],ray_count);
    if ray_count==0u {volume_add_total(20u,interval_tests);return;}
    if !streamed {atomicAdd(&volume_jobs[volume_config.dispatch.z+1u],1u);}
    atomicAdd(&volume_stats[2],1u);
    let query_count=select(candidates,ray_count,streamed);
    var first_live=0u;var t=begin;var transmittance=1.0;var color=vec3<f32>(0.0);var steps=0u;
    var max_resident=0u;var empty_skips=0u;var next_empty_check=-1.0;var cached_empty=false;
    var next_light=-1.0;var cached_sky=1.0;var max_occupied_lod=0.0;var volume_samples=0u;var cell_samples=0u;
    // VOLUME_MOTION_BEGIN
    var motion_moment=vec4<f32>(0.0);var motion_valid=true;
    // VOLUME_MOTION_END
    loop {
        if t>=end || transmittance<=0.001 || steps>=VOLUME_MAX_STEPS {break;}steps++;
        if streamed && VOLUME_BRICK_VISIBILITY {
            loop {if first_live>=query_count || volume_stream_read(first,first_live).time.y>t {break;}first_live++;}
        }
        var next_entry=end;var segment_end=end;var inside_count=0u;var march_step=1e30;
        let test_empty=volume_config.march.z>0.5 && t>=next_empty_check;
        var empty_until=end;var all_empty=true;
        for(var j=first_live;j<query_count;j++) {
            var interval:vec2<f32>;var index=j;
            if streamed {let record=volume_stream_read(first,j);interval=record.time;index=record.index;}
            else {
                if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
                if volume_selection_at(index).x!=1u {continue;}interval_tests++;
                interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);interval.x=max(interval.x,near_distance);
            }
            if interval.y<=interval.x || interval.y<=t {continue;}
            if interval.x>t {next_entry=min(next_entry,interval.x);if streamed && VOLUME_BRICK_VISIBILITY {break;}continue;}
            inside_count++;segment_end=min(segment_end,interval.y);
            let ray=volume_stream_ray(index,interval.y,direction);march_step=min(march_step,volume_ray_step(ray,t));
            if test_empty {let empty=volume_empty_brick(ray,t);empty_until=min(empty_until,empty.x);all_empty=all_empty&&empty.y>0.5;}
        }
        max_resident=max(max_resident,inside_count);
        if inside_count==0u {if next_entry<=t {break;}t=next_entry;continue;}
        if test_empty {next_empty_check=min(empty_until,min(segment_end,next_entry));cached_empty=all_empty;}
        if volume_config.march.z>0.5 && cached_empty && next_empty_check>t {t=next_empty_check;empty_skips++;continue;}
        let ds=min(march_step,min(segment_end-t,next_entry-t));if ds<=0.0 || t+ds<=t {break;}
        let sample_t=t+0.5*ds;let p=view.cam.xyz+direction*sample_t;
        var total_sigma=0.0;var emission=vec3<f32>(0.0);var shadow=1.0;var shadow_ready=false;
        // VOLUME_MOTION_BEGIN
        var delta_sigma=vec3<f32>(0.0);
        // VOLUME_MOTION_END
        for(var j=first_live;j<query_count;j++) {
            var interval:vec2<f32>;var index=j;
            if streamed {let record=volume_stream_read(first,j);interval=record.time;index=record.index;}
            else {
                if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
                if volume_selection_at(index).x!=1u {continue;}interval_tests++;
                interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);interval.x=max(interval.x,near_distance);
            }
            if streamed && VOLUME_BRICK_VISIBILITY && sample_t<interval.x {break;}
            if sample_t<interval.x || sample_t>interval.y || interval.y<=interval.x {continue;}
            let ray=volume_stream_ray(index,interval.y,direction);let tree=volume_trees[index];
            let local=ray.origin_exit.xyz+ray.direction_scale.xyz*sample_t;let lod=volume_lod(ray,sample_t);
            let sample=volume_sample_local(tree,local,volume_ray_direction(ray),lod);volume_samples++;cell_samples+=sample.cell_samples;
            let sigma=sample.sigma*ray.direction_scale.w;if sigma<=0.0 {continue;}max_occupied_lod=max(max_occupied_lod,lod);
            if !shadow_ready {
                if sample_t>=next_light {cached_sky=sky_visibility(p,vec3<f32>(0.0,1.0,0.0));next_light=sample_t+1.0;}
                shadow=mix(1.0,shadow_at(p,vec3<f32>(0.0,1.0,0.0)),light.sun_color.w);shadow_ready=true;
            }
            total_sigma+=sigma;emission+=sigma*volume_radiance(sample,tree,p,-direction,shadow,cached_sky);
            // VOLUME_MOTION_BEGIN
            if volume_config.dispatch.y!=0u {
                if tree.identity.x>=arrayLength(&volume_previous_instances) {motion_valid=false;}
                else {
                    let old=volume_previous_instances[tree.identity.x];let point=vec4<f32>(local,1.0);
                    let previous=vec3<f32>(dot(old.r0,point),dot(old.r1,point),dot(old.r2,point));
                    if all(abs(previous)<vec3<f32>(1e30)) {delta_sigma+=sigma*(previous-p);}else {motion_valid=false;}
                }
            }
            // VOLUME_MOTION_END
        }
        if total_sigma>0.0 {
            let transport=volume_transport_segment(transmittance,total_sigma,ds);color+=transport.opacity_weight*emission/total_sigma;
            // VOLUME_MOTION_BEGIN
            motion_moment+=transport.opacity_weight*vec4<f32>(delta_sigma/total_sigma,sample_t);
            // VOLUME_MOTION_END
            transmittance=transport.transmittance;
        }
        t+=ds;
    }
    volume_add_total(14u,steps);volume_add_total(16u,volume_samples);volume_add_total(18u,cell_samples);volume_add_total(20u,interval_tests);
    atomicMax(&volume_stats[3],steps);atomicMax(&volume_stats[7],max_resident);atomicMax(&volume_stats[9],empty_skips);
    if max_occupied_lod>0.0 {atomicMax(&volume_stats[10],u32(ceil(max_occupied_lod*256.0)));atomicAdd(&volume_stats[11],1u);}
    if t<end && transmittance>0.001 {atomicAdd(&volume_stats[1],1u);textureStore(volume_output,px,vec4<f32>(6000.0,0.0,6000.0,1.0));}
    else {
        textureStore(volume_output,px,vec4<f32>(color,1.0-transmittance));
        // VOLUME_MOTION_BEGIN
        let alpha=1.0-transmittance;if motion_valid && alpha>0.0 {textureStore(volume_motion_output,px,motion_moment/alpha);}
        // VOLUME_MOTION_END
    }
}

@compute @workgroup_size(8,8)
fn voxel_integrate(@builtin(global_invocation_id) id:vec3<u32>) {
    voxel_integrate_pixel(id);
}

@compute @workgroup_size(8,8)
fn voxel_integrate_active(@builtin(workgroup_id) group:vec3<u32>,
                          @builtin(local_invocation_id) local:vec3<u32>) {
    let job=group.x+(group.y/4u)*VOLUME_JOB_ROW;
    if job>=atomicLoad(&volume_jobs[3]) {return;}
    let tile_index=atomicLoad(&volume_jobs[4u+job]);
    let tile_width=volume_config.size_count.z;
    let tile=vec2<u32>(tile_index%tile_width,tile_index/tile_width);
    let offset=vec2<u32>(group.y&1u,(group.y>>1u)&1u)*8u;
    voxel_integrate_pixel(vec3<u32>(tile*VOLUME_TILE+offset+local.xy,0u));
}

fn voxel_integrate_pixel(id:vec3<u32>) {
    if any(id.xy>=volume_config.size_count.xy) {return;}
    let px=vec2<i32>(id.xy);
    textureStore(volume_output,px,vec4<f32>(0.0));
    // VOLUME_MOTION_BEGIN
    textureStore(volume_motion_output,px,vec4<f32>(0.0));
    // VOLUME_MOTION_END
    let tile_dims=(volume_config.size_count.xy+vec2<u32>(15u))/16u;
    let tile=id.xy/16u;
    let start=(tile.y*tile_dims.x+tile.x)*(VOLUME_CANDIDATES+1u);
    let count=atomicLoad(&volume_tiles[start]);
    if count==0u {return;}
    let tile_fallback=count>VOLUME_CANDIDATES;
    let candidates=select(count,volume_config.size_count.w,tile_fallback);
    let ndc=(vec2<f32>(id.xy)+0.5)/vec2<f32>(volume_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let direction=normalize(far.xyz/far.w-view.cam.xyz);
    let near_point=view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let near_distance=distance(view.cam.xyz,near_point.xyz/near_point.w);
    let depth=textureLoad(depth_tex,px,0);
    var limit=1e30;
    if depth>0.0 {let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);limit=distance(view.cam.xyz,hit.xyz/hit.w);}

    // Compact and sort actual intersections once. Projected tile overlap is
    // not ray overlap; overflow retains the original complete candidate set.
    var intervals:array<VolumeInterval,64>;
    var ray_count=0u;
    var interval_tests=0u;
    var begin=1e30;var end=0.0;
    for(var j=0u;j<candidates;j++) {
        var index=j;if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
        if volume_selection_at(index).x!=1u {continue;}
        interval_tests++;
        var interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);
        interval.x=max(interval.x,near_distance);
        if interval.y<=interval.x {continue;}
        if ray_count<VOLUME_CANDIDATES {intervals[ray_count]=VolumeInterval(interval,index,0u);}
        ray_count++;
        begin=min(begin,interval.x);end=max(end,interval.y);
    }
    atomicMax(&volume_stats[5],ray_count);
    if ray_count==0u {volume_add_total(20u,interval_tests);return;}
    let compact=ray_count<=VOLUME_CANDIDATES;
    if !compact {atomicAdd(&volume_stats[6],1u);}
    if compact {
        for(var j=1u;j<ray_count;j++) {
            let value=intervals[j];
            var k=j;
            // Bounded insertion sort; no sort occurs inside the march.
            for(var move_count=0u;move_count<VOLUME_CANDIDATES;move_count++) {
                if k==0u {break;}
                if intervals[k-1u].time.x<=value.time.x {break;}
                intervals[k]=intervals[k-1u];k--;
            }
            intervals[k]=value;
        }
    }
    atomicAdd(&volume_stats[2],1u);
    var rays:array<VolumeRay,16>;
    var resident_count=0u;var cursor=0u;var cached=compact;
    var max_resident=0u;var cache_overflow=false;var empty_skips=0u;
    var t=begin;var transmittance=1.0;var color=vec3<f32>(0.0);var steps=0u;
    // VOLUME_MOTION_BEGIN
    var motion_moment=vec4<f32>(0.0);var motion_valid=true;
    // VOLUME_MOTION_END
    var next_light=-1.0;var cached_sky=1.0;
    var max_occupied_lod=0.0;
    var volume_samples=0u;var cell_samples=0u;
    loop {
        if t>=end || transmittance<=0.001 || steps>=VOLUME_MAX_STEPS {break;}
        // Every path, including gaps and failed float progress, is bounded.
        steps++;
        var next_entry=end;var segment_end=end;var inside_count=0u;
        var march_step=1e30;
        if cached {
            var slot=0u;
            for(var removal=0u;removal<VOLUME_ACTIVE;removal++) {
                if slot>=resident_count {break;}
                if rays[slot].origin_exit.w<=t {
                    resident_count--;rays[slot]=rays[resident_count];
                } else {slot++;}
            }
            for(var insertion=0u;insertion<VOLUME_CANDIDATES;insertion++) {
                if cursor>=ray_count {break;}
                let interval=intervals[cursor];
                if interval.time.x>t {break;}
                cursor++;
                if interval.time.y<=t {continue;}
                if resident_count==VOLUME_ACTIVE {
                    // Permanently switch this ray to the complete scan. Its
                    // optical integral still includes every intersected tree.
                    cached=false;cache_overflow=true;break;
                }
                rays[resident_count]=volume_ray(interval.index,interval.time.y,direction);
                resident_count++;
            }
        }
        if cached {
            inside_count=resident_count;
            if cursor<ray_count {next_entry=intervals[cursor].time.x;}
            for(var j=0u;j<resident_count;j++) {
                segment_end=min(segment_end,rays[j].origin_exit.w);
                march_step=min(march_step,volume_ray_step(rays[j],t));
            }
        } else {
            let query_count=select(candidates,ray_count,compact);
            for(var j=0u;j<query_count;j++) {
                var interval:vec2<f32>;var index=j;
                if compact {interval=intervals[j].time;index=intervals[j].index;}
                else {
                    if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
                    if volume_selection_at(index).x!=1u {continue;}
                    interval_tests++;
                    interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);
                    interval.x=max(interval.x,near_distance);
                }
                if interval.y<=interval.x || interval.y<=t {continue;}
                if interval.x>t {next_entry=min(next_entry,interval.x);}
                else {
                    inside_count++;segment_end=min(segment_end,interval.y);
                    march_step=min(march_step,volume_ray_step(volume_ray(index,interval.y,direction),t));
                }
            }
        }
        max_resident=max(max_resident,inside_count);
        cache_overflow=cache_overflow||inside_count>VOLUME_ACTIVE;
        if inside_count==0u {if next_entry<=t {break;}t=next_entry;continue;}
        if cached && volume_config.march.z>0.5 {
            var empty_until=min(segment_end,next_entry);
            var all_empty=true;
            for(var j=0u;j<resident_count;j++) {
                if t>=rays[j].brick.x {rays[j].brick=volume_empty_brick(rays[j],t);}
                all_empty=all_empty&&rays[j].brick.y>0.5;
                empty_until=min(empty_until,rays[j].brick.x);
            }
            if all_empty && empty_until>t {
                // Both reconstructed mip supports are empty over this whole
                // interval, including any impending blend/LOD boundary.
                t=empty_until;empty_skips++;continue;
            }
        }
        let ds=min(march_step,min(segment_end-t,next_entry-t));
        if ds<=0.0 || t+ds<=t {break;}
        let sample_t=t+0.5*ds;
        let p=view.cam.xyz+direction*sample_t;
        var total_sigma=0.0;var emission=vec3<f32>(0.0);
        // VOLUME_MOTION_BEGIN
        var delta_sigma=vec3<f32>(0.0);
        // VOLUME_MOTION_END
        var shadow=1.0;var shadow_ready=false;
        var query_count=select(candidates,ray_count,compact);
        if cached {query_count=resident_count;}
        for(var j=0u;j<query_count;j++) {
            var ray:VolumeRay;
            if cached {ray=rays[j];}
            else {
                var index=j;var interval:vec2<f32>;
                if compact {index=intervals[j].index;interval=intervals[j].time;}
                else {
                    if !tile_fallback {index=atomicLoad(&volume_tiles[start+1u+j]);}
                    if volume_selection_at(index).x!=1u {continue;}
                    interval_tests++;
                    interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);
                }
                if sample_t<interval.x || sample_t>interval.y || interval.y<=interval.x {continue;}
                ray=volume_ray(index,interval.y,direction);
            }
            let tree=volume_trees[ray.identity>>12u];
            let local=ray.origin_exit.xyz+ray.direction_scale.xyz*sample_t;
            let lod=volume_lod(ray,sample_t);
            let sample=volume_sample_local(tree,local,volume_ray_direction(ray),lod);
            volume_samples++;cell_samples+=sample.cell_samples;
            let sigma=sample.sigma*ray.direction_scale.w;
            if sigma<=0.0 {continue;}
            max_occupied_lod=max(max_occupied_lod,lod);
            // Same one-metre spatial lighting approximation, now first
            // evaluated at occupied density rather than empty bounding air.
            // This may change lighting-cache phase; raw alpha is independent.
            if !shadow_ready {
                if sample_t>=next_light {cached_sky=sky_visibility(p,vec3<f32>(0.0,1.0,0.0));next_light=sample_t+1.0;}
                shadow=mix(1.0,shadow_at(p,vec3<f32>(0.0,1.0,0.0)),light.sun_color.w);
                shadow_ready=true;
            }
            total_sigma+=sigma;
            emission+=sigma*volume_radiance(sample,tree,p,-direction,shadow,cached_sky);
            // VOLUME_MOTION_BEGIN
            // Local is the exact point used to reconstruct this branch's
            // density. Previous source-owner rows include its complete rigid
            // branch pose. Accumulate ALL overlaps, not just a winning owner.
            if volume_config.dispatch.y!=0u {
                if tree.identity.x>=arrayLength(&volume_previous_instances) {motion_valid=false;}
                else {
                    let old=volume_previous_instances[tree.identity.x];
                    let point=vec4<f32>(local,1.0);
                    let previous=vec3<f32>(dot(old.r0,point),dot(old.r1,point),dot(old.r2,point));
                    if all(abs(previous)<vec3<f32>(1e30)) {delta_sigma+=sigma*(previous-p);}
                    else {motion_valid=false;}
                }
            }
            // VOLUME_MOTION_END
        }
        if total_sigma>0.0 {
            let transport=volume_transport_segment(transmittance,total_sigma,ds);
            color+=transport.opacity_weight*emission/total_sigma;
            // VOLUME_MOTION_BEGIN
            let opacity_weight=transport.opacity_weight;
            motion_moment+=opacity_weight*vec4<f32>(delta_sigma/total_sigma,sample_t);
            // VOLUME_MOTION_END
            transmittance=transport.transmittance;
        }
        t+=ds;
    }
    volume_add_total(14u,steps);
    volume_add_total(16u,volume_samples);
    volume_add_total(18u,cell_samples);
    volume_add_total(20u,interval_tests);
    atomicMax(&volume_stats[3],steps);
    atomicMax(&volume_stats[7],max_resident);
    if cache_overflow {atomicAdd(&volume_stats[8],1u);}
    atomicMax(&volume_stats[9],empty_skips);
    if max_occupied_lod>0.0 {
        atomicMax(&volume_stats[10],u32(ceil(max_occupied_lod*256.0)));
        atomicAdd(&volume_stats[11],1u);
    }
    if t<end && transmittance>0.001 {
        atomicAdd(&volume_stats[1],1u);
        // Explicitly invalid output; the fixture must fail this diagnostic.
        textureStore(volume_output,px,vec4<f32>(6000.0,0.0,6000.0,1.0));
    } else {
        textureStore(volume_output,px,vec4<f32>(color,1.0-transmittance));
        // VOLUME_MOTION_BEGIN
        let alpha=1.0-transmittance;
        if motion_valid && alpha>0.0 {
            textureStore(volume_motion_output,px,motion_moment/alpha);
        }
        // Missing/invalid metadata stays zero and rejects temporal history.
        // VOLUME_MOTION_END
    }
}

// Rasterized voxels (KERN_VOXEL_RASTER=1 or far trees, docs/log/voxel-raster-plan.md).
// Nanite-style: each occupied cell of a tree's level is an opaque cube kept
// with probability 1-exp(-optical depth), so the nearest kept cell is the
// stochastic first hit. voxel_splats lists the cells per view (one
// workgroup per tree), voxel_splat_raster.wgsl draws them into an id/depth
// buffer and voxel_splat_resolve shades the winning cell once per pixel.
// Static: per packed level (indexed like volume_levels) its range of
// occupied cells, then the cells: cell index, packed coord (10 bits/axis).
@group(2) @binding(14) var<storage,read> volume_splat_cells: array<vec2<u32>>;
// Header (VOLUME_SPLAT_HEADER records: view-projection columns; for the sun
// also six frustum planes and direction+texel, as f32 bits; record 11 x the
// tree count), then four records per tree drawn this view (owner rows,
// level origin and cell, as f32 bits), then one record per splat: tree, cell
// index, packed coord, 16-bit alpha | faces << 16 | appearance seed << 19.
@group(2) @binding(15) var<storage,read_write> volume_splats: array<vec4<u32>>;
// Indirect draw arguments: vertex count, instance count, first vertex, first instance.
// Draw arguments share volume_stats[22..26] (brick culling's second draw
// 26..30), leaving a binding for choices.
@group(2) @binding(17) var volume_splat_id: texture_2d<u32>;
@group(2) @binding(18) var volume_splat_depth: texture_depth_2d;
// Splats drawn into the main visibility buffer (voxel_forest Stage::Splats
// with the scene's targets): their surface for shade, per pixel, packed as
// f16 pairs: albedo rg, albedo b + roughness, normal xy, normal z + leaf
// fraction. A pixel no splat wins keeps whatever was there; shade reads it
// only where the visibility id says voxel.
@group(2) @binding(19) var volume_gbuffer: texture_storage_2d<rgba32uint,write>;
const VOLUME_SPLAT_HEADER: u32 = 12u;
const VOLUME_SPLAT_NONE: u32 = 0xffffffffu;
// One quad per splat (voxel_splat_raster.wgsl).
const VOLUME_SPLAT_VERTICES: u32 = 6u;
// Splats fainter than this are thinned (volume_splat_append). 0.5 instead
// of 0.25 halved the camera splat raster (terrain_fly high60 14 -> 7.6 ms,
// Sideways' mixed forest likewise) with no visible change after TAA.
const VOLUME_SPLAT_MIN_ALPHA: f32 = 0.5;
// KERN_VOXEL_DISC_DEPTH: optical depth over the drawn disc's mean chord (voxel_forest.rs).
const VOLUME_DISC_DEPTH: bool = true;
fn volume_splat_hash(x:u32)->u32 {
    var h=x*747796405u+2891336453u;
    h=((h>>((h>>28u)+4u))^h)*277803737u;
    return (h>>22u)^h;
}
// Selection w for the sun map: this group casts as voxels, its source does not.
const VOLUME_SUN_CASTER: u32 = 2u;
var<workgroup> volume_splat_level_shared: u32;
// Brick culling's batch of visible bricks (voxel_splats).
var<workgroup> volume_brick_table_shared: vec2<u32>;
var<workgroup> volume_brick_visible: atomic<u32>;
var<workgroup> volume_brick_ranges: array<vec2<u32>,64>;
var<workgroup> volume_brick_starts: array<u32,64>;
var<workgroup> volume_brick_count_shared: u32;
var<workgroup> volume_brick_cells_shared: u32;

// Far trees (config march.y > 0): owners past this distance from the
// camera, measured at the instance root exactly like cull.wgsl.
fn volume_far_tree(index:u32)->bool {
    let owner=instances[volume_trees[index].identity.x];
    return distance(vec3<f32>(owner.r0.w,owner.r1.w,owner.r2.w),view.cam.xyz)>volume_config.march.y;
}
// GPU-owned roots publish separate camera/shadow arrays after the shared branch
// prefix. Never launch the other consumer's root workgroups just to reject them.
fn volume_consumer_count()->u32 {return volume_config.size_count.w-u32(volume_config.proxy.w);}
fn volume_shadow_index(work:u32)->u32 {
 if work>=volume_consumer_count() {return volume_config.size_count.w;}
 return work+select(0u,u32(volume_config.proxy.w),f32(work)>=volume_config.proxy.x);
}
fn volume_proxy(index:u32)->bool {return f32(index)>=volume_config.proxy.x;}
// A proxy another representation stands in for (recipe_forest::Pool):
// bounds_min.w=1 draws nothing (a full tree, which casts itself), 2 only
// casts sun (a card tree: every non-full tree's shadow stays these voxels,
// so no shadow pops in where cards end). 3 is camera-only for GPU root demand.
fn volume_proxy_shown(index:u32)->bool {let mode=volume_trees[index].bounds_min.w;return mode==0.0 || mode==3.0;}
fn volume_proxy_casts(index:u32)->bool {let mode=volume_trees[index].bounds_min.w;return mode==0.0 || mode==2.0;}
fn volume_splat_level(index:u32)->u32 {
    let tree=volume_trees[index];
    if volume_proxy(index) {
        // Far proxies loosen their pixel error with sqrt(distance / proxy.z).
        var pixels=volume_config.proxy.y;
        if volume_config.proxy.z>0.0 {
            let t=distance(0.5*(tree.bounds_min.xyz+tree.bounds_max.xyz),view.cam.xyz);
            pixels*=sqrt(max(t/volume_config.proxy.z,1.0));
        }
        return volume_footprint_level(index,tree.identity.z-1u,pixels);
    }
    if volume_config.march.y>0.0 {return volume_footprint_level(index,tree.identity.z-1u,volume_config.march.w);}
    let selected=volume_selection_at(index);
    let ceiling=min(tree.identity.z-1u,selected.y);
    // Bit 12: the GPU forest selected this exact mip for the camera (its
    // error and splat booking assume it); never draw a finer one.
    if (volume_selection_words[index]&4096u)!=0u {return ceiling;}
    if selected.w==1u || volume_config.march.w<=0.0 {return select(0u,ceiling,selected.w==1u);}
    return volume_footprint_level(index,ceiling,volume_config.march.w);
}
// The integrator's projected-footprint rule, once at the crown centre so a
// tree never mixes two levels. Tree bounds are a world box.
fn volume_footprint_level(index:u32,ceiling:u32,pixels:f32)->u32 {
    if pixels<=0.0 {return 0u;}
    let tree=volume_trees[index];
    let world=0.5*(tree.bounds_min.xyz+tree.bounds_max.xyz);
    let t=max(distance(world,view.cam.xyz),1e-3);
    let projected=volume_projected_cell(tree,(world-view.cam.xyz)/t);
    let footprint=pixels*t/max(projected.x+pixels*projected.y,1e-8);
    return u32(clamp(floor(log2(max(footprint,1e-8))),0.0,f32(ceiling)));
}
// Whole tree (its world box) against six planes, before its cells.
fn volume_tree_outside(index:u32,plane:vec4<f32>)->bool {
    let tree=volume_trees[index];
    let centre=0.5*(tree.bounds_min.xyz+tree.bounds_max.xyz);
    let half=0.5*(tree.bounds_max.xyz-tree.bounds_min.xyz);
    return dot(plane.xyz,centre)+plane.w < -dot(abs(plane.xyz),half);
}
// Publish the tree's pose and level grid for the raster pass.
fn volume_splat_tree(index:u32,level:u32) {
    let tree=volume_trees[index];
    let owner=instances[tree.identity.x];
    let base=VOLUME_SPLAT_HEADER+index*4u;
    volume_splats[base]=bitcast<vec4<u32>>(owner.r0);
    volume_splats[base+1u]=bitcast<vec4<u32>>(owner.r1);
    volume_splats[base+2u]=bitcast<vec4<u32>>(owner.r2);
    volume_splats[base+3u]=bitcast<vec4<u32>>(volume_levels[tree.identity.y+level].origin_cell);
}
fn volume_splat_first()->u32 {return VOLUME_SPLAT_HEADER+volume_config.size_count.w*4u;}

struct VolumeSplatCell { local:vec3<f32>,centre:vec3<f32>,radius:f32,size:f32 };
// What every cell of one tree at one level shares, read once per thread
// instead of per cell (the cell loop is the largest vegetation pass): the
// owner's pose, the level grid, a cell's bounding radius, the pose's x
// scale and the tree's stable appearance id for the thinning hashes.
struct VolumeSplatPose { r0:vec4<f32>,r1:vec4<f32>,r2:vec4<f32>,grid:vec4<f32>,radius:f32,scale:f32,appearance:u32 };
fn volume_splat_pose(index:u32,level:u32)->VolumeSplatPose {
    let tree=volume_trees[index];
    let grid=volume_levels[tree.identity.y+level].origin_cell;
    let owner=instances[tree.identity.x];
    let r0=owner.r0;let r1=owner.r1;let r2=owner.r2;
    let scale=length(vec3<f32>(r0.x,r1.x,r2.x));
    let radius=0.5*grid.w*(scale+length(vec3<f32>(r0.y,r1.y,r2.y))+length(vec3<f32>(r0.z,r1.z,r2.z)));
    let stable=tree.bounds_max.w;
    return VolumeSplatPose(r0,r1,r2,grid,radius,scale,select(index,u32(max(stable-1.0,0.0)),stable>0.0));
}
fn volume_splat_cell(pose:VolumeSplatPose,coord_bits:u32)->VolumeSplatCell {
    let coord=vec3<f32>(f32(coord_bits&1023u),f32((coord_bits>>10u)&1023u),f32(coord_bits>>20u));
    let local=pose.grid.xyz+(coord+0.5)*pose.grid.w;
    let point=vec4<f32>(local,1.0);
    return VolumeSplatCell(local,vec3<f32>(dot(pose.r0,point),dot(pose.r1,point),dot(pose.r2,point)),pose.radius,pose.grid.w);
}
fn volume_splat_outside(cell:VolumeSplatCell,plane:vec4<f32>)->bool {
    return dot(plane.xyz,cell.centre)+plane.w < -cell.radius;
}
// `toward`: local unit direction from the cell to the viewer. The thirteen
// extinction directions are antipodal, so its sign does not change sigma.
// Splat records carry alpha/VOLUME_SPLAT_ALPHA_RANGE in 16 bits, so a
// sub-pixel splat can stand for several (its disc area grows with alpha).
const VOLUME_SPLAT_ALPHA_RANGE: f32 = 64.0;
fn volume_splat_alpha(record:vec4<u32>)->f32 {return f32(record.w&0xffffu)/65535.0*VOLUME_SPLAT_ALPHA_RANGE;}
// Disc area in pixels of a kept cell (radius 0.688 sqrt(alpha) cells) below
// which splats are thinned by area: kept with probability area/target and
// grown to the target, so expected coverage holds while a far crown costs
// about one splat per covered pixel. foliage_refine.wgsl books the same.
const VOLUME_SPLAT_AREA_TARGET: f32 = 1.0;
// World size of one camera pixel at a point (render size from the config).
fn volume_camera_texel(p:vec3<f32>)->f32 {
    let w=dot(vec3<f32>(view.view_proj[0].w,view.view_proj[1].w,view.view_proj[2].w),p)+view.view_proj[3].w;
    let row_y=vec3<f32>(view.view_proj[0].y,view.view_proj[1].y,view.view_proj[2].y);
    if w<=1e-4 {return 0.0;}
    return w/max(0.5*f32(volume_config.size_count.y)*length(row_y),1e-8);
}
// volume_sigma of a stored cell, reading only its three extinction values.
// Loading and unpacking the whole 128-byte cell for this made splat
// generation memory bound (RTX desktop, 1440p: 6.3 ms, 1.7 ms with this).
fn volume_cell_sigma(address:u32,d:VolumeDirection)->f32 {
    var value=0.0;
    for(var k=0u;k<3u;k++) {let i=d.indices[k];value+=volume_cells[address].extinction[i/4u][i%4u]*d.weights[k];}
    return max(value,0.0);
}
fn volume_splat_append(index:u32,pose:VolumeSplatPose,cell_ref:vec2<u32>,cell:VolumeSplatCell,toward:vec3<f32>,texel:f32) {
    let appearance=pose.appearance;
    // Area thinning below keeps a cell with probability at most
    // max(area at alpha 1, 1/64): test that first and skip the cell fetch
    // for most cells of a far crown. Same hash, so the same cells survive.
    var area_one=0.0;var area_draw=1.0;
    if texel>0.0 {
        let side=cell.size*pose.scale/texel;
        area_one=1.48711*side*side;
        area_draw=f32(volume_splat_hash((appearance*0x85ebca6bu)^(cell_ref.y*0xc2b2ae35u)^0x27d4eb2fu)>>8u)/16777216.0;
        if area_one<VOLUME_SPLAT_AREA_TARGET && area_draw>=max(area_one/VOLUME_SPLAT_AREA_TARGET,1.0/VOLUME_SPLAT_ALPHA_RANGE) {return;}
    }
    // Optical depth of the mean chord through this cell along the view.
    let sigma=volume_cell_sigma(cell_ref.x,volume_direction(toward));
    // The splat is drawn as a disc of constant area 1.48711 cell^2 from every
    // view (area_one), so the leaf area sigma*cell^3 spread over it gives a mean
    // chord of cell/1.48711. The cube chord cell/L1 was for a projected cube and
    // overstates coverage 1.18x (axis views) to 1.49x (diagonal) on the disc.
    let chord=select(cell.size/max(dot(abs(toward),vec3<f32>(1.0)),1.0),cell.size/1.48711,VOLUME_DISC_DEPTH);
    let alpha=1.0-exp(-sigma*chord);
    if alpha<1.0/255.0 {return;}
    // Faint cells are thinned to VOLUME_SPLAT_MIN_ALPHA: kept with
    // probability alpha/min by a hash stable per tree and cell (no flicker),
    // so expected coverage holds while the raster draws far fewer quads.
    var kept_alpha=alpha;
    if alpha<VOLUME_SPLAT_MIN_ALPHA {
        let h=volume_splat_hash((appearance*0x9e3779b9u)^cell_ref.y);
        if f32(h>>8u)/16777216.0>=alpha/VOLUME_SPLAT_MIN_ALPHA {return;}
        kept_alpha=VOLUME_SPLAT_MIN_ALPHA;
    }
    if texel>0.0 {
        let area=area_one*kept_alpha;
        if area<VOLUME_SPLAT_AREA_TARGET {
            let keep=max(area/VOLUME_SPLAT_AREA_TARGET,kept_alpha/VOLUME_SPLAT_ALPHA_RANGE);
            if area_draw>=keep {return;}
            kept_alpha/=keep;
        }
    }
    let faces=select(0u,1u,toward.x>0.0)|select(0u,2u,toward.y>0.0)|select(0u,4u,toward.z>0.0);
    let slot=volume_splat_first()+atomicAdd(&volume_stats[VOLUME_DRAW_WORD+1],1u);
    if slot>=volume_splat_limit() {return;}
    volume_splats[slot]=vec4<u32>(index,cell_ref.x,cell_ref.y,u32(min(kept_alpha/VOLUME_SPLAT_ALPHA_RANGE,1.0)*65535.0+0.5)|(faces<<16u)|((volume_splat_hash(appearance)&8191u)<<19u));
}

// Brick occlusion culling (KERN_SPLAT_BRICK_HZB=1), two passes as in Nanite.
// Only ~8-17% of the camera splats won a pixel (Mancave, 9 November): the
// rest hid behind other needles. Pass 1 (voxel_splats) tests every 4x4x4
// brick against the depth pyramid of the previous frame, at the
// view-projection it was built with; visible bricks splat, hidden ones are
// queued at the end of volume_splats. Once pass 1 rasters, the pyramid is
// rebuilt from this frame's depth and pass 2 (voxel_splat_retest) splats the
// queued bricks it does not hide. Stale history only costs the retest; a
// full queue splats the brick in pass 1. No brick is lost.
// The camera target's header records 4-7 keep that view-projection and
// record 8.x is 1 once a pyramid exists (only sun targets put planes there).
override VOXEL_BRICK_CULL: bool = false;
override VOXEL_COMPACT: bool = false;
// Paged assets (foliage_page_cache.rs) keep the same brick table after the
// level headers, but it points at four-word descriptors in volume_indices.
override VOXEL_PAGED_BRICKS: bool = false;
@group(2) @binding(20) var volume_brick_hzb: texture_2d<f32>;
// volume_stats words: 26-29 pass 2 draw arguments, 30 queued bricks,
// 31 pass 1 splat count, 32-34 pass 2 dispatch.
const VOLUME_CULL_WORD: u32 = 26u;
const VOLUME_HZB_VIEW: u32 = 4u;
// An eighth of the record space holds the queue (one record per brick).
fn volume_cull_queue_capacity()->u32 {
    if !VOXEL_BRICK_CULL {return 0u;}
    let total=arrayLength(&volume_splats);let first=volume_splat_first();
    return select(0u,(total-first)/8u,total>first);
}
fn volume_splat_limit()->u32 {return arrayLength(&volume_splats)-volume_cull_queue_capacity();}
fn volume_splat_brick_table(index:u32,level:u32)->vec2<u32> {
    return volume_splat_cells[arrayLength(&volume_levels)+volume_trees[index].identity.y+level];
}
// Brick b of a table: its cells (first, count in volume_splat_cells) and its
// packed origin coordinate.
fn volume_splat_brick(table:vec2<u32>,b:u32)->vec3<u32> {
    if VOXEL_PAGED_BRICKS {
        let at=table.x+b*4u;
        return vec3<u32>(volume_indices[at],volume_indices[at+1u],volume_indices[at+2u]);
    }
    let range=volume_splat_cells[table.x+b*2u];
    return vec3<u32>(range.x,range.y,volume_splat_cells[table.x+b*2u+1u].x);
}
fn volume_hzb_view()->mat4x4<f32> {
    return mat4x4<f32>(bitcast<vec4<f32>>(volume_splats[VOLUME_HZB_VIEW]),bitcast<vec4<f32>>(volume_splats[VOLUME_HZB_VIEW+1u]),
        bitcast<vec4<f32>>(volume_splats[VOLUME_HZB_VIEW+2u]),bitcast<vec4<f32>>(volume_splats[VOLUME_HZB_VIEW+3u]));
}

fn volume_brick_hidden(index:u32, level:u32, packed:u32, view_proj:mat4x4<f32>)->bool {
    let tree=volume_trees[index];
    let grid=volume_levels[tree.identity.y+level].origin_cell;
    let coord=vec3<f32>(f32(packed&1023u),f32((packed>>10u)&1023u),f32(packed>>20u));
    // Transform the envelope of cell centres, then add the billboard's
    // WORLD-space support. Local padding is unsafe for nonuniform/sheared trees.
    let owner=instances[tree.identity.x];
    let centre=vec4<f32>(grid.xyz+(coord+2.0)*grid.w,1.0);
    let world_centre=vec3<f32>(dot(owner.r0,centre),dot(owner.r1,centre),dot(owner.r2,centre));
    // Raster half-width <= 1.15*.688 cells up to alpha 1, jitter <= .25
    // per screen axis: sqrt(2)*(.79+.25) < 2. Thinned sub-pixel splats grow
    // past that but stay under a pixel, inside the pyramid's 2-texel margin.
    let padding=2.0*grid.w*length(vec3<f32>(owner.r0.x,owner.r1.x,owner.r2.x));
    let half=1.5*grid.w*vec3<f32>(dot(abs(owner.r0.xyz),vec3<f32>(1.0)),dot(abs(owner.r1.xyz),vec3<f32>(1.0)),dot(abs(owner.r2.xyz),vec3<f32>(1.0)))+padding;
    var ndc_min=vec2<f32>(1e30);var ndc_max=vec2<f32>(-1e30);var nearest=0.0;
    for(var i=0u;i<8u;i++) {
        let world=vec4<f32>(world_centre+select(-half,half,vec3<bool>((i&1u)!=0u,(i&2u)!=0u,(i&4u)!=0u)),1.0);
        let clip=view_proj*world;
        // Near-plane crossings and degenerate projections are always visible.
        if clip.w<=0.0001 || clip.z>=clip.w {return false;}
        let ndc=clip.xyz/clip.w;
        ndc_min=min(ndc_min,ndc.xy);ndc_max=max(ndc_max,ndc.xy);nearest=max(nearest,ndc.z);
    }
    if any(ndc_min>vec2<f32>(1.0)) || any(ndc_max<vec2<f32>(-1.0)) {return true;}
    let size=textureDimensions(volume_brick_hzb,0);
    let low=clamp(vec2<f32>(ndc_min.x*0.5+0.5,0.5-ndc_max.y*0.5)*vec2<f32>(size)-2.0,vec2<f32>(0.0),vec2<f32>(size-1u));
    let high=clamp(vec2<f32>(ndc_max.x*0.5+0.5,0.5-ndc_min.y*0.5)*vec2<f32>(size)+2.0,vec2<f32>(0.0),vec2<f32>(size-1u));
    let extent=max(high.x-low.x+1.0,high.y-low.y+1.0);
    let mip=min(u32(max(ceil(log2(max(extent,1.0)))-1.0,0.0)),textureNumLevels(volume_brick_hzb)-1u);
    let dims=textureDimensions(volume_brick_hzb,mip);
    // Shift-and-clamp matches odd-size mip folding, unlike proportional UV.
    let first=min(vec2<u32>(floor(low))>>vec2<u32>(mip),dims-1u);
    let last=min(vec2<u32>(ceil(high))>>vec2<u32>(mip),dims-1u);
    var farthest=1.0;
    for(var y=first.y;y<=last.y;y++) {for(var x=first.x;x<=last.x;x++) {
        farthest=min(farthest,textureLoad(volume_brick_hzb,vec2<i32>(i32(x),i32(y)),i32(mip)).x);
    }}
    // Reversed minimum depth: any sky hole (0) prevents rejection.
    return nearest+0.00001<farthest;
}

fn volume_splat_wanted(index:u32)->bool {
    var wanted=volume_config.march.y>0.0 && volume_far_tree(index);
    if volume_config.march.y<=0.0 && !volume_proxy(index) {wanted=volume_selection_at(index).x==1u;}
    if volume_proxy(index) {wanted=volume_proxy_shown(index);}
    for(var i=0u;i<6u;i++) {wanted=wanted&&!volume_tree_outside(index,view.planes[i]);}
    return wanted;
}
@compute @workgroup_size(64)
fn voxel_splat_tree_list(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=(group.x+group.y*65535u)*64u+lane;
    if index>=volume_consumer_count() || !volume_splat_wanted(index) {return;}
    let slot=atomicAdd(&volume_tiles[3],1u);
    let capacity=arrayLength(&volume_tiles)-4u;
    if slot<capacity {atomicStore(&volume_tiles[4u+slot],index);}
    // On overflow dispatch every tree and use the direct path. Never drop it.
    let count=select(volume_consumer_count(),slot+1u,slot<capacity);
    atomicMax(&volume_tiles[0],min(count,65535u));
    atomicMax(&volume_tiles[1],(count+65534u)/65535u);
    atomicMax(&volume_tiles[2],1u);
}

fn volume_splat_eye(index:u32)->vec3<f32> {
    let tree=volume_trees[index];
    let eye=vec4<f32>(view.cam.xyz,1.0);
    return vec3<f32>(dot(tree.inverse[0],eye),dot(tree.inverse[1],eye),dot(tree.inverse[2],eye));
}
// Cells first..first+count of the splat catalog that pass the frustum.
fn volume_splat_range(index:u32,pose:VolumeSplatPose,range:vec2<u32>,eye_local:vec3<f32>,first:u32,step:u32) {
    for(var i=first;i<range.y;i+=step) {
        let cell_ref=volume_splat_cells[range.x+i];
        let cell=volume_splat_cell(pose,cell_ref.y);
        var inside=true;
        for(var p=0u;p<6u;p++) {inside=inside&&!volume_splat_outside(cell,view.planes[p]);}
        if inside {volume_splat_append(index,pose,cell_ref,cell,normalize(eye_local-cell.local),volume_camera_texel(cell.centre));}
    }
}

@compute @workgroup_size(64)
fn voxel_splats(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let work=group.x+group.y*65535u;
    var index=select(volume_config.size_count.w,work,work<volume_consumer_count());
    if VOXEL_COMPACT {
        let count=atomicLoad(&volume_tiles[3]);
        if count<=arrayLength(&volume_tiles)-4u {
            index=volume_config.size_count.w;
            if work<count {index=atomicLoad(&volume_tiles[4u+work]);}
        }
    }
    if lane==0u {
        if work==0u {
            for(var c=0u;c<4u;c++) {volume_splats[c]=bitcast<vec4<u32>>(view.view_proj[c]);}
            // w: the first queue record, which the raster never reads as a splat.
            volume_splats[11]=vec4<u32>(volume_config.size_count.w,0u,0u,select(0u,volume_splat_limit(),VOXEL_BRICK_CULL));
            atomicStore(&volume_stats[VOLUME_DRAW_WORD+0],VOLUME_SPLAT_VERTICES);
        }
        var level=VOLUME_SPLAT_NONE;
        if index<volume_config.size_count.w {
            let wanted=volume_splat_wanted(index);
            if wanted {level=volume_splat_level(index);volume_splat_tree(index,level);}
        }
        volume_splat_level_shared=level;
    }
    let level=workgroupUniformLoad(&volume_splat_level_shared);
    if level==VOLUME_SPLAT_NONE {return;}
    let tree=volume_trees[index];
    let pose=volume_splat_pose(index,level);
    let eye_local=volume_splat_eye(index);
    if VOXEL_BRICK_CULL {
        // 64 bricks at a time: each lane tests one, then all lanes share
        // the visible bricks' cells. A lane per whole brick idled most
        // lanes of a far tree with a few bricks (valley 4K +2.75 ms).
        if lane==0u {volume_brick_table_shared=volume_splat_brick_table(index,level);}
        let bricks=workgroupUniformLoad(&volume_brick_table_shared);
        let history=volume_splats[VOLUME_HZB_VIEW+4u].x==1u;
        let previous=volume_hzb_view();
        for(var batch=0u;batch<bricks.y;batch+=64u) {
            if lane==0u {atomicStore(&volume_brick_visible,0u);}
            workgroupBarrier();
            let b=batch+lane;
            if b<bricks.y {
                let brick=volume_splat_brick(bricks,b);
                var draw=true;
                if history && volume_brick_hidden(index,level,brick.z,previous) {
                    let queued=atomicAdd(&volume_stats[VOLUME_CULL_WORD+4u],1u);
                    if queued<volume_cull_queue_capacity() {volume_splats[volume_splat_limit()+queued]=vec4<u32>(index,level,b,0u);draw=false;}
                }
                if draw {volume_brick_ranges[atomicAdd(&volume_brick_visible,1u)]=brick.xy;}
            }
            workgroupBarrier();
            if lane==0u {
                let n=atomicLoad(&volume_brick_visible);var total=0u;
                for(var k=0u;k<n;k++) {volume_brick_starts[k]=total;total+=volume_brick_ranges[k].y;}
                volume_brick_count_shared=n;volume_brick_cells_shared=total;
            }
            let total=workgroupUniformLoad(&volume_brick_cells_shared);
            let n=volume_brick_count_shared;
            for(var j=lane;j<total;j+=64u) {
                // Last brick starting at or before j.
                var low=0u;var high=n;
                while high-low>1u {let mid=(low+high)/2u;if volume_brick_starts[mid]<=j {low=mid;} else {high=mid;}}
                let range=volume_brick_ranges[low];
                volume_splat_range(index,pose,vec2<u32>(range.x+j-volume_brick_starts[low],1u),eye_local,0u,1u);
            }
            workgroupBarrier();
        }
        return;
    }
    volume_splat_range(index,pose,volume_splat_cells[tree.identity.y+level],eye_local,lane,64u);
}
// Between the passes, one thread: pass 2's records start after pass 1's
// (header 11.z, read by the raster) and its dispatch covers the queue.
@compute @workgroup_size(1)
fn voxel_splat_cull_base() {
    let splats=atomicLoad(&volume_stats[VOLUME_DRAW_WORD+1u]);
    atomicStore(&volume_stats[VOLUME_CULL_WORD+5u],splats);
    volume_splats[11].z=splats;
    let groups=(min(atomicLoad(&volume_stats[VOLUME_CULL_WORD+4u]),volume_cull_queue_capacity())+63u)/64u;
    atomicStore(&volume_stats[VOLUME_CULL_WORD+6u],min(groups,65535u));
    atomicStore(&volume_stats[VOLUME_CULL_WORD+7u],(groups+65534u)/65535u);
    atomicStore(&volume_stats[VOLUME_CULL_WORD+8u],1u);
}
// Pass 2: one thread per queued brick, against this frame's pyramid.
@compute @workgroup_size(64)
fn voxel_splat_retest(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let i=(group.x+group.y*65535u)*64u+lane;
    if i>=min(atomicLoad(&volume_stats[VOLUME_CULL_WORD+4u]),volume_cull_queue_capacity()) {return;}
    let queued=volume_splats[volume_splat_limit()+i];
    let index=queued.x;let level=queued.y;
    let brick=volume_splat_brick(volume_splat_brick_table(index,level),queued.z);
    if volume_brick_hidden(index,level,brick.z,view.view_proj) {return;}
    volume_splat_range(index,volume_splat_pose(index,level),brick.xy,volume_splat_eye(index),0u,1u);
}
// After pass 2, one thread: its draw arguments, and this frame's
// view-projection for the pyramid next frame tests against.
@compute @workgroup_size(1)
fn voxel_splat_cull_rest() {
    let base=atomicLoad(&volume_stats[VOLUME_CULL_WORD+5u]);
    let total=atomicLoad(&volume_stats[VOLUME_DRAW_WORD+1u]);
    atomicStore(&volume_stats[VOLUME_CULL_WORD],VOLUME_SPLAT_VERTICES);
    atomicStore(&volume_stats[VOLUME_CULL_WORD+1u],total-base);
    atomicStore(&volume_stats[VOLUME_CULL_WORD+2u],0u);
    atomicStore(&volume_stats[VOLUME_CULL_WORD+3u],0u);
    for(var c=0u;c<4u;c++) {volume_splats[VOLUME_HZB_VIEW+c]=volume_splats[c];}
    volume_splats[VOLUME_HZB_VIEW+4u]=vec4<u32>(1u,0u,0u,0u);
}

fn volume_splat_header(index:u32)->vec4<f32> {return bitcast<vec4<f32>>(volume_splats[index]);}
// The same cells into the sun map. The host writes the header: the sun's
// view-projection, frustum planes, direction towards the sun and texel size.
// Levels follow the texel: the finest whose cell is at least one texel.
fn volume_sun_splats_impl(index:u32,lane:u32,damaged:bool) {
    let sun=volume_splat_header(10u);
    if lane==0u {
        if index==0u {
            // Keep y: the cached map size every workgroup reads.
            volume_splats[11]=vec4<u32>(volume_config.size_count.w,volume_splats[11].y,0u,0u);
            atomicStore(&volume_stats[VOLUME_DRAW_WORD+0],VOLUME_SPLAT_VERTICES);
        }
        var level=VOLUME_SPLAT_NONE;
        if index<volume_config.size_count.w && damaged {
            // Far trees all cast; the sun selection marks only index 0.
            // Proxies always cast, also into maps whose source groups do not.
            var selected=volume_selection_at(0);
            if volume_config.march.y<=0.0 && !volume_proxy(index) {selected=volume_selection_at(index);}
            var wanted=(selected.x==1u && selected.w==VOLUME_SUN_CASTER) || volume_proxy(index);
            if volume_proxy(index) {wanted=volume_proxy_casts(index);}
            for(var i=0u;i<6u;i++) {wanted=wanted&&!volume_tree_outside(index,volume_splat_header(4u+i));}
            if wanted {
                let tree=volume_trees[index];
                let owner=instances[tree.identity.x];
                let finest=volume_levels[tree.identity.y].origin_cell.w*length(vec3<f32>(owner.r0.x,owner.r1.x,owner.r2.x));
                let ceiling=select(min(tree.identity.z-1u,selected.y),tree.identity.z-1u,volume_proxy(index));
                level=u32(clamp(floor(log2(max(sun.w/max(finest,1e-6),1e-8))),0.0,f32(ceiling)));
                volume_splat_tree(index,level);
            }
        }
        volume_splat_level_shared=level;
    }
    let level=workgroupUniformLoad(&volume_splat_level_shared);
    if level==VOLUME_SPLAT_NONE {return;}
    let tree=volume_trees[index];
    let d=vec4<f32>(sun.xyz,0.0);
    let toward=normalize(vec3<f32>(dot(tree.inverse[0],d),dot(tree.inverse[1],d),dot(tree.inverse[2],d)));
    let range=volume_splat_cells[tree.identity.y+level];
    let pose=volume_splat_pose(index,level);
    for(var i=lane;i<range.y;i+=64u) {
        let cell_ref=volume_splat_cells[range.x+i];
        let cell=volume_splat_cell(pose,cell_ref.y);
        var inside=true;
        for(var p=0u;p<6u;p++) {inside=inside&&!volume_splat_outside(cell,volume_splat_header(4u+p));}
        // No area thinning into the sun maps: filtered depth turned it into grain
        // and lost tree shadow density.
        if inside {volume_splat_append(index,pose,cell_ref,cell,toward,0.0);}
    }
}

// World texel of the sun map that shades p.
fn volume_sun_texel(p:vec3<f32>)->f32 {
    if light.near_shadow.z>0.5 {
        let c=light.near_shadow_view_proj*vec4<f32>(p,1.0);
        if all(abs(c.xy)<vec2<f32>(1.0)) {return light.near_shadow.x;}
    }
    return light.sky.w;
}
// Sun transmittance through the tree's own cells at the drawn level, from
// the cell after p's to the edge of its grid. KERN_VOXEL_SELF_SHADOW_COARSER
// marches that many levels coarser (fewer steps, softer; A/B only).
override VOXEL_SELF_SHADOW_COARSER: u32 = 0u;
fn volume_self_transmittance(tree:VolumeTree,grid:vec4<f32>,p:vec3<f32>,sun:vec3<f32>)->f32 {
    let finest=volume_levels[tree.identity.y].origin_cell.w;
    let index=min(u32(round(log2(max(grid.w/finest,1.0))))+VOXEL_SELF_SHADOW_COARSER,tree.identity.z-1u);
    let level=volume_levels[tree.identity.y+index];
    let local=vec3<f32>(dot(tree.inverse[0],vec4<f32>(p,1.0)),dot(tree.inverse[1],vec4<f32>(p,1.0)),dot(tree.inverse[2],vec4<f32>(p,1.0)));
    let d=normalize(vec3<f32>(dot(tree.inverse[0].xyz,sun),dot(tree.inverse[1].xyz,sun),dot(tree.inverse[2].xyz,sun)));
    let direction=volume_direction(d);
    let cell=level.origin_cell.w;
    var tau=0.0;
    for(var i=1u;i<=32u;i++) {
        let c=floor((local+d*(f32(i)*cell)-level.origin_cell.xyz)/cell);
        if any(c<vec3<f32>(0.0)) || any(c>=vec3<f32>(level.dims_offset.xyz)) {break;}
        let address=volume_cell_address(level,vec3<u32>(c));
        if address!=0u {tau+=volume_cell_sigma(address-1u,direction)*cell;}
    }
    return exp(-tau);
}

@compute @workgroup_size(8,8)
fn voxel_splat_gbuffer(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=volume_config.size_count.xy) {return;}
    let px=vec2<i32>(id.xy);
    let splat=textureLoad(volume_splat_id,px,0).x;
    if splat==0u {return;}
    let depth=textureLoad(depth_tex,px,0);
    let ndc=(vec2<f32>(id.xy)+0.5)/vec2<f32>(volume_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);
    let p=hit.xyz/hit.w;
    let direction=normalize(p-view.cam.xyz);
    let record=volume_splats[volume_splat_first()+splat-1u];
    let tree=volume_trees[record.x];
    let cell=unpack_volume_cell(volume_cells[record.y]);
    let d=vec4<f32>(direction,0.0);
    let local_d=vec3<f32>(dot(tree.inverse[0],d),dot(tree.inverse[1],d),dot(tree.inverse[2],d));
    let sigma=max(volume_sigma(cell,volume_direction(normalize(local_d))),1e-6);
    var empty:VolumeSample;
    let sample=volume_normalize_sample(volume_accumulate_cell(empty,cell,sigma));
    // The lobes' mean, each turned to face the viewer like a two-sided
    // leaf. A per-pixel random lobe (as Nanite foliage picks) read as noise
    // under motion even with TAA.
    var sum=vec3<f32>(0.0);
    for(var k=0u;k<VOLUME_NORMAL_COUNT;k++) {
        let w=max(sample.normals[k].w,0.0);
        if w<=0.0 {continue;}
        let local_n=sample.normals[k].xyz;
        let world_n=tree.inverse[0].xyz*local_n.x+tree.inverse[1].xyz*local_n.y+tree.inverse[2].xyz*local_n.z;
        if dot(world_n,world_n)<=1e-16 {continue;}
        var wn=normalize(world_n);
        if dot(wn,-direction)<0.0 {wn=-wn;}
        sum+=wn*w;
    }
    var n=select(-direction,normalize(sum),dot(sum,sum)>1e-12);
    if dot(n,-direction)<0.0 {n=-n;}
    // Rounded within its disc (voxel_splat_raster.wgsl, same centre, jitter
    // and radius): half the cell's normal, half a sphere's, so a cell reads
    // as a clump of leaves rather than one flat square.
    {
        let base=VOLUME_SPLAT_HEADER+record.x*4u;
        let r0=bitcast<vec4<f32>>(volume_splats[base]);
        let r1=bitcast<vec4<f32>>(volume_splats[base+1u]);
        let r2=bitcast<vec4<f32>>(volume_splats[base+2u]);
        let g=bitcast<vec4<f32>>(volume_splats[base+3u]);
        let coord=vec3<f32>(f32(record.z&1023u),f32((record.z>>10u)&1023u),f32(record.z>>20u));
        let local=vec4<f32>(g.xyz+(coord+0.5)*g.w,1.0);
        let right=normalize(vec3<f32>(view.view_proj[0].x,view.view_proj[1].x,view.view_proj[2].x));
        let up=normalize(vec3<f32>(view.view_proj[0].y,view.view_proj[1].y,view.view_proj[2].y));
        let scale=length(vec3<f32>(r0.x,r1.x,r2.x));
        // Match raster jitter exactly; physical slots/addresses are not appearance.
        let key=volume_splat_hash(record.w>>19u)^record.z;
        let jitter=vec2<f32>(f32(volume_splat_hash(key)&0xffffu),f32(volume_splat_hash(key^0x9e3779b9u)&0xffffu))/65535.0-0.5;
        let centre=vec3<f32>(dot(r0,local),dot(r1,local),dot(r2,local))+(right*jitter.x+up*jitter.y)*0.5*g.w*scale;
        let half=0.688*g.w*scale*sqrt(max(volume_splat_alpha(record),0.25));
        let d=vec2<f32>(dot(p-centre,right),dot(p-centre,up))/max(half,1e-6);
        let sphere=right*d.x+up*d.y-direction*sqrt(max(1.0-dot(d,d),0.0));
        n=normalize(mix(n,normalize(sphere),0.5));
    }
    // The tree shades itself along the sun through its own cells; the sun
    // maps (which hold this tree too, at their own texel) are read where the
    // ray leaves its box, for everything else. As the resolve did.
    let grid=bitcast<vec4<f32>>(volume_splats[VOLUME_SPLAT_HEADER+record.x*4u+3u]);
    let sun=light.sun_dir.xyz;
    // Only a whole plant (a proxy): a branch volume's own cells hold one
    // branch, and the sun maps carry its neighbours at this very point.
    var exit=0.0;var own=1.0;
    if volume_proxy(record.x) {
        exit=clamp(max(volume_interval(tree,p,sun,1e4).y,0.0)+volume_sun_texel(p)*2.0,0.0,63.0);
        own=clamp(volume_self_transmittance(tree,grid,p,sun),0.0,1.0);
    } else {
        // Ground cover (a volume under 1.2 m tall, e.g. a meadow cell):
        // exit 63.75 tells shade_voxel to skip the canopy-depth crown term,
        // which counted the tree above as its own crown.
        let per_m=length(vec3<f32>(tree.inverse[0].y,tree.inverse[1].y,tree.inverse[2].y));
        if (tree.bounds_max.y-tree.bounds_min.y)/max(per_m,1e-6)<1.2 {exit=63.75;}
        // The sun maps resolve a cell, not the leaves inside it: a mesh
        // crown shows its needles shading each other, a disc only its lit
        // mean (voxels read brighter than the mesh, Joep playtest 5). The
        // leaves of a cell see the sun through the cell itself on average
        // (1-exp(-tau))/tau, tau its optical depth along the sun.
        let ls=normalize(vec3<f32>(dot(tree.inverse[0].xyz,sun),dot(tree.inverse[1].xyz,sun),dot(tree.inverse[2].xyz,sun)));
        let tau=max(volume_sigma(cell,volume_direction(ls)),0.0)*grid.w;
        // Not ground cover: grass voxels already matched their blades.
        if exit<63.0 {own=select(1.0,(1.0-exp(-tau))/tau,tau>1e-4);}
    }
    // A mesh crown shows dark inner branches and gaps between its needles;
    // a disc shows the cell's mean colour. Darken discs toward the trunk and
    // the crown's foot, so a crown has shape under sky light alone (Joep,
    // playtest 5: voxel crowns flat and lighter than the mesh). Not ground
    // cover.
    var albedo=sample.albedo;
    if exit<63.0 && !volume_proxy(record.x) {
        let mid=0.5*(tree.bounds_min.xyz+tree.bounds_max.xyz);
        let half=max(0.5*(tree.bounds_max.xyz-tree.bounds_min.xyz),vec3<f32>(1e-3));
        let inner=1.0-clamp(length((p.xz-mid.xz)/half.xz),0.0,1.0);
        let low=1.0-clamp((p.y-tree.bounds_min.y)/(2.0*half.y),0.0,1.0);
        albedo*=1.0-0.45*clamp(0.6*inner+0.5*low,0.0,1.0);
    }
    let rough_leaf=u32(round(clamp(sample.roughness,0.0,1.0)*255.0))|(u32(round(clamp(sample.leaf_fraction,0.0,1.0)*255.0))<<8u);
    let own_exit=u32(round(own*255.0))|(u32(round(exit*4.0))<<8u);
    textureStore(volume_gbuffer,px,vec4<u32>(pack2x16float(albedo.rg),
        (pack2x16float(vec2<f32>(albedo.b,0.0))&0xffffu)|(rough_leaf<<16u),
        pack2x16float(n.xy),(pack2x16float(vec2<f32>(n.z,0.0))&0xffffu)|(own_exit<<16u)));
}

@compute @workgroup_size(8,8)
fn voxel_splat_resolve(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=volume_config.size_count.xy) {return;}
    let px=vec2<i32>(id.xy);
    textureStore(volume_output,px,vec4<f32>(0.0));
    // VOLUME_MOTION_BEGIN
    // Splat motion is milestone M4; zero rejects history.
    textureStore(volume_motion_output,px,vec4<f32>(0.0));
    // VOLUME_MOTION_END
    let splat=textureLoad(volume_splat_id,px,0).x;
    if splat==0u {return;}
    let depth=textureLoad(volume_splat_depth,px,0);
    // Reversed Z: opaque scene geometry in front wins.
    if textureLoad(depth_tex,px,0)>=depth {return;}
    let ndc=(vec2<f32>(id.xy)+0.5)/vec2<f32>(volume_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);
    let p=hit.xyz/hit.w;
    let direction=normalize(p-view.cam.xyz);
    let record=volume_splats[volume_splat_first()+splat-1u];
    let tree=volume_trees[record.x];
    let cell=unpack_volume_cell(volume_cells[record.y]);
    let d=vec4<f32>(direction,0.0);
    let local_d=vec3<f32>(dot(tree.inverse[0],d),dot(tree.inverse[1],d),dot(tree.inverse[2],d));
    let sigma=max(volume_sigma(cell,volume_direction(normalize(local_d))),1e-6);
    var empty:VolumeSample;
    let sample=volume_normalize_sample(volume_accumulate_cell(empty,cell,sigma));
    let up=vec3<f32>(0.0,1.0,0.0);
    // The sun maps hold this tree too, at their own texel, so looking them
    // up inside the crown made whole crowns shadow themselves black. Its own
    // cells shade it along the sun instead; the maps add everything else
    // from just outside its box.
    let grid=bitcast<vec4<f32>>(volume_splats[VOLUME_SPLAT_HEADER+record.x*4u+3u]);
    let sun=light.sun_dir.xyz;
    let exit=max(volume_interval(tree,p,sun,1e4).y,0.0);
    let others=shadow_at(p+sun*(exit+volume_sun_texel(p)*2.0),up);
    let shadow=mix(1.0,others*volume_self_transmittance(tree,grid,p,sun),light.sun_color.w);
    let cell_m=grid.w/max(length(vec3<f32>(tree.inverse[0].x,tree.inverse[1].x,tree.inverse[2].x)),1e-6);
    // Sky light reaches a cell through the crown above it too. The sky term
    // alone lit inner cells as if they faced open sky, so a voxel crown read
    // pale and blue next to the same tree as a mesh (Joep, 6 November).
    let crown_sky=0.2+0.8*volume_self_transmittance(tree,grid,p,up);
    let rad=volume_radiance(sample,tree,p,-direction,shadow,sky_visibility(p+up*cell_m,up)*crown_sky);
    textureStore(volume_output,px,vec4<f32>(rad,1.0));
}

@compute @workgroup_size(64)
fn voxel_sun_splats(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    volume_sun_splats_impl(volume_shadow_index(group.x+group.y*65535u),lane,true);
}
@group(3) @binding(0) var<uniform> shadow_damage_words:array<vec4<u32>,32>;
// The cached map's size rides in header row 11.y (render_sun).
@compute @workgroup_size(64)
fn voxel_cached_sun_splats(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    let index=volume_shadow_index(group.x+group.y*65535u);
    var damaged=false;
    if lane==0u && index<volume_config.size_count.w {
        let tree=volume_trees[index];
        let vp=mat4x4<f32>(volume_splat_header(0u),volume_splat_header(1u),volume_splat_header(2u),volume_splat_header(3u));
        damaged=shadow_damage_bounds(vp,f32(max(volume_splats[11].y,64u)),tree.bounds_min.xyz,tree.bounds_max.xyz);
    }
    // Only lane zero consumes this predicate; all lanes still participate in
    // the shared level load, including clean owners and the indirect reset.
    volume_sun_splats_impl(index,lane,damaged);
}
