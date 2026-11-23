// Aerial perspective and volumetric clouds (engine/docs/atmosphere.md,
// render::sky_volume). Runs after temporal reconstruction, like volumetric
// light, and writes a separate output that never enters TAA/DLSS history.
//
//   noise          once: tileable 3D cloud noise (shape and detail).
//   froxels        per view: a 32^3 camera volume of in-scattered light and
//                  transmittance along each view ray (Hillaire 2020), from
//                  the CPU bake's transmittance and multiple-scattering LUTs.
//   clouds         half resolution: raymarch the cloud layer up to the scene
//                  depth, sun light marched through the clouds with a
//                  multiple-scattering octave approximation.
//   cloud_resolve  half resolution: reproject last frame's clouds and blend,
//                  clamped to this frame's neighbourhood.
//   composite      full resolution: aerial perspective on surfaces, then the
//                  clouds (themselves behind their own aerial perspective).
//
// Units: atmosphere positions in km on a planet centred below the camera,
// world positions in m. Radiance in the renderer's physical units.
// cloud_weather.wgsl is prepended (shared with shade.wgsl).

struct U {
    // All three camera-relative (world position minus this frame's camera).
    inv_vp: mat4x4<f32>,       // unjittered: view rays
    prev_vp: mat4x4<f32>,      // unjittered, last frame: cloud history
    inv_depth_vp: mat4x4<f32>, // jittered, as the depth buffer was drawn
    camera: vec4<f32>,      // xyz world (m); w frame index
    sizes: vec4<f32>,       // depth w,h; cloud w,h
    output: vec4<f32>,      // output w,h; history valid; aerial on surfaces
    sun: vec4<f32>,         // towards the sun; w unused
    solar: vec4<f32>,       // illuminance above the atmosphere (lux rgb)
    above: vec4<f32>,       // mean sky radiance from above
    below: vec4<f32>,       // ground radiance from below
    rayleigh: vec4<f32>,    // scattering per km rgb; w scale height km
    mie: vec4<f32>,         // scattering per km, absorption per km, scale height km, g
    ozone: vec4<f32>,       // absorption per km rgb; w planet radius km
    extent: vec4<f32>,      // top radius km, aerial max distance km, cloud far distance km, unused
    cloud: vec4<f32>,       // coverage, base m, thickness m, extinction per m
    cloud_motion: vec4<f32>,// wind offset xz (m), shadow strength, enabled
    cloud_shape: vec4<f32>, // shape tile m, detail tile m, steps, light steps
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(3) var multi_lut: texture_2d<f32>;
@group(0) @binding(4) var clamp_samp: sampler;
@group(0) @binding(5) var noise_shape: texture_3d<f32>;
@group(0) @binding(6) var noise_detail: texture_3d<f32>;
@group(0) @binding(7) var repeat_samp: sampler;
@group(0) @binding(8) var froxel_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(9) var froxel_in: texture_3d<f32>;
@group(0) @binding(10) var cloud_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(11) var cloud_depth_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(12) var cloud_in: texture_2d<f32>;
@group(0) @binding(13) var cloud_depth_in: texture_2d<f32>;
@group(0) @binding(14) var history_in: texture_2d<f32>;
@group(0) @binding(15) var history_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(16) var colour_in: texture_2d<f32>;
@group(0) @binding(17) var colour_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(18) var noise_out: texture_storage_3d<rgba8unorm, write>;

const PI: f32 = 3.14159265;
const SLICES: u32 = 32u;
// Cloud depth (km) stored where no cloud was seen.
const NO_CLOUD_KM: f32 = 10000.0;

// ---- Atmosphere (matches render::atmosphere) ----

fn planet_radius() -> f32 { return u.ozone.w; }
fn top_radius() -> f32 { return u.extent.x; }

fn top_distance(p: vec3<f32>, d: vec3<f32>) -> f32 {
    let b = dot(p, d);
    let top = top_radius();
    return max(-b + sqrt(max(b * b + top * top - dot(p, p), 0.0)), 0.0);
}

fn hits_ground(p: vec3<f32>, d: vec3<f32>) -> bool {
    let b = dot(p, d);
    if b >= 0.0 { return false; }
    let r = planet_radius();
    return b * b - (dot(p, p) - r * r) >= 0.0;
}

// Scattering (rayleigh, mie) and extinction at height h (km).
struct Medium { rayleigh: vec3<f32>, mie: f32, extinction: vec3<f32> };
fn medium(height: f32) -> Medium {
    let h = max(height, 0.0);
    let ray = u.rayleigh.rgb * exp(-h / u.rayleigh.w);
    let density = exp(-h / u.mie.z);
    let mie = u.mie.x * density;
    let ozone = u.ozone.rgb * max(1.0 - abs(h - 25.0) / 15.0, 0.0);
    return Medium(ray, mie, ray + vec3<f32>(mie + u.mie.y * density) + ozone);
}

// The CPU LUT's horizon-aware coordinates (atmosphere.rs TransmittanceLut).
fn transmittance_to_sun(p: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    if hits_ground(p, d) { return vec3<f32>(0.0); }
    let bottom = planet_radius();
    let top = top_radius();
    let radius = clamp(length(p), bottom, top);
    let horizon = sqrt(top * top - bottom * bottom);
    let rho = sqrt(max(radius * radius - bottom * bottom, 0.0));
    let minimum = top - radius;
    let maximum = rho + horizon;
    let x = clamp((top_distance(p, d) - minimum) / max(maximum - minimum, 1e-6), 0.0, 1.0);
    let y = clamp(rho / horizon, 0.0, 1.0);
    let dims = vec2<f32>(textureDimensions(transmittance_lut));
    let uv = (vec2<f32>(x, y) * (dims - 1.0) + 0.5) / dims;
    return textureSampleLevel(transmittance_lut, clamp_samp, uv, 0.0).rgb;
}

fn multi_scatter(p: vec3<f32>, l: vec3<f32>) -> vec3<f32> {
    let radius = length(p);
    let mu = clamp(dot(p, l) / radius, -1.0, 1.0);
    let height = (radius - planet_radius()) / (top_radius() - planet_radius());
    let dims = vec2<f32>(textureDimensions(multi_lut));
    let uv = (clamp(vec2<f32>(mu * 0.5 + 0.5, height), vec2<f32>(0.0), vec2<f32>(1.0)) * (dims - 1.0) + 0.5) / dims;
    return textureSampleLevel(multi_lut, clamp_samp, uv, 0.0).rgb;
}

fn rayleigh_phase(mu: f32) -> f32 { return 3.0 * (1.0 + mu * mu) / (16.0 * PI); }
fn mie_phase(mu: f32, g: f32) -> f32 {
    let g2 = g * g;
    return 3.0 * (1.0 - g2) * (1.0 + mu * mu) / (8.0 * PI * (2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * mu, 1e-4), 1.5));
}
fn hg_phase(mu: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = max(1.0 + g2 - 2.0 * g * mu, 1e-4);
    return (1.0 - g2) / (4.0 * PI * denom * sqrt(denom));
}

