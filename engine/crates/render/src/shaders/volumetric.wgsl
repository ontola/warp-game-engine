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
    // Scattering tint; w: extinction (how much the medium dims the scene).
    tint: vec4<f32>,
    // Ambient in-scatter radiance (converged); w: medium length, 0 unbounded.
    ambient: vec4<f32>,
    lights: array<ScatterLight, 32>,
    // Bounded, shadowed mode (`Volumetric` with bounds): x occluder boxes,
    // y bounds (0: the analytic unbounded mode), z steps per light, w unused.
    march: vec4<f32>,
    // Sun towards (xyz) and its scattered irradiance (rgb, lux times the
    // medium's sun weight); w unused.
    sun_dir: vec4<f32>,
    sun_e: vec4<f32>,
    // Up to 8 bounds as (min, max) pairs: the medium fills only these boxes.
    bounds: array<vec4<f32>, 16>,
};
// An occluder: the rows of its world-to-unit-box transform (the box is
// [-0.5, 0.5]^3 in its own space).
struct Occluder { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, pad: vec4<f32> };

@group(0) @binding(0) var<uniform> u: Uniform;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var scatter_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var scatter_in: texture_2d<f32>;
@group(0) @binding(4) var colour_in: texture_2d<f32>;
@group(0) @binding(5) var colour_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var<storage, read> occluders: array<Occluder>;
// Light cookies and profiles (light_shapes.rs, as clustered_lights.wgsl).
@group(0) @binding(7) var light_shapes: texture_2d_array<f32>;
@group(0) @binding(8) var shape_sampler: sampler;

// A light's cookie and angular profile towards `outward` (source.w: cookie
// in bits 3..6, profile in bits 7..14).
fn light_shape(light: ScatterLight, outward: vec3<f32>) -> f32 {
    let packed = u32(light.source.w);
    if packed < 8u { return 1.0; }
    let cookie = (packed >> 3u) & 15u;
    let profile = (packed >> 7u) & 255u;
    let axis = light.direction_outer.xyz;
    let c = dot(outward, axis);
    var k = 1.0;
    if profile > 0u {
        k *= textureSampleLevel(light_shapes, shape_sampler, vec2<f32>(acos(clamp(c, -1.0, 1.0)) * 0.31830989, (f32(profile - 1u) + 0.5) / 256.0), 15, 0.0).r;
    }
    if cookie > 0u {
        let oc = light.direction_outer.w;
        if c <= 0.05 || oc <= 0.05 { return 0.0; }
        let t1 = normalize(select(cross(axis, vec3<f32>(0.0, 1.0, 0.0)), cross(axis, vec3<f32>(1.0, 0.0, 0.0)), abs(axis.y) > 0.9));
        let t2 = cross(axis, t1);
        let tan_outer = sqrt(max(1.0 - oc * oc, 0.0)) / oc;
        let uv = vec2<f32>(dot(outward, t1), dot(outward, t2)) / (c * tan_outer) * 0.5 + 0.5;
        k *= textureSampleLevel(light_shapes, shape_sampler, uv, i32(cookie - 1u), 0.0).r;
    }
    return k;
}

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

// ---- Bounded, shadowed mode ----
// The medium fills a few boxes (rooms); every light is ray-marched through
// them with a visibility test towards the light against the occluder boxes
// (walls, furniture, a door), and the sun the same way, so beams, a door's
// shaft and the shadows in them are real and nothing glows through a wall.
// The march offsets its steps by a fixed 4x4 Bayer pattern per pixel; the
// composite gathers 4x4 half-resolution pixels, which averages it away.

