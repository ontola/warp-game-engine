// What a game's surface shader (game_surface.wgsl, material flag SURFACE)
// may read, for the low tier (forward_lit.wgsl): the same names as in
// shade.wgsl, with only the View and Light fields the games use filled in
// (forward.rs `SurfaceUniform`). A surface that reads anything else does not
// compile here; the low tier then leaves that scene to the medium tier
// (frame.rs `forward_lit`).
struct View { cam:vec4<f32> };
// One buffer for both bindings (forward.rs SurfaceUniform): cam comes first.
struct Light { view_cam:vec4<f32>, sun_dir:vec4<f32>, sun_color:vec4<f32>, anim:vec4<f32> };
struct Instance { r0:vec4<f32>, r1:vec4<f32>, r2:vec4<f32>, sphere:vec4<f32>, first_cluster:u32, cluster_count:u32, scale:f32, flags:u32 };
@group(2) @binding(0) var<uniform> view:View;
@group(2) @binding(1) var<uniform> light:Light;
@group(2) @binding(2) var<storage,read> scene_tables:array<vec4<u32>>;
@group(2) @binding(3) var<storage,read> instances:array<Instance>;
fn aux_word(i:u32)->u32 {
    let v=scene_tables[scene_tables[1].x+(i>>2u)];
    return v[i&3u];
}
fn aux_len()->u32 { return scene_tables[1].y; }
// The low tier always asks a surface for its cheap version (shade.wgsl's
// override of the same name, set per scene by shade_features.rs).
const SURFACE_LOW_DETAIL: bool = true;
// The sky's irradiance from its nine SH coefficients, as shade.wgsl.
fn sky_irradiance(n:vec3<f32>)->vec3<f32> {
    let s=lit.sh;
    let e=0.429043*s[8].rgb*(n.x*n.x-n.y*n.y)+0.743125*s[6].rgb*n.z*n.z+0.886227*s[0].rgb-0.247708*s[6].rgb
        +2.0*0.429043*(s[4].rgb*n.x*n.y+s[7].rgb*n.x*n.z+s[5].rgb*n.y*n.z)
        +2.0*0.511664*(s[3].rgb*n.x+s[1].rgb*n.y+s[2].rgb*n.z);
    return max(e,vec3<f32>(0.0));
}
