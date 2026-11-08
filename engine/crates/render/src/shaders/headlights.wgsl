// Shared local direct light response. Only the first set owns shadow maps;
// small spill lights use finite cones/ranges and add no shadow rendering.
struct Headlight { view_proj:mat4x4<f32>, position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32> };
struct LocalLight { position_range:vec4<f32>, direction_outer:vec4<f32>, intensity_inner:vec4<f32>, source:vec4<f32> };
struct Headlights { lights:array<Headlight,8>, count:vec4<u32>, local:array<LocalLight,32>, local_min:vec4<f32>, local_max:vec4<f32>,
    spot_falloff:array<vec4<f32>,2> };
@group(0) @binding(17) var<uniform> headlights:Headlights;
@group(0) @binding(18) var headlight_depth:texture_depth_2d_array;

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
// A spot's depth (0..1 of its perspective) as metres from the lamp.
fn spot_metres(z:f32,far:f32)->f32 {
    let r=far/(SPOT_NEAR-far);
    return r*SPOT_NEAR/(z+r);
}
const SPOT_NEAR:f32=0.05;
const SPOT_TAPS=array<vec2<f32>,12>(vec2(-0.326,-0.406),vec2(-0.840,-0.074),vec2(-0.696,0.457),vec2(-0.203,0.621),
    vec2(0.962,-0.195),vec2(0.473,-0.480),vec2(0.519,0.767),vec2(0.185,-0.893),vec2(0.507,0.064),vec2(0.896,0.412),
    vec2(-0.322,-0.933),vec2(-0.792,-0.598));
// Hard (one compare) unless the game set a light size: then PCSS, a
// blocker search and a filter as wide as the penumbra there.
fn spot_visibility(index:u32,uv:vec2<f32>,z:f32,lamp:Headlight,slope:vec2<f32>,world_p:vec3<f32>)->f32 {
    let size=bitcast<f32>(headlights.count.z);
    if size<=0.0 {return textureSampleCompareLevel(headlight_depth,shadow_samp,uv,i32(index),z);}
    let far=lamp.position_range.w;
    let receiver=spot_metres(z,far);
    // A tangent plane is only local: on a curved bevel its extrapolation can
    // cross an unrelated blocker. Limit correction to 2 cm in light depth,
    // not a fixed projected epsilon (which grows with distance).
    let correction_limit=SPOT_NEAR*far/(far-SPOT_NEAR)*0.02/max(receiver*(receiver-0.02),0.001);
    // uv per metre at the receiver: the map spans 2 tan(outer) there.
    let c=lamp.direction_outer.w;let tan_half=sqrt(max(1.0-c*c,1e-6))/max(c,1e-3);
    let per_m=1.0/(2.0*receiver*tan_half);
    let dims=vec2<f32>(textureDimensions(headlight_depth));
    // Hash integer screen pixels: lamp-space gradient noise stretches into
    // conspicuous stripes on grazing surfaces and at close viewing distances.
    let screen=view.view_proj*vec4<f32>(world_p,1.0);
    let px=vec2<u32>(max((screen.xy/screen.w*vec2<f32>(0.5,-0.5)+0.5)*view.size.xy,vec2<f32>(0.0)));
    var seed=px.x*1973u+px.y*9277u+index*26699u+911u;
    seed=(seed^(seed>>16u))*2246822519u;
    seed=(seed^(seed>>13u))*3266489917u;
    seed=seed^(seed>>16u);
    let turn=6.2831853*f32(seed&65535u)/65536.0;
    let rot=mat2x2<f32>(cos(turn),sin(turn),-sin(turn),cos(turn));
    let search=clamp(size*2.0*per_m,2.0/dims.x,0.06);
    var blockers=0.0;var found=0.0;
    for(var i=0;i<12;i++) {
        let p=clamp(uv+rot*SPOT_TAPS[i]*search,vec2(0.0),vec2(1.0));
        let d=textureLoad(headlight_depth,vec2<i32>(p*dims),i32(index),0);
        let correction=clamp(dot(p-uv,slope),-correction_limit,correction_limit);
        let tap_z=z+correction;
        if d<tap_z {blockers+=spot_metres(clamp(d-correction,0.0,1.0),far);found+=1.0;}
    }
    if found<0.5 {return 1.0;}
    let blocker=blockers/found;
    let penumbra=size*max(receiver-blocker,0.0)/max(blocker,0.05);
    let radius=clamp(penumbra*per_m,1.0/dims.x,0.06);
    var lit=0.0;
    for(var i=0;i<12;i++) {
        let delta=rot*SPOT_TAPS[i]*radius;
        let correction=clamp(dot(delta,slope),-correction_limit,correction_limit);
        lit+=textureSampleCompareLevel(headlight_depth,shadow_samp,uv+delta,i32(index),z+correction);
    }
    return lit/12.0;
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

