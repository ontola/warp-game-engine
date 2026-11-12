// Clustered local lights in shade (clustered_lights.rs). Group 2 is theirs
// alone, the grid uniform; the arena is a region of the scene tables
// (offsets in vec4s), which holds the lights
// (5 vec4 each: LocalLight's layout, then the face; source.z is the first shadow face + 1,
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
    let b = light_grid.offsets.w + i * 5u;
    return LocalLight(bitcast<vec4<f32>>(scene_tables[b]), bitcast<vec4<f32>>(scene_tables[b + 1u]),
        bitcast<vec4<f32>>(scene_tables[b + 2u]), bitcast<vec4<f32>>(scene_tables[b + 3u]));
}

// Cookies and angular profiles (light_shapes.rs): layers 0..14 cookies,
// layer 15 one profile per row. source.w above the face count: cookie
// (bits 3..6), profile (bits 7..14).
@group(0) @binding(30) var light_shapes: texture_2d_array<f32>;

// The light's shape towards `outward` (unit, from the lamp): its angular
// profile times its cookie projected through the outer cone.
fn light_shape(lamp: LocalLight, outward: vec3<f32>) -> f32 {
    let packed = u32(lamp.source.w);
    let cookie = (packed >> 3u) & 15u;
    let profile = (packed >> 7u) & 255u;
    let axis = lamp.direction_outer.xyz;
    let c = dot(outward, axis);
    var k = 1.0;
    if profile > 0u {
        let theta = acos(clamp(c, -1.0, 1.0)) * 0.31830989;
        k *= textureSampleLevel(light_shapes, plant_samp, vec2<f32>(theta, (f32(profile - 1u) + 0.5) / 256.0), 15, 0.0).r;
    }
    if cookie > 0u {
        let oc = lamp.direction_outer.w;
        if c <= 0.05 || oc <= 0.05 { return 0.0; }
        let t1 = normalize(select(cross(axis, vec3<f32>(0.0, 1.0, 0.0)), cross(axis, vec3<f32>(1.0, 0.0, 0.0)), abs(axis.y) > 0.9));
        let t2 = cross(axis, t1);
        let tan_outer = sqrt(max(1.0 - oc * oc, 0.0)) / oc;
        let uv = vec2<f32>(dot(outward, t1), dot(outward, t2)) / (c * tan_outer) * 0.5 + 0.5;
        k *= textureSampleLevel(light_shapes, plant_samp, uv, i32(cookie - 1u), 0.0).r;
    }
    return k;
}

// Visibility of the light from p through its shadow tile (1: lit).
fn clustered_shadow(lamp: LocalLight, p: vec3<f32>, n: vec3<f32>) -> f32 {
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
    // A large source (a bounce light standing in for a lit patch of floor)
    // casts soft shadows: percentage-closer soft shadows when the source
    // is 30 cm or more across (small lamps keep the cheap 4-tap filter).
    let radius = sqrt(lamp.source.x);
    if radius >= 0.3 && radius / max(texel, 1e-5) > 1.5 {
        return clustered_soft_shadow(uv, ndc.z, layer, rect, tile, atlas, edge, radius, dist, params, lamp.position_range.w, p);
    }
    var lit = 0.0;
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(-o, -o), layer, ndc.z);
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(o, -o), layer, ndc.z);
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(-o, o), layer, ndc.z);
    lit += textureSampleCompareLevel(headlight_depth, shadow_samp, uv + vec2<f32>(o, o), layer, ndc.z);
    return lit * 0.25;
}

// Distance from the light for a face's stored depth (perspective, 0 at the
// near plane, 1 at `far`).
fn clustered_linear(z: f32, near: f32, far: f32) -> f32 { return far * near / max(far - z * (far - near), 1e-6); }

