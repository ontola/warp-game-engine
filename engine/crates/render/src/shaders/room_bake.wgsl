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
    // i: a probe, plus its light's slot times the probe count.
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
fn gather(p: vec3<f32>, n: vec3<f32>, seen_by: u32, slot: u32) -> vec3<f32> {
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
        sum += irradiance(slot * probe_count() + i, n) * w;
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

var<workgroup> red: array<array<f32, 64>, 36>;

fn reduce(li: u32, values: u32) {
    for (var s = 32u; s > 0u; s >>= 1u) {
        if li < s { for (var k = 0u; k < values; k++) { red[k][li] += red[k][li + s]; } }
        workgroupBarrier();
    }
}

// A second, larger estimate of the direct sky (u.mode.z rays from
// u.mode.x in `dirs`) where the first is unreliable: probes near ones that
// saw sky (`sky_flag`). Sky seen only through a small
// opening (a door, then a window) is hit by few of `sky_direct`'s rays, and
// that noise spread as blotches through every bounce's gathers. The new
// estimate replaces the first (independent rays: no bias); elsewhere the
// probe is copied.
@compute @workgroup_size(64)
fn sky_refine(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = probe_index(wid);
    let inside = i >= probe_count();
    let j = min(i, probe_count() - 1u);
    // Marked by `sky_flag`: within SKY_HOPS steps of a probe that saw sky.
    let flag = !inside && pos[j].w > 0.5 && (seen[j] & (1u << 30u)) != 0u;
    let rays = select(0u, u.mode.z, flag);
    let o = pos[j].xyz;
    let q = dirs[u.counts.w + j];
    for (var k = 0u; k < 12u; k++) { red[k][li] = 0.0; }
    for (var r = li; r < rays; r += 64u) {
        let d = rotate(q, dirs[u.mode.x + r].xyz);
        let h = hit(o, d);
        for (var a = 0u; a < 6u; a++) {
            let c = max(dot(AXES[a], d), 0.0);
            red[6u + a][li] += c;
            if h.w < 0.0 { red[a][li] += c; }
        }
    }
    workgroupBarrier();
    reduce(li, 12u);
    if li == 0u && !inside {
        for (var a = 0u; a < 6u; a++) {
            let p = prev[i * 6u + a];
            out[i * 6u + a] = select(p, vec4<f32>(red[a][0] / max(red[6u + a][0], 1e-6), p.yzw), flag);
        }
    }
}

// Which probes `sky_refine` estimates again: those that saw any sky in the
// first estimate (u.mode.z 0: bit 30 of `seen`), then, per further pass
// u.mode.z = k, those next to a marked probe they see (from bit 30 + (k+1)
// % 2 into bit 30 + k % 2): open space within that many steps of sky, a
// geometric reach a probe that by chance saw none still gets. The 26
// neighbour bits are kept.
@compute @workgroup_size(64)
fn sky_flag(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    let word = seen[i] & 0x3ffffffu;
    if u.mode.z == 0u {
        var hi = 0.0;
        for (var a = 0u; a < 6u; a++) { hi = max(hi, prev[i * 6u + a].x); }
        seen[i] = word | select(0u, 1u << 30u, hi > 0.0 && pos[i].w > 0.5);
        return;
    }
    let src = 30u + (u.mode.z + 1u) % 2u;
    let dst = 30u + u.mode.z % 2u;
    var on = (seen[i] & (1u << src)) != 0u;
    let x = i32(i % u.dims.x);
    let y = i32((i / u.dims.x) % u.dims.y);
    let z = i32(i / (u.dims.x * u.dims.y));
    var bit = 0u;
    for (var ox = -1; ox <= 1; ox++) { for (var oy = -1; oy <= 1; oy++) { for (var oz = -1; oz <= 1; oz++) {
        if ox == 0 && oy == 0 && oz == 0 { continue; }
        let b = bit;
        bit += 1u;
        if (word & (1u << b)) == 0u { continue; }
        let k = u32(x + ox) + u32(y + oy) * u.dims.x + u32(z + oz) * u.dims.x * u.dims.y;
        if (seen[k] & (1u << src)) != 0u { on = true; }
    }}}
    // Keep the source bit (others read it this pass), set the destination.
    seen[i] = (seen[i] & ~(1u << dst)) | select(0u, 1u << dst, on);
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

// One gather of bounced sky and sun: every bounce ray's hit lit by the
// previous pass's probes and the sun directly, weighted per axis: y (sky)
// and z (sun), x kept. Its w carries the standard deviation of y's and z's
// estimates (two halves): each axis's value is a weighted mean of the rays'
// values (weights c, cosine to the axis), whose variance is the rays'
// weighted variance times sum c^2 / (sum c)^2. `denoise` weighs neighbours
// by it.
fn bounce(i: u32, li: u32) {
    let valid = i < probe_count() && pos[i].w > 0.5;
    let j = min(i, probe_count() - 1u);
    let o = pos[j].xyz;
    let q = dirs[u.counts.w + j];
    // Rows: 0-5 sum c y, 6-11 sum c z, 12-17 sum c y^2, 18-23 sum c z^2,
    // 24-29 sum c, 30-35 sum c^2.
    for (var k = 0u; k < 36u; k++) { red[k][li] = 0.0; }
    if valid {
        for (var r = li; r < u.counts.y; r += 64u) {
            let d = rotate(q, dirs[u.mode.w + r].xyz);
            var c: array<f32, 6>;
            for (var a = 0u; a < 6u; a++) { c[a] = max(dot(AXES[a], d), 0.0); red[24u + a][li] += c[a]; red[30u + a][li] += c[a] * c[a]; }
            let h = hit(o, d);
            if h.w < 0.0 { continue; }
            let n = select(h.xyz, -h.xyz, dot(h.xyz, d) > 0.0);
            let at = o + d * h.w;
            let mask = masks[i * u.counts.y + r];
            let g = gather(at + n * 0.15, n, mask, 0u);
            let sun = u.sun.xyz;
            let lit = max(dot(n, sun), 0.0);
            var sun_e = 0.0;
            if sun.y > 0.0 && lit > 0.0 && (mask & 256u) != 0u { sun_e = lit; }
            let vy = u.sun.w * (g.x + g.y);
            let vz = u.sun.w * (sun_e + g.z);
            for (var a = 0u; a < 6u; a++) {
                red[a][li] += vy * c[a];
                red[6u + a][li] += vz * c[a];
                red[12u + a][li] += vy * vy * c[a];
                red[18u + a][li] += vz * vz * c[a];
            }
        }
    }
    workgroupBarrier();
    reduce(li, 36u);
    if li == 0u && i < probe_count() {
        for (var a = 0u; a < 6u; a++) {
            let p = prev[i * 6u + a];
            if !valid { out[i * 6u + a] = p; continue; }
            let den = max(red[24u + a][0], 1e-6);
            let my = red[a][0] / den;
            let mz = red[6u + a][0] / den;
            let share = red[30u + a][0] / (den * den);
            let sd = sqrt(max(vec2<f32>(red[12u + a][0] / den - my * my, red[18u + a][0] / den - mz * mz), vec2<f32>(0.0)) * share);
            out[i * 6u + a] = vec4<f32>(p.x, my, mz, bitcast<f32>(pack2x16float(sd)));
        }
    }
}

// Joint filter of the bounced light over the probe grid (SVGF's edge-
// stopping, Schied et al. 2017, on probes): each valid probe averages with
// the neighbours it sees, each weighted by 0.5 per step and by how far its
// value lies from this probe's in units of the two estimates' standard
// deviations, exp(-d^2 / (2 * 2^2 * (sd_i^2 + sd_j^2))): neighbours that
// differ by noise blend in, real differences (light through a doorway) do
// not. The standard deviations propagate (sum w^2 sd^2) / (sum w)^2 for the
// next iteration. y and z only; x is the direct sky, traced with many rays.
@compute @workgroup_size(64)
fn denoise(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    if pos[i].w < 0.5 { for (var a = 0u; a < 6u; a++) { out[i * 6u + a] = prev[i * 6u + a]; } return; }
    let x = i32(i % u.dims.x);
    let y = i32((i / u.dims.x) % u.dims.y);
    let z = i32(i / (u.dims.x * u.dims.y));
    let bits = seen[i];
    for (var a = 0u; a < 6u; a++) {
        let own = prev[i * 6u + a];
        let sd_i = unpack2x16float(bitcast<u32>(own.w));
        var sum = own.yz;
        var weight = vec2<f32>(1.0);
        var var_sum = sd_i * sd_i;
        var bit = 0u;
        for (var ox = -1; ox <= 1; ox++) { for (var oy = -1; oy <= 1; oy++) { for (var oz = -1; oz <= 1; oz++) {
            if ox == 0 && oy == 0 && oz == 0 { continue; }
            let b = bit;
            bit += 1u;
            if (bits & (1u << b)) == 0u { continue; }
            let j = u32(x + ox) + u32(y + oy) * u.dims.x + u32(z + oz) * u.dims.x * u.dims.y;
            let other = prev[j * 6u + a];
            let sd_j = unpack2x16float(bitcast<u32>(other.w));
            let diff = other.yz - own.yz;
            let noise = 8.0 * (sd_i * sd_i + sd_j * sd_j) + 1e-12;
            let w = pow(0.5, f32(abs(ox) + abs(oy) + abs(oz))) * exp(-diff * diff / noise);
            sum += other.yz * w;
            weight += w;
            var_sum += w * w * sd_j * sd_j;
        }}}
        let sd = sqrt(var_sum) / weight;
        out[i * 6u + a] = vec4<f32>(own.x, sum / weight, bitcast<f32>(pack2x16float(sd)));
    }
}

@compute @workgroup_size(64)
fn bounce_sky(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) { bounce(probe_index(wid), li); }

// Up to LAMPS_PER_PASS lamps' bounce in one pass (u.mode.x the first,
// u.sun.x how many): each bounce ray is traced once for all of them; lamp s
// reads and writes probe slot s (s times the probe count onwards).
const LAMPS_PER_PASS: u32 = 8u;

@compute @workgroup_size(64)
fn bounce_lamp(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = probe_index(wid);
    let slots = min(u32(u.sun.x), LAMPS_PER_PASS);
    let valid = i < probe_count() && pos[i].w > 0.5;
    let j = min(i, probe_count() - 1u);
    let o = pos[j].xyz;
    let q = dirs[u.counts.w + j];
    var acc: array<f32, 144>;
    // Per lamp and axis sum c Y^2 (Y the leaving light's luminance), and per
    // axis sum c and sum c^2: the variance of each axis's estimate, as the
    // sky bounce (`denoise_lamp` weighs neighbours by it).
    var acc2: array<f32, 48>;
    var den: array<f32, 6>;
    var den2: array<f32, 6>;
    for (var k = 0u; k < 144u; k++) { acc[k] = 0.0; }
    for (var k = 0u; k < 48u; k++) { acc2[k] = 0.0; }
    for (var a = 0u; a < 6u; a++) { den[a] = 0.0; den2[a] = 0.0; }
    if valid {
        for (var r = li; r < u.counts.y; r += 64u) {
            let d = rotate(q, dirs[u.mode.w + r].xyz);
            var c: array<f32, 6>;
            for (var a = 0u; a < 6u; a++) { c[a] = max(dot(AXES[a], d), 0.0); den[a] += c[a]; den2[a] += c[a] * c[a]; }
            let h = hit(o, d);
            if h.w < 0.0 { continue; }
            let n = select(h.xyz, -h.xyz, dot(h.xyz, d) > 0.0);
            let at = o + d * h.w;
            let mask = masks[i * u.counts.y + r];
            for (var s = 0u; s < slots; s++) {
                let l = u.mode.x + s;
                var to = vec3<f32>(0.0);
                var direct = light_at(l, at, &to);
                let cosine = dot(n, to);
                if cosine <= 0.0 || (mask & (1u << (9u + l))) == 0u { direct = vec3<f32>(0.0); }
                let leaving = u.sun.w * (direct * max(cosine, 0.0) + gather(at + n * 0.15, n, mask, s));
                let y = dot(leaving, LUMINANCE);
                for (var a = 0u; a < 6u; a++) {
                    acc[s * 18u + a * 3u] += leaving.x * c[a];
                    acc[s * 18u + a * 3u + 1u] += leaving.y * c[a];
                    acc[s * 18u + a * 3u + 2u] += leaving.z * c[a];
                    acc2[s * 6u + a] += y * y * c[a];
                }
            }
        }
    }
    // One reduction per lamp through the shared rows.
    for (var s = 0u; s < slots; s++) {
        for (var k = 0u; k < 18u; k++) { red[k][li] = acc[s * 18u + k]; }
        for (var a = 0u; a < 6u; a++) { red[18u + a][li] = den[a]; red[24u + a][li] = acc2[s * 6u + a]; red[30u + a][li] = den2[a]; }
        workgroupBarrier();
        reduce(li, 36u);
        if li == 0u && i < probe_count() {
            let at = (s * probe_count() + i) * 6u;
            for (var a = 0u; a < 6u; a++) {
                if !valid { out[at + a] = prev[at + a]; continue; }
                let d = max(red[18u + a][0], 1e-6);
                let m = vec3<f32>(red[a * 3u][0], red[a * 3u + 1u][0], red[a * 3u + 2u][0]) / d;
                let y = dot(m, LUMINANCE);
                let sd = sqrt(max(red[24u + a][0] / d - y * y, 0.0) * red[30u + a][0] / (d * d));
                out[at + a] = vec4<f32>(m, sd);
            }
        }
        workgroupBarrier();
    }
}

const LUMINANCE = vec3<f32>(0.2126, 0.7152, 0.0722);

// `denoise` for a lamp's bounce (rgb, its luminance's standard deviation in
// w), over max(u.mode.x, 1) lamp slots: neighbours weighted by how far
// their luminance lies from this probe's in units of both estimates'
// standard deviations.
@compute @workgroup_size(64)
fn denoise_lamp(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x + gid.y * 65535u * 64u;
    if index >= probe_count() * max(u.mode.x, 1u) { return; }
    let i = index % probe_count();
    let slot = index - i;
    if pos[i].w < 0.5 { for (var a = 0u; a < 6u; a++) { out[index * 6u + a] = prev[index * 6u + a]; } return; }
    let x = i32(i % u.dims.x);
    let y = i32((i / u.dims.x) % u.dims.y);
    let z = i32(i / (u.dims.x * u.dims.y));
    let bits = seen[i];
    for (var a = 0u; a < 6u; a++) {
        let own = prev[index * 6u + a];
        let lum = dot(own.xyz, LUMINANCE);
        var sum = own.xyz;
        var weight = 1.0;
        var var_sum = own.w * own.w;
        var bit = 0u;
        for (var ox = -1; ox <= 1; ox++) { for (var oy = -1; oy <= 1; oy++) { for (var oz = -1; oz <= 1; oz++) {
            if ox == 0 && oy == 0 && oz == 0 { continue; }
            let b = bit;
            bit += 1u;
            if (bits & (1u << b)) == 0u { continue; }
            let j = u32(x + ox) + u32(y + oy) * u.dims.x + u32(z + oz) * u.dims.x * u.dims.y;
            let other = prev[(slot + j) * 6u + a];
            let d = dot(other.xyz, LUMINANCE) - lum;
            let w = pow(0.5, f32(abs(ox) + abs(oy) + abs(oz))) * exp(-d * d / (8.0 * (own.w * own.w + other.w * other.w) + 1e-12));
            sum += other.xyz * w;
            weight += w;
            var_sum += w * w * other.w * other.w;
        }}}
        out[index * 6u + a] = vec4<f32>(sum / weight, sqrt(var_sum) / weight);
    }
}

// Average each valid probe's bounced channels (and with u.mode.z the direct
// sky) with the neighbours it sees: room_bake.rs `smooth_bounce`. Over
// max(u.mode.x, 1) light slots.
@compute @workgroup_size(64)
fn smooth_light(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x + gid.y * 65535u * 64u;
    if index >= probe_count() * max(u.mode.x, 1u) { return; }
    let i = index % probe_count();
    // Probe i of this slot, and the slot's first probe.
    let slot = index - i;
    if pos[i].w < 0.5 { for (var a = 0u; a < 6u; a++) { out[index * 6u + a] = prev[index * 6u + a]; } return; }
    var sum: array<vec3<f32>, 6>;
    for (var a = 0u; a < 6u; a++) { sum[a] = prev[index * 6u + a].xyz; }
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
        for (var a = 0u; a < 6u; a++) { sum[a] += prev[(slot + j) * 6u + a].xyz * w; }
        weight += w;
    }}}
    for (var a = 0u; a < 6u; a++) {
        let s = sum[a] / weight;
        let own = prev[index * 6u + a];
        out[index * 6u + a] = vec4<f32>(select(own.x, s.x, u.mode.z == 1u), s.y, s.z, 0.0);
    }
}

