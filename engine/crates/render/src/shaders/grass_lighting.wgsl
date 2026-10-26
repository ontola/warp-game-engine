// Shared statistical leaf response and local sunlight extinction. These
// source-height bins are the SAME medium as far grass, not a darkening decal.
struct GrassOptics {grid:vec4<f32>,dims:vec4<u32>,profile:vec4<f32>,receiver:vec4<f32>};
@group(0) @binding(13) var<uniform> grass_optics:GrassOptics;
@group(0) @binding(14) var grass_ground:texture_2d<f32>;
@group(0) @binding(15) var grass_density:texture_2d<f32>;
@group(0) @binding(16) var grass_tile_bounds:texture_2d<f32>;

fn grass_interval(value:f32,rate:f32,lo:f32,hi:f32,span:vec2<f32>)->vec2<f32> {
    if rate==0.0 {if value<lo||value>=hi {return vec2<f32>(1.0,0.0);}return span;}
    let a=(lo-value)/rate;let b=(hi-value)/rate;
    return vec2<f32>(max(span.x,min(a,b)),min(span.y,max(a,b)));
}
struct GrassSunSegment {tau:f32,solid_hit:bool,receiver_pending:bool};
fn grass_sun_triangle(cell:vec2<i32>,origin:vec3<f32>,direction:vec3<f32>,begin:f32,end:f32,coverage:f32,receiver_escape:bool,receiver_limit:f32)->GrassSunSegment {
    if end<=begin {return GrassSunSegment(0.0,false,receiver_escape);}
    let a=textureLoad(grass_ground,cell,0).rg;
    let b=textureLoad(grass_ground,cell+vec2<i32>(1,0),0).rg;
    let c=textureLoad(grass_ground,cell+vec2<i32>(0,1),0).rg;
    let e=textureLoad(grass_ground,cell+vec2<i32>(1,1),0).rg;
    let p=origin+direction*begin;
    let uv=(p.xz-grass_optics.grid.xy)/grass_optics.grid.z-vec2<f32>(cell);
    let rate=direction.xz/grass_optics.grid.z;
    let mid=uv+rate*((end-begin)*0.5);
    var ground=a+(b-a)*uv.x+(c-a)*uv.y;
    var derivative=(b-a)*rate.x+(c-a)*rate.y;
    var valid=min(a.x,min(b.x,c.x))>-1e8;
    if mid.x+mid.y>1.0 {
        ground=b+c-e+(e-c)*uv.x+(e-b)*uv.y;derivative=(e-c)*rate.x+(e-b)*rate.y;
        valid=min(b.x,min(c.x,e.x))>-1e8;
    }
    // Missing native chunks are open, not fictitious terrain at world Y=0.
    if !valid {return GrassSunSegment(0.0,false,false);}
    let height=p.y-ground.x;let rise=direction.y-derivative.x;
    // Height above an actual native triangle is affine along this segment.
    // Test solid ground BEFORE habitat/road clipping: a bare rock or road can
    // block sunlight just as grass-covered ground can. Only a roundoff-sized
    // tolerance is used, not the shadow map's much larger normal bias.
    let error=0.001+8.0*1.1920929e-7*(abs(p.y)+abs(ground.x)+abs(rise*(end-begin)));
    var pending=false;
    if min(height,height+rise*(end-begin)) < -error {
        // Refined rock can begin below its native approximation. Only an
        // explicitly bounded TERRAIN receiver may leave that approximation,
        // monotonically upward, inside a receiver-centred native-cell support
        // radius. A receiver next to a cell edge gets the same allowance as
        // one in the middle. The real sun
        // shadow map still sees the refined geometry. Never excuse a later
        // hill, a descending ray, or an unbounded underground receiver.
        if !receiver_escape || height < -grass_optics.receiver.x-error || rise<=0.0 || begin>=receiver_limit {
            return GrassSunSegment(0.0,true,false);
        }
        if end>=receiver_limit && height+rise*(receiver_limit-begin)<-error {
            return GrassSunSegment(0.0,true,false);
        }
        pending=height+rise*(end-begin)<-error;
    }
    if coverage<=0.0 {return GrassSunSegment(0.0,false,pending);}
    var span=grass_interval(ground.y,derivative.y,0.9,1e20,vec2<f32>(0.0,end-begin));
    if span.y<=span.x {return GrassSunSegment(0.0,false,pending);}
    let bin_height=grass_optics.grid.w*0.25;var tau=0.0;
    for(var bin=0u;bin<4u;bin++) {
        let part=grass_interval(height,rise,f32(bin)*bin_height,f32(bin+1u)*bin_height,span);
        tau+=grass_optics.profile[bin]*max(part.y-part.x,0.0);
    }
    return GrassSunSegment(tau*coverage*length(direction.xz),false,pending);
}
fn grass_sun_transmittance_receiver(p:vec3<f32>,sun:vec3<f32>,terrain_receiver:bool)->f32 {
    if !sun_visibility_required() { return 1.0; }
    if grass_optics.dims.z==0u||(grass_optics.dims.w&2u)==0u||sun.y<=0.0||length(sun.xz)<1e-6 {return 1.0;}
    let origin=p+sun*0.005;var t=0.0;var tau=0.0;
    let cell_size=grass_optics.grid.z;
    var cell=vec2<i32>(floor((origin.xz-grass_optics.grid.xy)/cell_size));
    var receiver_escape=terrain_receiver && grass_optics.receiver.x>0.0;
    // Native triangular interpolation uses the four corners of a grid cell;
    // its horizontal support diameter is cell_size*sqrt(2). Bound escape
    // around the actual receiver, not an arbitrary world-grid cell edge.
    // This changes no ray origin or shadow bias. The existing16m trace cap
    // still applies when a nearly vertical ray has little horizontal travel.
    let receiver_limit=min(cell_size*1.41421356237/length(sun.xz),16.0);
    // Local terrain + microshadow only (at most 8 traversed cells / 16 m).
    // Reaching a bound preserves accumulated optical depth; omitted terrain
    // is not assumed opaque. This is not a distant horizon shadow solution.
    for(var step=0u;step<8u;step++) {
        if any(cell<vec2<i32>(0))||any(cell>=vec2<i32>(grass_optics.dims.xy)-vec2<i32>(1))||t>=16.0||tau>=7.0 {break;}
        let tile=cell/8;let bounds=textureLoad(grass_tile_bounds,tile,0).rg;
        // These bounds include ALL known native terrain, including bare rock
        // and road, plus maximum tuft height. Unlike far-rendering bounds they
        // do not discard zero-density cells. Above the maximum an upward ray
        // cannot meet ground or grass before the tile boundary.
        if bounds.y<bounds.x||origin.y+sun.y*t>=bounds.y {
            var next=16.0;
            for(var axis=0u;axis<2u;axis++) {if sun.xz[axis]!=0.0 {
                let edge=(tile[axis]+select(0,1,sun.xz[axis]>0.0))*8;
                next=min(next,(grass_optics.grid[axis]+f32(edge)*cell_size-origin.xz[axis])/sun.xz[axis]);
            }}
            if next>t {
                receiver_escape=false;
                t=next;let q=(origin.xz+sun.xz*t-grass_optics.grid.xy)/cell_size;cell=vec2<i32>(floor(q));
                if sun.x<0.0&&q.x==f32(cell.x) {cell.x--;}
                if sun.z<0.0&&q.y==f32(cell.y) {cell.y--;}
                continue;
            }
        }
        var exits=vec2<f32>(1e20);
        for(var axis=0u;axis<2u;axis++) {if sun.xz[axis]!=0.0 {
            let edge=cell[axis]+select(0,1,sun.xz[axis]>0.0);
            exits[axis]=(grass_optics.grid[axis]+f32(edge)*cell_size-origin.xz[axis])/sun.xz[axis];
        }}
        let end=min(min(exits.x,exits.y),16.0);
        if end<=t {
            if exits.x<=t {cell.x+=i32(sign(sun.x));}if exits.y<=t {cell.y+=i32(sign(sun.z));}continue;
        }
        let coverage=textureLoad(grass_density,cell,0).r;
        let uv=(origin.xz+sun.xz*t-grass_optics.grid.xy)/cell_size-vec2<f32>(cell);
        let diagonal_rate=(sun.x+sun.z)/cell_size;var diagonal=end;
        if diagonal_rate!=0.0 {let crossing=t+(1.0-uv.x-uv.y)/diagonal_rate;if crossing>t&&crossing<end {diagonal=crossing;}}
        let first=grass_sun_triangle(cell,origin,sun,t,diagonal,coverage,receiver_escape,receiver_limit);
        if first.solid_hit {return 0.0;}tau+=first.tau;
        receiver_escape=first.receiver_pending;
        if diagonal<end {
            let second=grass_sun_triangle(cell,origin,sun,diagonal,end,coverage,receiver_escape,receiver_limit);
            if second.solid_hit {return 0.0;}tau+=second.tau;
            receiver_escape=second.receiver_pending;
        }
        // A still-buried receiver cannot leave its bounded support region.
        // Grace never re-arms after the ray first clears native ground.
        if receiver_escape && end>=receiver_limit {return 0.0;}
        t=end;if exits.x<=end {cell.x+=i32(sign(sun.x));}if exits.y<=end {cell.y+=i32(sign(sun.z));}
    }
    if receiver_escape && t>0.0 {return 0.0;}
    return exp(-tau);
}
fn grass_sun_transmittance(p:vec3<f32>,sun:vec3<f32>)->f32 {
    return grass_sun_transmittance_receiver(p,sun,false);
}

