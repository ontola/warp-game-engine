@group(0) @binding(0) var volume_layer: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[vertex],0.0,1.0);
}
@fragment fn fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    return textureLoad(volume_layer,vec2<i32>(p.xy),0);
}