// ---- Probe geometry (room_bake_gpu::bake_geometry): placement, visibility
// maps, reach, separation planes. A geometry bake's sky rays are the 256
// placement directions and its mask stride (u.counts.y) holds a probe's
// visibility map (64 words) and reach (16 words).

// Where probe i sits and whether it is still inside (room_bake.rs
// `probe_positions` and `Placement::new`): xyz, w 1 when not inside.
@compute @workgroup_size(64)
fn place(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    let behind = 0.03;
    let near = 0.2 * u.origin.w;
    let most = vec3<f32>(0.45 * u.origin.w);
    let base = grid_position(i);
    var offset = vec3<f32>(0.0);
    for (var it = 0; it < 4; it++) {
        let o = base + offset;
        var back = 0u;
        var nb = vec4<f32>(0.0, 0.0, 0.0, 1e30);
        var nf = vec4<f32>(0.0, 0.0, 0.0, 1e30);
        for (var r = 0u; r < u.counts.x; r++) {
            let d = dirs[r].xyz;
            let h = hit(o - d * behind, d);
            if h.w < 0.0 { continue; }
            let t = max(h.w - behind, 0.0);
            if dot(h.xyz, d) > 0.0 {
                back += 1u;
                if t < nb.w { nb = vec4<f32>(d, t); }
            } else if t < nf.w { nf = vec4<f32>(d, t); }
        }
        var step: vec3<f32>;
        if back * 4u >= u.counts.x { step = nb.xyz * (nb.w + near); }
        else if nf.w < near { step = -nf.xyz * (near - nf.w); }
        else { break; }
        offset = clamp(offset + step, -most, most);
    }
    let p = base + offset;
    var back = 0u;
    for (var r = 0u; r < u.counts.x; r++) {
        let d = dirs[r].xyz;
        let h = hit(p, d);
        if h.w >= 0.0 && dot(h.xyz, d) > 0.0 { back += 1u; }
    }
    pos[i] = vec4<f32>(p, select(1.0, 0.0, back * 4u >= u.counts.x));
}

