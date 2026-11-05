// Shared ABI/math for GPU meadow placement and its identical depth/colour/MV
// geometry. No scene instances or per-frame CPU blade uploads are required.
struct MeadowSettings {
    field:vec4<f32>, // terrain origin XZ, terrain spacing, world root spacing
    grid:vec4<u32>, // terrain point dimensions, stable seed, variant count
    shape:vec4<f32>, // growth, source radial bound, source height, fade start
    range:vec4<f32>, // fade end, strength, angle multiplier, current phase
    previous:vec4<f32>, // previous rendered phase, wind direction XZ, valid
    lods:array<vec4<u32>,3>,
    dispatch:vec4<i32>, // reserved origin, root grid width, root capacity
    distances:vec4<f32>, // first/second source-LOD distance, reserved
    support:vec4<f32>, // fixed basal radius, root-relative blade radius, reserved
    edits:vec4<f32>, // live linear RGB multiplier, retained world-cell fraction
    animation:vec4<f32>, // gust/direction scroll, current then previous
    wind_history:vec4<f32>, // prior direction/amplitude; 0 classic, 1 mixed, 2 compact windswept
    previous_camera:vec4<f32>, // previous camera XYZ and history validity
};
struct MeadowPlant {
    root_scale:vec4<f32>,
    yaw:vec4<f32>, // cosine, sine, native terrain gradient X/Z
    road_variant_tint:vec4<u32>, // packed road gradient, f32 usable clearance, variant, RGB10 pigment
    branches:array<vec4<f32>,4>, // current sin/cos, previous sin/cos
};
const MEADOW_TAU:f32=6.283185307179586;
const MEADOW_PLANT_WORDS:u32=28u;
// The material/shape profile remains >.5. Only homogeneous windswept (>1.5)
// omits serialized legacy branch poses; mixed sources keep all 28 words.
fn meadow_plant_words(profile:f32)->u32 {
    return select(MEADOW_PLANT_WORDS,12u,profile>1.5);
}
fn meadow_pool_words(capacity:u32,profile:f32)->u32 {
    return capacity*(meadow_plant_words(profile)+3u);
}
const MEADOW_RECIPE_VERTICES:u32=21u;

