// Stable material/habitat field shared by source blades, their aggregate and soil.
// Pure world-space functions: no camera, time, illumination or texture resources.

fn meadow_hash(cell:vec2<i32>,salt:u32,seed:u32)->u32 {
    var x=(bitcast<u32>(cell.x)*0x9e3779b1u) ^ (bitcast<u32>(cell.y)*0x85ebca77u) ^ (salt*0xc2b2ae3du) ^ seed;
    x^=x>>16u;x*=0x7feb352du;x^=x>>15u;x*=0x846ca68bu;return x^(x>>16u);
}
fn meadow_random(cell:vec2<i32>,salt:u32,seed:u32)->f32 {
    return f32(meadow_hash(cell,salt,seed)>>8u)/16777216.0;
}
// Shared source/aggregate/contact identity; never depends on a view or pose.
const MEADOW_DENSITY_SALT:u32=191u;
const MEADOW_ROOT_JITTER:f32=0.04;
const MEADOW_ROOT_CELL:f32=0.45;
fn meadow_patch_retained(cell:vec2<i32>,seed:u32,fraction:f32)->bool {
    if fraction>=1.0 {return true;}
    return fraction>0.0 && meadow_random(cell,MEADOW_DENSITY_SALT,seed)<fraction;
}
fn meadow_root_fraction(cell:vec2<i32>,seed:u32)->vec2<f32> {
    let random=vec2<f32>(meadow_random(cell,157u,seed),meadow_random(cell,163u,seed));
    return vec2<f32>(0.5)+(random*2.0-1.0)*MEADOW_ROOT_JITTER;
}
// Analytic basal contact footprint, NOT geometric union coverage or a shadow.
// Radius below one cell minus jitter makes four canonical neighbour centres
// sufficient: every omitted centre has zero support, including at cell seams.
fn meadow_contact_cover(xz:vec2<f32>,seed:u32,density:f32,radius:f32,source_cover:f32)->f32 {
    if density<=0.0 || radius<=0.0 || source_cover<=0.0 {return 0.0;}
    // World-space f32 rounding can exceed a fixed sub-millimetre guard on
    // kilometre-scale fields. Shrink only the bounded support by eight float
    // epsilons at this coordinate, so omitted neighbours remain exactly zero.
    let roundoff=8.0*1.1920928955078125e-7*(max(abs(xz.x),abs(xz.y))+MEADOW_ROOT_CELL);
    let r=min(radius,max(0.0,MEADOW_ROOT_CELL*(1.0-MEADOW_ROOT_JITTER)-0.0001-roundoff));
    if r<=0.0 {return 0.0;}
    let first=vec2<i32>(floor(xz/MEADOW_ROOT_CELL-0.5));var gap=1.0;
    for(var z=0;z<2;z++){for(var x=0;x<2;x++){
        let cell=first+vec2<i32>(x,z);
        if !meadow_patch_retained(cell,seed,density) {continue;}
        let root=(vec2<f32>(cell)+meadow_root_fraction(cell,seed))*MEADOW_ROOT_CELL;
        let d2=dot(xz-root,xz-root);
        let footprint=1.0-smoothstep(0.0625*r*r,r*r,d2);
        gap*=1.0-clamp(source_cover,0.0,1.0)*footprint;
    }}
    return 1.0-gap;
}
fn meadow_noise(p:vec2<f32>,seed:u32)->f32 {
    let cell=vec2<i32>(floor(p));let f=fract(p);let u=f*f*(3.0-2.0*f);
    return mix(mix(meadow_random(cell,103u,seed),meadow_random(cell+vec2<i32>(1,0),103u,seed),u.x),
        mix(meadow_random(cell+vec2<i32>(0,1),103u,seed),meadow_random(cell+vec2<i32>(1,1),103u,seed),u.x),u.y);
}
// Pigment varies in stable world patches, with smaller variation inside each
// patch. This is material albedo, not illumination/AO or a view-distance tint.
// Compute once per root; every source LOD and animated blade uses that value.
fn meadow_material_tint(xz:vec2<f32>,cell:vec2<i32>,seed:u32)->vec3<f32> {
    let broad=meadow_noise(xz/8.5+vec2<f32>(31.7,-14.2),seed);
    let local=meadow_noise(xz/2.3+vec2<f32>(-11.0,42.0),seed);
    let age=smoothstep(0.43,0.79,broad*0.78+local*0.22);
    let fresh=vec3<f32>(0.72,0.89,1.16);
    let olive=vec3<f32>(1.33,1.02,0.92);
    let straw=vec3<f32>(2.12,1.16,1.78);
    let living=mix(fresh,olive,smoothstep(0.25,0.77,local));
    let pigment=mix(living,straw,age*0.80);
    let vigor=0.83+0.25*meadow_noise(xz/3.7+vec2<f32>(7.0,19.0),seed)
        +0.12*meadow_random(cell,71u,seed);
    return pigment*vigor;
}
// Road/slope suitability before conservative native-terrain support rejection.
// Suitable interiors stay fully occupied: pigment/growth variation must not
// remove an entire shared patch and expose its footprint as a bald square.
// This envelope is not screen alpha or a per-blade density distribution.
fn meadow_habitat_coverage(xz:vec2<f32>,road:f32,up:f32,seed:u32)->f32 {
    let verge=smoothstep(0.5,1.4,road)*(1.0-smoothstep(10.0,22.0,road));
    let wood=smoothstep(1.0,3.0,road);
    return max(verge,wood)*smoothstep(0.7,0.74,up);
}
