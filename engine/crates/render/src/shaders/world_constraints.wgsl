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
    // The regions (regions.rs): grid node (0, 0), spacing, where their
    // words start in wc_index (styles, then two words a node), nodes
    // across, styles in the palette (0: none).
    region_origin: vec2<f32>,
    region_cell: f32,
    region_at: u32,
    region_size: vec2<u32>,
    region_styles: u32,
    // Where the footprints' corners start in wc_index (x, z bits each).
    shape_at: u32,
    // The zones (zones.rs): grid cell (0, 0), spacing, how many (0: none),
    // cells across, where their words start in wc_index.
    zone_origin: vec2<f32>,
    zone_cell: f32,
    zone_count: u32,
    zone_size: vec2<u32>,
    zone_at: u32,
    pad3: u32,
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

// outline_distance: signed metres from p to the closed outline of `count`
// corners at wc_index[at] (x, z bits each); negative inside.
fn wc_outline(at: u32, count: u32, p: vec2<f32>) -> f32 {
    let v0 = vec2<f32>(bitcast<f32>(wc_index[at]), bitcast<f32>(wc_index[at + 1u]));
    var d = dot(p - v0, p - v0);
    var sign = 1.0;
    var j = count - 1u;
    for (var i = 0u; i < count; i++) {
        let vi = vec2<f32>(bitcast<f32>(wc_index[at + 2u * i]), bitcast<f32>(wc_index[at + 2u * i + 1u]));
        let vj = vec2<f32>(bitcast<f32>(wc_index[at + 2u * j]), bitcast<f32>(wc_index[at + 2u * j + 1u]));
        let e = vj - vi;
        let w = p - vi;
        let b = w - e * clamp(dot(w, e) / dot(e, e), 0.0, 1.0);
        d = min(d, dot(b, b));
        let c0 = p.y >= vi.y;
        let c1 = p.y < vj.y;
        let c2 = e.x * w.y > e.y * w.x;
        if (c0 && c1 && c2) || (!c0 && !c1 && !c2) { sign = -sign; }
        j = i;
    }
    return sign * sqrt(d);
}

