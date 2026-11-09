// Shared local direct light response. Only the first set owns shadow maps;
// small spill lights use finite cones/ranges and add no shadow rendering.
struct Headlight { view_proj:mat4x4<f32>, position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32> };
struct LocalLight { position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32>, source:vec4<f32> };
struct Headlights { lights:array<Headlight,8>, count:vec4<u32>, local:array<LocalLight,32>, local_min:vec4<f32>, local_max:vec4<f32>,
    spot_falloff:array<vec4<f32>,2> };
struct HeadlightSample { direction:vec3<f32>,irradiance:vec3<f32> };
fn incident_light_count(p:vec3<f32>)->u32 {
    let shadowed=min(headlights.count.x,8u);
    // A conservative union of the finite light spheres skips the entire extra
    // loop on distant terrain/forest. Each retained light still range-tests.
    if headlights.count.y==0u || any(p<headlights.local_min.xyz) || any(p>headlights.local_max.xyz) {return shadowed;}
    return shadowed+min(headlights.count.y,32u);
}
fn incident_light_position(index:u32)->vec3<f32> {
    let shadowed=min(headlights.count.x,8u);
    if index<shadowed {return headlights.lights[index].position_range.xyz;}
    return headlights.local[index-shadowed].position_range.xyz;
}
fn local_light_sample(index:u32,p:vec3<f32>)->HeadlightSample {
    return local_light_at(headlights.local[index],p);
}
// One light's irradiance at p, unshadowed (also clustered_lights.wgsl's).
fn local_light_at(lamp:LocalLight,p:vec3<f32>)->HeadlightSample {
    var result:HeadlightSample;
    let offset=p-lamp.position_range.xyz;
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
    // Bias in metres before perspective projection. A fixed 1e-5 depth
    // offset grows to roughly 1.3 m at 80 m with a 5 cm near plane,
    // erasing the occlusion between neighbouring stones.
    let biased=p+bias_normal*(0.003+0.00015*dist)+result.direction*0.002;
    let clip=lamp.view_proj*vec4<f32>(biased,1.0);
    if clip.w<=0.0 {return result;}
    let ndc=clip.xyz/clip.w;
    if any(abs(ndc.xy)>vec2<f32>(1.0)) || ndc.z<0.0 || ndc.z>1.0 {return result;}
    let uv=ndc.xy*vec2<f32>(0.5,-0.5)+vec2<f32>(0.5);
    // Compare each filter tap against the receiving plane at that tap.
    // A constant centre depth makes sloping stone faces shadow themselves.
    let axis=select(vec3<f32>(0.0,1.0,0.0),vec3<f32>(1.0,0.0,0.0),abs(bias_normal.y)>0.9);
    let tangent=normalize(cross(bias_normal,axis));
    let bitangent=cross(bias_normal,tangent);
    let ct=lamp.view_proj*vec4<f32>(tangent,0.0);
    let cb=lamp.view_proj*vec4<f32>(bitangent,0.0);
    let dt=(ct.xyz-ndc*ct.w)/clip.w;
    let db=(cb.xyz-ndc*cb.w)/clip.w;
    let plane=cross(dt,db);
    var slope=vec2<f32>(0.0);
    if abs(plane.z)>1e-12 {slope=-plane.xy/plane.z*vec2<f32>(2.0,-2.0);}
    let visibility=spot_visibility(index,uv,ndc.z,lamp,slope,p);
    // Falloff exponent per spot: 2 is inverse square (the default). A
    // negative exponent uses its size with the local lights' (Godot's)
    // window (1 - (d/range)^4)^2 instead of a fade over the last fifth.
    let kk=headlights.spot_falloff[index/4u][index%4u];
    let k=abs(kk);
    let nr=d2/(lamp.position_range.w*lamp.position_range.w);
    let window=max(0.0,1.0-nr*nr);
    let fade=select(1.0-smoothstep(lamp.position_range.w*0.8,lamp.position_range.w,dist),window*window,kk<0.0);
    let spread=select(pow(max(d2,0.04),k*0.5),max(d2,0.04),k==2.0);
    result.irradiance=lamp.intensity_inner.rgb*(cone*fade*visibility/spread);
    return result;
}
