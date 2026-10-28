// The host prepends `enable primitive_index;` before the shared shader library.
// One indexed geometry pass writes the nearest actual source primitive. It
// retains the original triangle order and depth-index stream; no proxy exists.
@group(2) @binding(4) var meadow_visibility_ids:texture_2d<u32>;
struct MeadowVisibilityVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(flat) opacity:f32,
    @location(1) @interpolate(flat) owner:u32,
};
@vertex fn meadow_visibility_vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->MeadowVisibilityVertex {
    let owner=meadow_source_owner(instance,meadow_draw.x,meadow_draw.y);
    let source=meadow_source_vertex(vertex,owner,meadow_draw.x,meadow_draw.y);
    var out:MeadowVisibilityVertex;out.position=source.position;out.opacity=source.opacity;out.owner=owner;return out;
}
@fragment fn meadow_visibility_fs(in:MeadowVisibilityVertex,@builtin(primitive_index) primitive:u32)->@location(0) vec2<u32> {
    if in.opacity<=0.0 {discard;}
    // Zero owner is the cleared uncovered value. LOD occupies two high bits;
    // real source recipes are validated far below the30-bit primitive bound.
    var owner=in.owner+1u;
    // Certification is pixel-exact, from the same raster depth/inverse camera
    // used by the later representation merge. Its nearest source must have
    // alpha1; with an aggregate, the source must also be before its blend.
    if meadow_draw.z!=0u && in.opacity==1.0 {
        var complete=true;
        if meadow.support.z>0.5 {
            let ndc=in.position.xy/view.size.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
            let world4=view.inv_view_proj*vec4<f32>(ndc,in.position.z,1.0);
            let horizontal=distance((world4.xyz/world4.w).xz,view.cam.xz);
            complete=horizontal<=meadow.shape.w;
        }
        if complete {owner|=0x80000000u;}
    }
    return vec2<u32>(owner,(meadow_draw.x<<30u)|primitive);
}
@vertex fn meadow_visibility_resolve_vs(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[vertex],0.0,1.0);
}
fn meadow_cross2(a:vec2<f32>,b:vec2<f32>)->f32 {return a.x*b.y-a.y*b.x;}
// Perspective-correct source barycentrics, directly in homogeneous clip
// coordinates. No division by individual vertexW, so near-plane clipping and
// a vertex behind the camera do not require a guessed surface/depth bias.
fn meadow_visibility_weights(a:vec4<f32>,b:vec4<f32>,c:vec4<f32>,ndc:vec2<f32>)->vec3<f32> {
    let x=a.xy-ndc*a.w;let y=b.xy-ndc*b.w;let z=c.xy-ndc*c.w;
    let weights=vec3<f32>(meadow_cross2(y,z),meadow_cross2(z,x),meadow_cross2(x,y));
    return weights/(weights.x+weights.y+weights.z);
}
struct MeadowVisibleSurface {vertex:MeadowVertex,geometric:vec3<f32>};
fn meadow_visibility_surface(pixel:vec4<f32>)->MeadowVisibleSurface {
    var result:MeadowVisibleSurface;var out:MeadowVertex;
    let id=textureLoad(meadow_visibility_ids,vec2<i32>(pixel.xy),0).xy;
    if id.x==0u {return result;}
    let owner=(id.x&0x7fffffffu)-1u;let lod=id.y>>30u;let triangle=id.y&0x3fffffffu;
    if owner>=meadow_draw.y || lod>=3u {return result;}
    let variant=meadow_pool[owner*meadow_plant_words(meadow.wind_history.w)+10u];
    if variant>=3u {return result;}
    let mesh=bitcast<vec4<u32>>(meadow_recipe[variant*3u+lod]);
    if triangle>=mesh.y/3u {return result;}
    let a=meadow_source_vertex(triangle*3u,owner,lod,meadow_draw.y);
    let b=meadow_source_vertex(triangle*3u+1u,owner,lod,meadow_draw.y);
    let c=meadow_source_vertex(triangle*3u+2u,owner,lod,meadow_draw.y);
    let dimensions=vec2<f32>(textureDimensions(meadow_visibility_ids));
    let ndc=pixel.xy/dimensions*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
    let weights=meadow_visibility_weights(a.position,b.position,c.position,ndc);
    out.position=pixel;
    out.world=a.world*weights.x+b.world*weights.y+c.world*weights.z;
    out.previous_world=a.previous_world*weights.x+b.previous_world*weights.y+c.previous_world*weights.z;
    out.normal=a.normal*weights.x+b.normal*weights.y+c.normal*weights.z;
    out.color=a.color*weights.x+b.color*weights.y+c.color*weights.z;
    out.blade_color=a.blade_color*weights.x+b.blade_color*weights.y+c.blade_color*weights.z;
    out.pigment=a.pigment;
    out.opacity=a.opacity; // flat per root, same value on all triangle corners
    // Derive the actual deformed face, not screen-neighbor normals or a
    // derivative across two different blades. Geometry/wind is identical to
    // the visibility pass and shared source vertex function.
    result.vertex=out;result.geometric=meadow_safe_normal(cross(b.world-a.world,c.world-a.world));return result;
}
@fragment fn meadow_visibility_resolve_fs(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let surface=meadow_visibility_surface(pixel);if surface.vertex.opacity<=0.0 {discard;}
    return meadow_color_with_geometric(surface.vertex,surface.geometric);
}
@fragment fn meadow_visibility_resolve_motion_fs(@builtin(position) pixel:vec4<f32>)->MeadowMotionOut {
    let surface=meadow_visibility_surface(pixel);if surface.vertex.opacity<=0.0 {discard;}
    var out:MeadowMotionOut;out.color=meadow_color_with_geometric(surface.vertex,surface.geometric);
    out.reprojection=vec4<f32>(surface.vertex.previous_world-surface.vertex.world,distance(surface.vertex.world,view.cam.xyz));return out;
}