// The camera in planet coordinates (km): straight above the centre.
fn camera_planet() -> vec3<f32> {
    return vec3<f32>(0.0, planet_radius() + max(u.camera.y * 0.001, 0.001), 0.0);
}

fn view_ray(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    // A point well past the near plane (reversed Z: depth near/distance):
    // the camera-relative matrix still carries the f32 rounding of the
    // world-space view matrix (~1 cm 100 km out), which against a 5 cm near
    // plane turned the ray by degrees and made the clouds flash.
    let far = u.inv_vp * vec4<f32>(ndc, 1e-4, 1.0);
    return normalize(far.xyz / far.w);
}

fn scene_distance(uv: vec2<f32>, depth: f32) -> f32 {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let hit = u.inv_depth_vp * vec4<f32>(ndc, depth, 1.0);
    return length(hit.xyz / hit.w);
}

fn clouds_on() -> bool { return u.cloud_motion.w > 0.5; }

// ---- Froxels: aerial perspective ----

// Distance (km) at the far end of slice i (0-based), quadratic.
fn slice_distance(i: f32) -> f32 {
    let s = (i + 1.0) / f32(SLICES);
    return u.extent.y * s * s;
}

@compute @workgroup_size(8, 8)
fn froxels(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= vec2<u32>(SLICES)) { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) / f32(SLICES);
    let d = view_ray(uv);
    let origin = camera_planet();
    let l = u.sun.xyz;
    let mu = dot(d, l);
    let phase_r = rayleigh_phase(mu);
    let phase_m = mie_phase(mu, u.mie.w);
    var transmittance = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);
    var previous = 0.0;
    for (var i = 0u; i < SLICES; i++) {
        let end = slice_distance(f32(i));
        // Two samples per slice.
        for (var k = 0u; k < 2u; k++) {
            let a = mix(previous, end, f32(k) * 0.5);
            let b = mix(previous, end, f32(k + 1u) * 0.5);
            let ds = b - a;
            let p = origin + d * (0.5 * (a + b));
            let m = medium(length(p) - planet_radius());
            var sun = transmittance_to_sun(p, l);
            if clouds_on() {
                let world = u.camera.xyz + d * (0.5 * (a + b) * 1000.0);
                sun *= cloud_sun_transmittance(world, l, u.cloud, u.cloud_motion);
            }
            let scattering = m.rayleigh + vec3<f32>(m.mie);
            let source = ((m.rayleigh * phase_r + vec3<f32>(m.mie * phase_m)) * sun
                + scattering * multi_scatter(p, l)) * u.solar.rgb;
            let step = exp(-m.extinction * ds);
            let integral = select(vec3<f32>(ds), (1.0 - step) / m.extinction, m.extinction > vec3<f32>(1e-7));
            radiance += transmittance * source * integral;
            transmittance *= step;
        }
        previous = end;
        let mean = dot(transmittance, vec3<f32>(1.0 / 3.0));
        textureStore(froxel_out, vec3<i32>(vec3<u32>(id.xy, i)), vec4<f32>(radiance, mean));
    }
}