// A local road plane is exact only inside its native terrain triangle. Keep
// the entire fixed anchor disk inside all three edges; boundary patches use
// the original conservative whole-patch rejection instead of extrapolation.
fn meadow_road_plane_safe(fraction:vec2<f32>,terrain_cell:f32,anchor_radius:f32)->bool {
    var edge=vec3<f32>(fraction.x,fraction.y,(1.0-fraction.x-fraction.y)*0.7071067811865476);
    if fraction.x+fraction.y>1.0 {
        edge=vec3<f32>(1.0-fraction.x,1.0-fraction.y,(fraction.x+fraction.y-1.0)*0.7071067811865476);
    }
    return min(min(edge.x,edge.y),edge.z)*terrain_cell>=anchor_radius+0.0001;
}
// All vertices and source LODs of a blade share one unanimated anchor. A
// rejected blade is absent from depth, colour and motion alike. Zero gradient
// is the explicit whole-patch fallback; neither camera nor time affects this
// habitat decision, and the slight edge irregularity is never alpha dither.
fn meadow_road_anchor_allowed(packed_gradient:u32,usable_clearance:f32,
    anchor_offset:vec2<f32>,world_anchor:vec2<f32>,seed:u32)->bool {
    let gradient=unpack2x16float(packed_gradient);
    if dot(gradient,gradient)==0.0 {return true;}
    let anchor_key=vec2<i32>(floor(world_anchor*64.0));
    let threshold=0.12*meadow_random(anchor_key,149u,seed);
    return usable_clearance+dot(gradient,anchor_offset)>=threshold;
}
fn meadow_pack_tint(rgb:vec3<f32>)->u32 {
    let q=vec3<u32>(clamp(rgb*0.25,vec3<f32>(0.0),vec3<f32>(1.0))*1023.0+0.5);
    return q.r|(q.g<<10u)|(q.b<<20u);
}
fn meadow_unpack_tint(packed:u32)->vec3<f32> {
    return vec3<f32>(f32(packed&1023u),f32((packed>>10u)&1023u),f32((packed>>20u)&1023u))*(4.0/1023.0);
}
// Vertical shear keeps stems upright and places the basal canopy on the local
// terrain triangle. No per-vertex heightfield samples or extra plant uploads.
fn meadow_ground_shear(offset:vec3<f32>,slope:vec2<f32>)->vec3<f32> {
    return offset+vec3<f32>(0.0,dot(slope,offset.xz),0.0);
}
fn meadow_ground_normal(n:vec3<f32>,slope:vec2<f32>)->vec3<f32> {
    return meadow_safe_normal(vec3<f32>(n.x-slope.x*n.y,n.y,n.z-slope.y*n.y));
}
fn meadow_ground_scale(slope:vec2<f32>)->f32 {
    let s=length(slope);
    return sqrt(1.0+0.25*s*s)+0.5*s;
}
fn meadow_safe_normal(v:vec3<f32>)->vec3<f32> {
    let d=dot(v,v);if d>1e-20 {return v*inverseSqrt(d);}return vec3<f32>(0.0,1.0,0.0);
}
fn meadow_axis(direction:vec2<f32>)->vec3<f32> {
    let d=dot(direction,direction);if d<1e-12 {return vec3<f32>(0.0,0.0,1.0);}
    return vec3<f32>(-direction.y,0.0,direction.x)*inverseSqrt(d);
}
fn meadow_yaw(p:vec3<f32>,cs:vec2<f32>)->vec3<f32> {
    return vec3<f32>(cs.x*p.x+cs.y*p.z,p.y,-cs.y*p.x+cs.x*p.z);
}
fn meadow_rotate(p:vec3<f32>,axis:vec3<f32>,sc:vec2<f32>)->vec3<f32> {
    return p*sc.y+cross(axis,p)*sc.x+axis*dot(axis,p)*(1.0-sc.y);
}
// A source vertex and a source-area sample follow the same fixed-anchor
// transformation. No root, world identity, road decision or phase is invented
// by a volume producer. Geometric normals govern area; shading normals remain
// separate. A zero geometric normal has zero physical area, never an Up leaf.
struct MeadowDeformed {
    world:vec3<f32>,previous_world:vec3<f32>,normal:vec3<f32>,
    geometric_normal:vec3<f32>,area_scale:f32,
};
fn meadow_deform_source(point:vec3<f32>,shading_normal:vec3<f32>,geometric_normal:vec3<f32>,
    anchor:vec3<f32>,root_scale:vec4<f32>,yaw_slope:vec4<f32>,branch_sincos:vec4<f32>,
    wind_direction:vec2<f32>,history_valid:bool)->MeadowDeformed {
    var out:MeadowDeformed;
    let pivot_world=meadow_yaw(anchor,yaw_slope.xy);
    let axis=meadow_axis(wind_direction);
    let relative=meadow_yaw((point-anchor)*root_scale.w,yaw_slope.xy);
    out.world=root_scale.xyz+meadow_ground_shear(pivot_world+meadow_rotate(relative,axis,branch_sincos.xy),yaw_slope.zw);
    out.previous_world=root_scale.xyz+meadow_ground_shear(pivot_world+meadow_rotate(relative,axis,branch_sincos.zw),yaw_slope.zw);
    if !history_valid {out.previous_world=out.world;}
    out.normal=meadow_ground_normal(meadow_rotate(meadow_yaw(shading_normal,yaw_slope.xy),axis,branch_sincos.xy),yaw_slope.zw);
    let n2=dot(geometric_normal,geometric_normal);
    if n2>1e-20 {
        let n=meadow_rotate(meadow_yaw(geometric_normal*inverseSqrt(n2),yaw_slope.xy),axis,branch_sincos.xy);
        // det(vertical shear)=1, so the cofactor's length is the surface
        // Jacobian. Uniform per-blade growth contributes its squared scale;
        // the basal anchor is a translation, not a patch-wide scale.
        let cofactor=vec3<f32>(n.x-yaw_slope.z*n.y,n.y,n.z-yaw_slope.w*n.y);
        out.area_scale=root_scale.w*root_scale.w*length(cofactor);
        out.geometric_normal=meadow_safe_normal(cofactor);
    }
    return out;
}
// Integer harmonics use the same wrapped world gust phase as tree wind.
// Branch detail is stable world-cell data, never a frame/view/random dither.
fn meadow_branch_angle(root:vec2<f32>,direction:vec2<f32>,phase:f32,detail:f32,amplitude:f32)->f32 {
    let along=dot(root,direction);let across=dot(root,vec2<f32>(-direction.y,direction.x));
    let a=along*0.12-phase*9.0;let b=along*0.057+across*0.033-phase*17.0;
    let c=along*0.21+across*0.37-phase*41.0;
    return amplitude*(0.52+0.26*sin(a)+0.09*sin(b)+0.13*sin(c+detail));
}

