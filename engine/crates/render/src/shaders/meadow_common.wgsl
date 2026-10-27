// Shared ABI/math for GPU meadow placement and its identical depth/colour/MV
// geometry. No scene instances or per-vertex wind trigonometry are required.
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
};
struct MeadowPlant {
    root_scale:vec4<f32>,
    yaw:vec4<f32>, // cosine, sine, native terrain gradient X/Z
    road_variant_tint:vec4<u32>, // packed road gradient, f32 usable clearance, variant, RGB10 pigment
    branches:array<vec4<f32>,4>, // current sin/cos, previous sin/cos
};
const MEADOW_TAU:f32=6.283185307179586;
const MEADOW_PLANT_WORDS:u32=28u;
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
