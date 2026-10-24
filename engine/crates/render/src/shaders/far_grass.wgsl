// Appended to shade.wgsl. Statistical grass density follows native terrain;
// each ray interval is integrated analytically, never as an opaque grass roof.
struct FarGrassParams {
    grid:vec4<f32>, dims:vec4<u32>, full:vec4<u32>, profile:vec4<f32>,
    shading:array<vec4<f32>,4>, ranges:vec4<f32>, bounds:vec4<f32>,
};
@group(2) @binding(0) var<uniform> fg:FarGrassParams;
@group(2) @binding(1) var fg_terrain:texture_2d<f32>;
@group(2) @binding(2) var fg_coverage:texture_2d<f32>;
@group(2) @binding(3) var fg_tiles:texture_2d<f32>;
@group(2) @binding(4) var fg_color:texture_storage_2d<rgba16float,write>;
@group(2) @binding(5) var fg_distance:texture_storage_2d<r32float,write>;
@group(2) @binding(6) var<storage,read_write> fg_stats:array<atomic<u32>>;
const FG_MAX_ITERATIONS:u32=512u;

fn fg_clip(value:f32,rate:f32,lo:f32,hi:f32,span:vec2<f32>)->vec2<f32> {
    // Half-open vertical bins prevent a parallel ray on a shared bin face
    // from receiving both bins' extinction.
    if rate==0.0 {
        if value<lo || value>=hi {return vec2<f32>(1.0,0.0);}
        return span;
    }
    let a=(lo-value)/rate;let b=(hi-value)/rate;
    return vec2<f32>(max(span.x,min(a,b)),min(span.y,max(a,b)));
}
fn fg_road_clip(value:f32,rate:f32,span:vec2<f32>)->vec2<f32> {
    if rate==0.0 {
        if value<fg.ranges.w {return vec2<f32>(1.0,0.0);}
        return span;
    }
    let crossing=(fg.ranges.w-value)/rate;
    if rate>0.0 {return vec2<f32>(max(span.x,crossing),span.y);}
    return vec2<f32>(span.x,min(span.y,crossing));
}
struct FarAccum { transmission:f32, position:vec3<f32>, root_distance:f32, shading:vec4<f32> };
fn fg_triangle(cell:vec2<i32>,a:vec2<f32>,b:vec2<f32>,c:vec2<f32>,e:vec2<f32>,
    begin:f32,end:f32,direction:vec3<f32>,horizontal:f32,coverage:f32,acc:ptr<function,FarAccum>) {
    if end<=begin {return;}
    let p=view.cam.xyz+direction*begin;
    let uv=(p.xz-fg.grid.xy-vec2<f32>(cell)*fg.grid.z)/fg.grid.z;
    let rate=direction.xz/fg.grid.z;
    let mid_uv=uv+rate*((end-begin)*0.5);
    var field=a+(b-a)*uv.x+(c-a)*uv.y;
    var derivative=(b-a)*rate.x+(c-a)*rate.y;
    if mid_uv.x+mid_uv.y>1.0 {
        field=b+c-e+(e-c)*uv.x+(e-b)*uv.y;
        derivative=(e-c)*rate.x+(e-b)*rate.y;
    }
    let road_span=fg_road_clip(field.y,derivative.y,vec2<f32>(0.0,end-begin));
    if road_span.y<=road_span.x {return;}
    let h0=p.y-field.x;let dh=direction.y-derivative.x;
    let bin_height=fg.grid.w*0.25;
    for(var order=0u;order<4u;order++) {
        // Front-to-back order along the ray, even on downhill terrain.
        let bin=select(3u-order,order,dh>=0.0);
        let span=fg_clip(h0,dh,f32(bin)*bin_height,f32(bin+1u)*bin_height,road_span);
        let sigma=fg.profile[bin]*coverage*horizontal;
        if span.y<=span.x || sigma<=0.0 {continue;}
        let attenuation=exp(-sigma*(span.y-span.x));
        let weight=(*acc).transmission*(1.0-attenuation);
        let midpoint=0.5*(span.x+span.y);
        let distance=begin+midpoint;
        (*acc).position+=(view.cam.xyz+direction*distance)*weight;
        (*acc).root_distance+=distance*horizontal*weight;
        (*acc).shading+=fg.shading[bin]*weight;
        (*acc).transmission*=attenuation;
    }
}
fn fg_cell_at(t:f32,direction:vec3<f32>)->vec2<i32> {
    let q=(view.cam.xz+direction.xz*t-fg.grid.xy)/fg.grid.z;
    var cell=vec2<i32>(floor(q));
    for(var axis=0u;axis<2u;axis++) {
        if direction.xz[axis]<0.0 && q[axis]==f32(cell[axis]) {cell[axis]--;}
    }
    return cell;
}
fn fg_grid_face(axis:u32,edge:i32)->f32 {
    // The root slab and DDA exits must describe the same world-space face.
    return fg.grid[axis]+f32(edge)*fg.grid.z;
}
fn fg_exits(cell:vec2<i32>,group:i32,direction:vec3<f32>)->vec2<f32> {
    var result=vec2<f32>(1e30);
    for(var axis=0u;axis<2u;axis++) {
        let d=direction.xz[axis];
        if d==0.0 {continue;}
        let block=cell[axis]/group;
        let edge=select(block*group,(block+1)*group,d>0.0);
        let coordinate=fg_grid_face(axis,edge);
        result[axis]=(coordinate-view.cam.xz[axis])/d;
    }
    return result;
}
fn fg_lighting(p:vec3<f32>,shading:vec4<f32>,direction:vec3<f32>)->vec3<f32> {
    // One visibility/lighting evaluation at the absorbed-light centroid.
    // Same grass response; no far hero-contact query or per-bin GI loop.
    let n=vec3<f32>(0.0,1.0,0.0);let v=-direction;let l=light.sun_dir.xyz;
    // RGB already contains the source photo/height-factor covariance. W is
    // original per-tuft normalized height, not height/max_possible_tuft.
    let base=shading.rgb*1.25*(1.0-light.fog.w*0.25);
    let shadow=mix(1.0,shadow_at(p,n*sign(dot(n,l)+1e-4)),light.sun_color.w)*mix(1.0,grass_sun_transmittance(p,l),light.sun_color.w);
    let sun_e=light.sun_color.rgb*light.sun_dir.w*shadow;
    let sky_vis=sky_visibility(p,n);
    let rough=0.85;
    let response=grass_leaf_response(base,shading.w,v,sun_e,shadow,sky_vis,vec2<f32>(1.0),rough);
    let direct=response.direct+headlight_radiance(p,n,v,base/PI,rough,vec3<f32>(0.016));let indirect=response.indirect;let specular=response.specular;
    var radiance=apply_distance_fog(direct+indirect+specular,distance(p,view.cam.xyz),direction);
    if light.sky.z>0.5&&light.sky.z<1.5 {radiance=vec3<f32>(sky_vis)*6000.0;}
    else if light.sky.z>1.5&&light.sky.z<2.5 {radiance=base*6000.0;}
    else if light.sky.z>2.5&&light.sky.z<3.5 {radiance=direct;}
    else if light.sky.z>3.5&&light.sky.z<4.5 {radiance=indirect;}
    else if light.sky.z>4.5&&light.sky.z<5.5 {radiance=specular;}
    return radiance;
}
@compute @workgroup_size(8,8)
fn far_grass_integrate(@builtin(global_invocation_id) id:vec3<u32>) {
    if any(id.xy>=fg.dims.zw) {return;}
    let output=vec2<i32>(id.xy);
    textureStore(fg_color,output,vec4<f32>(0.0));
    textureStore(fg_distance,output,vec4<f32>(0.0));
    if view.extra.w > 0.5 && view.extra.w < 1.5 {return;}
    if fg.bounds.y<fg.bounds.x {return;}
    let full_pixel=min(vec2<f32>(id.xy*2u)+1.0,vec2<f32>(fg.full.xy)-0.5);
    let ndc=full_pixel/vec2<f32>(fg.full.xy)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let far=view.inv_view_proj*vec4<f32>(ndc,1e-4,1.0);
    let direction=normalize(far.xyz/far.w-view.cam.xyz);
    let horizontal=length(direction.xz);
    if horizontal<1e-8 {return;}
    let near=view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
    let minimum_range=select(view.grass.x,0.0,view.extra.w>=1.5);
    var span=vec2<f32>(max(minimum_range/horizontal,distance(view.cam.xyz,near.xyz/near.w)),fg.ranges.z/horizontal);
    // Closest of the source 2x2 opaque pixels (reversed Z). This intentionally
    // under-covers silhouettes rather than leaking grass onto a car/rock.
    var opaque=0.0;
    for(var corner=0u;corner<4u;corner++) {
        let p=min(id.xy*2u+vec2<u32>(corner&1u,corner>>1u),fg.full.xy-vec2<u32>(1u));
        opaque=max(opaque,textureLoad(depth_tex,vec2<i32>(p),0));
    }
    if opaque>0.0 {let hit=view.inv_view_proj*vec4<f32>(ndc,opaque,1.0);span.y=min(span.y,distance(view.cam.xyz,hit.xyz/hit.w));}
    let lo=vec3<f32>(fg.grid.x,fg.bounds.x,fg.grid.y);
    let hi=vec3<f32>(fg_grid_face(0u,i32(fg.dims.x)-1),fg.bounds.y,fg_grid_face(1u,i32(fg.dims.y)-1));
    for(var axis=0u;axis<3u;axis++) {
        if direction[axis]==0.0 {
            if view.cam[axis]<lo[axis] || view.cam[axis]>hi[axis] {return;}
        } else {
            let a=(lo[axis]-view.cam[axis])/direction[axis];let b=(hi[axis]-view.cam[axis])/direction[axis];
            span=vec2<f32>(max(span.x,min(a,b)),min(span.y,max(a,b)));
        }
    }
    if span.y<=span.x {return;}
    atomicAdd(&fg_stats[0],1u);
    var acc:FarAccum;acc.transmission=1.0;
    var t=span.x;
    // The slab test already proved that entry belongs to this field. The
    // reconstructed position may round a fraction of an ulp outside a face;
    // correct only its integer ownership, without moving either ray endpoint.
    var cell=clamp(fg_cell_at(t,direction),vec2<i32>(0),vec2<i32>(fg.dims.xy)-vec2<i32>(2));
    var iterations=0u;var grid_complete=false;
    let step=vec2<i32>(sign(direction.xz));
    loop {
        if t>=span.y || acc.transmission<=0.001 || iterations>=FG_MAX_ITERATIONS {break;}
        iterations++;
        if any(cell<vec2<i32>(0)) || any(cell>=vec2<i32>(fg.dims.xy)-vec2<i32>(1)) {
            // After a valid entry, monotone DDA can leave this convex grid
            // only through its exit face. No medium exists beyond that face,
            // even when its rounded t is one ulp below the root slab's end.
            grid_complete=true;break;
        }
        let exits=fg_exits(cell,1,direction);
        if min(exits.x,exits.y)<=t {
            // Float rounding exactly at a face changes the integer cell,
            // never skips a positive ray interval. The watchdog still counts.
            if exits.x<=t {cell.x+=step.x;}
            if exits.y<=t {cell.y+=step.y;}
            continue;
        }
        let tile_exits=fg_exits(cell,8,direction);
        let tile_end=min(span.y,min(tile_exits.x,tile_exits.y));
        let bounds=textureLoad(fg_tiles,cell/8,0).rg;
        let y0=view.cam.y+direction.y*t;let y1=view.cam.y+direction.y*tile_end;
        if tile_end>t && (bounds.y<bounds.x || min(y0,y1)>bounds.y || max(y0,y1)<bounds.x) {
            t=tile_end;cell=fg_cell_at(t,direction);continue;
        }
        let leave=min(span.y,min(exits.x,exits.y));
        if leave<=t {break;}
        let coverage=textureLoad(fg_coverage,cell,0).r;
        if coverage>0.0 {
            let a=textureLoad(fg_terrain,cell,0).rg;
            let b=textureLoad(fg_terrain,cell+vec2<i32>(1,0),0).rg;
            let c=textureLoad(fg_terrain,cell+vec2<i32>(0,1),0).rg;
            let e=textureLoad(fg_terrain,cell+vec2<i32>(1,1),0).rg;
            let uv=(view.cam.xz+direction.xz*t-fg.grid.xy-vec2<f32>(cell)*fg.grid.z)/fg.grid.z;
            let rate=(direction.x+direction.z)/fg.grid.z;
            var diagonal=leave;
            if rate!=0.0 {
                let crossing=t+(1.0-uv.x-uv.y)/rate;
                if crossing>t&&crossing<leave {diagonal=crossing;}
            }
            fg_triangle(cell,a,b,c,e,t,diagonal,direction,horizontal,coverage,&acc);
            if diagonal<leave {fg_triangle(cell,a,b,c,e,diagonal,leave,direction,horizontal,coverage,&acc);}
        }
        t=leave;
        if exits.x<=leave {cell.x+=step.x;}
        if exits.y<=leave {cell.y+=step.y;}
    }
    atomicMax(&fg_stats[2],iterations);
    if !grid_complete&&t<span.y&&acc.transmission>0.001 {
        atomicAdd(&fg_stats[1],1u);
        textureStore(fg_color,output,vec4<f32>(6000.0,0.0,6000.0,1.0));
        textureStore(fg_distance,output,vec4<f32>(fg.ranges.z));return;
    }
    let alpha=1.0-acc.transmission;
    if alpha<=0.0 {return;}
    let p=acc.position/alpha;let root_distance=acc.root_distance/alpha;
    let outer=1.0-smoothstep(fg.ranges.y,fg.ranges.z,root_distance);
    let radiance=fg_lighting(p,acc.shading/alpha,direction);
    // UNWEIGHTED with respect to the near/far handoff. The compositor alone
    // combines representations. Only the explicit outer 160..240 m fade is
    // applied here, once, to both premultiplied radiance and alpha.
    textureStore(fg_color,output,vec4<f32>(radiance*alpha,alpha)*outer);
    textureStore(fg_distance,output,vec4<f32>(root_distance));
}
