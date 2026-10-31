// Premultiplied smoke/fire upsampling after temporal reconstruction.
// Guide depths use the same output-ray to scene-depth mapping as the FX pass.
struct CompositeUniform {
    depth_from_output: mat4x4<f32>,
    full: vec4<f32>,
    low: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: CompositeUniform;
@group(0) @binding(1) var soft_layer: texture_2d<f32>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;

@vertex
fn fullscreen(@builtin(vertex_index) id:u32) -> @builtin(position) vec4<f32> {
    let p=vec2<f32>(f32((id<<1u)&2u),f32(id&2u));
    return vec4<f32>(p*2.0-1.0,0.0,1.0);
}
fn guide_depth(uv:vec2<f32>) -> f32 {
    let clip=u.depth_from_output*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),1.0,1.0);
    let depth_uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5;
    let dims=vec2<i32>(textureDimensions(scene_depth));
    let px=clamp(vec2<i32>(depth_uv*vec2<f32>(dims)),vec2<i32>(0),dims-1);
    return min(60000.0,u.full.z/max(textureLoad(scene_depth,px,0),1e-9));
}
@fragment
fn composite(@builtin(position) position:vec4<f32>) -> @location(0) vec4<f32> {
    let uv=position.xy/u.full.xy;
    let depth=guide_depth(uv);
    let coord=uv*u.low.xy-0.5;
    let base=vec2<i32>(floor(coord));
    let frac=fract(coord);
    var value=vec4<f32>(0.0);
    var weights=0.0;
    var fallback_depth=0.0;
    var fallback_q=vec2<i32>(0);
    for(var i=0u;i<4u;i++) {
        let offset=vec2<i32>(i32(i&1u),i32(i>>1u));
        let q=clamp(base+offset,vec2<i32>(0),vec2<i32>(u.low.xy)-1);
        let sample_depth=guide_depth((vec2<f32>(q)+0.5)*u.low.zw);
        // Behind the complete soft bucket, a roof edge or the horizon cannot
        // occlude smoke/fire. Rejecting it would punch that distant geometry
        // through foreground smoke. A bound >= the guide's 60000m clamp
        // deliberately never enables this shortcut.
        let behind_soft=depth>u.full.w && sample_depth>u.full.w;
        let gap=select(abs(sample_depth-depth)/max(0.02*min(sample_depth,depth),0.02),0.0,behind_soft);
        // At a steep continuous floor gradient every guide can miss the
        // bilateral tolerance. Remember only samples clipped at an equal or
        // nearer surface: a farther background guide is never a safe fallback
        // for a thin foreground object missing from the reduced grid.
        if sample_depth<=depth && sample_depth>fallback_depth {
            fallback_depth=sample_depth;
            fallback_q=q;
        }
        if gap<3.0 {
            let bilinear=select(1.0-frac.x,frac.x,offset.x==1)*select(1.0-frac.y,frac.y,offset.y==1);
            let w=bilinear*exp(-gap*gap);
            value+=textureLoad(soft_layer,q,0)*w;
            weights+=w;
        }
    }
    if weights<1e-5 {
        if fallback_depth>0.0 {return textureLoad(soft_layer,fallback_q,0);}
        return vec4<f32>(0.0);
    }
    return value/weights;
}
