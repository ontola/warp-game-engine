// Bounded spatial shading filter of the actually rasterized source layer.
// Coverage is immutable: filtering alpha only at occupied pixels would erode
// thin blades; filling empty pixels would spill onto roads and silhouettes.
// This reduces distant shading variance, not missing subpixel geometry.
override SOURCE_BLADE_WIDTH:f32=0.04;
override SOURCE_BLADE_HEIGHT:f32=0.9;
const SPATIAL_MAX_STRENGTH:f32=0.65;
struct SpatialView {
    view_proj:mat4x4<f32>,inv_view_proj:mat4x4<f32>,cam:vec4<f32>,
    planes:array<vec4<f32>,6>,params:vec4<f32>,size:vec4<f32>,
    extra:vec4<f32>,grass:vec4<f32>,fog_cull:vec4<f32>,
};
// Prefix of the actual per-view MeadowSettings. Live growth is already applied.
struct SpatialSettings {field:vec4<f32>,grid:vec4<u32>,shape:vec4<f32>};
@group(0) @binding(0) var source_color:texture_2d<f32>;
@group(0) @binding(1) var source_depth:texture_depth_2d;
@group(0) @binding(2) var opaque_depth:texture_depth_2d;
@group(0) @binding(3) var<uniform> spatial_view:SpatialView;
@group(0) @binding(4) var<uniform> spatial_settings:SpatialSettings;

fn spatial_world(pixel:vec2<i32>,depth:f32)->vec3<f32> {
    let ndc=(vec2<f32>(pixel)+0.5)/vec2<f32>(textureDimensions(source_color))*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let p=spatial_view.inv_view_proj*vec4<f32>(ndc,depth,1.0);
    return p.xyz/p.w;
}
fn spatial_strength(footprint:f32,width:f32)->f32 {
    // The distant authored source is one triangle: width(t)=W*(1-t).
    // Its mean width over normalized height is integral(0..1)=W/2.
    // Leave pixels unchanged while that mean spans at least one pixel;
    // full effect begins only when it falls below half a pixel.
    let mean_width=0.5*width;
    return SPATIAL_MAX_STRENGTH*smoothstep(mean_width,2.0*mean_width,footprint);
}
fn spatial_depth_weight(delta:f32,limit:f32)->f32 {
    return 1.0-smoothstep(0.5*limit,limit,abs(delta));
}
fn spatial_filter(pixel:vec2<i32>)->vec4<f32> {
    let center=textureLoad(source_color,pixel,0);
    if center.a<=0.0 || spatial_view.params.z>0.5 {return center;}
    let depth=textureLoad(source_depth,pixel,0);
    let opaque=textureLoad(opaque_depth,pixel,0);
    if !(depth>0.0 && depth<=1.0) || depth<opaque {return center;}
    let world=spatial_world(pixel,depth);
    let clip=spatial_view.view_proj*vec4<f32>(world,1.0);
    if !(clip.w>0.0) {return center;}
    let footprint=clip.w/max(spatial_view.cam.w,1.0);
    let growth=spatial_settings.shape.x;
    let strength=spatial_strength(footprint,max(SOURCE_BLADE_WIDTH*growth,0.0001));
    if strength<=0.0 {return center;}
    // Depth support is bounded by the source canopy and a 3x3 pixel footprint.
    // It does not expand with absolute camera distance or use a color key.
    let limit=max(SOURCE_BLADE_HEIGHT*growth,4.0*footprint);
    var rgb=4.0*center.rgb;
    var coverage=4.0*center.a;
    let dimensions=vec2<i32>(textureDimensions(source_color));
    for(var y=-1;y<=1;y++) {for(var x=-1;x<=1;x++) {
        if x==0 && y==0 {continue;}
        let tap=pixel+vec2<i32>(x,y);
        if any(tap<vec2<i32>(0)) || any(tap>=dimensions) {continue;}
        let color=textureLoad(source_color,tap,0);
        if color.a<=0.0 {continue;}
        let tap_depth=textureLoad(source_depth,tap,0);
        if !(tap_depth>0.0 && tap_depth<=1.0) || tap_depth<textureLoad(opaque_depth,tap,0) {continue;}
        let tap_world=spatial_world(tap,tap_depth);
        let tap_clip=spatial_view.view_proj*vec4<f32>(tap_world,1.0);
        let kernel=select(1.0,2.0,x==0)*select(1.0,2.0,y==0);
        let weight=kernel*spatial_depth_weight(tap_clip.w-clip.w,limit);
        rgb+=weight*color.rgb;
        coverage+=weight*color.a;
    }}
    // Conditional mean radiance, weighted by measured grass coverage. Alpha
    // stays bit-identical; the opaque-shade skip and motion mask remain valid.
    return vec4<f32>(mix(center.rgb,rgb*(center.a/coverage),strength),center.a);
}
@vertex fn vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn fs(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32> {
    return spatial_filter(vec2<i32>(p.xy));
}
