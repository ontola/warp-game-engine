// Froxel fog (froxel_fog.rs; Wronski 2014, Hillaire 2015): a camera-aligned
// grid of 160x90x64 cells, slices spaced exponentially from FOG_NEAR to
// gi.fog.z metres of view depth. `fog_inject` fills each cell with the light
// the medium scatters towards the camera there and its extinction: every
// clustered light with its shadow, cookie and profile, the sun through its
// shadow maps, and the room's ambient light (the irradiance volume, sky
// through openings included), at a position jittered along the cell each
// frame and blended with last frame's cell (reprojected). `froxel_integrate`
// (froxel_fog.rs) sums the cells front to back; shade dims every pixel by
// the transmittance to it and adds the light scattered in front of it.

@group(1) @binding(28) var fog_out: texture_storage_3d<rgba16float, write>;
@group(1) @binding(29) var fog_prev: texture_3d<f32>;
@group(1) @binding(30) var fog_volume: texture_3d<f32>;

const FOG_NEAR: f32 = 0.1;
const FOG_GRID: vec3<u32> = vec3<u32>(160u, 90u, 64u);

fn fog_on() -> bool { return gi.fog.x > 0.0; }

// View depth of slice coordinate s (0 at FOG_NEAR, 64 at the far end).
fn fog_slice_depth(s: f32) -> f32 { return FOG_NEAR * pow(gi.fog.z / FOG_NEAR, s / f32(FOG_GRID.z)); }
fn fog_depth_slice(w: f32) -> f32 { return log(max(w, FOG_NEAR) / FOG_NEAR) / log(gi.fog.z / FOG_NEAR) * f32(FOG_GRID.z); }

fn fog_hg(mu: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * mu;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(max(d, 1e-6)));
}

// A clustered light's shadow at a point in the air: one filtered compare
// (the cell's jitter and the history filter the rest).
fn fog_light_shadow(lamp: LocalLight, p: vec3<f32>) -> f32 {
    let d = p - lamp.position_range.xyz;
    var face = 0u;
    if (u32(lamp.source.w) & 7u) > 1u {
        let a = abs(d);
        if a.x >= a.y && a.x >= a.z { face = select(1u, 0u, d.x > 0.0); }
        else if a.y >= a.z { face = select(3u, 2u, d.y > 0.0); }
        else { face = select(5u, 4u, d.z > 0.0); }
    }
    let fb = light_grid.offsets.x + (u32(lamp.source.z) - 1u + face) * 6u;
    let rect = bitcast<vec4<f32>>(scene_tables[fb + 4u]);
    if rect.w < 0.0 { return 1.0; }
    let m = mat4x4<f32>(bitcast<vec4<f32>>(scene_tables[fb]), bitcast<vec4<f32>>(scene_tables[fb + 1u]),
        bitcast<vec4<f32>>(scene_tables[fb + 2u]), bitcast<vec4<f32>>(scene_tables[fb + 3u]));
    let clip = m * vec4<f32>(p, 1.0);
    if clip.w <= 0.0 { return 1.0; }
    let ndc = clip.xyz / clip.w;
    if ndc.z >= 1.0 { return 1.0; }
    let atlas = f32(textureDimensions(headlight_depth).x);
    let edge = 1.5 / (rect.z * atlas);
    let local = clamp(ndc.xy * vec2<f32>(0.5, -0.5) + 0.5, vec2<f32>(edge), vec2<f32>(1.0 - edge));
    return textureSampleCompareLevel(headlight_depth, shadow_samp, rect.xy + local * rect.z, i32(rect.w), ndc.z - 1e-4);
}

