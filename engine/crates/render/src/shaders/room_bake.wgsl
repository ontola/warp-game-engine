// The room bake on the GPU (room_bake_gpu.rs): the same transport as the
// CPU bake in room_bake.rs, step for step (ray sets, per-probe rotations,
// trilinear gathers with exact visibility, neighbour smoothing), so the two
// agree to float rounding. Seven storage buffers: arenas with accessors.

struct U {
    // xyz grid origin, w cell.
    origin: vec4<f32>,
    // xyz dims, w probe count.
    dims: vec4<u32>,
    // xyz towards the sun (unit), w reflectance.
    sun: vec4<f32>,
    // x sky rays, y bounce rays, z first triangle in `geo`, w first rotation in `dirs`.
    counts: vec4<u32>,
    // x the light baked (bounce_lamp), y lights, z 1: smoothing includes the
    // direct sky (x), w first bounce direction in `dirs`.
    mode: vec4<u32>,
};

@group(0) @binding(0) var<uniform> u: U;
// BVH nodes (two vec4 each: lo + first, hi + count), then triangles (three
// corners each) from u.counts.z.
@group(0) @binding(1) var<storage, read> geo: array<vec4<f32>>;
// Sky directions (u.counts.x), bounce directions (from u.mode.w), probe
// rotations as quaternions (from u.counts.w).
@group(0) @binding(2) var<storage, read> dirs: array<vec4<f32>>;
// Probe positions; w 1 for a valid probe.
@group(0) @binding(3) var<storage, read_write> pos: array<vec4<f32>>;
// Six axes per probe (rgb), last pass's and this one's.
@group(0) @binding(4) var<storage, read> prev: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> out: array<vec4<f32>>;
// Per probe the bits of the 26 neighbours it sees.
@group(0) @binding(6) var<storage, read_write> seen: array<u32>;
// Four vec4 per light: position + range, direction + falloff, intensity
// (candela) + source radius, inner and outer cone cosines.
@group(0) @binding(7) var<storage, read> lights: array<vec4<f32>>;
// Per probe and bounce ray (`prepare`): which corners, the sun and which
// lights its hit sees.
@group(0) @binding(8) var<storage, read_write> masks: array<u32>;

const AXES = array<vec3<f32>, 6>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0),
    vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0));
const NO_HIT: f32 = -1.0;

