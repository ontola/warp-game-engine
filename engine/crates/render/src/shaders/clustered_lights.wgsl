// Clustered local lights in shade (clustered_lights.rs). Group 2 is theirs
// alone, the grid uniform; the arena is a region of the scene tables
// (offsets in vec4s), which holds the lights
// (4 vec4 each, LocalLight's layout; source.z is the first shadow face + 1,
// source.w the face count), the shadow faces (6 vec4: view-projection, tile
// rect and layer, texel size) and the grid cells binned by
// clustered_bin.wgsl. Binding 48 so no other pass composed with shade's
// text collides with it.
struct LightGrid { origin: vec4<f32>, inv_cell: vec4<f32>, dims: vec4<u32>, offsets: vec4<u32> };
@group(2) @binding(48) var<uniform> light_grid: LightGrid;
// Off unless the scene opted in (Graphics::clustered_lights).
override FEATURE_CLUSTERED_LIGHTS: bool = false;

const NO_CELL: u32 = 0xffffffffu;

fn clustered_cell(p: vec3<f32>) -> u32 {
    if light_grid.dims.w == 0u { return NO_CELL; }
    let c = floor((p - light_grid.origin.xyz) * light_grid.inv_cell.xyz);
    let dims = vec3<f32>(light_grid.dims.xyz);
    if any(c < vec3<f32>(0.0)) || any(c >= dims) { return NO_CELL; }
    let u = vec3<u32>(c);
    return light_grid.offsets.y + ((u.z * light_grid.dims.y + u.y) * light_grid.dims.x + u.x) * 8u;
}

fn clustered_index(cell: u32, k: u32) -> u32 {
    let word = 1u + k / 2u;
    return (scene_tables[cell + word / 4u][word % 4u] >> (16u * (k & 1u))) & 0xffffu;
}

fn clustered_lamp(i: u32) -> LocalLight {
    let b = light_grid.offsets.w + i * 4u;
    return LocalLight(bitcast<vec4<f32>>(scene_tables[b]), bitcast<vec4<f32>>(scene_tables[b + 1u]),
        bitcast<vec4<f32>>(scene_tables[b + 2u]), bitcast<vec4<f32>>(scene_tables[b + 3u]));
}

// Visibility of the light from p through its shadow tile (1: lit).
fn clustered_shadow(lamp: LocalLight, p: vec3<f32>, n: vec3<f32>) -> f32 {
    let d = p - lamp.position_range.xyz;
    var face = 0u;
    if lamp.source.w > 1.5 {
        let a = abs(d);
        if a.x >= a.y && a.x >= a.z { face = select(1u, 0u, d.x > 0.0); }
        else if a.y >= a.z { face = select(3u, 2u, d.y > 0.0); }
        else { face = select(5u, 4u, d.z > 0.0); }
    }
    let fb = light_grid.offsets.x + (u32(lamp.source.z) - 1u + face) * 6u;
    let rect = bitcast<vec4<f32>>(scene_tables[fb + 4u]);
    // A face not drawn yet stays unshadowed.
    if rect.w < 0.0 { return 1.0; }
    let params = bitcast<vec4<f32>>(scene_tables[fb + 5u]);
    let m = mat4x4<f32>(bitcast<vec4<f32>>(scene_tables[fb]), bitcast<vec4<f32>>(scene_tables[fb + 1u]),
        bitcast<vec4<f32>>(scene_tables[fb + 2u]), bitcast<vec4<f32>>(scene_tables[fb + 3u]));
    let dist = length(d);
    // Receiver bias in texels of this face at this distance: along the
    // normal (slopes) and toward the light (acne on flat receivers).
    let texel = params.x * dist;
    let biased = p + n * (texel * 1.5 + 0.004) - d / max(dist, 1e-4) * (texel + 0.002);
    let clip = m * vec4<f32>(biased, 1.0);
    if clip.w <= 0.0 { return 1.0; }
    let ndc = clip.xyz / clip.w;
    if ndc.z >= 1.0 { return 1.0; }
    let atlas = f32(textureDimensions(headlight_depth).x);
    let tile = rect.z * atlas;
    // Stay a texel and a half inside the tile so no tap reads a neighbour.
    let edge = 1.5 / tile;
    let local = clamp(ndc.xy * vec2<f32>(0.5, -0.5) + 0.5, vec2<f32>(edge), vec2<f32>(1.0 - edge));
    let uv = rect.xy + local * rect.z;
    let o = 0.5 / atlas;
    let layer = i32(rect.w);
    var lit = 0.0;
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(-o, -o), layer, ndc.z);
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(o, -o), layer, ndc.z);
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(-o, o), layer, ndc.z);
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(o, o), layer, ndc.z);
    return lit * 0.25;
}

fn clustered_sample(i: u32, p: vec3<f32>, bias_normal: vec3<f32>) -> HeadlightSample {
    let lamp = clustered_lamp(i);
    var s = local_light_at(lamp, p);
    if lamp.source.z > 0.5 && any(s.irradiance > vec3<f32>(0.0)) { s.irradiance *= clustered_shadow(lamp, p, bias_normal); }
    return s;
}

fn clustered_radiance(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, rough: f32, f0: vec3<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    let cell = clustered_cell(p);
    if cell == NO_CELL { return result; }
    let count = scene_tables[cell].x;
    for (var k = 0u; k < count; k++) {
        let i = clustered_index(cell, k);
        // One-sided, as headlight_radiance: skip lights behind the surface.
        if dot(n, bitcast<vec4<f32>>(scene_tables[light_grid.offsets.w + i * 4u]).xyz - p) <= 0.0 { continue; }
        let s = clustered_sample(i, p, n);
        if all(s.irradiance == vec3<f32>(0.0)) { continue; }
        result += s.irradiance * (diffuse * max(dot(n, s.direction), 0.0) + ggx(n, v, s.direction, rough, f0));
    }
    return result;
}

fn clustered_grass_radiance(p: vec3<f32>, v: vec3<f32>, base: vec3<f32>, rough: f32) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    let cell = clustered_cell(p);
    if cell == NO_CELL { return result; }
    let count = scene_tables[cell].x;
    for (var k = 0u; k < count; k++) {
        let s = clustered_sample(clustered_index(cell, k), p, vec3<f32>(0.0, 1.0, 0.0));
        if all(s.irradiance == vec3<f32>(0.0)) { continue; }
        result += s.irradiance * grass_directional_response(base, v, s.direction, rough);
    }
    return result;
}
