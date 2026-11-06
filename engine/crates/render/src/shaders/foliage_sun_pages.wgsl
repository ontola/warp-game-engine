// Fixed world-space source-alpha shadow pages. Missing data is explicit;
// the caller keeps its prior fallback until a requested page is rendered.
struct FoliageSunPageEntry { key_layer:vec4<i32> };
struct FoliageSunPages {
    world_to_light:mat4x4<f32>, geometry:vec4<f32>, counts:vec4<u32>,
    entries:array<FoliageSunPageEntry>,
};
// Read through sun_pages_*() and sun_page_entry(i), from the scene table
// buffer (shade.wgsl); the struct above documents the layout.

// x=visibility, y=known. Layer-1 certifies an empty caster column; an absent
// entry never does. The field contains trees only: terrain is traced separately.
fn foliage_page_visibility(p:vec3<f32>,n:vec3<f32>,plane_n:vec3<f32>)->vec2<f32> {
    if sun_pages_counts().z==0u {return vec2<f32>(0.0);}
    let q=(sun_pages_world_to_light()*vec4<f32>(p,1.0)).xyz;
    if q.z<0.0 || q.z>1.0 {return vec2<f32>(0.0);}
    let key=vec2<i32>(floor(q.xy/sun_pages_geometry().x));
    var lo=0u;var hi=sun_pages_counts().x;
    for(var k=0u;k<10u;k++) {
        if lo>=hi {break;}
        let mid=(lo+hi)/2u;let entry=sun_page_entry(mid);
        if entry.x<key.x || (entry.x==key.x && entry.y<key.y) {lo=mid+1u;}else{hi=mid;}
    }
    if lo>=sun_pages_counts().x {return vec2<f32>(0.0);}
    let entry=sun_page_entry(lo);
    if any(entry.xy!=key) || u32(entry.w)!=sun_pages_counts().y {return vec2<f32>(0.0);}
    if entry.z == -1 {return vec2<f32>(1.0);}
    if entry.z<0 || u32(entry.z)>=sun_pages_counts().w {return vec2<f32>(0.0);}
    let cell=fract(q.xy/sun_pages_geometry().x);
    let uv=vec2<f32>(cell.x,1.0-cell.y)*sun_pages_geometry().y+sun_pages_geometry().z;
    // Numerical offset is measured in centimetres, never one metre-scale far
    // texel. Floor geometry does not cast in this atlas, so cannot shadow itself.
    let offset=(sun_pages_world_to_light()*vec4<f32>((n+light.sun_dir.xyz)*0.01,0.0)).z;
    let reference=q.z+offset;
    return vec2<f32>(textureSampleCompareLevel(far_sun_tex,shadow_samp,uv,entry.z,reference),1.0);
}