// In-scattered radiance (rgb) and mean transmittance (a) between the camera
// and `distance_km` along the ray through `uv`.
fn aerial(uv: vec2<f32>, distance_km: f32) -> vec4<f32> {
    let s = sqrt(max(distance_km, 0.0) / u.extent.y) * f32(SLICES) - 1.0;
    let w = (clamp(s, 0.0, f32(SLICES) - 1.0) + 0.5) / f32(SLICES);
    let v = textureSampleLevel(froxel_in, clamp_samp, vec3<f32>(uv, w), 0.0);
    // In the first slice blend from no atmosphere at the camera.
    let f = clamp(s + 1.0, 0.0, 1.0);
    return mix(vec4<f32>(0.0, 0.0, 0.0, 1.0), v, f);
}

// ---- Noise (once) ----

fn pcg3(v0: vec3<u32>) -> vec3<f32> {
    var v = v0 * 1664525u + 1013904223u;
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    return vec3<f32>(v & vec3<u32>(0xffffffu)) / 16777215.0;
}

fn wrap_cell(c: vec3<f32>, cells: f32) -> vec3<u32> {
    return vec3<u32>(((c % cells) + cells) % cells);
}

// Tileable inverted Worley noise (1 at feature points), p in [0, 1)^3.
fn worley(p: vec3<f32>, cells: f32) -> f32 {
    let q = p * cells;
    let i = floor(q);
    let f = q - i;
    var best = 1.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<f32>(f32(x), f32(y), f32(z));
                let r = pcg3(wrap_cell(i + o, cells) + vec3<u32>(u32(cells) * 7919u));
                best = min(best, length(o + r - f));
            }
        }
    }
    return 1.0 - clamp(best, 0.0, 1.0);
}

fn gradient(c: vec3<u32>) -> vec3<f32> { return normalize(pcg3(c + vec3<u32>(101u)) * 2.0 - 1.0 + vec3<f32>(1e-4)); }

// Tileable gradient noise in about [-1, 1].
fn perlin(p: vec3<f32>, cells: f32) -> f32 {
    let q = p * cells;
    let i = floor(q);
    let f = q - i;
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    var corners: array<f32, 8>;
    for (var k = 0u; k < 8u; k++) {
        let o = vec3<f32>(f32(k & 1u), f32((k >> 1u) & 1u), f32(k >> 2u));
        corners[k] = dot(gradient(wrap_cell(i + o, cells)), f - o);
    }
    let x0 = mix(corners[0], corners[1], s.x);
    let x1 = mix(corners[2], corners[3], s.x);
    let x2 = mix(corners[4], corners[5], s.x);
    let x3 = mix(corners[6], corners[7], s.x);
    return mix(mix(x0, x1, s.y), mix(x2, x3, s.y), s.z) * 1.6;
}

