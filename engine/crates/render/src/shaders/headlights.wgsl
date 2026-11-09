@group(0) @binding(17) var<uniform> headlights:Headlights;
@group(0) @binding(18) var headlight_depth:texture_depth_2d_array;

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