// Irradiance at p from direction n's hemisphere of ambient light: the room's
// irradiance volume where it has one (as shade's ambient), the sky outside.
fn fog_ambient_irradiance(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let open = sky_irradiance(n);
    if !FEATURE_IRRADIANCE { return open; }
    let s = irradiance_sample(p, n);
    let room = s.value;
    if room.w <= 0.0 { return open; }
    var inside = room.rgb;
    if aux_word(u32(gi.params.w) + 7u) >= 1u {
        inside = sky_irradiance(n) * clamp(room.x, 0.0, 1.0) + sky_irradiance(vec3<f32>(0.0, 1.0, 0.0)) * room.y
            + light.sun_color.rgb * light.sun_dir.w * max(room.z, 0.0) + s.emission * light.anim.y;
    }
    return mix(open, inside, room.w);
}

fn fog_rand(seed: u32) -> f32 {
    var s = seed * 747796405u + 2891336453u;
    s = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return f32(((s >> 22u) ^ s) >> 8u) / 16777216.0;
}

var<workgroup> fog_tile_depth: array<f32, 64>;

// One workgroup per column of cells, one thread per slice. The threads first
// find the column's farthest visible surface (the least depth in its tile of
// the depth buffer, reversed Z): air behind it is never seen through this
// column, and a cell partly behind a wall must not take the outdoor sky or
// sun from there into the room's fog. A cell wholly behind it scatters
// nothing; a cell it cuts samples only in front of it.
@compute @workgroup_size(64, 1, 1)
fn fog_inject(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let cell = vec3<u32>(wid.xy, li);
    let size = vec2<u32>(view.size.xy);
    let lo = cell.xy * size / FOG_GRID.xy;
    let hi = max(min((cell.xy + 1u) * size / FOG_GRID.xy, size), lo + 1u);
    let count = (hi - lo).x * (hi - lo).y;
    var least = 1.0;
    for (var k = li; k < count; k += 64u) {
        let q = lo + vec2<u32>(k % (hi - lo).x, k / (hi - lo).x);
        least = min(least, textureLoad(depth_tex, vec2<i32>(q), 0));
    }
    fog_tile_depth[li] = least;
    workgroupBarrier();
    for (var s = 32u; s > 0u; s >>= 1u) {
        if li < s { fog_tile_depth[li] = min(fog_tile_depth[li], fog_tile_depth[li + s]); }
        workgroupBarrier();
    }
    let surface = select(1e9, view.params.y / fog_tile_depth[0], fog_tile_depth[0] > 0.0);
    let frame = u32(gi.fog_albedo.w);
    let jitter = fog_rand(cell.x ^ (cell.y << 8u) ^ (cell.z << 16u) ^ (frame * 0x9e3779b9u));
    // The sample: jittered across the cell in x and y too, so that over
    // frames the cell averages the air it holds.
    let jxy = vec2<f32>(fog_rand(cell.x * 7919u ^ cell.y * 104729u ^ cell.z * 1299709u ^ (frame * 2654435761u)), fog_rand(cell.x * 15485863u ^ cell.y * 32452843u ^ cell.z * 49979687u ^ (frame * 40503u)));
    let uv = (vec2<f32>(cell.xy) + jxy) / vec2<f32>(FOG_GRID.xy);
    let start = fog_slice_depth(f32(cell.z));
    if start >= surface { textureStore(fog_out, cell, vec4<f32>(0.0, 0.0, 0.0, gi.fog.x)); return; }
    let w = min(fog_slice_depth(f32(cell.z) + jitter), mix(start, surface, 0.999));
    // The cell's ray from the camera, at view depth w (along the axis).
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let hn = view.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let hc = view.inv_view_proj * vec4<f32>(0.0, 0.0, 1.0, 1.0);
    let dir = normalize(hn.xyz / hn.w - view.cam.xyz);
    let forward = normalize(hc.xyz / hc.w - view.cam.xyz);
    let p = view.cam.xyz + dir * (w / max(dot(dir, forward), 1e-3));
    let to_eye = normalize(view.cam.xyz - p);
    // Air the camera cannot see (behind the surface at the sample's own
    // pixel: above a ceiling, outside a wall in a tile that also shows the
    // far side of a room) scatters nothing into what it can: the cell
    // averages only its visible air.
    let spx = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
    let pd = textureLoad(depth_tex, vec2<i32>(spx), 0);
    let hidden = pd > 0.0 && w > view.params.y / pd;
    let g = gi.fog.y;
    var lin = vec3<f32>(0.0);
    // Clustered lights, shadowed, with cookies and profiles.
    let c = clustered_cell(p);
    if FEATURE_CLUSTERED_LIGHTS && c != NO_CELL {
        let count = scene_tables[c].x;
        for (var k = 0u; k < count; k++) {
            let lamp = clustered_lamp(clustered_index(c, k));
            var s = local_light_at(lamp, p);
            if all(s.irradiance == vec3<f32>(0.0)) { continue; }
            if u32(lamp.source.w) >= 8u { s.irradiance *= light_shape(lamp, -s.direction); }
            if lamp.source.z > 0.5 { s.irradiance *= fog_light_shadow(lamp, p); }
            lin += s.irradiance * fog_hg(dot(-s.direction, to_eye), g);
        }
    }
    // The sun, through its shadow maps.

    if light.sun_dir.w > 0.0 {
        let l = light.sun_dir.xyz;
        let sh = shadow_at_surface(p, l, l, 0.0);
        lin += light.sun_color.rgb * light.sun_dir.w * sh * fog_hg(dot(-l, to_eye), g);
    }
    // Ambient (isotropic): the mean radiance of the two hemispheres.
    lin += (fog_ambient_irradiance(p, vec3<f32>(0.0, 1.0, 0.0)) + fog_ambient_irradiance(p, vec3<f32>(0.0, -1.0, 0.0))) / (2.0 * PI);
    if hidden { lin = vec3<f32>(0.0); }
    // The medium fills its bounds (all space when unbounded).
    let inside = gi.fog_bounds[0].w < 0.5 || (all(p >= gi.fog_bounds[0].xyz) && all(p <= gi.fog_bounds[1].xyz));
    let sigma_t = select(0.0, gi.fog.x, inside);
    var cur = vec4<f32>(lin * gi.fog_albedo.rgb * sigma_t, sigma_t);
    // Last frame's cell where this point was, 0.9 kept.
    if gi.fog.w > 1.5 {
        let pc = gi.fog_prev * vec4<f32>(p, 1.0);
        if pc.w > 1e-4 {
            let puv = vec2<f32>(pc.x / pc.w * 0.5 + 0.5, 0.5 - pc.y / pc.w * 0.5);
            let pz = fog_depth_slice(pc.w) / f32(FOG_GRID.z);
            if all(puv >= vec2<f32>(0.0)) && all(puv <= vec2<f32>(1.0)) && pz >= 0.0 && pz <= 1.0 {
                let prev = textureSampleLevel(fog_prev, canopy_sampler, vec3<f32>(puv, pz), 0.0);
                // A hidden sample leaves the history as it is: the cell averages
                // its visible air, not its visible share of it.
                cur = mix(prev, cur, select(0.1, 0.0, hidden));
            }
        }
    }
    textureStore(fog_out, cell, cur);
}

// The fog in front of a pixel at view depth w: rgb the light scattered
// towards the camera, a the transmittance.
fn froxel_fog(px: vec2<i32>, size: vec2<u32>, w: f32) -> vec4<f32> {
    if !fog_on() { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    let uv = (vec2<f32>(px) + 0.5) / vec2<f32>(size);
    // The integrated volume holds each slice's far side.
    let z = (fog_depth_slice(min(w, gi.fog.z)) - 0.5) / f32(FOG_GRID.z);
    let s = textureSampleLevel(fog_volume, canopy_sampler, vec3<f32>(uv, clamp(z, 0.0, 1.0)), 0.0);
    // In front of the first slice's far side: scale towards nothing.
    let first = fog_slice_depth(1.0);
    if w < first { let f = max(w - FOG_NEAR, 0.0) / (first - FOG_NEAR); return vec4<f32>(s.rgb * f, mix(1.0, s.a, f)); }
    return s;
}
