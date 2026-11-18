// All input colors are straight-alpha sRGB; hardware sRGB targets blend in linear light.
@group(0) @binding(0) var<uniform> display: vec4<f32>;
// Pictures are premultiplied RGBA8 holding sRGB values; plain shapes bind a blank one.
@group(1) @binding(0) var picture: texture_2d<f32>;
@group(1) @binding(1) var picture_sampler: sampler;
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) shape: vec4<f32>,
};
@vertex fn vs(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>, @location(2) clip: vec4<f32>, @location(3) shape: vec4<f32>) -> Out {
    var out: Out;
    out.position=vec4<f32>(position/display.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.0,1.0);
    out.color=color;out.clip=clip;out.shape=shape;
    return out;
}
@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    let circle_distance=distance(in.position.xy,in.shape.xy)-in.shape.z;
    let line_distance=abs(in.shape.x)-in.shape.y;
    // Derivatives remain outside divergent branches/discard for portable validation.
    let circle_aa=max(fwidth(circle_distance)*0.5,0.5);
    let line_aa=max(fwidth(line_distance)*0.5,0.5);
    // Ring (w = 4 + half width): distance from the circle's centre line.
    let ring_distance=abs(circle_distance)-(in.shape.w-4.0);
    let ring_aa=max(fwidth(ring_distance)*0.5,0.5);
    // Sampled outside branches: uniform control flow for implicit derivatives.
    let texel=textureSample(picture,picture_sampler,in.shape.xy);
    if any(in.position.xy<in.clip.xy) || any(in.position.xy>=in.clip.xy+in.clip.zw) {discard;}
    var coverage=1.0;
    if in.shape.w>0.5 && in.shape.w<1.5 {coverage=1.0-smoothstep(-circle_aa,circle_aa,circle_distance);}
    if in.shape.w>1.5 && in.shape.w<2.5 {coverage=1.0-smoothstep(-line_aa,line_aa,line_distance);}
    var tint=in.color;
    if in.shape.w>3.5 {coverage=1.0-smoothstep(-ring_aa,ring_aa,ring_distance);}
    if in.shape.w>2.5 && in.shape.w<3.5 {tint=vec4<f32>(tint.rgb*texel.rgb/max(texel.a,1.0/255.0),tint.a*texel.a);}
    var color=clamp(tint.rgb,vec3<f32>(0.0),vec3<f32>(1.0));
    if display.z>0.5 {color=select(pow((color+0.055)/1.055,vec3<f32>(2.4)),color/12.92,color<=vec3<f32>(0.04045));}
    let alpha=clamp(tint.a,0.0,1.0)*coverage;
    return vec4<f32>(color*alpha,alpha);
}