// IEEE half bits of a finite non-negative value, as room_bake.rs `f16_bits`
// rounds it (to nearest; tiny values flush to zero).
fn f16_bits(value: f32) -> u32 {
    let v = clamp(value, 0.0, 65504.0);
    if v < 6.1e-5 { return 0u; }
    let bits = bitcast<u32>(v);
    var e = i32((bits >> 23u) & 255u) - 127 + 15;
    var m = (bits & 0x7fffffu) + 0x1000u;
    if (m & 0x800000u) != 0u { e += 1; m = 0u; } else { m = m >> 13u; }
    return (u32(clamp(e, 1, 30)) << 10u) | (m & 0x3ffu);
}

// DDGI's octahedral direction (room_bake.rs `octahedral_direction`).
fn octahedral(uv: vec2<f32>) -> vec3<f32> {
    let f = uv * 2.0 - 1.0;
    var n = vec3<f32>(f.x, f.y, 1.0 - abs(f.x) - abs(f.y));
    if n.z < 0.0 {
        let x = n.x;
        let y = n.y;
        n.x = (1.0 - abs(y)) * select(-1.0, 1.0, x >= 0.0);
        n.y = (1.0 - abs(x)) * select(-1.0, 1.0, y >= 0.0);
    }
    return normalize(n);
}

