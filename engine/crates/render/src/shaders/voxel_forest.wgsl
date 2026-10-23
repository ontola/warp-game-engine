// Appended to the shared physical-lighting helpers in shade.wgsl.
// This is fractional surface-density integration, not opaque voxel cubes.
struct VolumeLevel { origin_cell: vec4<f32>, dims_offset: vec4<u32> };
struct VolumeCell {
    albedo_rough: vec4<f32>,
    extinction: array<vec4<f32>,4>,
    normals: array<vec4<f32>,2>,
};
struct VolumeTree {
    inverse: array<vec4<f32>,3>,
    bounds_min: vec4<f32>, bounds_max: vec4<f32>, identity: vec4<u32>,
};
struct VolumeConfig { size_count: vec4<u32>, march: vec4<f32> };
@group(2) @binding(0) var<storage,read> volume_levels: array<VolumeLevel>;
@group(2) @binding(1) var<storage,read> volume_indices: array<u32>;
@group(2) @binding(2) var<storage,read> volume_cells: array<VolumeCell>;
@group(2) @binding(3) var<storage,read> volume_trees: array<VolumeTree>;
@group(2) @binding(4) var<storage,read_write> volume_tiles: array<u32>;
@group(2) @binding(5) var<storage,read_write> volume_stats: array<atomic<u32>>;
@group(2) @binding(6) var volume_output: texture_storage_2d<rgba16float,write>;
@group(2) @binding(7) var<uniform> volume_config: VolumeConfig;

const VOLUME_TILE: u32 = 16u;
const VOLUME_CANDIDATES: u32 = 64u;
const VOLUME_MAX_STEPS: u32 = 1024u;

// One invocation owns one tile; no order-dependent append or counter reset.
// Near-plane intersections conservatively cover the whole screen.
@compute @workgroup_size(8,8)
fn voxel_tiles(@builtin(global_invocation_id) id: vec3<u32>) {
    let tile_dims=(volume_config.size_count.xy+vec2<u32>(15u))/16u;
    if any(id.xy>=tile_dims) { return; }
    let start=(id.y*tile_dims.x+id.x)*(VOLUME_CANDIDATES+1u);
    let tile_lo=vec2<f32>(id.xy*VOLUME_TILE);
    let tile_hi=tile_lo+vec2<f32>(16.0);
    var count=0u;
    for(var i=0u;i<volume_config.size_count.w;i++) {
        let tree=volume_trees[i];
        var lo=vec2<f32>(1e30);
        var hi=vec2<f32>(-1e30);
        var in_front=0u;
        for(var corner=0u;corner<8u;corner++) {
            let p=vec3<f32>(
                select(tree.bounds_min.x,tree.bounds_max.x,(corner&1u)!=0u),
                select(tree.bounds_min.y,tree.bounds_max.y,(corner&2u)!=0u),
                select(tree.bounds_min.z,tree.bounds_max.z,(corner&4u)!=0u));
            let clip=view.view_proj*vec4<f32>(p,1.0);
            if clip.w>view.params.y {
                in_front++;
                let pixel=(clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5)*vec2<f32>(volume_config.size_count.xy);
                lo=min(lo,pixel); hi=max(hi,pixel);
            }
        }
        if in_front==0u { continue; }
        if in_front<8u || (all(hi>=tile_lo)&&all(lo<=tile_hi)) {
            if count<VOLUME_CANDIDATES {volume_tiles[start+1u+count]=i;}
            count++;
        }
    }
    volume_tiles[start]=count;
    atomicMax(&volume_stats[4],count);
    if count>VOLUME_CANDIDATES {atomicAdd(&volume_stats[0],1u);}
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

struct VolumeDirection { indices:vec3<u32>,weights:vec3<f32> };
fn volume_direction(d:vec3<f32>)->VolumeDirection {
    let magnitude=abs(d);
    var axes=array<u32,3>(0u,1u,2u);
    if magnitude[axes[0]]<magnitude[axes[1]] {let swap=axes[0];axes[0]=axes[1];axes[1]=swap;}
    if magnitude[axes[1]]<magnitude[axes[2]] {let swap=axes[1];axes[1]=axes[2];axes[2]=swap;}
    if magnitude[axes[0]]<magnitude[axes[1]] {let swap=axes[0];axes[0]=axes[1];axes[1]=swap;}
    let a=axes[0];let b=axes[1];let c=axes[2];
    var pair=2u;
    if min(a,b)==0u {pair=select(1u,0u,max(a,b)==1u);}
    let diagonal=3u+pair*2u+select(0u,1u,d[a]*d[b]<0.0);
    let corner=9u+select(0u,2u,d.x*d.y<0.0)+select(0u,1u,d.x*d.z<0.0);
    let weight=vec3<f32>(magnitude[a]-magnitude[b],(magnitude[b]-magnitude[c])*1.41421356,magnitude[c]*1.73205081);
    return VolumeDirection(vec3<u32>(a,diagonal,corner),weight/max(dot(weight,vec3<f32>(1.0)),1e-8));
}
fn volume_sigma(cell:VolumeCell,d:VolumeDirection)->f32 {
    var value=0.0;
    for(var k=0u;k<3u;k++) {let i=d.indices[k];value+=cell.extinction[i/4u][i%4u]*d.weights[k];}
    return max(value,0.0);
}
struct VolumeSample { sigma:f32,albedo:vec3<f32>,roughness:f32,normals:array<f32,6> };
fn volume_sample(tree:VolumeTree,world:vec3<f32>,direction:VolumeDirection)->VolumeSample {
    // First packed level is explicit in the fixture; no hidden LOD switch.
    let level=volume_levels[tree.identity.y];
    let p=vec4<f32>(world,1.0);
    let local=vec3<f32>(dot(tree.inverse[0],p),dot(tree.inverse[1],p),dot(tree.inverse[2],p));
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
        let index=volume_indices[level.dims_offset.w+c.x+level.dims_offset.x*(c.y+level.dims_offset.y*c.z)];
        if index==0u {continue;}
        let cell=volume_cells[index-1u];
        let sigma=volume_sigma(cell,direction)*weight;
        result.sigma+=sigma;
        result.albedo+=cell.albedo_rough.rgb*sigma;
        result.roughness+=cell.albedo_rough.a*sigma;
        for(var k=0u;k<6u;k++) {result.normals[k]+=cell.normals[k/4u][k%4u]*sigma;}
    }
    if result.sigma>0.0 {
        result.albedo/=result.sigma; result.roughness/=result.sigma;
        for(var k=0u;k<6u;k++) {result.normals[k]/=result.sigma;}
    }
    return result;
}

