// Constraints the world is generated around (world_constraints.rs, line for
// line): roads and pads the land bends to. Include after terrain_field.wgsl;
// world_constraints::wgsl(group, binding) fills in the three bindings.
// terrain_eval_world is terrain_eval with the constraints. Each sample reads
// one coarse grid cell's short list, a fixed cost on any GPU.

struct WcHeader {
    origin: vec2<f32>,
    cell: f32,
    arms: u32,
    size: vec2<u32>,
    segments: u32,
    pad0: u32,
};

struct WcSegment {
    a: vec4<f32>,
    b: vec4<f32>,
    w: vec4<f32>,
};

@group(WC_GROUP) @binding(WC_HEADER) var<storage, read> wc_header: WcHeader;
@group(WC_GROUP) @binding(WC_SEGMENTS) var<storage, read> wc_segments: array<WcSegment>;
// size.x * size.y + 1 cell offsets, then the items they point at.
@group(WC_GROUP) @binding(WC_INDEX) var<storage, read> wc_index: array<u32>;

const WC_VERGE: f32 = 1.0;
const WC_MAX_REACH: f32 = 100.0;
const WC_BANK: f32 = 1.8;
const WC_OPEN: f32 = 9000.0;
const WC_STEEPEN: f32 = 0.0047;
const WC_MAX_ARMS: u32 = 8u;

struct WcNear {
    distance: f32,
    side: f32,
    along: f32,
    track: u32,
    half: f32,
    height: f32,
};

struct WcPull {
    y: f32,
    band: f32,
    slope: f32,
    core: f32,
    distance: f32,
};

fn wc_smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Items of the cell under (x, z): (first, end) into wc_index.
fn wc_list(x: f32, z: f32) -> vec2<u32> {
    let h = wc_header;
    let fi = floor((x - h.origin.x) / h.cell);
    let fj = floor((z - h.origin.y) / h.cell);
    if h.segments == 0u || fi < 0.0 || fj < 0.0 || fi >= f32(h.size.x) || fj >= f32(h.size.y) { return vec2<u32>(0u, 0u); }
    let c = u32(fj) * h.size.x + u32(fi);
    let base = h.size.x * h.size.y + 1u;
    return vec2<u32>(base + wc_index[c], base + wc_index[c + 1u]);
}

fn wc_segment_near(s: WcSegment, p: vec2<f32>) -> WcNear {
    let a = s.a.xz;
    let ab = s.b.xz - a;
    let len2 = dot(ab, ab);
    var t = 0.0;
    if len2 > 0.0 { t = clamp(dot(p - a, ab) / len2, 0.0, 1.0); }
    let d = distance(a + ab * t, p);
    let cross = ab.x * (p.y - a.y) - ab.y * (p.x - a.x);
    var n: WcNear;
    n.distance = d;
    n.side = select(d, -d, cross > 0.0);
    n.along = s.a.w + (s.b.w - s.a.w) * t;
    n.track = u32(s.w.z);
    n.half = s.w.x + (s.w.y - s.w.x) * t;
    n.height = s.a.y + (s.b.y - s.a.y) * t;
    return n;
}

// How far the land may stand above or below the road u metres past its
// edge, and how steep that bank is.
fn wc_bank(u_in: f32) -> vec2<f32> {
    let u = max(u_in, 0.0);
    let r = u / WC_MAX_REACH;
    let r5 = r * r * r * r * r;
    return vec2<f32>(u / WC_BANK + WC_STEEPEN * u * u + WC_OPEN * r5 * r, 1.0 / WC_BANK + 2.0 * WC_STEEPEN * u + 6.0 * WC_OPEN * r5 / WC_MAX_REACH);
}

fn wc_arm(y: f32, u: f32, core: f32, distance: f32) -> WcPull {
    let b = wc_bank(u);
    return WcPull(y, b.x, b.y, core, distance);
}

fn wc_smax(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 { return max(a, b); }
    let h = max(k - abs(a - b), 0.0) / k;
    return max(a, b) + h * h * k * 0.25;
}

fn wc_smin(a: f32, b: f32, k: f32) -> f32 { return -wc_smax(-a, -b, k); }

// The nearest road to (x, z) within its reach; track u32 max when none.
fn wc_near(x: f32, z: f32) -> WcNear {
    let p = vec2<f32>(x, z);
    let list = wc_list(x, z);
    var best: WcNear;
    best.distance = 3.4e38;
    best.track = 0xffffffffu;
    for (var i = list.x; i < list.y; i++) {
        let s = wc_segments[wc_index[i]];
        if s.w.w > 0.5 { continue; }
        let n = wc_segment_near(s, p);
        if n.distance < best.distance { best = n; }
    }
    return best;
}

