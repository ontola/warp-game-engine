// Full source and full aggregate are rendered independently. Mix complete
// premultiplied representations only here; a faded source never writes depth
// into the aggregate layer. The selected motion belongs to visible coverage.
override MEADOW_AGGREGATE_HALF_RES:bool=false;
struct MergeView {view_proj:mat4x4<f32>,inv_view_proj:mat4x4<f32>,cam:vec4<f32>,planes:array<vec4<f32>,6>,params:vec4<f32>,size:vec4<f32>,extra:vec4<f32>,grass:vec4<f32>,fog_cull:vec4<f32>};
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var aggregate:texture_2d<f32>;
@group(0) @binding(2) var source_depth:texture_depth_2d;
@group(0) @binding(3) var aggregate_depth:texture_depth_2d;
@group(0) @binding(4) var source_motion:texture_2d<f32>;
@group(0) @binding(5) var aggregate_motion:texture_2d<f32>;
@group(0) @binding(6) var<uniform> merge_view:MergeView;
@group(0) @binding(7) var<uniform> transition:vec4<f32>;
@group(0) @binding(8) var opaque_depth:texture_depth_2d;
@vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32>{
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.0,1.0);
}
struct MergeResult {color:vec4<f32>,reprojection:vec4<f32>};
struct HalfAggregate {color:vec4<f32>,reprojection:vec4<f32>,depth:f32};
fn half_aggregate(pixel:vec2<i32>,ndc:vec2<f32>)->HalfAggregate {
    var out:HalfAggregate;
    let full=vec2<f32>(textureDimensions(source));let low=textureDimensions(aggregate);
    let q=(vec2<f32>(pixel)+0.5)/full*vec2<f32>(low)-0.5;
    let first=vec2<i32>(floor(q));let f=fract(q);
    // Match motion.wgsl exactly. Subtracting two world positions separated by
    // only the near plane loses direction precision at large world offsets.
    let direction4=merge_view.inv_view_proj*vec4<f32>(ndc,0.0,1.0);
    var origin=merge_view.cam.xyz;var ray=direction4.xyz-origin*direction4.w;
    if merge_view.params.z>0.5 {
        let near4=merge_view.inv_view_proj*vec4<f32>(ndc,1.0,1.0);
        let middle4=merge_view.inv_view_proj*vec4<f32>(ndc,0.5,1.0);
        if abs(near4.w)<1e-12 || abs(middle4.w)<1e-12 {return out;}
        origin=near4.xyz/near4.w;ray=middle4.xyz/middle4.w-origin;
    }
    if !all(abs(ray)<vec3<f32>(1e30)) || dot(ray,ray)<1e-20 {return out;}
    ray=normalize(ray);
    let opaque=textureLoad(opaque_depth,pixel,0);
    var strongest=0.0;
    for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++) {
        let tap=clamp(first+vec2<i32>(i32(x),i32(y)),vec2<i32>(0),vec2<i32>(low)-1);
        let weight=select(1.0-f.x,f.x,x!=0u)*select(1.0-f.y,f.y,y!=0u);
        if weight<=0.0 {continue;}
        let color=textureLoad(aggregate,tap,0);if color.a<=0.0 {continue;}
        let depth=textureLoad(aggregate_depth,tap,0);if !(depth>0.0 && depth<=1.0) {continue;}
        let tap_ndc=(vec2<f32>(tap)+0.5)/vec2<f32>(low)*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
        let tap4=merge_view.inv_view_proj*vec4<f32>(tap_ndc,depth,1.0);
        if abs(tap4.w)<1e-12 {continue;}
        let tap_world=tap4.xyz/tap4.w;
        if !all(abs(tap_world)<vec3<f32>(1e30)) {continue;}
        // Move the chosen low-resolution representative onto THIS full pixel
        // ray. Depth testing, fading and motion.w must describe this same point.
        let along=dot(tap_world-origin,ray);if along<=0.0 {continue;}
        let world=origin+ray*along;
        let clip=merge_view.view_proj*vec4<f32>(world,1.0);
        if !(clip.w>0.0) {continue;}
        let full_depth=clip.z/clip.w;
        if !(full_depth>0.0 && full_depth<=1.0) || full_depth<opaque {continue;}
        // Rejected taps contribute zero, never renormalize them into opaque
        // coverage. A nearest-depth OIT union cannot separate all subpixel
        // layers: this remains an explicit reduced-resolution approximation.
        out.color+=color*weight;
        let covered=color.a*weight;
        if covered>strongest {
            strongest=covered;out.depth=full_depth;
            let motion=textureLoad(aggregate_motion,tap,0);
            out.reprojection=vec4<f32>(0.0);
            if all(abs(motion)<vec4<f32>(1e30)) && motion.w>0.0 {
                // Never copy the distance measured along another pixel ray or
                // average unrelated branch velocities into invented motion.
                out.reprojection=vec4<f32>(motion.xyz,distance(world,merge_view.cam.xyz));
            }
        }
    }}
    return out;
}
fn merge_half_layers(pixel:vec2<i32>)->MergeResult {
    let a=textureLoad(source,pixel,0);
    let ndc=(vec2<f32>(pixel)+0.5)/vec2<f32>(textureDimensions(source))*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let layer=half_aggregate(pixel,ndc);var b=layer.color;let bd=layer.depth;
    let ad=select(0.0,textureLoad(source_depth,pixel,0),a.a>0.0);
    var out:MergeResult;if max(ad,bd)<=0.0 {return out;}
    let world4=merge_view.inv_view_proj*vec4<f32>(ndc,max(ad,bd),1.0);
    let horizontal=distance((world4.xyz/world4.w).xz,merge_view.cam.xz);
    if bd>0.0 {
        let far4=merge_view.inv_view_proj*vec4<f32>(ndc,bd,1.0);
        b*=1.0-smoothstep(transition.z,transition.w,distance((far4.xyz/far4.w).xz,merge_view.cam.xz));
    }
    let w=smoothstep(transition.x,transition.y,horizontal);
    out.color=mix(a,b,w);
    out.reprojection=select(textureLoad(source_motion,pixel,0),layer.reprojection,b.a*w>a.a*(1.0-w));
    if out.color.a<=0.0 {out.reprojection=vec4<f32>(0.0);}
    return out;
}
fn merge_layers(pixel:vec2<i32>)->MergeResult {
    if MEADOW_AGGREGATE_HALF_RES {return merge_half_layers(pixel);}
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
