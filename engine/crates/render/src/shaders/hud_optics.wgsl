struct View {rect:vec4<f32>,settings:vec4<f32>};
struct Params {display:vec4<f32>,views:array<View,4>};
@group(0) @binding(0) var instruments:texture_2d<f32>;
@group(0) @binding(1) var linear:sampler;
@group(0) @binding(2) var<uniform> u:Params;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32>{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));return vec4<f32>(p[i],0.,1.);
}
fn instrument(at:vec2<f32>,rect:vec4<f32>)->vec4<f32>{
    let uv=clamp(rect.xy+at*rect.zw,rect.xy+vec2<f32>(.5),rect.xy+rect.zw-vec2<f32>(.5))/u.display.xy;
    let texel=textureSampleLevel(instruments,linear,uv,0.);
    return select(vec4<f32>(0.),texel,all(at>=vec2<f32>(0.))&&all(at<=vec2<f32>(1.)));
}
@fragment fn fs(@builtin(position) screen:vec4<f32>)->@location(0) vec4<f32>{
    var color=vec4<f32>(0.);
    for(var i=0u;i<u32(u.display.z);i++){
        let v=u.views[i];let uv=(screen.xy-v.rect.xy)/v.rect.zw;
        if any(uv<vec2<f32>(0.))||any(uv>=vec2<f32>(1.)){continue;}
        let p=uv-.5;let at=.5+p*(1.+v.settings.x*dot(p,p)*4.);
        let px=vec2<f32>(max(1.,v.rect.w/900.))/v.rect.zw;
        let core=instrument(at,v.rect);
        let glow=(instrument(at+vec2<f32>(px.x*1.6,0.),v.rect)+instrument(at-vec2<f32>(px.x*1.6,0.),v.rect)+instrument(at+vec2<f32>(0.,px.y*1.6),v.rect)+instrument(at-vec2<f32>(0.,px.y*1.6),v.rect))*.25;
        let halo=(instrument(at+px*3.5,v.rect).rgb+instrument(at-px*3.5,v.rect).rgb+instrument(at+px*vec2<f32>(3.5,-3.5),v.rect).rgb+instrument(at+px*vec2<f32>(-3.5,3.5),v.rect).rgb)*.25;
        let ghost=instrument(at+p*px*2.,v.rect).rgb*vec3<f32>(.08,.18,.23);
        let radiance=core.rgb*.70+(glow.rgb*.14+halo*.07+ghost*.18)*v.settings.y;
        let rim=smoothstep(.66,1.20,length(p*vec2<f32>(1.75,1.85)));
        let shade=(rim*.15+smoothstep(.55,1.,uv.y)*rim*.09)*select(1.,.35,v.settings.z>.5);
        color=vec4<f32>(radiance+vec3<f32>(.008,.023,.032)*shade,min(.38,shade+min(.25,glow.a*.20+core.a*.08)));
        break;
    }
    if u.display.w<.5 {color=vec4<f32>(select(1.055*pow(max(color.rgb,vec3<f32>(0.)),vec3<f32>(1./2.4))-.055,color.rgb*12.92,color.rgb<=vec3<f32>(.0031308)),color.a);}
    return color;
}
