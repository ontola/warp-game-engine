// Visible source rear lenses, on per-car material slots. No new lights or
// shadow passes. car_model selects only original rear triangles, not whole
// shared glass materials. Lamp-only material fields: second.xy are red/white
// surround radiance, second.z red core radiance; second.w is 0: rear atlas,
// 1: authored red lens, 2: authored clear reverse. flake.xy/zw are two authored
// body-local core centres; wear.xy are their radii, wear.z the texture's bright
// reference. No model names, world-space light or exposure compensation here.
fn car_lamp_emission(flags:u32, source_base:vec3<f32>, inst:Instance, p:vec3<f32>)->vec3<f32> {
    let tag=flags>>16u;
    if tag==0u {return vec3<f32>(0.0);}
    let cm=car_materials[tag-1u];
    if u32(cm.colour.w+0.5)!=7u {return vec3<f32>(0.0);}
    let levels=clamp(cm.second.xy,vec2<f32>(0.0),vec2<f32>(8000.0));
    if all(levels==vec2<f32>(0.0)){return vec3<f32>(0.0);}
    let base=clamp(source_base,vec3<f32>(0.0),vec3<f32>(1.0));
    let peak=max(base.r,max(base.g,base.b));
    let visible=smoothstep(0.003,0.03,peak);
    var red=1.0-smoothstep(0.08,0.18,base.g/max(base.r,0.0001));
    red*=1.0-smoothstep(0.1,0.25,base.b/max(base.r,0.0001));
    var white=smoothstep(0.35,0.7,min(base.r,min(base.g,base.b))/max(peak,0.0001));
    if cm.second.w>0.5 && cm.second.w<1.5 {red=1.0;white=0.0;}
    if cm.second.w>1.5 {red=0.0;white=1.0;}
    var core=0.0;
    if levels.x>0.0 && all(cm.wear.xy>vec2<f32>(0.0)) && cm.wear.z>0.0 {
        // The inverse body instance already removes ride translation and full
        // body rotation. Keep both the core and its ribs fixed to the lens.
        let local=car_local(inst,p).xy;
        let a=(local-cm.flake.xy)/cm.wear.xy;
        let b=(local-cm.flake.zw)/cm.wear.xy;
        let interior=1.0-smoothstep(0.0,1.0,min(dot(a,a),dot(b,b)));
        let texture_core=smoothstep(0.1,1.0,peak/cm.wear.z);
        core=interior*interior*texture_core*visible;
    }
    // Linear texture intensity retains darker ribs/red surround; only the
    // authored interior reaches the existing AgX highlight shoulder. Its red
    // spectrum is unchanged: near-white is tone-map highlight desaturation,
    // not a white material patch or bloom painted over neighbouring pixels.
    let red_radiance=levels.x*peak*visible+clamp(cm.second.z,0.0,60000.0)*core;
    let white_radiance=levels.y*visible*sqrt(peak);
    return min(vec3<f32>(60000.0),vec3<f32>(1.0,0.008,0.003)*red_radiance*red
        +vec3<f32>(1.0,0.94,0.82)*white_radiance*white);
}
