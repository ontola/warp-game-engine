// Opt-in in-scattering from local lights in a homogeneous medium
// (engine/docs/volumetric-light.md). Two passes after temporal
// reconstruction: `scatter` integrates every opted-in light along each
// half-resolution view ray, `composite` upsamples with depth weights and adds
// the result to a copy of the resolved HDR. Nothing here runs unless
// `Lighting::volumetric` is set and at least one light has a scatter weight.
//
// Integral per light, with closest approach tc and miss distance h,
// h'^2 = h^2 + source_radius^2: t = tc + h' tan(a) turns an inverse-square
// integrand into a constant (the closed atan form), t = tc + h' sinh(a) does
// the same for Growing Guns' 1/d floodlights; the remaining smooth factors
// (spot cone, range fade, phase, transmittance) use Gauss-Legendre in a.

struct ScatterLight { position_range: vec4<f32>, direction_outer: vec4<f32>, intensity_inner: vec4<f32>, source: vec4<f32> };
// source: x source radius^2, y falloff exponent, z 1 when the outer cone is
// narrow enough (< ~87 deg) to clip the ray to it, w unused.
struct Uniform {
    inverse_depth_vp: mat4x4<f32>,
    camera_near: vec4<f32>,
    // Depth (internal) width, height; half-resolution width, height.
    sizes: vec4<f32>,
    // Output width, height, unused, unused.
    output: vec4<f32>,
    // Density (scattering = extinction, 1/m), anisotropy g, intensity, light count.
    medium: vec4<f32>,
    tint: vec4<f32>,
    lights: array<ScatterLight, 32>,
};

@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var scatter_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var scatter_in: texture_2d<f32>;
@group(0) @binding(4) var colour_in: texture_2d<f32>;
@group(0) @binding(5) var colour_out: texture_storage_2d<rgba16float, write>;

const PI: f32 = 3.14159265;
const SAMPLES: u32 = 6u;
// Ray length for empty depth (sky); the light ranges bound the integral.
const SKY_DISTANCE: f32 = 10000.0;
// Stored linear view depth for sky pixels (half float range).
const SKY_DEPTH: f32 = 60000.0;

fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denom = max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4);
    return (1.0 - g2) / (4.0 * PI * denom * sqrt(denom));
}

// The ray interval inside the forward nappe of the outer cone, intersected
// with [t0, t1]; empty when x >= y. Only narrows the sampled interval: the
// cone factor is still evaluated per sample, so a conservative answer (the
// unclipped interval) is always correct.
fn clip_cone(o: vec3<f32>, v: vec3<f32>, light: ScatterLight, t0: f32, t1: f32) -> vec2<f32> {
    let axis = light.direction_outer.xyz;
    let c2 = light.direction_outer.w * light.direction_outer.w;
    let d0 = o - light.position_range.xyz;
    let va = dot(v, axis);
    let d0a = dot(d0, axis);
    let qa = va * va - c2;
    let qb = va * d0a - c2 * dot(d0, v);
    let qc = d0a * d0a - c2 * dot(d0, d0);
    if abs(qa) < 1e-4 { return vec2<f32>(t0, t1); }
    let disc = qb * qb - qa * qc;
    if disc < 0.0 {
        if qa < 0.0 { return vec2<f32>(1.0, 0.0); }
        return vec2<f32>(t0, t1);
    }
    let s = sqrt(disc);
    let r1 = (-qb - s) / qa;
    let r2 = (-qb + s) / qa;
    let lo = min(r1, r2);
    let hi = max(r1, r2);
    var span = vec2<f32>(-1e30, 1e30);
    if qa < 0.0 {
        // Inside the double cone between the roots: reject the backward nappe.
        if d0a + va * 0.5 * (lo + hi) < 0.0 { return vec2<f32>(1.0, 0.0); }
        span = vec2<f32>(lo, hi);
    } else if va > 0.0 {
        span = vec2<f32>(hi, 1e30);
    } else {
        span = vec2<f32>(-1e30, lo);
    }
    return vec2<f32>(max(span.x, t0), min(span.y, t1));
}

