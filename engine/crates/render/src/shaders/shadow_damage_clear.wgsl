// Sparse tile clear: clean tiles emit degenerate triangles, retaining depth.
// The same bit numbering is used by shadow_damage.wgsl and raster consumers.
@group(0) @binding(0) var<uniform> mask:array<vec4<u32>,32>;
@vertex fn vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) tile:u32)->@builtin(position) vec4<f32> {
 let word=mask[tile>>7u][(tile>>5u)&3u];
 if (word&(1u<<(tile&31u)))==0u {return vec4<f32>(2.,2.,1.,1.);}
 let corners=array<vec2<f32>,6>(vec2<f32>(0.,0.),vec2<f32>(1.,0.),vec2<f32>(1.,1.),vec2<f32>(0.,0.),vec2<f32>(1.,1.),vec2<f32>(0.,1.));
 let uv=(vec2<f32>(f32(tile&63u),f32(tile>>6u))+corners[vertex])/64.;
 return vec4<f32>(uv.x*2.-1.,1.-uv.y*2.,1.,1.);
}
