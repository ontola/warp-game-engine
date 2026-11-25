// World-owned vegetation bricks. The provider partitions XZ interiors, so a
// source sample belongs to exactly one brick even when its root lies in a halo.
// Cell optics are the same resource-free VolumeCell/Beer helpers as foliage.
// This native-only path writes fractional C/alpha, never opaque surface depth.
struct BrickConfig { size_count:vec4<u32>, limits:vec4<u32>, quality:vec4<f32> };
@group(2) @binding(0) var<storage,read> brick_levels:array<VolumeLevel>;
@group(2) @binding(1) var<storage,read> brick_cells:array<VolumeCell>;
@group(2) @binding(2) var<storage,read> brick_status:array<vec4<u32>>;
// dispatch xyz, active jobs, then tile masks and compact tile indices.
@group(2) @binding(3) var<storage,read_write> brick_work:array<atomic<u32>>;
@group(2) @binding(4) var<storage,read_write> brick_stats:array<atomic<u32>>;
@group(2) @binding(5) var brick_output:texture_storage_2d<rgba16float,write>;
@group(2) @binding(6) var<uniform> brick_config:BrickConfig;

const BRICK_TILE:u32=16u;
const BRICK_LEVELS:u32=4u;
const BRICK_MAX_GROUPS:u32=16u;
const BRICK_JOB_ROW:u32=65535u;

fn brick_ready(index:u32)->bool {
    let state=brick_status[index];
    return brick_config.limits.x!=0u && state.x==brick_config.limits.x
        && state.y==state.x && state.z==0u;
}
fn brick_upper(level:VolumeLevel)->vec3<f32> {
    return level.origin_cell.xyz+vec3<f32>(level.dims_offset.xyz)*level.origin_cell.w;
}
fn brick_interval(level:VolumeLevel,origin:vec3<f32>,direction:vec3<f32>,limit:f32)->vec2<f32> {
    let lower=level.origin_cell.xyz;let upper=brick_upper(level);
    var enter=0.0;var leave=limit;
    for(var axis=0u;axis<3u;axis++) {
        if abs(direction[axis])<1e-12 {
            // Half-open ownership prevents two neighboring bricks from
            // integrating the same ray lying exactly on their common face.
            if origin[axis]<lower[axis] || origin[axis]>=upper[axis] {return vec2<f32>(1.0,0.0);}
        } else {
            let a=(lower[axis]-origin[axis])/direction[axis];
            let b=(upper[axis]-origin[axis])/direction[axis];
            enter=max(enter,min(a,b));leave=min(leave,max(a,b));
        }
    }
    return vec2<f32>(enter,leave);
}
fn brick_add_total(word:u32,value:u32) {
    if value==0u {return;}
    let old=atomicAdd(&brick_stats[word],value);
    if old>0xffffffffu-value {atomicAdd(&brick_stats[word+1u],1u);}
}