fn remap(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    return c + (v - a) / max(b - a, 1e-5) * (d - c);
}

// z of the dispatch selects the texture: the shape texture's slices first,
// then the detail texture's (the caller dispatches each separately with
// u.cloud_shape.w as the size: see sky_volume.rs).
@compute @workgroup_size(4, 4, 4)
fn noise(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = u32(u.output.x);
    let detail = u.output.y > 0.5;
    if any(id >= vec3<u32>(size)) { return; }
    let p = (vec3<f32>(id) + 0.5) / f32(size);
    var value: vec4<f32>;
    if detail {
        let a = worley(p, 4.0);
        let b = worley(p, 8.0);
        let c = worley(p, 16.0);
        value = vec4<f32>(a, b, c, a * 0.625 + b * 0.25 + c * 0.125);
    } else {
        // Perlin-Worley (Schneider 2015): Perlin dilated by Worley, so the
        // billows of the Worley cells stay connected.
        let fbm = perlin(p, 4.0) * 0.55 + perlin(p, 8.0) * 0.3 + perlin(p, 16.0) * 0.15;
        let perlin01 = clamp(fbm * 0.7 + 0.5, 0.0, 1.0);
        let w0 = worley(p, 4.0) * 0.625 + worley(p, 8.0) * 0.25 + worley(p, 16.0) * 0.125;
        let pw = clamp(remap(perlin01, 0.0, 1.0, w0, 1.0), 0.0, 1.0);
        let w2 = worley(p, 8.0) * 0.625 + worley(p, 16.0) * 0.25 + worley(p, 32.0) * 0.125;
        let w3 = worley(p, 16.0) * 0.625 + worley(p, 32.0) * 0.25 + worley(p, 64.0) * 0.125;
        let w4 = worley(p, 32.0) * 0.75 + worley(p, 64.0) * 0.25;
        value = vec4<f32>(pw, w2, w3, w4);
    }
    textureStore(noise_out, vec3<i32>(id), value);
}

// ---- Clouds ----

// Altitude (m above the datum) along a view ray, on a sphere of the
// planet's radius: h0 + t mu + t^2 (1 - mu^2) / 2R. Kept in this form
// (instead of |o + t d| - R with o six million metres from the centre) so
// f32 rounding does not move the cloud layer as the camera climbs.
fn curvature(mu: f32) -> f32 { return (1.0 - mu * mu) / (2.0 * planet_radius() * 1000.0); }
fn altitude_along(h0: f32, mu: f32, t: f32) -> f32 { return h0 + t * (mu + t * curvature(mu)); }

// Ray distances where the altitude equals h, sorted; x > y when never.
fn altitude_roots(h0: f32, mu: f32, h: f32) -> vec2<f32> {
    let k = curvature(mu);
    let c = h0 - h;
    let disc = mu * mu - 4.0 * k * c;
    if disc < 0.0 { return vec2<f32>(1.0, -1.0); }
    // Numerically stable quadratic roots.
    let q = -0.5 * (mu + select(-1.0, 1.0, mu >= 0.0) * sqrt(disc));
    if abs(q) < 1e-12 { return vec2<f32>(1.0, -1.0); }
    let a = c / q;
    // Straight up or down the curvature term vanishes: one crossing.
    let b = select(-1e30 * sign(mu), q / k, k > 1e-12);
    return vec2<f32>(min(a, b), max(a, b));
}

// Near and far ray distances (m) inside the cloud layer, first piece only.
fn shell_segment(h0: f32, mu: f32) -> vec2<f32> {
    let base = u.cloud.y;
    let top = base + u.cloud.z;
    // Below the top: between the two crossings of the top altitude.
    let below_top = altitude_roots(h0, mu, top);
    if below_top.x > below_top.y { return vec2<f32>(-1.0); }
    var t0 = max(below_top.x, 0.0);
    var t1 = below_top.y;
    if t1 <= 0.0 { return vec2<f32>(-1.0); }
    // Above the base: outside the two crossings of the base altitude.
    let below_base = altitude_roots(h0, mu, base);
    if below_base.x <= below_base.y {
        if below_base.x > t0 { t1 = min(t1, below_base.x); }
        else { t0 = max(t0, below_base.y); }
    }
    // The planet hides what is below the horizon.
    let ground = altitude_roots(h0, mu, 0.0);
    if ground.x <= ground.y && ground.x > 0.0 { t1 = min(t1, ground.x); }
    if t1 <= t0 { return vec2<f32>(-1.0); }
    return vec2<f32>(t0, t1);
}