// Whether the segment a -> b passes through any occluder of bound `bound`
// (its list: start and count in the bound's w components).
fn blocked(a: vec3<f32>, b: vec3<f32>, bound: u32) -> bool {
    let start = u32(u.bounds[2u * bound].w);
    let end = start + u32(u.bounds[2u * bound + 1u].w);
    for (var i = start; i < end; i++) {
        let o = occluders[i];
        let la = vec3<f32>(dot(o.r0.xyz, a) + o.r0.w, dot(o.r1.xyz, a) + o.r1.w, dot(o.r2.xyz, a) + o.r2.w);
        let lb = vec3<f32>(dot(o.r0.xyz, b) + o.r0.w, dot(o.r1.xyz, b) + o.r1.w, dot(o.r2.xyz, b) + o.r2.w);
        let d = lb - la;
        let inv = 1.0 / select(d, vec3<f32>(1e-12), abs(d) < vec3<f32>(1e-12));
        let t0 = (vec3<f32>(-0.5) - la) * inv;
        let t1 = (vec3<f32>(0.5) - la) * inv;
        let enter = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), max(min(t0.z, t1.z), 0.0));
        let exit = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), min(max(t0.z, t1.z), 1.0));
        if exit > enter { return true; }
    }
    return false;
}

// The bound p is in, or -1.
fn bound_of(p: vec3<f32>) -> i32 {
    let count = u32(u.march.y);
    for (var i = 0u; i < count; i++) {
        if all(p >= u.bounds[2u * i].xyz) && all(p <= u.bounds[2u * i + 1u].xyz) { return i32(i); }
    }
    return -1;
}

// The view ray's span over all bounds, within [0, tmax].
fn bounds_span(o: vec3<f32>, v: vec3<f32>, tmax: f32) -> vec2<f32> {
    let inv = 1.0 / select(v, vec3<f32>(1e-12), abs(v) < vec3<f32>(1e-12));
    var span = vec2<f32>(1e30, -1e30);
    let count = u32(u.march.y);
    for (var i = 0u; i < count; i++) {
        let t0 = (u.bounds[2u * i].xyz - o) * inv;
        let t1 = (u.bounds[2u * i + 1u].xyz - o) * inv;
        let enter = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), max(min(t0.z, t1.z), 0.0));
        let exit = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), min(max(t0.z, t1.z), tmax));
        if exit > enter { span = vec2<f32>(min(span.x, enter), max(span.y, exit)); }
    }
    return span;
}

fn bayer4(p: vec2<u32>) -> f32 {
    let m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    return (m[(p.y & 3u) * 4u + (p.x & 3u)] + 0.5) / 16.0;
}

// In-scattered radiance along the view ray inside the bounds, every light
// and the sun, occluded; before density, tint and intensity.
fn marched_scatter(o: vec3<f32>, v: vec3<f32>, tmax: f32, jitter: f32) -> vec3<f32> {
    let span = bounds_span(o, v, tmax);
    if span.y <= span.x { return vec3<f32>(0.0); }
    let sigma = u.medium.x;
    let g = u.medium.y;
    let steps = max(u32(u.march.z), 1u);
    let dt = (span.y - span.x) / f32(steps);
    let count = min(u32(u.medium.w), 32u);
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < steps; k++) {
        let t = span.x + (f32(k) + jitter) * dt;
        let p = o + v * t;
        let bi = bound_of(p);
        if bi < 0 { continue; }
        let bound = u32(bi);
        let to_camera = exp(-sigma * t);
        for (var i = 0u; i < count; i++) {
            let light = u.lights[i];
            let off = p - light.position_range.xyz;
            let d2 = dot(off, off);
            let r2 = light.position_range.w * light.position_range.w;
            if d2 >= r2 { continue; }
            let dist = sqrt(d2);
            let outward = off / max(dist, 1e-6);
            let cone = smoothstep(light.direction_outer.w, light.intensity_inner.w, dot(outward, light.direction_outer.xyz)) * light_shape(light, outward);
            if cone <= 0.0 { continue; }
            let n = d2 / r2;
            let fade = max(0.0, 1.0 - n * n);
            let spread = pow(d2 + light.source.x, -0.5 * light.source.y);
            if blocked(p, light.position_range.xyz + outward * sqrt(light.source.x), bound) { continue; }
            acc += light.intensity_inner.rgb * (cone * fade * fade * spread * hg_phase(dot(outward, -v), g) * exp(-sigma * dist) * to_camera * dt);
        }
        if u.sun_dir.w > 0.5 && !blocked(p, p + u.sun_dir.xyz * 200.0, bound) {
            acc += u.sun_e.rgb * (hg_phase(dot(u.sun_dir.xyz, -v), g) * to_camera * dt);
        }
    }
    return acc;
}

