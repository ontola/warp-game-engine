// World-grid traversal of shared source-derived plant volumes. A tree may
// overlap grid cells, but integration is clipped to disjoint ray segments.
// There is no forest-wide candidate scan and no fixed-size candidate loss.
struct VolumeTree {
    inverse: array<vec4<f32>,3>,
    bounds_min: vec4<f32>, bounds_max: vec4<f32>, identity: vec4<u32>,
};
struct VolumeConfig {
    size_count: vec4<u32>, march: vec4<f32>,
    world_min: vec4<f32>, world_max: vec4<f32>,
    // decision-word offset, publication header offset, reserved, reserved
    selection: vec4<u32>,
};
@group(2) @binding(0) var<storage,read> volume_levels: array<VolumeLevel>;
@group(2) @binding(1) var<storage,read> volume_indices: array<u32>;
@group(2) @binding(2) var<storage,read> volume_cells: array<VolumeCell>;
@group(2) @binding(3) var<storage,read> volume_trees: array<VolumeTree>;
@group(2) @binding(4) var<storage,read> world_grid: array<u32>;
@group(2) @binding(5) var<storage,read_write> world_stats: array<atomic<u32>>;
@group(2) @binding(6) var volume_output: texture_storage_2d<rgba16float,write>;
@group(2) @binding(7) var<uniform> volume_config: VolumeConfig;
@group(2) @binding(8) var<storage,read> world_selection: array<u32>;
@group(2) @binding(9) var<storage,read> world_members: array<u32>;

const VOLUME_EMPTY_CEILING: bool = true;
const WORLD_CACHE: u32 = 32u;

// Inverse(M followed by the exact source wind shear). No CPU animation,
// density rebuild, frozen trees or camera-dependent phase at the LOD swap.
fn world_tree(index:u32)->VolumeTree {
    var tree=volume_trees[index];
    let owner=instances[tree.identity.x];
    let shear=wind_coefficients[tree.identity.x].xy;
    let root_y=owner.r1.w;
    for(var row=0u;row<3u;row++) {
        let correction=dot(tree.inverse[row].xz,shear);
        tree.inverse[row].y-=correction;
        tree.inverse[row].w+=correction*root_y;
    }
    return tree;
}
fn world_selected(tree:u32)->bool {
    let group=world_grid[world_grid[7]+tree];
    return source_replacement_group_selected(volume_config.size_count.w,group);
}
fn world_total(word:u32,value:u32) {
    let old=atomicAdd(&world_stats[word],value);
    if old>0xffffffffu-value {atomicAdd(&world_stats[word+1u],1u);}
}

// SHARED_VOLUME_HELPERS