// PCSS for one shadow face: 8 taps find the average blocker within the
// widest penumbra the source could cast, the penumbra is the source radius
// times (receiver - blocker) / blocker, and 16 compare taps on a Vogel disc
// of that width (turned per position) filter it. Widths are capped at 24
// texels of the tile (a 256-texel face).
fn clustered_soft_shadow(uv: vec2<f32>, z: f32, layer: i32, rect: vec4<f32>, tile: f32, atlas: f32, edge: f32,
        radius: f32, dist: f32, params: vec4<f32>, far: f32, p: vec3<f32>) -> f32 {
    let near = params.y;
    let receiver = clustered_linear(z, near, far);
    // Texels of this face per metre at the receiver.
    let per_m = 1.0 / max(params.x * receiver, 1e-6);
    let lo = rect.xy + vec2<f32>(edge * rect.z);
    let hi = rect.xy + vec2<f32>((1.0 - edge) * rect.z);
    let turn = fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453) * 6.2831853;
    // Blocker search over the penumbra a blocker at a quarter of the way
    // would cast.
    let search = clamp(radius * 3.0 * per_m, 1.0, 24.0) / atlas;
    var blockers = 0.0;
    var count = 0.0;
    for (var k = 0; k < 8; k++) {
        let a = turn + f32(k) * 2.3999632;
        let r = sqrt((f32(k) + 0.5) / 8.0) * search;
        let tap = clamp(uv + vec2<f32>(cos(a), sin(a)) * r, lo, hi);
        let stored = textureLoad(headlight_depth, vec2<i32>(tap * atlas), layer, 0);
        if stored < z {
            blockers += clustered_linear(stored, near, far);
            count += 1.0;
        }
    }
    if count < 0.5 { return 1.0; }
    let blocker = blockers / count;
    let penumbra = clamp(radius * (receiver - blocker) / max(blocker, 1e-3) * per_m, 1.0, 24.0) / atlas;
    var lit = 0.0;
    for (var k = 0; k < 16; k++) {
        let a = turn + f32(k) * 2.3999632;
        let r = sqrt((f32(k) + 0.5) / 16.0) * penumbra;
        let tap = clamp(uv + vec2<f32>(cos(a), sin(a)) * r, lo, hi);
        lit += textureSampleCompareLevel(headlight_depth, shadow_samp, tap, layer, z);
    }
    return lit / 16.0;
}

fn clustered_sample(i: u32, p: vec3<f32>, bias_normal: vec3<f32>) -> HeadlightSample {
    let lamp = clustered_lamp(i);
    var s = local_light_at(lamp, p);
    if u32(lamp.source.w) >= 8u && any(s.irradiance > vec3<f32>(0.0)) { s.irradiance *= light_shape(lamp, -s.direction); }
    if lamp.source.z > 0.5 && any(s.irradiance > vec3<f32>(0.0)) { s.irradiance *= clustered_shadow(lamp, p, bias_normal); }
    return s;
}

// Specular of a light with a source radius (Karis 2013, "representative
// point"): the highlight comes from the point of the source sphere closest
// to the mirror ray, and the lobe is widened by the source's angular size
// with the energy renormalised by (a / a')^2. A point light's highlight is
// only the GGX lobe of one direction, far smaller than a 0.5 m lamp panel.
fn sphere_light_specular(lamp: LocalLight, p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, centre: vec3<f32>, rough: f32, f0: vec3<f32>) -> vec3<f32> {
    let radius = sqrt(max(lamp.source.x, 0.0));
    if radius <= 0.0 { return ggx(n, v, centre, rough, f0); }
    let to = lamp.position_range.xyz - p;
    let dist = max(length(to), 1e-4);
    let r = reflect(-v, n);
    let to_ray = dot(to, r) * r - to;
    let closest = to + to_ray * clamp(radius / max(length(to_ray), 1e-6), 0.0, 1.0);
    let a = max(rough * rough, 2e-3);
    let wide = min(a + radius / (2.0 * dist), 1.0);
    let norm = (a / wide) * (a / wide);
    return ggx(n, v, normalize(closest), rough, f0) * norm;
}

// Area-light specular with linearly transformed cosines (Heitz, Dupuy,
// Hill, Neubelt 2016). The tables (light_face.rs, fitted to `ggx` by
// examples/ltc_fit.rs) sit just below the lights in the arena: 64 x 64
// entries over roughness and sqrt(1 - n.v), two vec4 each (the inverse
// matrix m00, m02, m20, m22, then the albedo without Fresnel and its
// Schlick weight).
const LTC_SIZE: u32 = 64u;
const LTC_VEC4S: u32 = 8192u;