// One texel of probe i's 8x8 visibility map per thread: mean and mean
// squared distance to the nearest surface over 4 rays, capped at 1.5 cells.
@compute @workgroup_size(64)
fn visibility_map(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let i = probe_index(wid);
    if i >= probe_count() { return; }
    let q = pos[i];
    var word = 0u;
    if q.w > 0.5 {
        let cap = u.origin.w * 1.5;
        let tx = f32(li % 8u);
        let ty = f32(li / 8u);
        var sum = 0.0;
        var sum2 = 0.0;
        for (var s = 0u; s < 4u; s++) {
            let jitter = vec2<f32>(0.25 + 0.5 * f32(s & 1u), 0.25 + 0.5 * f32(s >> 1u));
            let d = octahedral((vec2<f32>(tx, ty) + jitter) / 8.0);
            let h = hit(q.xyz, d);
            let t = select(cap, min(h.w, cap), h.w >= 0.0);
            sum += t;
            sum2 += t * t;
        }
        word = f16_bits(sum / 4.0) | (f16_bits(sum2 / 4.0) << 16u);
    }
    masks[i * u.counts.y + li] = word;
}

// The plane of the nearest surface probe i faces along 26 directions, in
// out[i * 6] (room_bake.rs `separation_planes`).
@compute @workgroup_size(64)
fn planes(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    let p = pos[i].xyz;
    var nearest = u.origin.w * 1.75;
    var plane = vec4<f32>(0.0);
    for (var x = -1; x <= 1; x++) { for (var y = -1; y <= 1; y++) { for (var z = -1; z <= 1; z++) {
        if x == 0 && y == 0 && z == 0 { continue; }
        let d = normalize(vec3<f32>(f32(x), f32(y), f32(z)));
        let h = hit(p, d);
        if h.w < 0.0 { continue; }
        let n = select(h.xyz, -h.xyz, dot(h.xyz, d) > 0.0);
        if h.w < nearest {
            nearest = h.w;
            plane = vec4<f32>(n, -dot(n, p + d * h.w));
        }
    }}}
    out[i * 6u] = plane;
}