fn height_fraction(altitude: f32) -> f32 {
    return clamp((altitude - u.cloud.y) / u.cloud.z, 0.0, 1.0);
}

// Extinction per m at `altitude` (m), world position world (m).
fn cloud_density(altitude: f32, world: vec3<f32>, cheap: bool) -> f32 {
    let h = height_fraction(altitude);
    let w = cloud_weather(world.xz + u.cloud_motion.xy, u.cloud.x);
    if w <= 0.0 { return 0.0; }
    // Cumulus-like profile: a flat-ish base, rounded tops that reach higher
    // where the weather is denser.
    let top = mix(0.55, 1.0, w);
    let profile = smoothstep(0.0, 0.12, h) * (1.0 - smoothstep(top * 0.6, top, h));
    if profile <= 0.0 { return 0.0; }
    let drift = vec3<f32>(u.cloud_motion.x, 0.0, u.cloud_motion.y);
    // Skew the shape with height so tops lean downwind.
    let shape_p = (world + drift + vec3<f32>(h * 400.0, 0.0, h * 150.0)) / u.cloud_shape.x;
    let s = textureSampleLevel(noise_shape, repeat_samp, shape_p, 0.0);
    let fbm = s.g * 0.625 + s.b * 0.25 + s.a * 0.125;
    let base = clamp(remap(s.r, fbm - 1.0, 1.0, 0.0, 1.0), 0.0, 1.0);
    // The weather lowers the threshold the 3D shape must clear: sparse
    // turrets at the edge of a cell, a lumpy mass with holes at its centre.
    var d = clamp(remap(base * profile, mix(1.0, 0.6, w), 1.0, 0.0, 1.0), 0.0, 1.0);
    if d <= 0.0 { return 0.0; }
    if !cheap {
        let detail = textureSampleLevel(noise_detail, repeat_samp, (world + drift) / u.cloud_shape.y, 0.0).a;
        // Wispy at the base, billowy at the top.
        let erode = mix(detail, 1.0 - detail, clamp(h * 4.0, 0.0, 1.0));
        d = clamp(remap(d, erode * 0.4, 1.0, 0.0, 1.0), 0.0, 1.0);
    }
    return d * u.cloud.w;
}

// Interleaved gradient noise (Jimenez), shifted each frame: every 3x3
// block of pixels holds an even spread of offsets, so the resolve's 3x3
// filter averages the march noise away in one frame instead of leaving
// per-pixel speckle for the history to chase.
fn interleaved_noise(p: vec2<f32>, frame: f32) -> f32 {
    let q = p + 5.588238 * (frame % 64.0);
    return fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
}

