// Full source and full aggregate are rendered independently. Mix complete
// premultiplied representations only here; a faded source never writes depth
// into the aggregate layer. The selected motion belongs to visible coverage.
struct MergeView {view_proj:mat4x4<f32>,inv_view_proj:mat4x4<f32>,cam:vec4<f32>,planes:array<vec4<f32>,6>,params:vec4<f32>,size:vec4<f32>,extra:vec4<f32>,grass:vec4<f32>,fog_cull:vec4<f32>};
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var aggregate:texture_2d<f32>;
@group(0) @binding(2) var source_depth:texture_depth_2d;
@group(0) @binding(3) var aggregate_depth:texture_depth_2d;
@group(0) @binding(4) var source_motion:texture_2d<f32>;
@group(0) @binding(5) var aggregate_motion:texture_2d<f32>;
@group(0) @binding(6) var<uniform> merge_view:MergeView;
@group(0) @binding(7) var<uniform> transition:vec4<f32>;
@group(0) @binding(8) var aggregate_reveal:texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32>{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.0,1.0);
}
struct MergeResult {color:vec4<f32>,reprojection:vec4<f32>};
fn merge_layers(pixel:vec2<i32>)->MergeResult {
    let a=textureLoad(source,pixel,0);
    var b=textureLoad(aggregate,pixel,0); // already lit, resolved premultiplied union
    let ad=select(0.0,textureLoad(source_depth,pixel,0),a.a>0.0);
    let bd=select(0.0,textureLoad(aggregate_depth,pixel,0),b.a>0.0);
    var out:MergeResult;if max(ad,bd)<=0.0 {return out;}
    let ndc=(vec2<f32>(pixel)+0.5)/vec2<f32>(textureDimensions(source))*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let world4=merge_view.inv_view_proj*vec4<f32>(ndc,max(ad,bd),1.0);
    let horizontal=distance((world4.xyz/world4.w).xz,merge_view.cam.xz);
    if bd>0.0 {
        let far_world4=merge_view.inv_view_proj*vec4<f32>(ndc,bd,1.0);
        let far_horizontal=distance((far_world4.xyz/far_world4.w).xz,merge_view.cam.xz);
        b*=1.0-smoothstep(transition.z,transition.w,far_horizontal);
    }
    let w=smoothstep(transition.x,transition.y,horizontal);
    out.color=mix(a,b,w);
    // Dominant weighted coverage, with an explicit empty-layer fallback.
    let choose_b=b.a*w>a.a*(1.0-w);
    out.reprojection=select(textureLoad(source_motion,pixel,0),textureLoad(aggregate_motion,pixel,0),choose_b);
    if out.color.a<=0.0 {out.reprojection=vec4<f32>(0.0);}
    return out;
}
@fragment fn fs(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32>{return merge_layers(vec2<i32>(p.xy)).color;}
struct MergeMotion {@location(0)color:vec4<f32>,@location(1)reprojection:vec4<f32>};
@fragment fn motion_fs(@builtin(position)p:vec4<f32>)->MergeMotion {
    let merged=merge_layers(vec2<i32>(p.xy));var out:MergeMotion;out.color=merged.color;out.reprojection=merged.reprojection;return out;
}