fn probe_count() -> u32 { return u.dims.w; }
fn grid_position(i: u32) -> vec3<f32> {
    let x = i % u.dims.x;
    let y = (i / u.dims.x) % u.dims.y;
    let z = i / (u.dims.x * u.dims.y);
    return u.origin.xyz + vec3<f32>(f32(x), f32(y), f32(z)) * u.origin.w;
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn slab(o: vec3<f32>, inv: vec3<f32>, lo: vec3<f32>, hi: vec3<f32>, far: f32) -> bool {
    let a = (lo - o) * inv;
    let b = (hi - o) * inv;
    let lo_t = min(a, b);
    let hi_t = max(a, b);
    let enter = max(max(max(lo_t.x, lo_t.y), lo_t.z), 0.0);
    let exit = min(min(min(hi_t.x, hi_t.y), hi_t.z), far);
    return exit >= enter;
}

// Möller-Trumbore, as room_bake.rs `intersect`; NO_HIT when it misses.
fn intersect(o: vec3<f32>, d: vec3<f32>, p0: vec3<f32>, p1: vec3<f32>, p2: vec3<f32>) -> f32 {
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let p = cross(d, e2);
    let det = dot(e1, p);
    if abs(det) < 1e-10 { return NO_HIT; }
    let inv = 1.0 / det;
    let s = o - p0;
    let uu = dot(s, p) * inv;
    if uu < 0.0 || uu > 1.0 { return NO_HIT; }
    let q = cross(s, e1);
    let v = dot(d, q) * inv;
    if v < 0.0 || uu + v > 1.0 { return NO_HIT; }
    return dot(e2, q) * inv;
}

// The nearest hit along unit d from o, at least 2 cm out: xyz the
// triangle's normal, w the distance (NO_HIT: none). As `Occluders::hit`.
fn hit(o: vec3<f32>, d: vec3<f32>) -> vec4<f32> {
    if u.counts.z == 0u { return vec4<f32>(0.0, 0.0, 0.0, NO_HIT); }
    let inv = 1.0 / d;
    var best = vec4<f32>(0.0, 0.0, 0.0, NO_HIT);
    var stack: array<u32, 64>;
    var top = 1u;
    stack[0] = 0u;
    while top > 0u {
        top -= 1u;
        let a = geo[stack[top] * 2u];
        let b = geo[stack[top] * 2u + 1u];
        let far = select(1e30, best.w, best.w >= 0.0);
        if !slab(o, inv, a.xyz, b.xyz, far) { continue; }
        let first = bitcast<u32>(a.w);
        let count = bitcast<u32>(b.w);
        if count > 0u {
            for (var k = first; k < first + count; k++) {
                let t = u.counts.z + k * 3u;
                let p0 = geo[t].xyz;
                let p1 = geo[t + 1u].xyz;
                let p2 = geo[t + 2u].xyz;
                let dist = intersect(o, d, p0, p1, p2);
                let limit = select(1e30, best.w, best.w >= 0.0);
                if dist > 0.02 && dist < limit { best = vec4<f32>(normalize(cross(p1 - p0, p2 - p0)), dist); }
            }
        } else if top + 2u <= 64u {
            stack[top] = first;
            stack[top + 1u] = first + 1u;
            top += 2u;
        }
    }
    return best;
}

// Whether anything lies along unit d from o between 2 cm and `far`: the
// same answer as `hit` nearer than far, stopping at the first triangle.
fn blocked(o: vec3<f32>, d: vec3<f32>, far: f32) -> bool {
    if u.counts.z == 0u { return false; }
    let inv = 1.0 / d;
    var stack: array<u32, 64>;
    var top = 1u;
    stack[0] = 0u;
    while top > 0u {
        top -= 1u;
        let a = geo[stack[top] * 2u];
        let b = geo[stack[top] * 2u + 1u];
        if !slab(o, inv, a.xyz, b.xyz, far) { continue; }
        let first = bitcast<u32>(a.w);
        let count = bitcast<u32>(b.w);
        if count > 0u {
            for (var k = first; k < first + count; k++) {
                let t = u.counts.z + k * 3u;
                let dist = intersect(o, d, geo[t].xyz, geo[t + 1u].xyz, geo[t + 2u].xyz);
                if dist > 0.02 && dist < far { return true; }
            }
        } else if top + 2u <= 64u {
            stack[top] = first;
            stack[top + 1u] = first + 1u;
            top += 2u;
        }
    }
    return false;
}

fn irradiance(i: u32, n: vec3<f32>) -> vec3<f32> {
    let n2 = n * n;
    let x = prev[i * 6u + select(1u, 0u, n.x >= 0.0)].xyz;
    let y = prev[i * 6u + select(3u, 2u, n.y >= 0.0)].xyz;
    let z = prev[i * 6u + select(5u, 4u, n.z >= 0.0)].xyz;
    return (x * n2.x + y * n2.y + z * n2.z) / max(n2.x + n2.y + n2.z, 1e-12);
}

// The previous pass's irradiance at surface point p facing n: trilinear over
// the valid probes in front of it that see it (room_bake.rs `corners` and
// `gather`).
// `seen`: bit c set where corner c sees p (`corner_mask`).
fn gather(p: vec3<f32>, n: vec3<f32>, seen_by: u32) -> vec3<f32> {
    let dims = vec3<f32>(u.dims.xyz);
    let maxc = dims - 1.0;
    let f = clamp((p - u.origin.xyz) / u.origin.w, vec3<f32>(0.0), maxc);
    let base = min(floor(f), maxc - 1.0);
    let t = f - base;
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var c = 0u; c < 8u; c++) {
        let o = vec3<u32>(c & 1u, (c >> 1u) & 1u, (c >> 2u) & 1u);
        let i = u32(base.x) + o.x + (u32(base.y) + o.y) * u.dims.x + (u32(base.z) + o.z) * u.dims.x * u.dims.y;
        let q = pos[i];
        if q.w < 0.5 || (seen_by & (1u << c)) == 0u { continue; }
        let to = q.xyz - p;
        let d = length(to);
        let facing = clamp(dot(to, n) / max(d, 1e-4) * 2.0 + 1.0, 0.0, 1.0);
        let w = select(1.0 - t.x, t.x, o.x == 1u) * select(1.0 - t.y, t.y, o.y == 1u) * select(1.0 - t.z, t.z, o.z == 1u)
            * (facing * facing + 1e-4);
        sum += irradiance(i, n) * w;
        weight += w;
    }
    return select(vec3<f32>(0.0), sum / weight, weight > 1e-6);
}