fn volume_radiance(sample:VolumeSample,tree:VolumeTree,p:vec3<f32>,v:vec3<f32>,shadow:f32,sky_vis:f32)->vec3<f32> {
    let owner=instances[tree.identity.x];
    // Asset albedo already includes the source material tint.
    let base=sample.albedo*(0.82+0.3*hash2(vec2<f32>(owner.r0.w,owner.r2.w)));
    let sun_e=light.sun_color.rgb*light.sun_dir.w*shadow;
    let bounce=gi.bounce.rgb*(gi.bounce.w*sky_vis+light.sun_dir.w*max(light.sun_dir.y,0.0)*shadow);
    let rough=clamp(sample.roughness,0.04,1.0);
    var direct=vec3<f32>(0.0);var indirect=vec3<f32>(0.0);var specular=vec3<f32>(0.0);
    for(var k=0u;k<6u;k++) {
        let weight=sample.normals[k];
        if weight<=0.0 {continue;}
        // Inverse-transpose local axes, folded towards the viewer exactly as
        // the source two-sided needle normals. Never average opposing normals.
        var n=normalize(tree.inverse[k/2u].xyz)*select(1.0,-1.0,(k&1u)!=0u);
        if dot(n,v)<0.0 {n=-n;}
        let ndl=dot(n,light.sun_dir.xyz);
        let ambient=(sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis))*0.85;
        let refl=reflect(-v,n);
        let env=mix(bounce/PI,environment(refl,rough,bounce),sky_vis);
        let back=vec3<f32>(0.175,0.21,0.1)*max(-ndl,0.0);
        direct+=(base/PI*sun_e*max(ndl,0.0)+sun_e*back/PI+sun_e*ggx(n,v,light.sun_dir.xyz,rough,vec3<f32>(0.01)))*weight;
        indirect+=base/PI*ambient*weight;
        specular+=env*env_brdf(vec3<f32>(0.01),rough,max(dot(n,v),1e-4))*0.85*weight;
    }
    let distance_to_camera=distance(p,view.cam.xyz);
    let fog=max(clamp((distance_to_camera-light.fog_color.w)/max(light.fog.x-light.fog_color.w,1.0),0.0,1.0),1.0-exp(-light.fog.y*distance_to_camera));
    var rad=mix(direct+indirect+specular,light.fog_color.rgb,fog);
    if light.sky.z>0.5&&light.sky.z<1.5 {rad=vec3<f32>(sky_vis)*6000.0;}
    else if light.sky.z>1.5&&light.sky.z<2.5 {rad=base*6000.0;}
    else if light.sky.z>2.5&&light.sky.z<3.5 {rad=direct;}
    else if light.sky.z>3.5&&light.sky.z<4.5 {rad=indirect;}
    else if light.sky.z>4.5&&light.sky.z<5.5 {rad=specular;}
    return rad;
}

