// Visible source rear lenses, on per-car material slots. No new lights or
// shadow passes. car_model selects only original rear triangles, not whole
// shared glass materials. second.xy are red/white peak radiance (cd/m2),
// second.w is 0: rear atlas, 1: authored red lens, 2: authored clear reverse.
fn car_lamp_emission(flags:u32, source_base:vec3<f32>)->vec3<f32> {
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
    // Retain the source lens texture's ribs, rather than solid colour panels.
    // Emission is neither shadowed nor exposure-compensated; daytime remains
    // reflective when off and brake/reverse stay independently visible.
    let lens=visible*sqrt(peak);
    return (vec3<f32>(1.0,0.008,0.003)*levels.x*red
        +vec3<f32>(1.0,0.94,0.82)*levels.y*white)*lens;
}