// Which of the eight probes around p see it (bit per corner, as `gather`
// numbers them), valid or not: the same for every pass, traced once.
fn corner_mask(p: vec3<f32>) -> u32 {
    let maxc = vec3<f32>(u.dims.xyz) - 1.0;
    let base = min(floor(clamp((p - u.origin.xyz) / u.origin.w, vec3<f32>(0.0), maxc)), maxc - 1.0);
    var bits = 0u;
    for (var c = 0u; c < 8u; c++) {
        let o = vec3<u32>(c & 1u, (c >> 1u) & 1u, (c >> 2u) & 1u);
        let i = u32(base.x) + o.x + (u32(base.y) + o.y) * u.dims.x + (u32(base.z) + o.z) * u.dims.x * u.dims.y;
        let to = pos[i].xyz - p;
        let d = length(to);
        if !(d > 1e-4 && blocked(p, to / d, d)) { bits |= 1u << c; }
    }
    return bits;
}

// A local light's irradiance at p: towards it (xyz) and its value, or zero
// (crate::sprites::light_at).
fn light_at(l: u32, p: vec3<f32>, to: ptr<function, vec3<f32>>) -> vec3<f32> {
    let a = lights[l * 4u];
    let b = lights[l * 4u + 1u];
    let c = lights[l * 4u + 2u];
    let e = lights[l * 4u + 3u];
    let offset = p - a.xyz;
    let d2 = dot(offset, offset);
    let range = a.w;
    if d2 >= range * range || d2 < 1e-10 { return vec3<f32>(0.0); }
    let outward = offset / sqrt(d2);
    let dl = length(b.xyz);
    let dir = select(vec3<f32>(0.0), b.xyz / dl, dl > 0.0);
    let t = clamp((dot(outward, dir) - e.y) / max(e.x - e.y, 1e-6), 0.0, 1.0);
    let cone = t * t * (3.0 - 2.0 * t);
    if cone <= 0.0 { return vec3<f32>(0.0); }
    let normalized = d2 / (range * range);
    let fade = max(1.0 - normalized * normalized, 0.0);
    let near = c.w * c.w;
    var spread = max(d2, near);
    if !(b.w == 2.0 || b.w == 0.0) { spread = pow(spread, b.w * 0.5); }
    *to = -outward;
    return c.xyz * (cone * fade * fade / spread);
}

fn probe_index(wid: vec3<u32>) -> u32 { return wid.x + wid.y * 65535u; }

var<workgroup> red: array<array<f32, 64>, 24>;

fn reduce(li: u32, values: u32) {
    for (var s = 32u; s > 0u; s >>= 1u) {
        if li < s { for (var k = 0u; k < values; k++) { red[k][li] += red[k][li + s]; } }
        workgroupBarrier();
    }
}

// Direct sky per axis and whether the probe is inside something (a quarter
// of its sky rays see a back face): `bake`'s first step.
@compute @workgroup_size(64)
fn sky_direct(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = probe_index(wid);
    let inside = i >= probe_count();
    let o = pos[min(i, probe_count() - 1u)].xyz;
    let q = dirs[u.counts.w + min(i, probe_count() - 1u)];
    for (var k = 0u; k < 13u; k++) { red[k][li] = 0.0; }
    if !inside {
        for (var r = li; r < u.counts.x; r += 64u) {
            let d = rotate(q, dirs[r].xyz);
            let h = hit(o, d);
            if h.w >= 0.0 && dot(h.xyz, d) > 0.0 { red[12][li] += 1.0; }
            for (var a = 0u; a < 6u; a++) {
                let c = max(dot(AXES[a], d), 0.0);
                red[6u + a][li] += c;
                if h.w < 0.0 { red[a][li] += c; }
            }
        }
    }
    workgroupBarrier();
    reduce(li, 13u);
    if li == 0u && !inside {
        let ok = red[12][0] * 4.0 < f32(u.counts.x);
        pos[i].w = select(0.0, 1.0, ok);
        for (var a = 0u; a < 6u; a++) {
            out[i * 6u + a] = vec4<f32>(select(0.0, red[a][0] / max(red[6u + a][0], 1e-6), ok), 0.0, 0.0, 0.0);
        }
    }
}