@compute @workgroup_size(8, 8)
fn clouds(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(u.sizes.zw);
    if any(id.xy >= size) { return; }
    // The farthest depth of the 2x2 footprint: clouds in front of a
    // silhouette stay, the composite drops them where geometry is nearer.
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let base = vec2<i32>(id.xy) * 2;
    var depth = 1.0;
    for (var k = 0; k < 4; k++) {
        depth = min(depth, textureLoad(scene_depth, min(base + vec2<i32>(k & 1, k >> 1u), last), 0));
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / u.sizes.zw;
    let d = view_ray(uv);
    var tmax = u.extent.z * 1000.0;
    if depth > 0.0 { tmax = min(tmax, scene_distance(uv, depth)); }
    let h0 = u.camera.y;
    let seg = shell_segment(h0, d.y);
    var result = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var front = NO_CLOUD_KM;
    // The two spare depth channels enclose the contributing part of this ray.
    var first_km = NO_CLOUD_KM;
    var last_km = 0.0;
    if seg.y > 0.0 && seg.x < tmax {
        let t0 = seg.x;
        let t1 = min(seg.y, tmax);
        let steps = u32(u.cloud_shape.z);
        let span = t1 - t0;
        // Steps grow with distance (t = t0 + span x^2): looking along the
        // layer from inside it the segment runs tens of km, and even steps a
        // km long make every near sample all-or-nothing, a speckle the
        // history cannot average away.
        let inv_steps = 1.0 / f32(steps);
        let l = u.sun.xyz;
        let mu = dot(d, l);
        // Two-lobe phase: a strong forward silver lining and a soft back lobe.
        let phase = mix(hg_phase(mu, 0.8), hg_phase(mu, -0.25), 0.3);
        let jitter = interleaved_noise(vec2<f32>(id.xy), u.camera.w);
        var transmittance = 1.0;
        var radiance = vec3<f32>(0.0);
        var weighted = 0.0;
        var weight = 0.0;
        let light_steps = u32(u.cloud_shape.w);
        for (var i = 0u; i < steps; i++) {
            if transmittance < 0.01 { break; }
            let x = (f32(i) + jitter) * inv_steps;
            let t = t0 + span * x * x;
            let dt = span * 2.0 * x * inv_steps;
            let altitude = altitude_along(h0, d.y, t);
            let world = u.camera.xyz + d * t;
            let sigma = cloud_density(altitude, world, false);
            if sigma <= 0.0 { continue; }
            // Sun through the cloud: growing steps towards the sun.
            var tau = 0.0;
            var reach = 0.0;
            var step_len = u.cloud.z / f32(1u << light_steps) * 2.0;
            for (var j = 0u; j < light_steps; j++) {
                // Jittered like the view march: fixed sample points band the
                // self-shadow into steps under a big cloud.
                let q = reach + step_len * jitter;
                tau += cloud_density(altitude_along(altitude, l.y, q), world + l * q, true) * step_len;
                reach += step_len;
                step_len *= 2.0;
            }
            let h = height_fraction(altitude);
            let pk = vec3<f32>(0.0, planet_radius() + max(altitude, 1.0) * 0.001, 0.0);
            let sun = u.solar.rgb * transmittance_to_sun(pk, l);
            // Octaves of multiple scattering (Wrenninge): each less
            // attenuated and less forward than the last.
            // Contribution falls slower than extinction: light that has
            // scattered several times reaches deep into the cloud, which is
            // what makes thick cumulus white instead of grey.
            var lit = 0.0;
            var a = 1.0;
            var b = 1.0;
            var g = 1.0;
            for (var n = 0u; n < 4u; n++) {
                lit += a * mix(hg_phase(mu, 0.8 * g), hg_phase(mu, -0.25 * g), 0.3) * exp(-tau * b);
                a *= 0.7;
                b *= 0.4;
                g *= 0.5;
            }
            // Powder: dark edges facing the viewer on the sunny side.
            let powder = 1.0 - exp(-sigma * 120.0);
            let direct = sun * lit * mix(1.0, powder * 2.0, 0.5 * clamp(1.0 - mu, 0.0, 1.0));
            let ambient = u.above.rgb * mix(0.5, 1.0, h) + u.below.rgb * (1.0 - h) * 0.6;
            let source = (direct + ambient * 0.9) * sigma;
            let step = exp(-sigma * dt);
            radiance += transmittance * source * (1.0 - step) / sigma;
            let before = transmittance;
            transmittance *= step;
            if before - transmittance > 1e-5 {
                first_km = min(first_km, max(t0, t - dt * 0.5) * 0.001);
                last_km = min(t1, t + dt * 0.5) * 0.001;
            }
            weighted += (before - transmittance) * t;
            weight += before - transmittance;
        }
        result = vec4<f32>(radiance, transmittance);
        if weight > 1e-4 { front = weighted / weight * 0.001; }
        if 1.0 - transmittance < 1e-3 { front = NO_CLOUD_KM; }
    }
    textureStore(cloud_out, vec2<i32>(id.xy), result);
    textureStore(cloud_depth_out, vec2<i32>(id.xy), vec4<f32>(front, first_km, last_km, 0.0));
}

@compute @workgroup_size(8, 8)
fn cloud_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(u.sizes.zw);
    if any(id.xy >= size) { return; }
    let p = vec2<i32>(id.xy);
    let last = vec2<i32>(size) - 1;
    // A tent filter over the 3x3 neighbourhood is the new frame (the march
    // noise is per pixel; this takes it down by more than half), and the
    // neighbourhood's mean and spread bound the history: tighter than its
    // min and max, so stale cloud does not trail behind moving edges.
    var current = vec4<f32>(0.0);
    var m1 = vec4<f32>(0.0);
    var m2 = vec4<f32>(0.0);
    for (var k = 0; k < 9; k++) {
        let o = vec2<i32>(k % 3 - 1, k / 3 - 1);
        let q = clamp(p + o, vec2<i32>(0), last);
        let s = textureLoad(cloud_in, q, 0);
        let w = f32((2 - abs(o.x)) * (2 - abs(o.y))) / 16.0;
        current += s * w;
        m1 += s;
        m2 += s * s;
    }
    m1 /= 9.0;
    let sigma = sqrt(max(m2 / 9.0 - m1 * m1, vec4<f32>(0.0)));
    let lo = m1 - 1.25 * sigma;
    let hi = m1 + 1.25 * sigma;
    var out = current;
    if u.output.z > 0.5 {
        let uv = (vec2<f32>(id.xy) + 0.5) / u.sizes.zw;
        let d = view_ray(uv);
        let front = textureLoad(cloud_depth_in, p, 0).r;
        let reach = min(front, u.extent.z) * 1000.0;
        let clip = u.prev_vp * vec4<f32>(d * reach, 1.0);
        if clip.w > 0.0 {
            let prev = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            if all(prev >= vec2<f32>(0.0)) && all(prev <= vec2<f32>(1.0)) {
                let history = clamp(textureSampleLevel(history_in, clamp_samp, prev, 0.0), lo, hi);
                out = mix(history, current, 0.1);
            }
        }
    }
    textureStore(history_out, p, out);
}