// A tile scans at most16 group bounds, never any individual meadow roots.
// One writer per tile gives deterministic masks; only job scheduling varies.
@compute @workgroup_size(8,8)
fn vegetation_brick_tiles(@builtin(global_invocation_id) id:vec3<u32>) {
    let tile_dims=(brick_config.size_count.xy+vec2<u32>(15u))/16u;
    if any(id.xy>=tile_dims) {return;}
    let tile_index=id.y*tile_dims.x+id.x;
    let tile_min=vec2<f32>(id.xy*16u);let tile_max=tile_min+16.0;
    var mask=0u;
    for(var index=0u;index<brick_config.size_count.w;index++) {
        if !brick_ready(index) {
            if tile_index==0u {atomicAdd(&brick_stats[0],1u);}
            continue;
        }
        let level=brick_levels[index*BRICK_LEVELS];
        let lower=level.origin_cell.xyz;let upper=brick_upper(level);
        var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var front=0u;
        for(var corner=0u;corner<8u;corner++) {
            let p=vec3<f32>(select(lower.x,upper.x,(corner&1u)!=0u),
                select(lower.y,upper.y,(corner&2u)!=0u),select(lower.z,upper.z,(corner&4u)!=0u));
            let clip=view.view_proj*vec4<f32>(p,1.0);
            if clip.w>view.params.y {
                front++;
                let pixel=(clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5)*vec2<f32>(brick_config.size_count.xy);
                lo=min(lo,pixel);hi=max(hi,pixel);
            }
        }
        if front>0u && (front<8u || (all(hi>=tile_min)&&all(lo<=tile_max))) {mask|=1u<<index;}
    }
    atomicStore(&brick_work[4u+tile_index],mask);
    if mask==0u {return;}
    let job=atomicAdd(&brick_work[3],1u);
    atomicStore(&brick_work[4u+brick_config.limits.z+job],tile_index);
    atomicMax(&brick_work[0],min(job+1u,BRICK_JOB_ROW));
    atomicMax(&brick_work[1],4u*(job/BRICK_JOB_ROW+1u));
    atomicStore(&brick_work[2],1u);
    atomicAdd(&brick_stats[1],1u);
    let extent=min(vec2<u32>(16u),brick_config.size_count.xy-id.xy*16u);
    atomicAdd(&brick_stats[2],extent.x*extent.y);
    atomicMax(&brick_stats[4],countOneBits(mask));
}

fn brick_radiance(sample:VolumeSample,p:vec3<f32>,v:vec3<f32>)->vec3<f32> {
    let base=sample.albedo;let rough=clamp(sample.roughness,0.04,1.0);
    var direct=vec3<f32>(0.0);var indirect=vec3<f32>(0.0);var specular=vec3<f32>(0.0);
    var mean_sky=0.0;
    for(var k=0u;k<VOLUME_NORMAL_COUNT;k++) {
        let moment=sample.normals[k];
        if moment.w<=0.0 || dot(moment.xyz,moment.xyz)<=1e-16 {continue;}
        var n=normalize(moment.xyz);if dot(n,v)<0.0 {n=-n;}
        var shadow=1.0;
        if light.sun_dir.w>0.0 {
            let toward_sun=n*select(-1.0,1.0,dot(n,light.sun_dir.xyz)>=0.0);
            // Cell moments retain source shading normals, not an invented up
            // normal. Geometry has been aggregated, so this normal is also
            // the receiver plane approximation for shared shadow filtering.
            shadow=mix(1.0,shadow_at_surface(p,toward_sun,n,0.0),light.sun_color.w);
        }
        let sky_vis=sky_visibility(p,n);let back_vis=sky_visibility(p,-n);
        mean_sky+=sky_vis*moment.w;
        let bounce=gi.bounce.rgb*(gi.bounce.w*sky_vis+light.sun_dir.w*max(light.sun_dir.y,0.0)*shadow);
        let front=sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis);
        let back=sky_irradiance(-n)*back_vis+bounce*(1.0-back_vis);
        let env=mix(bounce/PI,environment(reflect(-v,n),rough,bounce),sky_vis);
        let response=thin_foliage_response(base,n,v,rough,sample.leaf_fraction,sample.leaf_fraction,
            light.sun_color.rgb*light.sun_dir.w*shadow,front,back,env);
        direct+=response.direct*moment.w;indirect+=response.indirect*moment.w;specular+=response.specular*moment.w;
        for(var lamp=0u;lamp<incident_light_count(p);lamp++) {
            let toward=incident_light_position(lamp)-p;
            let incident=incident_light_sample(lamp,p,n*select(-1.0,1.0,dot(n,toward)>=0.0));
            if any(incident.irradiance>vec3<f32>(0.0)) {
                direct+=incident.irradiance*volume_thin_leaf_direct(base,n,v,incident.direction,rough,sample.leaf_fraction)*moment.w;
            }
        }
    }
    var rad=apply_distance_fog(direct+indirect+specular,distance(p,view.cam.xyz),-v);
    if light.sky.z>0.5&&light.sky.z<1.5 {rad=vec3<f32>(mean_sky)*6000.0;}
    else if light.sky.z>1.5&&light.sky.z<2.5 {rad=base*6000.0;}
    else if light.sky.z>2.5&&light.sky.z<3.5 {rad=direct;}
    else if light.sky.z>3.5&&light.sky.z<4.5 {rad=indirect;}
    else if light.sky.z>4.5&&light.sky.z<5.5 {rad=specular;}
    return rad;
}

