// Composed with source meadow lighting, but not the atlas raster shader.
// One shared leaf-lighting evaluation per resolved far pixel. Material and
// normal moments approximate colour ordering; product revealage stays exact.
@group(2) @binding(4) var meadow_material_moments:texture_2d_array<f32>;
@group(2) @binding(5) var meadow_surface_depth:texture_depth_2d;
@vertex fn meadow_resolve_vs(@builtin(vertex_index)i:u32)->@builtin(position)vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn meadow_resolve_fs(@builtin(position)p:vec4<f32>)->@location(0)vec4<f32> {
    let pixel=vec2<i32>(p.xy);let material=textureLoad(meadow_material_moments,pixel,0,0);
    let alpha=1.0-clamp(textureLoad(meadow_material_moments,pixel,2,0).r,0.0,1.0);
    if material.a<=0.0 || alpha<=0.0 {return vec4<f32>(0.0);}
    let depth=textureLoad(meadow_surface_depth,pixel,0);if depth<=0.0 {return vec4<f32>(0.0);}
    let uv=p.xy/vec2<f32>(textureDimensions(meadow_surface_depth));
    let world4=view.inv_view_proj*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),depth,1.0);
    var surface:MeadowVertex;surface.world=world4.xyz/world4.w;
    let moments=textureLoad(meadow_material_moments,pixel,1,0)/material.a;
    // Face-forwarding before accumulation avoids antipodal cancellation.
    // An exact zero still has no directional evidence: use view-facing, never
    // invent a ground/up normal for the aggregate material.
    surface.normal=meadow_safe_normal(view.cam.xyz-surface.world);
    if dot(moments.xyz,moments.xyz)>1e-12 {surface.normal=normalize(moments.xyz);}
    surface.color=vec4<f32>(material.rgb/material.a,clamp(moments.w,0.04,1.0));surface.opacity=alpha;
    return meadow_color_with_geometric(surface,surface.normal);
}
