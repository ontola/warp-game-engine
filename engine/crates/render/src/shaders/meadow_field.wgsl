// Stable material/habitat field shared by source blades, their aggregate and soil.
// Pure world-space functions: no camera, time, illumination or texture resources.

fn meadow_hash(cell:vec2<i32>,salt:u32,seed:u32)->u32 {
    var x=bitcast<u32>(cell.x)*0x9e3779b1u ^ bitcast<u32>(cell.y)*0x85ebca77u ^ salt*0xc2b2ae3du ^ seed;
    x^=x>>16u;x*=0x7feb352du;x^=x>>15u;x*=0x846ca68bu;return x^(x>>16u);
}
fn meadow_random(cell:vec2<i32>,salt:u32,seed:u32)->f32 {
    return f32(meadow_hash(cell,salt,seed)>>8u)/16777216.0;
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
