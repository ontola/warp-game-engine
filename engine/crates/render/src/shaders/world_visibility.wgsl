// One terrain visibility policy for opaque, glass, grass and voxel materials.
// Detailed raster maps remain responsible for mesh / moving object shadows.
struct WorldVisibility { params:vec4<f32> }; // enabled, node budget, representation error, reserved
@group(0) @binding(25) var<uniform> world_visibility:WorldVisibility;

fn world_terrain_sun_query(p:vec3<f32>,surface_n:vec3<f32>,representation_error:f32)->TerrainTraceResult {
    if world_visibility.params.x<0.5 {
        return terrain_result(3u,0u,0.0,p,vec3<f32>(0.0),vec3<f32>(0.0),0u,0u);
    }
    if light.sun_dir.y<=0.0 {
        return terrain_result(1u,0u,0.0,p,vec3<f32>(0.0),vec3<f32>(0.0),0u,0u);
    }
    var origin=p+surface_n*0.025;
    let ground=ground_at(origin.xz);
    // Only correct a receiver within the scene-declared terrain/render-mesh
    // discrepancy. Do not lift arbitrary underground objects or distant rays.
    // Native triangle details still come from the close raster shadow maps.
    if ground>-1e8 && origin.y<ground+0.025 && origin.y>=ground-representation_error-0.025 {
        origin.y=ground+0.025;
    }
    return terrain_trace_upward(heightfield,origin,light.sun_dir.xyz,0.01,u32(world_visibility.params.y));
}

fn world_terrain_sun_visibility(p:vec3<f32>,surface_n:vec3<f32>,representation_error:f32)->f32 {
    let query=world_terrain_sun_query(p,surface_n,representation_error);
    // Unknown is deliberately conservative. Diagnostic23 makes it magenta;
    // acceptance requires no unresolved receivers in the tested surface ROI.
    return select(0.0,1.0,query.info.x==3u);
}

fn world_terrain_debug(p:vec3<f32>,surface_n:vec3<f32>,work:bool,representation_error:f32)->vec3<f32> {
    let query=world_terrain_sun_query(p,surface_n,representation_error);
    if query.info.x==2u{return vec3<f32>(1.0,0.0,1.0);}
    if work{return vec3<f32>(f32(query.info.z)/max(world_visibility.params.y,1.0));}
    return vec3<f32>(select(0.0,1.0,query.info.x==3u));
}