struct Ltc { minv: mat3x3<f32>, albedo: vec2<f32> }

fn ltc_lookup(rough: f32, ndv: f32) -> Ltc {
    let f = vec2<f32>(clamp(rough, 0.0, 1.0), sqrt(1.0 - clamp(ndv, 0.0, 1.0))) * f32(LTC_SIZE - 1u);
    let i0 = min(floor(f), vec2<f32>(f32(LTC_SIZE - 2u)));
    let t = f - i0;
    let base = light_grid.offsets.w - LTC_VEC4S;
    var m = vec4<f32>(0.0);
    var a = vec2<f32>(0.0);
    for (var k = 0u; k < 4u; k++) {
        let c = vec2<u32>(i0) + vec2<u32>(k & 1u, k >> 1u);
        let w = select(1.0 - t.x, t.x, (k & 1u) != 0u) * select(1.0 - t.y, t.y, (k >> 1u) != 0u);
        let at = base + (c.x + c.y * LTC_SIZE) * 2u;
        m += bitcast<vec4<f32>>(scene_tables[at]) * w;
        a += bitcast<vec4<f32>>(scene_tables[at + 1u]).xy * w;
    }
    return Ltc(mat3x3<f32>(vec3<f32>(m.x, 0.0, m.y), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(m.z, 0.0, m.w)), a);
}

// The edge's share of the cosine integral over a spherical polygon,
// theta / sin(theta) by a fit that stays accurate near theta = pi (Heitz's
// later LTC code).
fn ltc_edge(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let x = dot(a, b);
    let y = abs(x);
    let u = 0.8543985 + (0.4965155 + 0.0145206 * y) * y;
    let w = 3.4175940 + (4.1616724 + y) * y;
    let v = u / w;
    let theta_over_sin = select(0.5 * inverseSqrt(max(1.0 - x * x, 1e-7)) - v, v, x > 0.0);
    return cross(a, b).z * theta_over_sin;
}

// What the LTC needs of the shaded pixel, the same for every light: the
// matrix into the cosine's space and the Fresnel-weighted albedo.
struct LtcPixel { m: mat3x3<f32>, albedo: vec3<f32> }

fn ltc_pixel(n: vec3<f32>, v: vec3<f32>, rough: f32, f0: vec3<f32>) -> LtcPixel {
    let l = ltc_lookup(rough, dot(n, v));
    let t1 = normalize(v - n * dot(v, n) + select(vec3<f32>(0.0), vec3<f32>(1e-4, 0.0, 0.0), abs(dot(v, n)) > 0.9999));
    let t2 = cross(n, t1);
    return LtcPixel(l.minv * transpose(mat3x3<f32>(t1, t2, n)), f0 * l.albedo.x + (1.0 - f0) * l.albedo.y);
}

// The polygon clipped to the horizon, streamed edge by edge (no arrays):
// where an edge leaves the upper hemisphere the clipped polygon runs along
// the horizon to where the next edge enters it.
struct LtcClip { sum: f32, left: vec3<f32>, open: bool, first_entry: vec3<f32>, entered: bool }

fn ltc_clipped_edge(clip: ptr<function, LtcClip>, a: vec3<f32>, b: vec3<f32>) {
    if a.z >= 0.0 && b.z >= 0.0 { (*clip).sum += ltc_edge(normalize(a), normalize(b)); return; }
    if a.z < 0.0 && b.z < 0.0 { return; }
    let x = normalize(a + (b - a) * (a.z / (a.z - b.z)));
    if a.z >= 0.0 {
        (*clip).sum += ltc_edge(normalize(a), x);
        (*clip).left = x;
        (*clip).open = true;
    } else {
        if (*clip).open { (*clip).sum += ltc_edge((*clip).left, x); (*clip).open = false; }
        else if !(*clip).entered { (*clip).first_entry = x; }
        (*clip).entered = true;
        (*clip).sum += ltc_edge(x, normalize(b));
    }
}

