// Reproject only exact integer XY translations with unchanged depth mapping.
struct State {vp:mat4x4<f32>,counts:vec4<u32>,control:vec4<u32>};
@group(0) @binding(0) var<uniform> state:State;
@group(0) @binding(1) var previous:texture_depth_2d;
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.,-1.),vec2<f32>(3.,-1.),vec2<f32>(-1.,3.));
    return vec4<f32>(p[i],0.,1.);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
    let old=vec2<i32>(p.xy)-bitcast<vec2<i32>>(state.control.zw);
    if any(old<vec2<i32>(0)) || any(old>=vec2<i32>(i32(state.control.y))) {return 1.;}
    return textureLoad(previous,old,0);
}