// Metres from p to a pad's rim or a footprint's outline; negative inside.
fn wc_rim(s: WcSegment, p: vec2<f32>) -> f32 {
    if s.w.w > 1.5 { return wc_outline(wc_header.shape_at + 2u * u32(s.b.x), u32(s.b.y), p); }
    return distance(p, s.a.xz) - s.b.x;
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

// Metres to the nearest road edge, pad rim or footprint outline; negative inside.
fn wc_clearance(x: f32, z: f32) -> f32 {
    let p = vec2<f32>(x, z);
    let list = wc_list(x, z);
    var c = 3.4e38;
    for (var i = list.x; i < list.y; i++) {
        let s = wc_segments[wc_index[i]];
        if s.w.w > 0.5 {
            c = min(c, wc_rim(s, p));
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
            let d = wc_rim(seg, p);
            // A bare footprint is gravel inside, fading out over the verge.
            var core = 0.0;
            if seg.w.w > 1.5 { core = seg.w.x * wc_smooth(WC_VERGE, 0.0, d); }
            pull = wc_arm(seg.a.y, d, core, d);
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
    s.film *= 1.0 - core;
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


// Regions::style_at: the styles of the four surrounding nodes mixed by
// their weights and bilinear shares; one style alone exactly; `st` where
// the world has no regions.
fn wc_style(x: f32, z: f32, st: TerrainStyle) -> TerrainStyle {
    let h = wc_header;
    if h.region_styles == 0u { return st; }
    let w = h.region_size.x;
    let hh = h.region_size.y;
    let fx = clamp((x - h.region_origin.x) / h.region_cell, 0.0, f32(w - 1u));
    let fz = clamp((z - h.region_origin.y) / h.region_cell, 0.0, f32(hh - 1u));
    let i0 = min(u32(floor(fx)), max(w, 2u) - 2u);
    let j0 = min(u32(floor(fz)), max(hh, 2u) - 2u);
    let i1 = min(i0 + 1u, w - 1u);
    let j1 = min(j0 + 1u, hh - 1u);
    let tx = fx - f32(i0);
    let tz = fz - f32(j0);
    let nodes = h.region_at + h.region_styles * 16u;
    var at = array<u32, 4>(nodes + 2u * (j0 * w + i0), nodes + 2u * (j0 * w + i1), nodes + 2u * (j1 * w + i0), nodes + 2u * (j1 * w + i1));
    var f = array<f32, 4>((1.0 - tx) * (1.0 - tz), tx * (1.0 - tz), (1.0 - tx) * tz, tx * tz);
    // One style alone: return it untouched (no rounding from the mix).
    var only = 0xffffffffu;
    var mixed = false;
    for (var c = 0u; c < 4u; c++) {
        let ids = wc_index[at[c]];
        let ws = wc_index[at[c] + 1u];
        for (var k = 0u; k < 4u; k++) {
            if f[c] > 0.0 && ((ws >> (8u * k)) & 255u) > 0u {
                let id = (ids >> (8u * k)) & 255u;
                if only == 0xffffffffu { only = id; } else if id != only { mixed = true; }
            }
        }
    }
    var acc: array<f32, 16>;
    if !mixed {
        for (var v = 0u; v < 16u; v++) { acc[v] = bitcast<f32>(wc_index[h.region_at + only * 16u + v]); }
    } else {
        var sum = 0.0;
        for (var c = 0u; c < 4u; c++) {
            let ids = wc_index[at[c]];
            let ws = wc_index[at[c] + 1u];
            for (var k = 0u; k < 4u; k++) {
                let share = f32((ws >> (8u * k)) & 255u) / 255.0 * f[c];
                if share > 0.0 {
                    let base = h.region_at + ((ids >> (8u * k)) & 255u) * 16u;
                    for (var v = 0u; v < 16u; v++) { acc[v] += bitcast<f32>(wc_index[base + v]) * share; }
                    sum += share;
                }
            }
        }
        for (var v = 0u; v < 16u; v++) { acc[v] /= sum; }
    }
    return TerrainStyle(acc[0], acc[1], acc[2], acc[3], acc[4], acc[5], acc[6], acc[7],
        acc[8], acc[9], acc[10], acc[11], acc[12], acc[13], acc[14], acc[15]);
}

fn terrain_eval_world(x: f32, z: f32, cut: f32, seed: u32, st: TerrainStyle) -> TerrainSample {
    let s = terrain_eval(x, z, cut, seed, wc_style(x, z, st));
    if wc_header.segments == 0u { return s; }
    return wc_apply(x, z, s);
}

// Zones (zones.rs, line for line): the zones listed in the zone cell under
// (x, z), as (first, end) into wc_index.
fn wc_zone_list(x: f32, z: f32) -> vec2<u32> {
    let h = wc_header;
    let fi = floor((x - h.zone_origin.x) / h.zone_cell);
    let fj = floor((z - h.zone_origin.y) / h.zone_cell);
    if h.zone_count == 0u || fi < 0.0 || fj < 0.0 || fi >= f32(h.zone_size.x) || fj >= f32(h.zone_size.y) { return vec2<u32>(0u, 0u); }
    let cells = h.zone_at + 8u * h.zone_count;
    let c = u32(fj) * h.zone_size.x + u32(fi);
    let items = cells + h.zone_size.x * h.zone_size.y + 1u;
    return vec2<u32>(items + wc_index[cells + c], items + wc_index[cells + c + 1u]);
}

// How much of a zone a cell item's point is: 1 for a cell wholly inside.
fn wc_zone_share(item: u32, p: vec2<f32>) -> f32 {
    if (item & 0x80000000u) != 0u { return 1.0; }
    let r = wc_header.zone_at + 8u * item;
    let d = wc_outline(wc_header.zone_at + wc_index[r], wc_index[r + 1u], p);
    let h = 0.5 * max(bitcast<f32>(wc_index[r + 3u]), 1e-3);
    let t = clamp((d + h) / (2.0 * h), 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

// Zones::weight: the share of tag `tag` at (x, z), its strongest zone there.
fn wc_zone(x: f32, z: f32, tag: u32) -> f32 {
    let p = vec2<f32>(x, z);
    let list = wc_zone_list(x, z);
    var w = 0.0;
    for (var i = list.x; i < list.y; i++) {
        let item = wc_index[i];
        if wc_index[wc_header.zone_at + 8u * (item & 0x7fffffffu) + 2u] == tag { w = max(w, wc_zone_share(item, p)); }
    }
    return w;
}

// Zones::zoning: (covered, density, wealth, age) at (x, z).
fn wc_zoning(x: f32, z: f32) -> vec4<f32> {
    let p = vec2<f32>(x, z);
    let list = wc_zone_list(x, z);
    var sum = 0.0;
    var acc = vec3<f32>(0.0);
    for (var i = list.x; i < list.y; i++) {
        let item = wc_index[i];
        let w = wc_zone_share(item, p);
        if w <= 0.0 { continue; }
        let r = wc_header.zone_at + 8u * (item & 0x7fffffffu);
        acc += vec3<f32>(bitcast<f32>(wc_index[r + 4u]), bitcast<f32>(wc_index[r + 5u]), bitcast<f32>(wc_index[r + 6u])) * w;
        sum += w;
    }
    if sum <= 0.0 { return vec4<f32>(0.0); }
    return vec4<f32>(min(sum, 1.0), acc / sum);
}