// A face light's specular per unit of its sample's irradiance: the face
// radiance (intensity over area) times the LTC integral of the specular
// over the face (Heitz et al. 2016: transformed into the cosine's space,
// clipped to its horizon, integrated edge by edge), with the light's cone,
// falloff, shape and shadow carried by the irradiance.
fn face_light_specular(lamp: LocalLight, face: vec4<f32>, p: vec3<f32>, px: LtcPixel) -> vec3<f32> {
    let centre = lamp.position_range.xyz - p;
    // Only the emitting side shows.
    if dot(centre, lamp.direction_outer.xyz) >= 0.0 { return vec3<f32>(0.0); }
    let u = face.xyz;
    let w = normalize(cross(lamp.direction_outer.xyz, u)) * select(face.w, length(u), face.w < 0.0);
    // A rectangle's four corners, or a disc as the octagon of its area.
    let disc = face.w < 0.0;
    let count = select(4u, 8u, disc);
    let area = select(4.0 * length(u) * face.w, PI * dot(u, u), disc);
    let k = select(1.0, sqrt(PI / (2.0 * sqrt(2.0))), disc);
    let c = px.m * centre;
    let mu = px.m * u * k;
    let mw = px.m * w * k;
    var clip = LtcClip(0.0, vec3<f32>(0.0), false, vec3<f32>(0.0), false);
    var prev = vec3<f32>(0.0);
    for (var j = 0u; j <= count; j++) {
        var q: vec3<f32>;
        if disc {
            let a = f32(j % 8u) * PI * 0.25;
            q = c + mu * cos(a) + mw * sin(a);
        } else {
            let s = vec2<f32>(select(-1.0, 1.0, (j % 4u) == 1u || (j % 4u) == 2u), select(-1.0, 1.0, (j % 4u) >= 2u));
            q = c + mu * s.x + mw * s.y;
        }
        if j > 0u { ltc_clipped_edge(&clip, prev, q); }
        prev = q;
    }
    // A polygon that began below the horizon: its last exit closes along
    // the horizon to its first entry.
    if clip.open && clip.entered { clip.sum += ltc_edge(clip.left, clip.first_entry); }
    let integral = abs(clip.sum) / (2.0 * PI);
    let d2 = dot(centre, centre);
    let spread = select(pow(max(d2, lamp.source.x), lamp.source.y * 0.5), max(d2, lamp.source.x), lamp.source.y == 2.0 || lamp.source.y == 0.0);
    return px.albedo * (integral * spread / max(area, 1e-8));
}

fn clustered_radiance(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diffuse: vec3<f32>, rough: f32, f0: vec3<f32>) -> vec3<f32> {
    var result = vec3<f32>(0.0);
    let cell = clustered_cell(p);
    if cell == NO_CELL { return result; }
    let count = scene_tables[cell].x;
    // The LTC's per-pixel part, looked up once for every face light.
    var ltc: LtcPixel;
    var have_ltc = false;
    for (var k = 0u; k < count; k++) {
        let i = clustered_index(cell, k);
        // One-sided, as headlight_radiance: skip lights behind the surface.
        if dot(n, bitcast<vec4<f32>>(scene_tables[light_grid.offsets.w + i * 5u]).xyz - p) <= 0.0 { continue; }
        let s = clustered_sample(i, p, n);
        if all(s.irradiance == vec3<f32>(0.0)) { continue; }
        // Bit 15 of source.w: diffuse only (a bounce light; no highlight).
        let source_w = u32(bitcast<vec4<f32>>(scene_tables[light_grid.offsets.w + i * 5u + 3u]).w);
        var spec = vec3<f32>(0.0);
        if (source_w & 32768u) == 0u {
            let face = bitcast<vec4<f32>>(scene_tables[light_grid.offsets.w + i * 5u + 4u]);
            if face.w != 0.0 {
                if !have_ltc { ltc = ltc_pixel(n, v, rough, f0); have_ltc = true; }
                spec = face_light_specular(clustered_lamp(i), face, p, ltc);
            } else { spec = sphere_light_specular(clustered_lamp(i), p, n, v, s.direction, rough, f0); }
        }
        result += s.irradiance * (diffuse * max(dot(n, s.direction), 0.0) + spec);
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