struct BrickHit { time:vec2<f32>,index:u32,padding:u32 };
fn brick_next_face(level:VolumeLevel,coord:i32,axis:u32,origin:vec3<f32>,direction:vec3<f32>,exit:f32)->f32 {
    if abs(direction[axis])<1e-12 {return 1e30;}
    let boundary=coord+select(0,1,direction[axis]>0.0);
    // The slab interval is authoritative. Once in the outward-facing final
    // cell, every remaining interval ends no later than this brick/opaque
    // exit. Use that exact endpoint rather than an independently accumulated
    // approximation of it: all remaining optical density is still integrated.
    if boundary<=0 || boundary>=i32(level.dims_offset[axis]) {return exit;}
    let plane=level.origin_cell[axis]+f32(boundary)*level.origin_cell.w;
    return (plane-origin[axis])/direction[axis];
}
// A mip is acceleration data only: it never replaces the selected optical
// mip. Require exact zeros, including the provider's nonempty leaf marker and
// normal weights. Sigma alone is insufficient: dividing a tiny positive
// child by eight can underflow, while normalized occupancy remains nonzero.
// Published providers must preserve nonnegative child moments in their mips.
fn brick_exactly_empty(cell:VolumeCell)->bool {
    for(var k=0u;k<4u;k++) {
        if any(cell.extinction[k]!=vec4<f32>(0.0)) {return false;}
    }
    for(var k=0u;k<6u;k++) {if cell.normals[k].w!=0.0 {return false;}}
    return true;
}
fn brick_empty_ancestor(group:u32,mip:u32,coord:vec3<i32>)->u32 {
    var ancestor=BRICK_LEVELS-1u;
    loop {
        if ancestor<=mip {return mip;}
        let level=brick_levels[group*BRICK_LEVELS+ancestor];
        let c=vec3<u32>(coord)>>vec3<u32>(ancestor-mip);
        let index=level.dims_offset.w+c.x+level.dims_offset.x*(c.y+level.dims_offset.y*c.z);
        if brick_exactly_empty(brick_cells[index]) {return ancestor;}
        ancestor--;
    }
    return mip;
}
// Boundaries are evaluated in the ORIGINAL selected-mip grid. Reconstructing
// floor(ray(end)) can select the wrong side after f32 cancellation; using the
// original face comparisons also preserves simultaneous edge/corner ties.
fn brick_empty_exit(level:VolumeLevel,base:vec3<i32>,width:i32,origin:vec3<f32>,direction:vec3<f32>,exit:f32)->f32 {
    var end=exit;
    for(var axis=0u;axis<3u;axis++) {
        let last=base[axis]+select(0,width-1,direction[axis]>0.0);
        end=min(end,brick_next_face(level,last,axis,origin,direction,exit));
    }
    return end;
}
fn brick_jump_coordinate(level:VolumeLevel,coord:vec3<i32>,base:vec3<i32>,width:i32,
    origin:vec3<f32>,direction:vec3<f32>,exit:f32,end:f32)->vec3<i32> {
    var result=coord;
    for(var axis=0u;axis<3u;axis++) {
        if abs(direction[axis])<1e-12 {continue;}
        let step=select(-1,1,direction[axis]>0.0);
        // At most eight selected-grid boundaries lie in an ancestor. Binary
        // search their monotone analytic times: four comparisons per axis,
        // independent of how many fine intervals the ray has bypassed.
        var lo=0;var hi=select(coord[axis]-base[axis]+1,base[axis]+width-coord[axis],step>0);
        for(var probe=0u;probe<4u;probe++) {
            if lo>=hi {break;}
            let middle=(lo+hi+1)/2;
            let before=coord[axis]+(middle-1)*step;
            if brick_next_face(level,before,axis,origin,direction,exit)<=end {lo=middle;}
            else {hi=middle-1;}
        }
        result[axis]+=lo*step;
    }
    return result;
}
fn brick_projected_cell(cell:f32,direction:vec3<f32>)->vec2<f32> {
    let row_w=vec3<f32>(view.view_proj[0].w,view.view_proj[1].w,view.view_proj[2].w);
    let row_x=vec3<f32>(view.view_proj[0].x,view.view_proj[1].x,view.view_proj[2].x);
    let row_y=vec3<f32>(view.view_proj[0].y,view.view_proj[1].y,view.view_proj[2].y);
    let depth_rate=max(dot(row_w,direction),1e-6);
    let ndc=vec2<f32>(dot(row_x,direction),dot(row_y,direction))/depth_rate;
    let jx=(row_x-ndc.x*row_w)*(0.5*f32(brick_config.size_count.x));
    let jy=(row_y-ndc.y*row_w)*(0.5*f32(brick_config.size_count.y));
    let diameter=cell*1.73205081;
    return vec2<f32>(diameter*sqrt(dot(jx,jx)+dot(jy,jy))/depth_rate,0.5*diameter/depth_rate);
}
struct BrickMipChoice { fine:u32,coarse:u32,weight:f32 };
fn brick_choose_mips(projected:vec2<f32>,entry:f32)->BrickMipChoice {
    var result=BrickMipChoice(0u,0u,0.0);
    if brick_config.quality.x<=0.0 {return result;}
    let ratio=brick_config.quality.x*max(entry,1e-5)/(projected.x+brick_config.quality.x*projected.y);
    let coordinate=log2(max(ratio,1.0));
    let coarse=u32(clamp(floor(coordinate),0.0,3.0));
    result.fine=coarse;result.coarse=coarse;
    // Both endpoints must be eligible. Blend only in the declared short
    // interval AFTER eligibility; no coarse optical support is used early.
    if brick_config.quality.y>0.0 && coarse>0u {
        let weight=clamp((coordinate-f32(coarse))/brick_config.quality.y,0.0,1.0);
        if weight<1.0 {
            result.fine=coarse-1u;
            result.weight=weight*weight*(3.0-2.0*weight);
            if weight==0.0 {result.coarse=result.fine;}
        }
    }
    return result;
}
// At the coarse endpoint, every child segment must use the SAME clipped
// coarse-segment lighting point. Otherwise changing traversal scale introduces
// a separate lighting quadrature pop even when optical moments are identical.
fn brick_cell_midpoint(level:VolumeLevel,c:vec3<u32>,origin:vec3<f32>,direction:vec3<f32>,hit:vec2<f32>)->f32 {
    var enter=hit.x;var exit=hit.y;
    for(var axis=0u;axis<3u;axis++) {
        if abs(direction[axis])<1e-12 {continue;}
        let lower=level.origin_cell[axis]+f32(c[axis])*level.origin_cell.w;
        let upper=level.origin_cell[axis]+f32(c[axis]+1u)*level.origin_cell.w;
        let a=(lower-origin[axis])/direction[axis];let b=(upper-origin[axis])/direction[axis];
        enter=max(enter,min(a,b));exit=min(exit,max(a,b));
    }
    return (enter+max(enter,exit))*0.5;
}
fn brick_pixel(pixel:vec2<u32>,mask:u32) {
    if any(pixel>=brick_config.size_count.xy) {return;}
    let ndc=(vec2<f32>(pixel)+0.5)/vec2<f32>(brick_config.size_count.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let origin=view.cam.xyz;let direction=normalize(far.xyz/far.w-origin);
    let near_point=view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let near_t=distance(origin,near_point.xyz/near_point.w);
    var limit=1e30;let depth=textureLoad(depth_tex,vec2<i32>(pixel),0);
    if depth>0.0 {let p=view.inv_view_proj*vec4<f32>(ndc,depth,1.0);limit=distance(origin,p.xyz/p.w);}
    var hits:array<BrickHit,16>;var count=0u;
    for(var index=0u;index<brick_config.size_count.w;index++) {
        if (mask&(1u<<index))==0u {continue;}
        var interval=brick_interval(brick_levels[index*4u],origin,direction,limit);
        interval.x=max(interval.x,near_t);
        if interval.y<=interval.x {continue;}
        hits[count]=BrickHit(interval,index,0u);count++;
    }
    if count==0u {return;}
    atomicAdd(&brick_stats[3],1u);atomicMax(&brick_stats[5],count);
    for(var i=1u;i<count;i++) {
        let value=hits[i];var j=i;
        loop {if j==0u {break;}if hits[j-1u].time.x<=value.time.x {break;}hits[j]=hits[j-1u];j--;}
        hits[j]=value;
    }
    let optical_direction=volume_direction(direction);
    let signed_step=vec3<i32>(sign(direction));
    var transmittance=1.0;var color=vec3<f32>(0.0);var steps=0u;var occupied=0u;var max_mip=0u;
    var skipped_empty=0u;var skip_regions=0u;
    var jump_regions=0u;var jump_fallbacks=0u;
    var mip_hits:array<u32,4>;var blended_hits=0u;var blended_segments=0u;var optical_reads=0u;
    var over_budget=0u;var max_finest_pixels=0.0;
    var failed=false;
    var previous_exit=near_t;
    for(var hit_index=0u;hit_index<count;hit_index++) {
        var hit=hits[hit_index];let fine=brick_levels[hit.index*4u];
        // World-key ownership is exact, but f32 origin+extent can overlap its
        // neighbor by one ULP. Never integrate that arithmetic seam twice.
        hit.time.x=max(hit.time.x,previous_exit);previous_exit=hit.time.y;
        if hit.time.y<=hit.time.x {continue;}
        // Entry is the closest point in this brick. A mip-zero fallback does
        // NOT mean its footprint fits: report that failure for source retention.
        let projected=brick_projected_cell(fine.origin_cell.w,direction);
        var finest_pixels=1e30;
        if hit.time.x>projected.y {finest_pixels=min(projected.x/(hit.time.x-projected.y),1e30);}
        max_finest_pixels=max(max_finest_pixels,finest_pixels);
        if brick_config.quality.x>0.0 && finest_pixels>brick_config.quality.x {over_budget++;}
        let choice=brick_choose_mips(projected,hit.time.x);let mip=choice.fine;
        mip_hits[mip]++;
        if choice.weight>0.0 {blended_hits++;}
        max_mip=max(max_mip,choice.coarse);
        let level=brick_levels[hit.index*4u+mip];let cell_size=level.origin_cell.w;
        let at=(origin+direction*hit.time.x-level.origin_cell.xyz)/cell_size;
        // At an exact grid face, select the cell on the entering side.
        var coord=vec3<i32>(floor(at));
        for(var axis=0u;axis<3u;axis++) {
            if direction[axis]<0.0 && at[axis]==floor(at[axis]) {coord[axis]--;}
        }
        coord=clamp(coord,vec3<i32>(0),vec3<i32>(level.dims_offset.xyz)-1);
        var next=vec3<f32>(1e30);
        for(var axis=0u;axis<3u;axis++) {
            next[axis]=brick_next_face(level,coord[axis],axis,origin,direction,hit.time.y);
        }
        var t=hit.time.x;
        loop {
            if t>=hit.time.y {break;}
            if steps+jump_regions>=brick_config.limits.y {failed=true;atomicAdd(&brick_stats[13],1u);break;}
            if any(coord<vec3<i32>(0))||any(coord>=vec3<i32>(level.dims_offset.xyz)) {failed=true;atomicAdd(&brick_stats[14],1u);break;}
            if brick_config.limits.w!=0u && mip+1u<BRICK_LEVELS {
                let ancestor=brick_empty_ancestor(hit.index,mip,coord);
                if ancestor>mip {
                    let shift=ancestor-mip;
                    let parent=vec3<u32>(coord)>>vec3<u32>(shift);
                    if (brick_config.limits.w&2u)!=0u {
                        let width=i32(1u<<shift);let base=vec3<i32>(parent)*width;
                        let end=brick_empty_exit(level,base,width,origin,direction,hit.time.y);
                        if end>t && end<=hit.time.y {
                            if end==hit.time.y {
                                // Preserve the analytic opaque/brick tail
                                // exactly, including an exit inside the parent.
                                t=end;jump_regions++;continue;
                            }
                            let advanced=brick_jump_coordinate(level,coord,base,width,origin,direction,hit.time.y,end);
                            if all(advanced>=vec3<i32>(0)) && all(advanced<vec3<i32>(level.dims_offset.xyz)) && any(advanced!=coord) {
                                coord=advanced;t=end;
                                for(var axis=0u;axis<3u;axis++) {
                                    next[axis]=brick_next_face(level,coord[axis],axis,origin,direction,hit.time.y);
                                }
                                jump_regions++;continue;
                            }
                        }
                        // Numerical uncertainty must only cost work. The
                        // selected fine DDA below remains the fallback.
                        jump_fallbacks++;
                    }
                    if (brick_config.limits.w&1u)!=0u {
                    let skipped_before=skipped_empty;
                    // Bypass selected-cell loads and optical reconstruction
                    // throughout this certified empty region. Retain the
                    // original face equations rather than reconstructing a
                    // new coordinate at a coarse exit: that preserves f32
                    // ties, terminal tails and every occupied interval bit
                    // for bit, and counts real bypassed cells cheaply.
                    loop {
                        if t>=hit.time.y {break;}
                        if steps+jump_regions>=brick_config.limits.y {failed=true;atomicAdd(&brick_stats[13],1u);break;}
                        if any(coord<vec3<i32>(0))||any(coord>=vec3<i32>(level.dims_offset.xyz)) {failed=true;atomicAdd(&brick_stats[14],1u);break;}
                        if any((vec3<u32>(coord)>>vec3<u32>(shift))!=parent) {break;}
                        let end=min(hit.time.y,min(next.x,min(next.y,next.z)));
                        if end>t {skipped_empty++;t=end;}
                        var crossed=false;
                        for(var axis=0u;axis<3u;axis++) {
                            if next[axis]<=t {
                                coord[axis]+=signed_step[axis];
                                next[axis]=brick_next_face(level,coord[axis],axis,origin,direction,hit.time.y);crossed=true;
                            }
                        }
                        steps++;
                        if !crossed && t<hit.time.y {failed=true;atomicAdd(&brick_stats[15],1u);break;}
                    }
                    if skipped_empty>skipped_before {skip_regions++;}
                    if failed {break;}
                    continue;
                    }
                }
            }
            let end=min(hit.time.y,min(next.x,min(next.y,next.z)));
            if end>t {
                let c=vec3<u32>(coord);
                let cell=brick_cells[level.dims_offset.w+c.x+level.dims_offset.x*(c.y+level.dims_offset.y*c.z)];
                optical_reads++;
                var sigma=volume_sigma(cell,optical_direction);
                var sample:VolumeSample;
                var lighting_time=(t+end)*0.5;
                if choice.weight>0.0 {
                    // Nested cell boundaries permit ONE finer-grid traversal.
                    // Mix extinction and extinction-weighted moments, never
                    // opacity, and never black material from an empty mip.
                    if sigma>0.0 {sample=volume_accumulate_cell(sample,cell,sigma*(1.0-choice.weight));}
                    let coarse=brick_levels[hit.index*4u+choice.coarse];let parent=c>>vec3<u32>(1u);
                    let coarse_cell=brick_cells[coarse.dims_offset.w+parent.x+coarse.dims_offset.x*(parent.y+coarse.dims_offset.y*parent.z)];
                    optical_reads++;blended_segments++;
                    let coarse_sigma=volume_sigma(coarse_cell,optical_direction);
                    if coarse_sigma>0.0 {sample=volume_accumulate_cell(sample,coarse_cell,coarse_sigma*choice.weight);}
                    sigma=sample.sigma;
                    if sigma>0.0 {lighting_time=mix(lighting_time,brick_cell_midpoint(coarse,parent,origin,direction,hit.time),choice.weight);}
                }else if sigma>0.0 {
                    sample=volume_accumulate_cell(sample,cell,sigma);
                }
                if sigma>0.0 {
                    sample=volume_normalize_sample(sample);
                    let transport=volume_transport_segment(transmittance,sigma,end-t);
                    let p=origin+direction*lighting_time;
                    color+=brick_radiance(sample,p,-direction)*transport.opacity_weight;
                    transmittance=transport.transmittance;occupied++;
                }
                t=end;
            }
            // Advance ALL simultaneous faces. No tiny positive step is
            // dropped and no arbitrary minimum march distance is introduced.
            var crossed=false;
            for(var axis=0u;axis<3u;axis++) {
                if next[axis]<=t {
                    coord[axis]+=signed_step[axis];
                    next[axis]=brick_next_face(level,coord[axis],axis,origin,direction,hit.time.y);crossed=true;
                }
            }
            steps++;
            if !crossed && t<hit.time.y {failed=true;atomicAdd(&brick_stats[15],1u);break;}
        }
        if failed {break;}
    }
    atomicMax(&brick_stats[6],steps);atomicMax(&brick_stats[7],max_mip);
    brick_add_total(8u,steps);brick_add_total(10u,occupied);
    brick_add_total(16u,skipped_empty);brick_add_total(18u,skip_regions);
    brick_add_total(20u,jump_regions);brick_add_total(22u,jump_fallbacks);
    for(var mip=0u;mip<4u;mip++) {brick_add_total(24u+mip*2u,mip_hits[mip]);}
    brick_add_total(32u,blended_hits);brick_add_total(34u,blended_segments);
    brick_add_total(36u,optical_reads);brick_add_total(38u,over_budget);
    atomicMax(&brick_stats[40],bitcast<u32>(max_finest_pixels));
    if failed {
        // Never present a truncated integral as a valid replacement. The
        // fixture must fail this counter; magenta makes integration defects
        // visible. Validated dimensions mathematically bound DDA work.
        atomicAdd(&brick_stats[12],1u);textureStore(brick_output,vec2<i32>(pixel),vec4<f32>(6000.0,0.0,6000.0,1.0));return;
    }
    textureStore(brick_output,vec2<i32>(pixel),vec4<f32>(color,1.0-transmittance));
}

@compute @workgroup_size(8,8)
fn vegetation_brick_integrate(@builtin(workgroup_id) work:vec3<u32>,@builtin(local_invocation_id) local:vec3<u32>) {
    let job=work.x+(work.y/4u)*BRICK_JOB_ROW;
    if job>=atomicLoad(&brick_work[3]) {return;}
    let tile_index=atomicLoad(&brick_work[4u+brick_config.limits.z+job]);
    let tile=vec2<u32>(tile_index%brick_config.size_count.z,tile_index/brick_config.size_count.z);
    let offset=vec2<u32>(work.y&1u,(work.y>>1u)&1u)*8u;
    brick_pixel(tile*16u+offset+local.xy,atomicLoad(&brick_work[4u+tile_index]));
}