// In-scattered radiance from one light along o + v t, t in [0, tmax], before
// the medium's density, tint and intensity. Same irradiance model as
// `local_light_sample` (headlights.wgsl) — cone smoothstep, (1 - (d/R)^4)^2
// range fade — except that the spread is (d^2 + source^2)^(falloff / 2), which
// differs from max(d^2, source^2)^(falloff / 2) only inside the source radius
// and keeps the integrand smooth.
//
// Substitution, with r'^2 = (t - tc)^2 + h'^2:
//   falloff >= 1.75: t - tc = h' tan(a),  dt / r'^f = r'^(2-f) / h' da (exact atan form at 2);
//   otherwise:       t - tc = h' sinh(a), dt / r'^f = r'^(1-f) da      (exact asinh form at 1).
// The remaining factors are smooth in a and use 6-point Gauss-Legendre.
fn light_scatter(o: vec3<f32>, v: vec3<f32>, tmax: f32, light: ScatterLight, sigma: f32, g: f32) -> vec3<f32> {
    let p = light.position_range.xyz;
    let range = light.position_range.w;
    let rel = p - o;
    let tc = dot(rel, v);
    let h2 = max(dot(rel, rel) - tc * tc, 0.0);
    let r2 = range * range;
    if h2 >= r2 { return vec3<f32>(0.0); }
    let half_chord = sqrt(r2 - h2);
    var span = vec2<f32>(max(tc - half_chord, 0.0), min(tc + half_chord, tmax));
    if light.source.z > 0.5 { span = clip_cone(o, v, light, span.x, span.y); }
    if span.y <= span.x { return vec3<f32>(0.0); }
    let s2 = light.source.x;
    let falloff = light.source.y;
    let hp = sqrt(h2 + s2);
    let tangent = falloff >= 1.75;
    let x = (span - tc) / hp;
    let a = select(vec2<f32>(asinh(x.x), asinh(x.y)), vec2<f32>(atan(x.x), atan(x.y)), tangent);
    let exponent = select(0.5 - 0.5 * falloff, 1.0 - 0.5 * falloff, tangent);
    let jacobian = select(1.0, 1.0 / hp, tangent);
    let centre = 0.5 * (a.x + a.y);
    let half_width = 0.5 * (a.y - a.x);
    var nodes = array<f32, 6>(-0.9324695, -0.6612094, -0.2386192, 0.2386192, 0.6612094, 0.9324695);
    var weights = array<f32, 6>(0.1713245, 0.3607616, 0.4679139, 0.4679139, 0.3607616, 0.1713245);
    var acc = 0.0;
    for (var k = 0u; k < SAMPLES; k++) {
        let ak = centre + half_width * nodes[k];
        let t = tc + hp * select(sinh(ak), tan(ak), tangent);
        let off = o + v * t - p;
        let d2 = dot(off, off);
        let dist = sqrt(d2);
        let outward = off / max(dist, 1e-6);
        let cone = smoothstep(light.direction_outer.w, light.intensity_inner.w, dot(outward, light.direction_outer.xyz));
        let n = d2 / r2;
        let fade = max(0.0, 1.0 - n * n);
        var spread = 1.0;
        if exponent != 0.0 { spread = pow(d2 + s2, exponent); }
        // Light travels along `outward`; the camera receives it along -v.
        let phase = hg_phase(dot(outward, -v), g);
        // Approximate transmittance: camera to sample and lamp to sample.
        let transmittance = exp(-sigma * (t + dist));
        acc += weights[k] * cone * fade * fade * phase * transmittance * spread;
    }
    return light.intensity_inner.rgb * (acc * half_width * jacobian);
}

fn view_depth(depth: f32) -> f32 {
    if depth <= 0.0 { return SKY_DEPTH; }
    return min(u.camera_near.w / depth, SKY_DEPTH);
}

@compute @workgroup_size(8, 8)
fn scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let half = vec2<u32>(u.sizes.zw);
    if any(id.xy >= half) { return; }
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let base = vec2<i32>(id.xy) * 2;
    // Checkerboard of the nearest and farthest depth of each 2x2 footprint,
    // so the upsample finds a matching neighbour on both sides of an edge.
    let farthest = ((id.x + id.y) & 1u) == 1u;
    var best = min(base, last);
    var best_depth = textureLoad(scene_depth, best, 0);
    for (var k = 1; k < 4; k++) {
        let q = min(base + vec2<i32>(k & 1, k >> 1u), last);
        let d = textureLoad(scene_depth, q, 0);
        // Reversed Z: a smaller depth is farther.
        if (farthest && d < best_depth) || (!farthest && d > best_depth) { best = q; best_depth = d; }
    }
    let uv = (vec2<f32>(best) + 0.5) / u.sizes.xy;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let origin = u.camera_near.xyz;
    let near = u.inverse_depth_vp * vec4<f32>(ndc, 1.0, 1.0);
    let v = normalize(near.xyz / near.w - origin);
    var tmax = SKY_DISTANCE;
    if best_depth > 0.0 {
        let hit = u.inverse_depth_vp * vec4<f32>(ndc, best_depth, 1.0);
        tmax = length(hit.xyz / hit.w - origin);
    }
    var sum = vec3<f32>(0.0);
    let count = min(u32(u.medium.w), 32u);
    for (var i = 0u; i < count; i++) {
        sum += light_scatter(origin, v, tmax, u.lights[i], u.medium.x, u.medium.y);
    }
    let colour = sum * u.tint.rgb * (u.medium.x * u.medium.z);
    textureStore(scatter_out, vec2<i32>(id.xy), vec4<f32>(colour, view_depth(best_depth)));
}

@compute @workgroup_size(8, 8)
fn composite(@builtin(global_invocation_id) id: vec3<u32>) {
    let out_size = vec2<u32>(u.output.xy);
    if any(id.xy >= out_size) { return; }
    let colour = textureLoad(colour_in, vec2<i32>(id.xy), 0);
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let centre = vec2<f32>(id.xy) + 0.5;
    let dp = min(vec2<i32>(centre * u.sizes.xy / u.output.xy), last);
    let z = view_depth(textureLoad(scene_depth, dp, 0));
    let hp = centre * u.sizes.zw / u.output.xy - 0.5;
    let base = floor(hp);
    let f = hp - base;
    let half_last = vec2<i32>(u.sizes.zw) - 1;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    var nearest = vec3<f32>(0.0);
    var nearest_gap = 1e30;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let q = clamp(vec2<i32>(base) + o, vec2<i32>(0), half_last);
        let s = textureLoad(scatter_in, q, 0);
        let bilinear = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        // Relative depth difference: 5% of the nearer depth is one unit.
        let gap = abs(s.a - z) / max(0.05 * min(s.a, z), 0.05);
        let w = bilinear * exp(-gap * gap);
        sum += s.rgb * w;
        weight += w;
        if gap < nearest_gap { nearest_gap = gap; nearest = s.rgb; }
    }
    var scattered = nearest;
    if weight > 0.05 { scattered = sum / weight; }
    textureStore(colour_out, vec2<i32>(id.xy), vec4<f32>(colour.rgb + scattered, colour.a));
}