// The neighbours each valid probe sees directly (room_bake.rs
// `visible_neighbours`), one bit each in x, y, z loop order.
@compute @workgroup_size(64)
fn neighbours(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    var bits = 0u;
    if pos[i].w > 0.5 {
        let x = i32(i % u.dims.x);
        let y = i32((i / u.dims.x) % u.dims.y);
        let z = i32(i / (u.dims.x * u.dims.y));
        let p = pos[i].xyz;
        var bit = 0u;
        for (var ox = -1; ox <= 1; ox++) { for (var oy = -1; oy <= 1; oy++) { for (var oz = -1; oz <= 1; oz++) {
            if ox == 0 && oy == 0 && oz == 0 { continue; }
            let b = bit;
            bit += 1u;
            let n = vec3<i32>(x + ox, y + oy, z + oz);
            if any(n < vec3<i32>(0)) || any(n >= vec3<i32>(u.dims.xyz)) { continue; }
            let j = u32(n.x) + u32(n.y) * u.dims.x + u32(n.z) * u.dims.x * u.dims.y;
            if pos[j].w < 0.5 { continue; }
            let d = pos[j].xyz - p;
            if blocked(p, normalize(d), length(d)) { continue; }
            bits |= 1u << b;
        }}}
    }
    seen[i] = bits;
}

// Per bounce ray, what stays the same through every pass and light (the
// shadow rays that dominated a pass): which corners see its hit (bits 0-7),
// whether the sun does (bit 8) and light l (bit 9 + l).
@compute @workgroup_size(64)
fn prepare(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = probe_index(wid);
    if i >= probe_count() { return; }
    let o = pos[i].xyz;
    let q = dirs[u.counts.w + i];
    for (var r = li; r < u.counts.y; r += 64u) {
        let d = rotate(q, dirs[u.mode.w + r].xyz);
        let h = hit(o, d);
        var mask = 0u;
        if h.w >= 0.0 {
            let n = select(h.xyz, -h.xyz, dot(h.xyz, d) > 0.0);
            let at = o + d * h.w;
            mask = corner_mask(at + n * 0.15);
            let sun = u.sun.xyz;
            if sun.y > 0.0 && dot(n, sun) > 0.0 && !blocked(at + n * 0.03, sun, 1e30) { mask |= 256u; }
            for (var l = 0u; l < u.mode.y; l++) {
                var to = vec3<f32>(0.0);
                let direct = light_at(l, at, &to);
                if dot(n, to) <= 0.0 || all(direct <= vec3<f32>(0.0)) { continue; }
                let far = distance(lights[l * 4u].xyz, at);
                if !blocked(at + n * 0.03, to, far - max(lights[l * 4u + 2u].w, 0.05)) { mask |= 1u << (9u + l); }
            }
        }
        masks[i * u.counts.y + r] = mask;
    }
}

