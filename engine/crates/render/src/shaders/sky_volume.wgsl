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
    let near = u.inv_vp * vec4<f32>(ndc, 1.0, 1.0);
    return normalize(near.xyz / near.w - u.camera.xyz);
}

fn scene_distance(uv: vec2<f32>, depth: f32) -> f32 {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let hit = u.inv_depth_vp * vec4<f32>(ndc, depth, 1.0);
    return length(hit.xyz / hit.w - u.camera.xyz);
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

// Near and far ray distances (m) inside the cloud shell, first segment only.
fn shell_segment(o: vec3<f32>, d: vec3<f32>) -> vec2<f32> {
    let r = planet_radius() * 1000.0;
    let inner = r + u.cloud.y;
    let outer = inner + u.cloud.z;
    let b = dot(o, d);
    let c2 = dot(o, o);
    let disc_outer = b * b - (c2 - outer * outer);
    if disc_outer < 0.0 { return vec2<f32>(-1.0); }
    let so = sqrt(disc_outer);
    var t0 = max(-b - so, 0.0);
    var t1 = -b + so;
    if t1 <= 0.0 { return vec2<f32>(-1.0); }
    let disc_inner = b * b - (c2 - inner * inner);
    let radius = sqrt(c2);
    if disc_inner >= 0.0 {
        let si = sqrt(disc_inner);
        let i0 = -b - si;
        let i1 = -b + si;
        if radius < inner {
            // Below the layer: from leaving the inner sphere to the outer one.
            t0 = max(i1, 0.0);
        } else if i0 > 0.0 {
            // In or above the layer, looking down through it: stop at the base.
            t1 = min(t1, i0);
        }
    }
    // The planet hides what is below the horizon.
    let disc_ground = b * b - (c2 - r * r);
    if disc_ground >= 0.0 && -b - sqrt(disc_ground) > 0.0 { t1 = min(t1, -b - sqrt(disc_ground)); }
    if t1 <= t0 { return vec2<f32>(-1.0); }
    return vec2<f32>(t0, t1);
}

fn height_fraction(p: vec3<f32>) -> f32 {
    return clamp((length(p) - planet_radius() * 1000.0 - u.cloud.y) / u.cloud.z, 0.0, 1.0);
}

// Extinction per m at planet position p (m), world position world (m).
fn cloud_density(p: vec3<f32>, world: vec3<f32>, cheap: bool) -> f32 {
    let h = height_fraction(p);
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

// A fixed random offset per pixel, advanced by the golden ratio each frame
// (an R1 sequence): stratified over time, so the history converges in a few
// frames instead of shimmering, and no structure to beat against the steps.
fn interleaved_noise(p: vec2<f32>, frame: f32) -> f32 {
    return fract(pcg3(vec3<u32>(vec2<u32>(p), 0u)).x + (frame % 1024.0) * 0.61803398875);
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
    let o = vec3<f32>(0.0, planet_radius() * 1000.0 + u.camera.y, 0.0);
    let seg = shell_segment(o, d);
    var result = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var front = NO_CLOUD_KM;
    if seg.y > 0.0 && seg.x < tmax {
        let t0 = seg.x;
        let t1 = min(seg.y, tmax);
        let steps = u32(u.cloud_shape.z);
        let dt = (t1 - t0) / f32(steps);
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
            let t = t0 + (f32(i) + jitter) * dt;
            let p = o + d * t;
            let world = u.camera.xyz + d * t;
            let sigma = cloud_density(p, world, false);
            if sigma <= 0.0 { continue; }
            // Sun through the cloud: growing steps towards the sun.
            var tau = 0.0;
            var reach = 0.0;
            var step_len = u.cloud.z / f32(1u << light_steps) * 2.0;
            for (var j = 0u; j < light_steps; j++) {
                // Jittered like the view march: fixed sample points band the
                // self-shadow into steps under a big cloud.
                let q = reach + step_len * jitter;
                tau += cloud_density(p + l * q, world + l * q, true) * step_len;
                reach += step_len;
                step_len *= 2.0;
            }
            let h = height_fraction(p);
            let pk = p * 0.001;
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
            weighted += (before - transmittance) * t;
            weight += before - transmittance;
        }
        result = vec4<f32>(radiance, transmittance);
        if weight > 1e-4 { front = weighted / weight * 0.001; }
        if 1.0 - transmittance < 1e-3 { front = NO_CLOUD_KM; }
    }
    textureStore(cloud_out, vec2<i32>(id.xy), result);
    textureStore(cloud_depth_out, vec2<i32>(id.xy), vec4<f32>(front, 0.0, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn cloud_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(u.sizes.zw);
    if any(id.xy >= size) { return; }
    let p = vec2<i32>(id.xy);
    let current = textureLoad(cloud_in, p, 0);
    var lo = current;
    var hi = current;
    let last = vec2<i32>(size) - 1;
    for (var k = 0; k < 9; k++) {
        if k == 4 { continue; }
        let q = clamp(p + vec2<i32>(k % 3 - 1, k / 3 - 1), vec2<i32>(0), last);
        let s = textureLoad(cloud_in, q, 0);
        lo = min(lo, s);
        hi = max(hi, s);
    }
    var out = current;
    if u.output.z > 0.5 {
        let uv = (vec2<f32>(id.xy) + 0.5) / u.sizes.zw;
        let d = view_ray(uv);
        let front = textureLoad(cloud_depth_in, p, 0).r;
        let reach = min(front, u.extent.z) * 1000.0;
        let clip = u.prev_vp * vec4<f32>(u.camera.xyz + d * reach, 1.0);
        if clip.w > 0.0 {
            let prev = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
            if all(prev >= vec2<f32>(0.0)) && all(prev <= vec2<f32>(1.0)) {
                let history = clamp(textureSampleLevel(history_in, clamp_samp, prev, 0.0), lo, hi);
                out = mix(history, current, 0.12);
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
    if sky && u.output.w > 0.5 {
        // Below the horizon the background is the bake's ground seen from
        // ground level: past the end of the terrain, from altitude, fade it
        // with the air up to where the ray meets the planet.
        let d = view_ray(uv);
        let o = camera_planet();
        let b = dot(o, d);
        let r = planet_radius();
        let disc = b * b - (dot(o, o) - r * r);
        if b < 0.0 && disc >= 0.0 {
            let ap = aerial(uv, -b - sqrt(disc));
            colour = vec4<f32>(colour.rgb * ap.a + ap.rgb, colour.a);
        }
    }
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