// Metres to the nearest road edge or pad rim; negative inside.
fn wc_clearance(x: f32, z: f32) -> f32 {
    let p = vec2<f32>(x, z);
    let list = wc_list(x, z);
    var c = 3.4e38;
    for (var i = list.x; i < list.y; i++) {
        let s = wc_segments[wc_index[i]];
        if s.w.w > 0.5 {
            c = min(c, distance(p, s.a.xz) - s.b.x);
        } else {
            let n = wc_segment_near(s, p);
            c = min(c, n.distance - n.half);
        }
    }
    return c;
}

// One natural sample at ground coordinates (x, z), bent to the constraints.
fn wc_apply(x: f32, z: f32, s_in: TerrainSample) -> TerrainSample {
    var s = s_in;
    let p = vec2<f32>(x, z);
    let list = wc_list(x, z);
    if list.y <= list.x { return s; }
    var pulls: array<WcPull, 8>;
    var n = 0u;
    var last = 0xffffffffu;
    var last_track = -2.0;
    for (var i = list.x; i < list.y; i++) {
        let k = wc_index[i];
        let seg = wc_segments[k];
        var pull: WcPull;
        if seg.w.w > 0.5 {
            let d = distance(p, seg.a.xz);
            pull = wc_arm(seg.a.y, d - seg.b.x, 0.0, d);
        } else {
            let near = wc_segment_near(seg, p);
            pull = wc_arm(near.height, near.distance - near.half - WC_VERGE, wc_smooth(near.half + WC_VERGE, near.half, near.distance), near.distance);
        }
        let same = seg.w.w < 0.5 && last != 0xffffffffu && last + 1u == k && last_track == seg.w.z;
        last = k;
        last_track = select(seg.w.z, -1.0, seg.w.w > 0.5);
        if same && n > 0u {
            if pull.distance < pulls[n - 1u].distance { pulls[n - 1u] = pull; }
            continue;
        }
        if n == WC_MAX_ARMS { continue; }
        pulls[n] = pull;
        n++;
    }
    if n == 0u { return s; }
    let land = s.pos.y - s.water;
    var lo = -3.4e38;
    var hi = 3.4e38;
    var lo_slope = 0.0;
    var hi_slope = 0.0;
    var core = 0.0;
    var tight = 3.4e38;
    for (var i = 0u; i < n; i++) {
        let q = pulls[i];
        if q.y - q.band > lo { lo = q.y - q.band; lo_slope = q.slope; }
        if q.y + q.band < hi { hi = q.y + q.band; hi_slope = q.slope; }
        core = max(core, q.core);
        tight = min(tight, q.band);
    }
    let k = min(0.5 * tight, 2.0);
    var y = 0.0;
    if lo > hi { y = 0.5 * (lo + hi); } else { y = wc_smin(wc_smax(land, lo, k), hi, k); }
    let moved = abs(y - land);
    let pull = max(wc_smooth(0.0, 0.5, moved), core);
    if pull <= 0.0 { return s; }
    s.pos.x += (x - s.pos.x) * pull;
    s.pos.z += (z - s.pos.z) * pull;
    let level = s.pos.y;
    s.water = clamp(level - y, 0.0, s.water) * (1.0 - core);
    s.pos.y = y + s.water;
    var slope = 0.0;
    if y > land + 1e-3 { slope = lo_slope; } else if y < land - 1e-3 { slope = hi_slope; }
    let bank_up = 1.0 / sqrt(1.0 + slope * slope);
    s.up = max(s.up + (bank_up - s.up) * wc_smooth(0.0, 0.5, moved), s.up * (1.0 - core) + core);
    s.wet *= 1.0 - core;
    s.snow *= 1.0 - 0.7 * core;
    s.moor *= 1.0 - core;
    s.rock *= 1.0 - core;
    // A steep cutting or embankment is bare rock, not stretched grass.
    s.rock = max(s.rock, wc_smooth(0.9, 1.4, slope) * wc_smooth(0.0, 0.5, moved) * (1.0 - core));
    s.scree += (1.0 - s.scree) * core;
    return s;
}

fn terrain_eval_world(x: f32, z: f32, cut: f32, seed: u32, st: TerrainStyle) -> TerrainSample {
    let s = terrain_eval(x, z, cut, seed, st);
    if wc_header.segments == 0u { return s; }
    return wc_apply(x, z, s);
}