@compute @workgroup_size(8,8)
fn world_integrate(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=volume_config.size_count.xy) {return;}
    let px=vec2<i32>(id.xy);
    textureStore(volume_output,px,vec4<f32>(0.0));
    // Selector publishes the complete generation only after all group flags.
    if world_selection[volume_config.selection.y]!=0x53525031u||world_selection[volume_config.selection.y+4u]!=1u {return;}
    let ndc=(vec2<f32>(id.xy)+0.5)/vec2<f32>(volume_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let direction=normalize(far.xyz/far.w-view.cam.xyz);
    let near=view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let near_distance=distance(view.cam.xyz,near.xyz/near.w);
    let depth=textureLoad(depth_tex,px,0);
    var limit=1e30;
    if depth>0.0 {let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);limit=distance(view.cam.xyz,hit.xyz/hit.w);}
    var box:VolumeTree;
    box.bounds_min=volume_config.world_min;box.bounds_max=volume_config.world_max;
    let interval=volume_interval(box,view.cam.xyz,direction,limit);
    var t=max(interval.x,near_distance);let finish=interval.y;
    if t>=finish {return;}
    atomicAdd(&world_stats[0],1u);
    let origin=vec2<i32>(bitcast<i32>(world_grid[0]),bitcast<i32>(world_grid[1]));
    let dims=vec2<i32>(i32(world_grid[2]),i32(world_grid[3]));
    let pitch=bitcast<f32>(world_grid[4]);
    let p=view.cam.xyz+direction*t;
    var absolute=vec2<i32>(floor(p.xz/pitch));
    for(var axis=0u;axis<2u;axis++) {
        if direction.xz[axis]<0.0&&p.xz[axis]==f32(absolute[axis])*pitch {absolute[axis]--;}
    }
    var cell=clamp(absolute-origin,vec2<i32>(0),dims-vec2<i32>(1));
    var color=vec3<f32>(0.0);var transmittance=1.0;
    var segments=0u;var iterations=0u;var tests=0u;var samples=0u;var reads=0u;
    var empty_jumps=0u;var max_refs=0u;var max_hits=0u;var fallbacks=0u;
    var failed=false;
    // A line crosses at most nx+nz grid faces. Source support is clipped to
    // every segment; neighbouring references never double its optical mass.
    loop {
        if t>=finish||transmittance<=0.001||any(cell<vec2<i32>(0))||any(cell>=dims) {break;}
        if segments>u32(dims.x+dims.y)+2u {failed=true;break;}
        segments++;
        var exits=vec2<f32>(1e30);
        for(var axis=0u;axis<2u;axis++) {
            if abs(direction.xz[axis])>1e-10 {
                let face=f32(origin[axis]+cell[axis]+select(0,1,direction.xz[axis]>0.0))*pitch;
                exits[axis]=(face-view.cam.xz[axis])/direction.xz[axis];
            }
        }
        let end=min(finish,min(exits.x,exits.y));
        let grid_index=u32(cell.x+cell.y*dims.x);
        let first=world_grid[world_grid[5]+grid_index*2u];
        let refs=world_grid[world_grid[5]+grid_index*2u+1u];
        max_refs=max(max_refs,refs);
        var hits:array<VolumeInterval,32>;
        var hit_count=0u;var begin=end;var last=t;
        for(var i=0u;i<refs;i++) {
            let tree_index=world_grid[world_grid[6]+first+i];
            if !world_selected(tree_index) {continue;}
            tests++;
            var hit=volume_interval(volume_trees[tree_index],view.cam.xyz,direction,end);
            hit.x=max(hit.x,t);
            if hit.y<=hit.x {continue;}
            if hit_count<WORLD_CACHE {hits[hit_count]=VolumeInterval(hit,tree_index,0u);}
            hit_count++;begin=min(begin,hit.x);last=max(last,hit.y);
        }
        max_hits=max(max_hits,hit_count);
        let cached=hit_count<=WORLD_CACHE;
        if !cached {fallbacks++;}
        var s=begin;
        // Cache overflow scans ONLY this cell's complete reference range.
        // It never silently drops a tree whose source was omitted.
        loop {
            if s>=last||transmittance<=0.001 {break;}
            iterations++;
            if iterations>1048576u {failed=true;break;}
            var next=last;var step=1e30;var active_count=0u;var empty_to=last;var all_empty=true;
            let count=select(refs,hit_count,cached);
            for(var i=0u;i<count;i++) {
                var tree_index=0u;var hit=vec2<f32>(0.0);
                if cached {tree_index=hits[i].index;hit=hits[i].time;}
                else {
                    tree_index=world_grid[world_grid[6]+first+i];
                    if !world_selected(tree_index) {continue;}
                    tests++;hit=volume_interval(volume_trees[tree_index],view.cam.xyz,direction,end);hit.x=max(hit.x,t);
                }
                if hit.y<=s||hit.y<=hit.x {continue;}
                if hit.x>s {next=min(next,hit.x);continue;}
                active_count++;next=min(next,hit.y);
                let ray=volume_ray(tree_index,hit.y,direction);
                step=min(step,volume_ray_step(ray,s));
                let empty=volume_empty_brick(ray,s);
                all_empty=all_empty&&empty.y>0.5;empty_to=min(empty_to,empty.x);
            }
            if active_count==0u {if next<=s {failed=true;break;}s=next;continue;}
            empty_to=min(empty_to,next);
            if all_empty&&empty_to>s {s=empty_to;empty_jumps++;continue;}
            let ds=min(step,next-s);
            if ds<=0.0||s+ds<=s {failed=true;break;}
            let sample_t=s+0.5*ds;let sample_p=view.cam.xyz+direction*sample_t;
            var sigma_sum=0.0;var emission=vec3<f32>(0.0);
            var shadow=1.0;var sky=1.0;var light_ready=false;
            for(var i=0u;i<count;i++) {
                var tree_index=0u;var hit=vec2<f32>(0.0);
                if cached {tree_index=hits[i].index;hit=hits[i].time;}
                else {
                    tree_index=world_grid[world_grid[6]+first+i];
                    if !world_selected(tree_index) {continue;}
                    tests++;hit=volume_interval(volume_trees[tree_index],view.cam.xyz,direction,end);hit.x=max(hit.x,t);
                }
                if sample_t<hit.x||sample_t>=hit.y {continue;}
                let ray=volume_ray(tree_index,hit.y,direction);
                let tree=world_tree(tree_index);
                let local=ray.origin_exit.xyz+ray.direction_scale.xyz*sample_t;
                let value=volume_sample_local(tree,local,volume_ray_direction(ray),volume_lod(ray,sample_t));
                samples++;reads+=value.cell_samples;
                let sigma=value.sigma*ray.direction_scale.w;
                if sigma<=0.0 {continue;}
                if !light_ready {
                    sky=sky_visibility(sample_p,vec3<f32>(0.0,1.0,0.0));
                    shadow=mix(1.0,shadow_at(sample_p,vec3<f32>(0.0,1.0,0.0)),light.sun_color.w);
                    light_ready=true;
                }
                sigma_sum+=sigma;
                emission+=sigma*volume_radiance(value,tree,sample_p,-direction,shadow,sky);
            }
            if sigma_sum>0.0 {
                let transport=volume_transport_segment(transmittance,sigma_sum,ds);
                color+=transport.opacity_weight*emission/sigma_sum;
                transmittance=transport.transmittance;
            }
            s+=ds;
        }
        if failed {break;}
        t=end;
        if exits.x<=end {cell.x+=select(-1,1,direction.x>0.0);}
        if exits.y<=end {cell.y+=select(-1,1,direction.z>0.0);}
    }
    if failed {atomicAdd(&world_stats[1],1u);textureStore(volume_output,px,vec4<f32>(6000.0,0.0,6000.0,1.0));}
    else {textureStore(volume_output,px,vec4<f32>(color,1.0-transmittance));}
    atomicMax(&world_stats[2],max_refs);atomicMax(&world_stats[3],max_hits);
    atomicAdd(&world_stats[4],fallbacks);atomicMax(&world_stats[5],iterations);
    world_total(8u,segments);world_total(10u,iterations);world_total(12u,tests);
    world_total(14u,samples);world_total(16u,reads);world_total(18u,empty_jumps);
}