// Open space between a and b (room_bake.rs `open_between`).
fn open_between(a: vec3<f32>, b: vec3<f32>) -> bool {
    let to = b - a;
    let len = length(to);
    if len < 1e-6 { return true; }
    let d = to / len;
    return !blocked(a - d * 0.03, d, len + 0.03);
}

// Which voxels of probe i's neighbourhood (one cell either side, 4 voxels
// per cell) it reaches through open space: a flood fill from its own voxel
// (room_bake.rs `bake_reach`), 16 words after its visibility map.
@compute @workgroup_size(64)
fn reach(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if i >= probe_count() { return; }
    var bits: array<u32, 16>;
    for (var k = 0u; k < 16u; k++) { bits[k] = 0u; }
    let q = pos[i];
    if q.w > 0.5 {
        let voxel = u.origin.w / 4.0;
        let corner = grid_position(i) - vec3<f32>(u.origin.w);
        let start = clamp(vec3<i32>(floor((q.xyz - corner) / voxel)), vec3<i32>(0), vec3<i32>(7));
        var stack: array<u32, 512>;
        var top = 0u;
        let s0 = u32(start.x) + u32(start.y) * 8u + u32(start.z) * 64u;
        if open_between(q.xyz, corner + (vec3<f32>(start) + 0.5) * voxel) { bits[s0 / 32u] |= 1u << (s0 % 32u); stack[0] = s0; top = 1u; }
        while top > 0u {
            top -= 1u;
            let v = stack[top];
            let vi = vec3<i32>(i32(v % 8u), i32((v / 8u) % 8u), i32(v / 64u));
            let from_c = corner + (vec3<f32>(vi) + 0.5) * voxel;
            for (var k = 0; k < 6; k++) {
                var w = vi;
                let step = select(-1, 1, (k & 1) == 0);
                if k < 2 { w.x += step; } else if k < 4 { w.y += step; } else { w.z += step; }
                if any(w < vec3<i32>(0)) || any(w > vec3<i32>(7)) { continue; }
                let wi = u32(w.x) + u32(w.y) * 8u + u32(w.z) * 64u;
                if (bits[wi / 32u] & (1u << (wi % 32u))) != 0u { continue; }
                if !open_between(from_c, corner + (vec3<f32>(w) + 0.5) * voxel) { continue; }
                bits[wi / 32u] |= 1u << (wi % 32u);
                stack[top] = wi;
                top += 1u;
            }
        }
    }
    for (var k = 0u; k < 16u; k++) { masks[i * u.counts.y + 64u + k] = bits[k]; }
}