// Shape/colour/wind technique adapted from SimonDev Quick_Grass, MIT.
// See engine/licenses/Quick_Grass-MIT.txt and docs/log/meadow-windswept-r63.md.
// Upstream noise.glsl credits https://www.shadertoy.com/view/ttc3zr for hashes.
// The source's cubic-smoothed value noise is sampled per blade, not per
// vertex identity or view. Periodic lattice coordinates make time rebasing
// continuous without replacing the source's travelling field by a sine.
fn meadow_noise_hash(cell:vec2<i32>)->f32 {
    var src=bitcast<vec2<u32>>(vec2<f32>(cell&vec2<i32>(4095)));
    let m=0x5bd1e995u;var h=1190494759u;
    src*=m;src^=src>>vec2<u32>(24u);src*=m;
    h*=m;h^=src.x;h*=m;h^=src.y;
    h^=h>>13u;h*=m;h^=h>>15u;
    return bitcast<f32>((h&0x007fffffu)|0x3f800000u)-1.0;
}
fn meadow_value_noise(p:vec2<f32>)->f32 {
    let i=vec2<i32>(floor(p));let f=fract(p);let u=f*f*(3.0-2.0*f);
    return 2.0*mix(mix(meadow_noise_hash(i),meadow_noise_hash(i+vec2<i32>(1,0)),u.x),
        mix(meadow_noise_hash(i+vec2<i32>(0,1)),meadow_noise_hash(i+vec2<i32>(1,1)),u.x),u.y)-1.0;
}
fn meadow_anchor_hash(anchor:vec3<f32>,salt:u32)->u32 {
    let q=bitcast<vec2<u32>>(vec2<i32>(round(anchor.xz*1024.0)));
    var x=(q.x*0x9e3779b1u)^(q.y*0x85ebca77u)^salt;
    x^=x>>16u;x*=0x7feb352du;x^=x>>15u;x*=0x846ca68bu;x^=x>>16u;
    return x;
}
fn meadow_anchor_random(anchor:vec3<f32>,salt:u32)->f32 {
    return f32(meadow_anchor_hash(anchor,salt)>>8u)/16777216.0;
}
fn meadow_rest_lean(anchor:vec3<f32>)->f32 {
    return 0.1+0.3*meadow_anchor_random(anchor,0x68bc21ebu);
}
// Source vertices already carry their authored t. Only arbitrary surface
// samples must invert the rest centreline; both paths use this derivative.
fn meadow_blade_coordinate(t:f32,height:f32,lean:f32)->vec2<f32> {
    let a=lean*t*t;
    return vec2<f32>(t,1.0/(height*(cos(a)-2.0*a*sin(a))));
}
// Rest y/H=t*cos(lean*t²) is monotone for lean<=.4. Recover the normalized
// coordinate for discrete atlas/voxel samples. Three Newton steps converge
// below f32 packing error throughout [0,1].
fn meadow_blade_parameter(y:f32,height:f32,lean:f32)->vec2<f32> {
    let normalized_y=max(y/height,0.0);var t=clamp(normalized_y,0.0,1.0);
    for(var k=0u;k<3u;k++) {
        let a=lean*t*t;let derivative=cos(a)-2.0*a*sin(a);
        t=clamp(t-(t*cos(a)-normalized_y)/derivative,0.0,1.0);
    }
    return meadow_blade_coordinate(t,height,lean);
}
// xyz is the world-space rotation axis; w is the angle at the tip.
fn meadow_blade_wind(world_anchor:vec2<f32>,scroll:vec2<f32>,direction:vec2<f32>,strength:f32)->vec4<f32> {
    let offset=meadow_value_noise(world_anchor*0.05+vec2<f32>(scroll.y));
    let cs=vec2<f32>(cos(offset),sin(offset));
    let d=vec2<f32>(cs.x*direction.x-cs.y*direction.y,cs.y*direction.x+cs.x*direction.y);
    let gust=0.625+0.375*meadow_value_noise(world_anchor*0.25+vec2<f32>(scroll.x));
    return vec4<f32>(meadow_axis(d),1.25*gust*gust*strength);
}
fn meadow_blade_cofactor(n:vec3<f32>,q:vec3<f32>,axis:vec3<f32>,angle_derivative:f32)->vec3<f32> {
    // F(q)=R(A*t(q.y))*q. Cofactor of I+b*Y^T, b=A*t'*cross(axis,q).
    // Rotating normals alone loses the curvature derivative and surface area.
    let b=cross(axis,q)*angle_derivative;
    return n*(1.0+b.y)-vec3<f32>(0.0,dot(b,n),0.0);
}
// The shared nonlinear transform receives t and dt/dy. Pose, history, normal
// cofactor and physical area are identical for source vertices and samples.
struct MeadowBladeWind {current:vec4<f32>,previous:vec4<f32>};
fn meadow_wind_at_anchor(anchor:vec3<f32>,root:vec4<f32>,yaw:vec4<f32>,settings:MeadowSettings)->MeadowBladeWind {
    let xz=root.xz+meadow_yaw(anchor,yaw.xy).xz;
    var out:MeadowBladeWind;
    out.current=meadow_blade_wind(xz,settings.animation.xy,settings.previous.yz,settings.range.y*settings.range.z);
    out.previous=meadow_blade_wind(xz,settings.animation.zw,settings.wind_history.xy,settings.wind_history.z);
    return out;
}
fn meadow_deform_blade_with_wind(point:vec3<f32>,shading_normal:vec3<f32>,geometric_normal:vec3<f32>,
    anchor:vec4<f32>,root_scale:vec4<f32>,yaw_slope:vec4<f32>,branch_sincos:vec4<f32>,
    settings:MeadowSettings,h:vec2<f32>,pose:MeadowBladeWind)->MeadowDeformed {
    var out:MeadowDeformed;
    let pivot=meadow_yaw(anchor.xyz,yaw_slope.xy);
    let q=meadow_yaw((point-anchor.xyz)*root_scale.w,yaw_slope.xy);
    let wind=pose.current;let prior=pose.previous;
    let a=wind.w*h.x;let pa=prior.w*h.x;
    let sc=vec2<f32>(sin(a),cos(a));let psc=vec2<f32>(sin(pa),cos(pa));
    out.world=root_scale.xyz+meadow_ground_shear(pivot+meadow_rotate(q,wind.xyz,sc),yaw_slope.zw);
    out.previous_world=root_scale.xyz+meadow_ground_shear(pivot+meadow_rotate(q,prior.xyz,psc),yaw_slope.zw);
    if settings.previous.w<0.5 {out.previous_world=out.world;}
    let derivative=wind.w*h.y/max(root_scale.w,1e-6);
    let sn=meadow_blade_cofactor(meadow_yaw(shading_normal,yaw_slope.xy),q,wind.xyz,derivative);
    out.normal=meadow_ground_normal(meadow_rotate(sn,wind.xyz,sc),yaw_slope.zw);
    let n2=dot(geometric_normal,geometric_normal);
    if n2>1e-20 {
        let gn=meadow_blade_cofactor(meadow_yaw(geometric_normal*inverseSqrt(n2),yaw_slope.xy),q,wind.xyz,derivative);
        let rotated=meadow_rotate(gn,wind.xyz,sc);
        let cofactor=vec3<f32>(rotated.x-yaw_slope.z*rotated.y,rotated.y,rotated.z-yaw_slope.w*rotated.y);
        out.geometric_normal=meadow_safe_normal(cofactor);
        out.area_scale=root_scale.w*root_scale.w*length(cofactor);
    }
    return out;
}
fn meadow_deform_blade_at_coordinate(point:vec3<f32>,shading_normal:vec3<f32>,geometric_normal:vec3<f32>,
    anchor:vec4<f32>,root_scale:vec4<f32>,yaw_slope:vec4<f32>,branch_sincos:vec4<f32>,
    settings:MeadowSettings,h:vec2<f32>)->MeadowDeformed {
    let pose=meadow_wind_at_anchor(anchor.xyz,root_scale,yaw_slope,settings);
    return meadow_deform_blade_with_wind(point,shading_normal,geometric_normal,anchor,root_scale,yaw_slope,
        branch_sincos,settings,h,pose);
}
fn meadow_deform_blade(point:vec3<f32>,shading_normal:vec3<f32>,geometric_normal:vec3<f32>,
    anchor:vec4<f32>,root_scale:vec4<f32>,yaw_slope:vec4<f32>,branch_sincos:vec4<f32>,
    settings:MeadowSettings)->MeadowDeformed {
    if anchor.w<=0.0 {
        return meadow_deform_source(point,shading_normal,geometric_normal,anchor.xyz,root_scale,yaw_slope,
            branch_sincos,settings.previous.yz,settings.previous.w>=0.5);
    }
    let h=meadow_blade_parameter(point.y-anchor.y,anchor.w,meadow_rest_lean(anchor.xyz));
    return meadow_deform_blade_at_coordinate(point,shading_normal,geometric_normal,anchor,root_scale,
        yaw_slope,branch_sincos,settings,h);
}
// Authored source t is normal.w, shared by ordinary raster and visibility
// reconstruction. Avoid recovering a coordinate already in the vertex ABI.
fn meadow_deform_vertex(point:vec3<f32>,normal:vec4<f32>,geometric_normal:vec3<f32>,
    anchor:vec4<f32>,root_scale:vec4<f32>,yaw_slope:vec4<f32>,branch_sincos:vec4<f32>,
    settings:MeadowSettings)->MeadowDeformed {
    if anchor.w<=0.0 {
        return meadow_deform_source(point,normal.xyz,geometric_normal,anchor.xyz,root_scale,yaw_slope,
            branch_sincos,settings.previous.yz,settings.previous.w>=0.5);
    }
    let h=meadow_blade_coordinate(normal.w,anchor.w,meadow_rest_lean(anchor.xyz));
    return meadow_deform_blade_at_coordinate(point,normal.xyz,geometric_normal,anchor,root_scale,
        yaw_slope,branch_sincos,settings,h);
}