fn view_depth(depth: f32) -> f32 {
    if depth <= 0.0 { return SKY_DEPTH; }
    return min(u.camera_near.w / depth, SKY_DEPTH);
}

@compute @workgroup_size(8, 8)
fn scatter(@builtin(global_invocation_id) id: vec3<u32>) {
    let half = vec2<u32>(u.sizes.zw);
    if any(id.xy >= half) { return; }
    // Bounded mode marches every second pixel each way (quarter
    // resolution); the composite reads only those.
    if u.march.y > 0.5 && ((id.x | id.y) & 1u) != 0u { return; }
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
    if u.ambient.w > 0.0 { tmax = min(tmax, u.ambient.w); }
    var sum = vec3<f32>(0.0);
    var ambient_reach = tmax;
    if u.march.y > 0.5 {
        sum = marched_scatter(origin, v, tmax, bayer4(id.xy / 2u));
        let span = bounds_span(origin, v, tmax);
        ambient_reach = max(span.y - span.x, 0.0);
    } else {
        let count = min(u32(u.medium.w), 32u);
        for (var i = 0u; i < count; i++) {
            sum += light_scatter(origin, v, tmax, u.lights[i], u.medium.x, u.medium.y);
        }
    }
    let colour = sum * u.tint.rgb * (u.medium.x * u.medium.z) + u.ambient.rgb * (1.0 - exp(-u.medium.x * ambient_reach));
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
    // Bounded mode: 4x4 of the marched (even) half-resolution pixels, a
    // tent over the Bayer period of the march; otherwise the 2x2 bilinear
    // footprint.
    let wide = u.march.y > 0.5;
    let taps = select(4, 16, wide);
    let qp = centre * u.sizes.zw / u.output.xy * 0.5 - 0.5;
    let qbase = floor(qp);
    let qf = qp - qbase;
    for (var k = 0; k < taps; k++) {
        var o = vec2<i32>(k & 1, k >> 1u);
        var bilinear = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
        var q = clamp(vec2<i32>(base) + o, vec2<i32>(0), half_last);
        if wide {
            o = vec2<i32>(k & 3, k >> 2u) - 1;
            let d = abs(vec2<f32>(o) - qf);
            bilinear = max(2.0 - d.x, 0.0) * max(2.0 - d.y, 0.0);
            q = clamp((vec2<i32>(qbase) + o) * 2, vec2<i32>(0), half_last - (half_last & vec2<i32>(1)));
        }
        let s = textureLoad(scatter_in, q, 0);
        // Relative depth difference: 5% of the nearer depth is one unit.
        let gap = abs(s.a - z) / max(0.05 * min(s.a, z), 0.05);
        let w = bilinear * exp(-gap * gap);
        sum += s.rgb * w;
        weight += w;
        if gap < nearest_gap { nearest_gap = gap; nearest = s.rgb; }
    }
    var scattered = nearest;
    if weight > 0.05 { scattered = sum / weight; }
    // Extinction of the scene behind the medium (0 by default: only adds).
    var reach = z;
    if u.ambient.w > 0.0 { reach = min(reach, u.ambient.w); }
    var behind = colour.rgb;
    if u.tint.w > 0.0 { behind *= exp(-u.tint.w * u.medium.x * reach); }
    textureStore(colour_out, vec2<i32>(id.xy), vec4<f32>(behind + scattered, colour.a));
}
