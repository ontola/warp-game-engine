// Shared car-only spot response; all views sample one shadow layer per light.
struct Headlight { view_proj:mat4x4<f32>, position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32> };
struct Headlights { lights:array<Headlight,4>, count:vec4<u32> };
@group(0) @binding(17) var<uniform> headlights:Headlights;
@group(0) @binding(18) var headlight_depth:texture_depth_2d_array;

fn headlight_radiance(p:vec3<f32>,n:vec3<f32>,v:vec3<f32>,diffuse:vec3<f32>,rough:f32,f0:vec3<f32>)->vec3<f32> {
    var result=vec3<f32>(0.0);
    for(var index=0u;index<min(headlights.count.x,4u);index++) {
        let lamp=headlights.lights[index];
        let offset=p-lamp.position_range.xyz;let d2=dot(offset,offset);let dist=sqrt(d2);
        if dist<0.05 || dist>=lamp.position_range.w {continue;}
        let outward=offset/dist;let l=-outward;
        let cone=smoothstep(lamp.direction_outer.w,lamp.intensity_inner.w,dot(outward,lamp.direction_outer.xyz));
        let ndl=max(dot(n,l),0.0);
        if cone<=0.0 || ndl<=0.0 {continue;}
        // A small continuous receiver bias. No unshadowed fallback outside
        // the finite light frustum, and no screen-space-only occlusion.
        let biased=p+n*(0.003+0.00015*dist);
        let clip=lamp.view_proj*vec4<f32>(biased,1.0);
        if clip.w<=0.0 {continue;}
        let ndc=clip.xyz/clip.w;
        if any(abs(ndc.xy)>vec2<f32>(1.0)) || ndc.z<0.0 || ndc.z>1.0 {continue;}
        let uv=ndc.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
        let visibility=textureSampleCompareLevel(headlight_depth,shadow_samp,uv,i32(index),ndc.z-0.00001);
        let fade=1.0-smoothstep(lamp.position_range.w*0.8,lamp.position_range.w,dist);
        let irradiance=lamp.intensity_inner.rgb*(cone*fade*visibility/max(d2,0.04));
        result+=irradiance*(diffuse*ndl+ggx(n,v,l,rough,f0));
    }
    return result;
}