@compute @workgroup_size(8,8)
fn voxel_integrate(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=volume_config.size_count.xy) {return;}
    let px=vec2<i32>(id.xy);
    textureStore(volume_output,px,vec4<f32>(0.0));
    let tile_dims=(volume_config.size_count.xy+vec2<u32>(15u))/16u;
    let tile=id.xy/16u;
    let start=(tile.y*tile_dims.x+tile.x)*(VOLUME_CANDIDATES+1u);
    let count=volume_tiles[start];
    if count==0u {return;}
    let fallback=count>VOLUME_CANDIDATES;
    let candidates=select(count,volume_config.size_count.w,fallback);
    let ndc=(vec2<f32>(id.xy)+0.5)/vec2<f32>(volume_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let direction=normalize(far.xyz/far.w-view.cam.xyz);
    let near_point=view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let near_distance=distance(view.cam.xyz,near_point.xyz/near_point.w);
    let depth=textureLoad(depth_tex,px,0);
    var limit=1e30;
    if depth>0.0 {let hit=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);limit=distance(view.cam.xyz,hit.xyz/hit.w);}
    var begin=1e30;var end=0.0;
    for(var j=0u;j<candidates;j++) {
        var index=j;if !fallback {index=volume_tiles[start+1u+j];}
        let interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);
        if interval.y>interval.x {begin=min(begin,interval.x);end=max(end,interval.y);}
    }
    begin=max(begin,near_distance);
    if end<=begin {return;}
    atomicAdd(&volume_stats[2],1u);
    var t=begin;var transmittance=1.0;var color=vec3<f32>(0.0);var steps=0u;
    var next_light=-1.0;var cached_sky=1.0;
    loop {
        if t>=end || transmittance<=0.001 || steps>=VOLUME_MAX_STEPS {break;}
        // Count every iteration, including air gaps and precision failures.
        // Tiny scaled assets must never bypass the watchdog bound.
        steps++;
        var next_entry=end;var segment_end=end;var inside_interval=false;
        // Skip true gaps between tree bounds and split integration at entries /
        // exits so a thin distant crown is never skipped by a large air step.
        for(var j=0u;j<candidates;j++) {
            var index=j;if !fallback {index=volume_tiles[start+1u+j];}
            let interval=volume_interval(volume_trees[index],view.cam.xyz,direction,limit);
            if interval.y<=interval.x || interval.y<=t {continue;}
            if interval.x>t {next_entry=min(next_entry,interval.x);}
            else {inside_interval=true;segment_end=min(segment_end,interval.y);}
        }
        if !inside_interval {if next_entry<=t {break;} t=next_entry;continue;}
        let ds=min(volume_config.march.x,min(segment_end-t,next_entry-t));
        if ds<=0.0 || t+ds<=t {break;}
        let sample_t=t+0.5*ds;
        let p=view.cam.xyz+direction*sample_t;
        var total_sigma=0.0;var emission=vec3<f32>(0.0);
        // Deliberate first-runtime spatial approximation: cache coarse
        // canopy/terrain hemisphere visibility for one metre along the ray.
        // Direct sun shadow still evaluates at the actual occupied sample.
        if sample_t>=next_light {cached_sky=sky_visibility(p,vec3<f32>(0.0,1.0,0.0));next_light=sample_t+1.0;}
        var shadow=1.0;var shadow_ready=false;
        for(var j=0u;j<candidates;j++) {
            var index=j;if !fallback {index=volume_tiles[start+1u+j];}
            let tree=volume_trees[index];
            if any(p<tree.bounds_min.xyz)||any(p>tree.bounds_max.xyz) {continue;}
            let ray=vec4<f32>(direction,0.0);
            let local_direction=vec3<f32>(dot(tree.inverse[0],ray),dot(tree.inverse[1],ray),dot(tree.inverse[2],ray));
            let scale=length(local_direction);
            let sample=volume_sample(tree,p,volume_direction(local_direction/max(scale,1e-8)));
            let sigma=sample.sigma*scale;
            if sigma<=0.0 {continue;}
            if !shadow_ready {shadow=mix(1.0,shadow_at(p,vec3<f32>(0.0,1.0,0.0)),light.sun_color.w);shadow_ready=true;}
            total_sigma+=sigma;
            emission+=sigma*volume_radiance(sample,tree,p,-direction,shadow,cached_sky);
        }
        if total_sigma>0.0 {
            let attenuation=exp(-total_sigma*ds);
            color+=transmittance*(1.0-attenuation)*emission/total_sigma;
            transmittance*=attenuation;
        }
        t+=ds;
    }
    atomicMax(&volume_stats[3],steps);
    if t<end && transmittance>0.001 {
        atomicAdd(&volume_stats[1],1u);
        // Explicitly invalid output; the fixture must fail this diagnostic.
        textureStore(volume_output,px,vec4<f32>(6000.0,0.0,6000.0,1.0));
    } else {textureStore(volume_output,px,vec4<f32>(color,1.0-transmittance));}
}
