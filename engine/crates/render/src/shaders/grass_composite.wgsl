@group(0) @binding(0) var nearest_layer: texture_2d<f32>;
@vertex
fn vs(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[vertex],0.0,1.0);
}
@fragment
fn fs(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let pixel=vec2<i32>(position.xy);
    return textureLoad(nearest_layer,pixel,0);
}
