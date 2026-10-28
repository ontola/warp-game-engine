// Shared local direct light response. Only the first set owns shadow maps;
// small spill lights use finite cones/ranges and add no shadow rendering.
struct Headlight { view_proj:mat4x4<f32>, position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32> };
struct LocalLight { position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32>, source:vec4<f32> };
struct Headlights { lights:array<Headlight,8>, count:vec4<u32>, local:array<LocalLight,16>, local_min:vec4<f32>, local_max:vec4<f32> };
@group(0) @binding(17) var<uniform> headlights:Headlights;
@group(0) @binding(18) var headlight_depth:texture_depth_2d_array;

struct HeadlightSample { direction:vec3<f32>,irradiance:vec3<f32> };
fn incident_light_count(p:vec3<f32>)->u32 {
    let shadowed=min(headlights.count.x,8u);
    // A conservative union of the finite light spheres skips the entire extra
    // loop on distant terrain/forest. Each retained light still range-tests.
    if headlights.count.y==0u || any(p<headlights.local_min.xyz) || any(p>headlights.local_max.xyz) {return shadowed;}
    return shadowed+min(headlights.count.y,16u);
}
fn incident_light_position(index:u32)->vec3<f32> {
    let shadowed=min(headlights.count.x,8u);
    if index<shadowed {return headlights.lights[index].position_range.xyz;}
    return headlights.local[index-shadowed].position_range.xyz;
}
fn local_light_sample(index:u32,p:vec3<f32>)->HeadlightSample {
    var result:HeadlightSample;
    let lamp=headlights.local[index];let offset=p-lamp.position_range.xyz;
    let d2=dot(offset,offset);let range=lamp.position_range.w;
    if d2>=range*range || d2<1e-10 {return result;}
    let outward=offset*inverseSqrt(d2);
    let cone=smoothstep(lamp.direction_outer.w,lamp.intensity_inner.w,dot(outward,lamp.direction_outer.xyz));
    if cone<=0.0 {return result;}
    // Smooth compact support, finite near-source response. Deliberately no
    // visibility fallback disguised as GI: these tiny lights are unshadowed.
    let normalized=d2/(range*range);let fade=max(0.0,1.0-normalized*normalized);
    result.direction=-outward;
    // Inverse square by default; source.y < 2 carries further (Godot's
    // omni attenuation). The power form is exact at 2 and costs one pow.
    let spread=select(pow(max(d2,lamp.source.x),lamp.source.y*0.5),max(d2,lamp.source.x),lamp.source.y==2.0||lamp.source.y==0.0);
    result.irradiance=lamp.intensity_inner.rgb*(cone*fade*fade/spread);
    return result;
}
fn incident_light_sample(index:u32,p:vec3<f32>,bias_normal:vec3<f32>)->HeadlightSample {
    let shadowed=min(headlights.count.x,8u);
    if index<shadowed {return headlight_sample(index,p,bias_normal);}
    return local_light_sample(index-shadowed,p);
}
// Incident light is independent of the receiving material. Evaluate its cone,
// finite range and shadow only ONCE per lamp, before the blade quadrature.
fn headlight_sample(index:u32,p:vec3<f32>,bias_normal:vec3<f32>)->HeadlightSample {
    var result:HeadlightSample;
    let lamp=headlights.lights[index];
    let offset=p-lamp.position_range.xyz;let d2=dot(offset,offset);let dist=sqrt(d2);
    if dist<0.05 || dist>=lamp.position_range.w {return result;}
    let outward=offset/dist;result.direction=-outward;
    let cone=smoothstep(lamp.direction_outer.w,lamp.intensity_inner.w,dot(outward,lamp.direction_outer.xyz));
    if cone<=0.0 {return result;}
    // A small continuous receiver bias. No unshadowed fallback outside
    // the finite light frustum, and no screen-space-only occlusion.
    let biased=p+bias_normal*(0.003+0.00015*dist);
    let clip=lamp.view_proj*vec4<f32>(biased,1.0);
    if clip.w<=0.0 {return result;}
    let ndc=clip.xyz/clip.w;
    if any(abs(ndc.xy)>vec2<f32>(1.0)) || ndc.z<0.0 || ndc.z>1.0 {return result;}
    let uv=ndc.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    let visibility=textureSampleCompareLevel(headlight_depth,shadow_samp,uv,i32(index),ndc.z-0.00001);
    let fade=1.0-smoothstep(lamp.position_range.w*0.8,lamp.position_range.w,dist);
    result.irradiance=lamp.intensity_inner.rgb*(cone*fade*visibility/max(d2,0.04));
    return result;
}
fn headlight_radiance(p:vec3<f32>,n:vec3<f32>,v:vec3<f32>,diffuse:vec3<f32>,rough:f32,f0:vec3<f32>)->vec3<f32> {
    var result=vec3<f32>(0.0);
    for(var index=0u;index<incident_light_count(p);index++) {
        // Opaque surfaces remain one-sided, including their cheap rejection.
        if dot(n,incident_light_position(index)-p)<=0.0 {continue;}
        let sample=incident_light_sample(index,p,n);
        if all(sample.irradiance==vec3<f32>(0.0)) {continue;}
        let ndl=max(dot(n,sample.direction),0.0);
        result+=sample.irradiance*(diffuse*ndl+ggx(n,v,sample.direction,rough,f0));
    }
    return result;
}
fn headlight_grass_radiance(p:vec3<f32>,v:vec3<f32>,base:vec3<f32>,rough:f32)->vec3<f32> {
    var result=vec3<f32>(0.0);
    for(var index=0u;index<incident_light_count(p);index++) {
        let sample=incident_light_sample(index,p,vec3<f32>(0.0,1.0,0.0));
        if all(sample.irradiance==vec3<f32>(0.0)) {continue;}
        result+=sample.irradiance*grass_directional_response(base,v,sample.direction,rough);
    }
    return result;
}