// One gather of bounced light: every bounce ray's hit lit by the previous
// pass's probes (and the sun, or light u.mode.x, directly), weighted per
// axis. Sky mode writes y (sky) and z (sun), keeping x; lamp mode rgb.
fn bounce(i: u32, li: u32, lamp: bool) {
    let valid = i < probe_count() && pos[i].w > 0.5;
    let j = min(i, probe_count() - 1u);
    let o = pos[j].xyz;
    let q = dirs[u.counts.w + j];
    for (var k = 0u; k < 24u; k++) { red[k][li] = 0.0; }
    if valid {
        for (var r = li; r < u.counts.y; r += 64u) {
            let d = rotate(q, dirs[u.mode.w + r].xyz);
            var c: array<f32, 6>;
            for (var a = 0u; a < 6u; a++) { c[a] = max(dot(AXES[a], d), 0.0); red[18u + a][li] += c[a]; }
            let h = hit(o, d);
            if h.w < 0.0 { continue; }
            let n = select(h.xyz, -h.xyz, dot(h.xyz, d) > 0.0);
            let at = o + d * h.w;
            let mask = masks[i * u.counts.y + r];
            let g = gather(at + n * 0.15, n, mask);
            if lamp {
                var to = vec3<f32>(0.0);
                var direct = light_at(u.mode.x, at, &to);
                let cosine = dot(n, to);
                if cosine <= 0.0 { direct = vec3<f32>(0.0); }
                if (mask & (1u << (9u + u.mode.x))) == 0u { direct = vec3<f32>(0.0); }
                let leaving = u.sun.w * (direct * max(cosine, 0.0) + g);
                for (var a = 0u; a < 6u; a++) {
                    red[a * 3u][li] += leaving.x * c[a];
                    red[a * 3u + 1u][li] += leaving.y * c[a];
                    red[a * 3u + 2u][li] += leaving.z * c[a];
                }
            } else {
                let sun = u.sun.xyz;
                let lit = max(dot(n, sun), 0.0);
                var sun_e = 0.0;
                if sun.y > 0.0 && lit > 0.0 && (mask & 256u) != 0u { sun_e = lit; }
                let s = g.x + g.y;
                let z = sun_e + g.z;
                for (var a = 0u; a < 6u; a++) {
                    red[a][li] += u.sun.w * s * c[a];
                    red[6u + a][li] += u.sun.w * z * c[a];
                }
            }
        }
    }
    workgroupBarrier();
    reduce(li, 24u);
    if li == 0u && i < probe_count() {
        for (var a = 0u; a < 6u; a++) {
            let p = prev[i * 6u + a];
            if !valid { out[i * 6u + a] = p; continue; }
            let den = max(red[18u + a][0], 1e-6);
            if lamp {
                out[i * 6u + a] = vec4<f32>(red[a * 3u][0], red[a * 3u + 1u][0], red[a * 3u + 2u][0], 0.0) / den;
            } else {
                out[i * 6u + a] = vec4<f32>(p.x, red[a][0] / den, red[6u + a][0] / den, 0.0);
            }
        }
    }
}

@compute @workgroup_size(64)
fn bounce_sky(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) { bounce(probe_index(wid), li, false); }

@compute @workgroup_size(64)
fn bounce_lamp(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) { bounce(probe_index(wid), li, true); }

// Average each valid probe's bounced channels (and with u.mode.z the direct
// sky) with the neighbours it sees: room_bake.rs `smooth_bounce`.
@compute @workgroup_size(64)
fn smooth_light(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    if pos[i].w < 0.5 { for (var a = 0u; a < 6u; a++) { out[i * 6u + a] = prev[i * 6u + a]; } return; }
    var sum: array<vec3<f32>, 6>;
    for (var a = 0u; a < 6u; a++) { sum[a] = prev[i * 6u + a].xyz; }
    var weight = 1.0;
    let x = i32(i % u.dims.x);
    let y = i32((i / u.dims.x) % u.dims.y);
    let z = i32(i / (u.dims.x * u.dims.y));
    let bits = seen[i];
    var bit = 0u;
    for (var ox = -1; ox <= 1; ox++) { for (var oy = -1; oy <= 1; oy++) { for (var oz = -1; oz <= 1; oz++) {
        if ox == 0 && oy == 0 && oz == 0 { continue; }
        let b = bit;
        bit += 1u;
        if (bits & (1u << b)) == 0u { continue; }
        let j = u32(x + ox) + u32(y + oy) * u.dims.x + u32(z + oz) * u.dims.x * u.dims.y;
        let w = pow(0.5, f32(abs(ox) + abs(oy) + abs(oz)));
        for (var a = 0u; a < 6u; a++) { sum[a] += prev[j * 6u + a].xyz * w; }
        weight += w;
    }}}
    for (var a = 0u; a < 6u; a++) {
        let s = sum[a] / weight;
        let own = prev[i * 6u + a];
        out[i * 6u + a] = vec4<f32>(select(own.x, s.x, u.mode.z == 1u), s.y, s.z, 0.0);
    }
}
