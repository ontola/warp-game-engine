// The low tier's glow (forward.rs `Bloom`): from the finished display image,
// at an eighth of its size. A bright pass of what is nearly white, a
// separable blur, then screened over the image as Godot's glow is
// (tonemap.wgsl `display`). Four small passes instead of the HDR bloom chain.
struct Params {texel:vec2<f32>,direction:vec2<f32>,strength:f32,threshold:f32,pad:vec2<f32>};
@group(0) @binding(0) var src:texture_2d<f32>;
@group(0) @binding(1) var samp:sampler;
@group(0) @binding(2) var<uniform> params:Params;

struct Out {@builtin(position) pos:vec4<f32>,@location(0) uv:vec2<f32>};
@vertex fn vs(@builtin(vertex_index) i:u32)->Out {
 let p=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
 var out:Out;out.pos=vec4<f32>(p*2.0-1.0,0.0,1.0);out.uv=vec2<f32>(p.x,1.0-p.y);return out;
}

// Four bilinear taps spread over the 8x8 texels under each eighth-size texel.
@fragment fn bright(in:Out)->@location(0) vec4<f32> {
 let t=params.texel;
 var c=vec3<f32>(0.0);
 for(var k=0u;k<4u;k++) {
  let o=vec2<f32>(select(-1.0,1.0,(k&1u)!=0u),select(-1.0,1.0,(k&2u)!=0u))*t*2.0;
  let s=textureSampleLevel(src,samp,in.uv+o,0.0).rgb;
  let peak=max(s.r,max(s.g,s.b));
  c+=s*smoothstep(params.threshold,1.0,peak);
 }
 return vec4<f32>(c*0.25,1.0);
}

// Nine texels in five bilinear taps along `direction`.
@fragment fn blur(in:Out)->@location(0) vec4<f32> {
 let d=params.direction*params.texel;
 var c=textureSampleLevel(src,samp,in.uv,0.0).rgb*0.2270270;
 c+=(textureSampleLevel(src,samp,in.uv+d*1.3846154,0.0).rgb+textureSampleLevel(src,samp,in.uv-d*1.3846154,0.0).rgb)*0.3162162;
 c+=(textureSampleLevel(src,samp,in.uv+d*3.2307692,0.0).rgb+textureSampleLevel(src,samp,in.uv-d*3.2307692,0.0).rgb)*0.0702703;
 return vec4<f32>(c,1.0);
}

// Screened over the image: the blend adds src * (1 - dst).
@fragment fn add(in:Out)->@location(0) vec4<f32> {
 return vec4<f32>(min(textureSampleLevel(src,samp,in.uv,0.0).rgb*params.strength,vec3<f32>(1.0)),1.0);
}