@compute @workgroup_size(8, 8)
fn composite(@builtin(global_invocation_id) id: vec3<u32>) {
    let out_size = vec2<u32>(u.output.xy);
    if any(id.xy >= out_size) { return; }
    var colour = textureLoad(colour_in, vec2<i32>(id.xy), 0);
    let uv = (vec2<f32>(id.xy) + 0.5) / u.output.xy;
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let dp = min(vec2<i32>(uv * u.sizes.xy), last);
    let depth = textureLoad(scene_depth, dp, 0);
    let sky = depth <= 0.0;
    var distance_km = 1e9;
    // Below the horizon the background already carries its haze (shade.wgsl
    // background_sky).
    if !sky {
        distance_km = scene_distance((vec2<f32>(dp) + 0.5) / u.sizes.xy, depth) * 0.001;
        if u.output.w > 0.5 {
            let ap = aerial(uv, distance_km);
            colour = vec4<f32>(colour.rgb * ap.a + ap.rgb, colour.a);
        }
    }
    if clouds_on() {
        // Bilinear over the half-resolution clouds, a tap counting as clear
        // where this pixel's surface is in front of the tap's cloud.
        let hp = uv * u.sizes.zw - 0.5;
        let base = floor(hp);
        let f = hp - base;
        let half_last = vec2<i32>(u.sizes.zw) - 1;
        var cloud = vec4<f32>(0.0);
        var front = 0.0;
        var front_weight = 0.0;
        for (var k = 0; k < 4; k++) {
            let o = vec2<i32>(k & 1, k >> 1u);
            let q = clamp(vec2<i32>(base) + o, vec2<i32>(0), half_last);
            let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
            var s = textureLoad(history_in, q, 0);
            let c = textureLoad(cloud_depth_in, q, 0).r;
            if !sky && distance_km < c { s = vec4<f32>(0.0, 0.0, 0.0, 1.0); }
            cloud += s * w;
            if c < NO_CLOUD_KM * 0.5 { front += c * w; front_weight += w; }
        }
        let coverage = 1.0 - cloud.a;
        if coverage > 1e-4 {
            let at = select(u.extent.z, front / front_weight, front_weight > 0.0);
            let ap = aerial(uv, at);
            colour = vec4<f32>(colour.rgb * cloud.a + cloud.rgb * ap.a + ap.rgb * coverage, colour.a);
        }
    }
    textureStore(colour_out, vec2<i32>(id.xy), vec4<f32>(min(colour.rgb, vec3<f32>(65504.0)), colour.a));
}