struct GrassResponse {direct:vec3<f32>,indirect:vec3<f32>,specular:vec3<f32>};
// One shared angular response for sunlight and finite lights. The existing
// transmission approximation stays unchanged; this adds no ambient energy.
fn grass_leaf_direct(base:vec3<f32>,n:vec3<f32>,v:vec3<f32>,l:vec3<f32>,rough:f32)->vec3<f32> {
    let nl=dot(n,l);
    let back=vec3<f32>(0.12,0.16,0.05)*max(-nl,0.0);
    var specular=vec3<f32>(0.0);
    // GGX has no reflected contribution behind a blade. Avoid normalizing
    // v+l in that case too: directly opposed light/view has no half vector.
    if nl>0.0 {specular=ggx(n,v,l,rough,vec3<f32>(0.016));}
    return base/PI*max(nl,0.0)+back/PI+specular;
}
fn grass_leaf_sample(v:vec3<f32>,index:u32,samples:u32)->vec4<f32> {
    if samples==1u {return vec4<f32>(0.0,1.0,0.0,1.0);}
    var axis=vec2<f32>(1.0,0.0);if length(v.xz)>1e-5 {axis=normalize(v.xz);}
    let across=vec2<f32>(-axis.y,axis.x);
    let angle=(f32(index)+0.5)*PI*0.5;
    let xz=axis*cos(angle)+across*sin(angle);
    var n=normalize(vec3<f32>(xz.x,0.45,xz.y));
    let weight=abs(dot(n,v));if dot(n,v)<0.0 {n=-n;}
    return vec4<f32>(n,weight);
}
// BRDF times projected incidence, averaged over the same visible blades as
// the sun. A grazing/downward-to-lamp direction is not rejected using +Y.
fn grass_directional_response(base:vec3<f32>,v:vec3<f32>,l:vec3<f32>,rough:f32)->vec3<f32> {
    let samples=select(1u,4u,(grass_optics.dims.w&1u)!=0u);
    var result=vec3<f32>(0.0);var weight=0.0;
    for(var i=0u;i<samples;i++) {
        let blade=grass_leaf_sample(v,i,samples);
        result+=grass_leaf_direct(base,blade.xyz,v,l,rough)*blade.w;weight+=blade.w;
    }
    return result/max(weight,1e-6);
}
fn grass_leaf_response(base:vec3<f32>,height:f32,v:vec3<f32>,sun_e:vec3<f32>,sun_visibility:f32,sky_vis:f32,contact:vec2<f32>,rough:f32)->GrassResponse {
    // An unresolved tuft is a distribution of two-sided inclined blades,
    // not a horizontal glossy sheet. Four quadrature normals rotate with
    // viewing azimuth, so no fixed world/cardinal highlight axis is created.
    // Projected-area weights condition that distribution on visible blades.
    let ao=mix(0.45,1.0,clamp(height,0.0,1.0));
    let bounce=gi.bounce.rgb*(gi.bounce.w*sky_vis+light.sun_dir.w*max(light.sun_dir.y,0.0)*sun_visibility);
    let f0=vec3<f32>(0.016);let diffuse=base/PI;
    var sum=0.0;var response:GrassResponse;
    let samples=select(1u,4u,(grass_optics.dims.w&1u)!=0u);
    for(var i=0u;i<samples;i++) {
        let blade=grass_leaf_sample(v,i,samples);let n=blade.xyz;let weight=blade.w;
        let nv=max(dot(n,v),1e-4);
        response.direct+=sun_e*grass_leaf_direct(base,n,v,light.sun_dir.xyz,rough)*weight;
        let ambient=(sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis))*ao*contact.x;
        response.indirect+=diffuse*ambient*weight;
        let env=mix(bounce/PI,environment(reflect(-v,n),rough,bounce),sky_vis)*mix(contact.y,contact.x,smoothstep(0.08,0.6,rough));
        response.specular+=env*env_brdf(f0,rough,nv)*ao*weight;
        sum+=weight;
    }
    response.direct/=max(sum,1e-6);response.indirect/=max(sum,1e-6);response.specular/=max(sum,1e-6);
    return response;
}
