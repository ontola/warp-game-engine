// Dead Miles' own materials: everything in the land is a box or a ball, so
// the surfaces carry the detail. Each kind (`surface = N`, see
// lib/mat.module.luau) is drawn procedurally in world space around the
// entity's colour, so one look still varies from building to building:
//
//  0 asphalt   cracked, patched, aggregate, puddles that catch the lamps
//  1 pavement  slabs with grout, a few cracked, dirt in the joints
//  2 grass     patchy, dry and dark tufts, bare earth showing through
//  3 dirt      packed earth with pebbles and ruts
//  4 brick     running bond, mortar, burnt bricks, grime from the ground up
//  5 plaster   render over brick: stains, rain streaks, bare patches
//  6 concrete  form lines, pits, cracks, soot
//  7 wood      planks with grain and gaps
//  8 metal     painted sheet, scratched, rust coming through
//  9 rust      a wreck: rust with flakes of the old paint, or burnt black
// 10 roof      tar felt in strips, gravel, standing water
// 11 foliage   leaves in clumps with dark gaps
// 12 cloth     sacking weave, dusty
// 13 rubber    tyres and bags: dull, a little wear
// 14 glass     dark windows: dirty, cracked, shards
// 15 paint     car body: gloss over grime, rust at the sills
// 16 logs      a log wall: round logs, dark chinks
//
// Edit while the game runs: it recompiles on the next frame, and a version
// that does not compile keeps the last good one running. Functions are
// prefixed dm_ (the file is compiled with the engine's shading code).

struct SurfaceIn {
    kind: u32,
    instance: u32,
    p: vec3<f32>,
    local: vec3<f32>,
    n: vec3<f32>,
    dir: vec3<f32>,
    bary: vec3<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    dx: vec3<f32>,
    dy: vec3<f32>,
    tint: vec3<f32>,
    rough: f32,
    metal: f32,
    scale: f32,
    time: f32,
};

struct SurfaceOut {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    rough: f32,
    metal: f32,
    emission: vec3<f32>,
};

// ---------------------------------------------------------------------------
// Noise

fn dm_h21(p: vec2<f32>) -> f32 {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

fn dm_h22(p: vec2<f32>) -> vec2<f32> {
    var q = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.xx + q.yz) * q.zy);
}

fn dm_h31(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn dm_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = dm_h21(i);
    let b = dm_h21(i + vec2<f32>(1.0, 0.0));
    let c = dm_h21(i + vec2<f32>(0.0, 1.0));
    let d = dm_h21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn dm_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(dm_h31(i), dm_h31(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(dm_h31(i + vec3<f32>(0.0, 1.0, 0.0)), dm_h31(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(dm_h31(i + vec3<f32>(0.0, 0.0, 1.0)), dm_h31(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(dm_h31(i + vec3<f32>(0.0, 1.0, 1.0)), dm_h31(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// Detail that fades to its average once a pixel covers `size` metres or
// more of it (fw: the pixel's footprint in metres): no shimmer from afar.
fn dm_fade(size: f32, fw: f32) -> f32 {
    return 1.0 - smoothstep(0.6, 2.5, fw / size);
}

// Fractal noise, octaves dropped (faded) as they get finer than a pixel.
fn dm_fbm(p: vec2<f32>, fw: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    var f = fw;
    for (var i = 0; i < 4; i++) {
        let k = 1.0 - smoothstep(0.3, 1.0, f);
        sum += amp * mix(0.5, dm_noise(q), k);
        q = q * 2.03 + vec2<f32>(17.1, 9.4);
        f *= 2.03;
        amp *= 0.5;
    }
    return sum / 0.9375;
}

// Cells: x the distance to the nearest centre, y the gap to the second
// nearest (0 on an edge between two cells), z a number per cell.
fn dm_cells(p: vec2<f32>) -> vec3<f32> {
    let i = floor(p);
    let f = fract(p);
    var d1 = 9.0;
    var d2 = 9.0;
    var id = vec2<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2<f32>(f32(x), f32(y));
            let c = o + dm_h22(i + o);
            let d = length(c - f);
            if d < d1 { d2 = d1; d1 = d; id = i + o; } else if d < d2 { d2 = d; }
        }
    }
    return vec3<f32>(d1, d2 - d1, dm_h21(id + 3.7));
}

// Thin, broken crack lines over cells of `size` metres.
fn dm_cracks(uv: vec2<f32>, size: f32, width: f32, fw: f32, amount: f32) -> f32 {
    // Warped twice, so the edges wander and kink instead of running
    // straight from corner to corner like tiles.
    let q0 = uv / size;
    let wq = q0 + (vec2<f32>(dm_noise(q0 * 2.3 + 1.7), dm_noise(q0 * 2.3 + 9.1)) - 0.5) * 0.55
        + (vec2<f32>(dm_noise(q0 * 9.0 + 3.3), dm_noise(q0 * 9.0 + 6.6)) - 0.5) * 0.12 * dm_fade(size / 9.0, fw);
    let c = dm_cells(wq);
    let w = width / size;
    let line = 1.0 - smoothstep(w * 0.4, w + fw / size, c.y);
    let keep = smoothstep(1.0 - amount, 1.0 - amount + 0.2, dm_noise(uv / size * 1.7 + 4.0));
    return line * keep * dm_fade(width * 2.0, fw);
}

// ---------------------------------------------------------------------------
// One surface sample: albedo, roughness, metalness and a height for bumps.

struct DmS {
    albedo: vec3<f32>,
    rough: f32,
    metal: f32,
    h: f32,
};

fn dm_luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }

// Shift a colour's brightness by k (1 keeps it) and pull it towards grey.
fn dm_shade(c: vec3<f32>, k: f32, desat: f32) -> vec3<f32> {
    let g = vec3<f32>(dm_luma(c));
    return mix(c, g, desat) * k;
}

const DM_MORTAR: vec3<f32> = vec3<f32>(0.20, 0.19, 0.17);
const DM_SOOT: vec3<f32> = vec3<f32>(0.018, 0.016, 0.015);
const DM_EARTH: vec3<f32> = vec3<f32>(0.085, 0.062, 0.042);
const DM_RUST: vec3<f32> = vec3<f32>(0.13, 0.05, 0.022);
const DM_RUST_DARK: vec3<f32> = vec3<f32>(0.045, 0.022, 0.012);
const DM_BRICK: vec3<f32> = vec3<f32>(0.24, 0.085, 0.05);

fn dm_asphalt(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    var c = tint;
    // Worn lanes and blotches over metres, grit at the scale of millimetres.
    let blot = dm_fbm(uv * 0.3, fw * 0.3);
    c *= 0.75 + 0.5 * blot;
    let grit = mix(0.5, dm_noise(uv * 55.0), dm_fade(0.02, fw));
    c *= 0.86 + 0.28 * grit;
    let stones = mix(0.0, smoothstep(0.78, 0.9, dm_noise(uv * 23.0 + 5.0)), dm_fade(0.05, fw));
    c = mix(c, tint * 1.5 + 0.005, stones * 0.35);
    // Repair patches: rectangles of newer, darker tar.
    let cell = floor(uv / vec2<f32>(3.1, 2.3));
    let ph = dm_h21(cell);
    var h = (grit - 0.5) * 0.002 + stones * 0.0015;
    if ph > 0.8 {
        let f = fract(uv / vec2<f32>(3.1, 2.3));
        let lo = vec2<f32>(0.15 + 0.2 * dm_h21(cell + 1.0), 0.1 + 0.3 * dm_h21(cell + 2.0));
        let hi = lo + vec2<f32>(0.3 + 0.3 * dm_h21(cell + 3.0), 0.3 + 0.4 * dm_h21(cell + 4.0));
        let inside = step(lo.x, f.x) * step(f.x, hi.x) * step(lo.y, f.y) * step(f.y, hi.y);
        c = mix(c, c * 0.8, inside);
        h += inside * 0.003;
    }
    // Oil stains.
    let oil = smoothstep(0.72, 0.8, dm_noise(uv * 0.9 + 31.0));
    c *= 1.0 - 0.45 * oil;
    // Cracks, wider and more of them where it is worn.
    let crack = dm_cracks(uv, 2.2, 0.01, fw, 0.25 + 0.35 * (1.0 - blot));
    c = mix(c, tint * 0.25, crack);
    h -= crack * 0.01;
    // Standing water in the dips: dark and glossy, it mirrors the lamps.
    let wet = smoothstep(0.6, 0.7, dm_fbm(uv * 0.22 + 7.0, fw * 0.22));
    c *= 1.0 - 0.4 * wet;
    let rough = mix(0.88 - 0.1 * oil, 0.18, wet);
    return DmS(c, rough, 0.0, h * (1.0 - wet));
}

fn dm_pavement(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let size = 0.6;
    let q = uv / size;
    let cell = floor(q);
    let f = fract(q);
    let id = dm_h21(cell);
    var c = tint * (0.86 + 0.24 * id);
    c *= 0.9 + 0.2 * dm_fbm(uv * 1.3, fw * 1.3);
    let grain = mix(0.5, dm_noise(uv * 70.0), dm_fade(0.015, fw));
    c *= 0.92 + 0.16 * grain;
    // Grout: the joints sit low and gather dirt.
    let e = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y)) * size;
    let joint = 1.0 - smoothstep(0.004, 0.01 + fw * 0.6, e);
    let dirt = 1.0 - smoothstep(0.0, 0.06, e);
    c = mix(c, DM_EARTH * 1.4, dirt * 0.35);
    c = mix(c, DM_EARTH * 0.6, joint);
    var h = -joint * 0.006 + (grain - 0.5) * 0.001;
    // A few slabs cracked across, a few sunk and stained.
    if id > 0.86 {
        let a = 6.2831 * dm_h21(cell + 9.0);
        let d = abs(dot(f - 0.5, vec2<f32>(cos(a), sin(a))) + 0.1 * (dm_noise(q * 6.0) - 0.5)) * size;
        let crack = (1.0 - smoothstep(0.002, 0.006 + fw * 0.5, d)) * dm_fade(0.01, fw);
        c = mix(c, DM_EARTH * 0.5, crack);
        h -= crack * 0.005;
    } else if id < 0.08 {
        c *= 0.7;
        h -= 0.004;
    }
    let stain = smoothstep(0.62, 0.75, dm_fbm(uv * 0.5 + 13.0, fw * 0.5));
    c *= 1.0 - 0.35 * stain;
    return DmS(c, 0.9 - 0.2 * stain, 0.0, h);
}

fn dm_grass(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let big = dm_fbm(uv * 0.18, fw * 0.18);
    let mid = dm_fbm(uv * 0.9 + 3.0, fw * 0.9);
    // Dry straw, dark lush tufts and bare earth.
    let straw = vec3<f32>(dm_luma(tint) * 1.5, dm_luma(tint) * 1.3, dm_luma(tint) * 0.6);
    var c = mix(tint * 0.75, straw, smoothstep(0.45, 0.75, big) * 0.6);
    c = mix(c, tint * 0.55, smoothstep(0.55, 0.75, mid) * 0.7);
    let bare = smoothstep(0.62, 0.7, dm_fbm(uv * 0.35 + 21.0, fw * 0.35));
    let earth = DM_EARTH * (0.8 + 0.4 * mid);
    c = mix(c, earth, bare);
    // Blades: bright and dark specks at a few centimetres.
    let blade = mix(0.5, dm_noise(uv * vec2<f32>(48.0, 48.0)), dm_fade(0.03, fw));
    let blade2 = mix(0.5, dm_noise(uv * 120.0 + 5.0), dm_fade(0.012, fw));
    c *= 0.7 + 0.45 * blade * (1.0 - bare * 0.6) + 0.15 * (blade2 - 0.5);
    let h = (blade - 0.5) * 0.008 * (1.0 - bare);
    return DmS(c, 0.95, 0.0, h);
}

fn dm_dirt(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    var c = tint * (0.75 + 0.5 * dm_fbm(uv * 0.4, fw * 0.4));
    let fine = mix(0.5, dm_noise(uv * 40.0), dm_fade(0.03, fw));
    c *= 0.85 + 0.3 * fine;
    // Pebbles.
    let pc = dm_cells(uv * 14.0);
    let pebble = (1.0 - smoothstep(0.12, 0.28, pc.x + 0.1 * dm_noise(uv * 40.0))) * step(0.7, pc.z) * dm_fade(0.04, fw);
    c = mix(c, mix(tint, vec3<f32>(dm_luma(tint)), 0.6) * (0.8 + 0.7 * fract(pc.z * 7.0)), pebble * 0.7);
    // Ruts along the way (x) and damp in them.
    let rut = smoothstep(0.55, 0.85, dm_noise(vec2<f32>(uv.x * 0.15, uv.y * 1.6)));
    c *= 1.0 - 0.3 * rut;
    let h = (fine - 0.5) * 0.004 + pebble * 0.006 - rut * 0.01;
    return DmS(c, 0.95 - 0.25 * rut, 0.0, h);
}

// Running bond: x along the wall, y up. `local_y` is 0 at the foot of the
// wall piece and 1 at its top.
fn dm_brick_pattern(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let bw = 0.225;
    let bh = 0.075;
    let row = floor(uv.y / bh);
    let off = select(0.0, 0.5, (i32(row) & 1) == 1);
    let col = floor(uv.x / bw + off);
    let f = vec2<f32>(fract(uv.x / bw + off), fract(uv.y / bh));
    let id = dm_h21(vec2<f32>(col, row));
    var c = tint * (0.78 + 0.4 * id);
    if id > 0.9 { c = tint * 0.45; } // over-fired, dark
    c = mix(c, dm_shade(tint, 1.1, 0.4), step(id, 0.07)); // a pale one
    let pit = mix(0.5, dm_noise(uv * 90.0), dm_fade(0.012, fw));
    c *= 0.88 + 0.24 * pit;
    let e = min(min(f.x, 1.0 - f.x) * bw, min(f.y, 1.0 - f.y) * bh);
    let mortar = 1.0 - smoothstep(0.004, 0.009 + fw * 0.5, e);
    let m = mortar * dm_fade(0.01, fw);
    c = mix(c, DM_MORTAR * (0.8 + 0.4 * pit), m);
    // With distance the joints average into the colour.
    c = mix(c, mix(tint, DM_MORTAR, 0.15), 1.0 - dm_fade(0.02, fw));
    return DmS(c, 0.92, 0.0, -m * 0.006 + (pit - 0.5) * 0.0015);
}

// Grime on a wall: rising damp at the foot, rain streaks down from the
// (broken) top, soot in patches.
fn dm_weather(c: vec3<f32>, uv: vec2<f32>, ly: f32, fw: f32) -> vec3<f32> {
    let damp = 1.0 - smoothstep(0.0, 0.55 + 0.3 * dm_noise(vec2<f32>(uv.x * 0.8, 3.0)), uv.y);
    var o = c * (1.0 - 0.45 * damp);
    let streak = dm_noise(vec2<f32>(uv.x * 7.0, uv.y * 0.35)) * smoothstep(0.35, 1.0, ly);
    o *= 1.0 - 0.35 * smoothstep(0.55, 0.85, streak);
    let soot = smoothstep(0.6, 0.78, dm_fbm(uv * vec2<f32>(0.6, 0.3) + 41.0, fw * 0.6));
    o = mix(o, DM_SOOT, soot * 0.7);
    return o;
}

fn dm_brick(uv: vec2<f32>, ly: f32, tint: vec3<f32>, fw: f32) -> DmS {
    var s = dm_brick_pattern(uv, tint, fw);
    s.albedo = dm_weather(s.albedo, uv, ly, fw);
    return s;
}

fn dm_plaster(uv: vec2<f32>, ly: f32, tint: vec3<f32>, fw: f32) -> DmS {
    var c = tint * (0.85 + 0.3 * dm_fbm(uv * 0.7, fw * 0.7));
    let stucco = mix(0.5, dm_noise(uv * 45.0), dm_fade(0.03, fw));
    c *= 0.9 + 0.2 * stucco;
    // Hairline cracks, and patches where the render fell off the bricks.
    let crack = dm_cracks(uv, 1.3, 0.005, fw, 0.3);
    c = mix(c, tint * 0.35, crack);
    let n = dm_fbm(uv * 0.55 + 9.0, fw * 0.55);
    let bare = smoothstep(0.66, 0.68, n);
    let edge = smoothstep(0.62, 0.66, n) - bare;
    var h = (stucco - 0.5) * 0.003 - crack * 0.004;
    if bare > 0.0 {
        let b = dm_brick_pattern(uv, DM_BRICK, fw);
        c = mix(c, b.albedo, bare);
        h = mix(h, b.h - 0.01, bare);
    }
    c *= 1.0 - 0.3 * edge;
    c = dm_weather(c, uv, ly, fw);
    return DmS(c, 0.93, 0.0, h);
}

fn dm_concrete(uv: vec2<f32>, ly: f32, side: bool, tint: vec3<f32>, fw: f32) -> DmS {
    var c = tint * (0.8 + 0.4 * dm_fbm(uv * 0.5, fw * 0.5));
    let fine = mix(0.5, dm_noise(uv * 60.0), dm_fade(0.02, fw));
    c *= 0.9 + 0.2 * fine;
    // Air pits.
    let pc = dm_cells(uv * 30.0);
    let pit = (1.0 - smoothstep(0.08, 0.14, pc.x)) * step(0.75, pc.z) * dm_fade(0.02, fw);
    c *= 1.0 - 0.5 * pit;
    var h = (fine - 0.5) * 0.002 - pit * 0.003;
    if side {
        // Form-work joints every 60 cm.
        let lift = abs(fract(uv.y / 0.6 + 0.5) - 0.5) * 0.6;
        let line = (1.0 - smoothstep(0.003, 0.008 + fw, lift)) * dm_fade(0.01, fw);
        c *= 1.0 - 0.3 * line;
        h -= line * 0.003;
        c = dm_weather(c, uv, ly, fw);
    }
    let crack = dm_cracks(uv, 1.4, 0.008, fw, 0.35);
    c = mix(c, tint * 0.3, crack);
    h -= crack * 0.006;
    return DmS(c, 0.9, 0.0, h);
}

// Planks along x (uv.x), `w` wide.
fn dm_wood(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let w = 0.14;
    let row = floor(uv.y / w);
    let id = dm_h21(vec2<f32>(row, 1.0));
    // Boards end at staggered joints.
    let len = 1.6 + 1.4 * id;
    let along = uv.x / len + id * 3.0;
    let board = floor(along);
    let bid = dm_h21(vec2<f32>(row, board));
    var c = tint * (0.75 + 0.45 * bid);
    let grain = dm_noise(vec2<f32>(uv.x * 2.5, uv.y * 90.0 + bid * 40.0));
    let fine = mix(0.5, grain, dm_fade(0.02, fw));
    c *= 0.78 + 0.4 * fine;
    let knot = dm_cells(vec2<f32>(uv.x * 1.3, uv.y * 7.0));
    let k = (1.0 - smoothstep(0.05, 0.12, knot.x)) * step(0.85, knot.z) * dm_fade(0.03, fw);
    c *= 1.0 - 0.6 * k;
    let fy = fract(uv.y / w);
    let gap = 1.0 - smoothstep(0.004, 0.01 + fw * 0.6, min(fy, 1.0 - fy) * w);
    let fx = fract(along);
    let end = 1.0 - smoothstep(0.003, 0.008 + fw * 0.6, min(fx, 1.0 - fx) * len);
    let joint = max(gap, end) * dm_fade(0.01, fw);
    c = mix(c, c * 0.15, joint);
    // Weathered grey on old wood.
    c = mix(c, vec3<f32>(dm_luma(c)) * 1.1, 0.25 * dm_fbm(uv * 0.8, fw));
    return DmS(c, 0.85, 0.0, -joint * 0.006 + (fine - 0.5) * 0.002);
}

fn dm_logs(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let d = 0.24;
    let row = floor(uv.y / d);
    let f = fract(uv.y / d);
    let id = dm_h21(vec2<f32>(row, 7.0));
    let round = sqrt(max(0.0, 1.0 - (2.0 * f - 1.0) * (2.0 * f - 1.0)));
    var c = tint * (0.7 + 0.4 * id) * (0.55 + 0.45 * round);
    let bark = mix(0.5, dm_noise(vec2<f32>(uv.x * 4.0, uv.y * 60.0)), dm_fade(0.02, fw));
    c *= 0.75 + 0.5 * bark;
    let chink = 1.0 - smoothstep(0.0, 0.12, min(f, 1.0 - f));
    c = mix(c, DM_EARTH * 0.5, chink * 0.8);
    return DmS(c, 0.9, 0.0, round * 0.03 + (bark - 0.5) * 0.004);
}

fn dm_rusty(c0: vec3<f32>, uv: vec2<f32>, amount: f32, fw: f32) -> DmS {
    // Rust eats in from blotches; their rims are flaking paint.
    let n = dm_fbm(uv * 1.4, fw * 1.4) * 0.75 + dm_fbm(uv * 5.0 + 3.0, fw * 5.0) * 0.25;
    let r = smoothstep(1.0 - amount - 0.03, 1.0 - amount + 0.03, n);
    let rim = smoothstep(1.0 - amount - 0.07, 1.0 - amount - 0.02, n) - r;
    let fine = mix(0.5, dm_noise(uv * 50.0), dm_fade(0.02, fw));
    // Rust in fine streaks and pits, not in big dark and light blobs.
    let tone = dm_fbm(vec2<f32>(uv.x * 7.0, uv.y * 2.5) + 11.0, fw * 7.0);
    let pits = mix(0.5, dm_noise(uv * 18.0 + 2.0), dm_fade(0.04, fw));
    let rust = mix(mix(DM_RUST_DARK, DM_RUST, 0.4), DM_RUST, smoothstep(0.2, 0.85, tone)) * (0.7 + 0.35 * fine + 0.25 * pits);
    var c = c0 * (0.85 + 0.3 * dm_fbm(uv * 0.7, fw * 0.7));
    c = mix(c, rust, r);
    c = mix(c, c0 * 0.4 + DM_RUST_DARK, rim * 0.7);
    // A few scratches in the paint.
    let sc = dm_noise(vec2<f32>(uv.x * 22.0 + uv.y * 3.0, uv.y * 1.2));
    let scratch = smoothstep(0.965, 0.99, sc) * (1.0 - r) * dm_fade(0.01, fw);
    c = mix(c, vec3<f32>(0.18), scratch * 0.4);
    let metal = (1.0 - r) * 0.5;
    return DmS(c, mix(0.5, 0.95, r), metal, r * 0.002 + rim * 0.001 + (fine - 0.5) * 0.002 * r);
}

fn dm_roof(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    var c = tint * (0.8 + 0.4 * dm_fbm(uv * 0.4, fw * 0.4));
    let strip = abs(fract(uv.x / 0.95) - 0.5) * 0.95;
    let seam = (1.0 - smoothstep(0.43, 0.47, strip)) ;
    let lap = smoothstep(0.44, 0.47, strip) * dm_fade(0.02, fw);
    c *= 1.0 + 0.25 * lap;
    let grit = mix(0.5, dm_noise(uv * 80.0), dm_fade(0.015, fw));
    c *= 0.85 + 0.3 * grit;
    let wet = smoothstep(0.62, 0.68, dm_fbm(uv * 0.3 + 3.0, fw * 0.3));
    c *= 1.0 - 0.4 * wet;
    let moss = smoothstep(0.65, 0.8, dm_fbm(uv * 0.8 + 17.0, fw * 0.8));
    c = mix(c, vec3<f32>(0.04, 0.06, 0.025), moss * 0.6);
    return DmS(c, mix(0.9, 0.1, wet), 0.0, lap * 0.004 + (grit - 0.5) * 0.002 + seam * 0.0);
}

fn dm_foliage(p: vec3<f32>, uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    // Clumps of leaves (3D, so balls work), each leaf a small cell.
    let a = dm_noise3(p * 2.2) * 0.65 + dm_noise3(p * 5.0 + 3.0) * 0.35;
    let clump = smoothstep(0.35, 0.65, a);
    let leaf = dm_cells(uv * 11.0);
    let k = dm_fade(0.05, fw);
    let lit = mix(0.5, 1.0 - leaf.x, k);
    var c = tint * (0.5 + 0.7 * clump) * (0.7 + 0.6 * lit) * (0.85 + 0.3 * mix(0.5, leaf.z, k));
    // Some leaves turned yellow, dark holes between the clumps.
    let turned = smoothstep(0.7, 0.85, dm_noise3(p * 1.3 + 9.0)) * step(0.6, leaf.z);
    c = mix(c, vec3<f32>(dm_luma(tint) * 1.7, dm_luma(tint) * 1.25, dm_luma(tint) * 0.35), turned * 0.7 * k);
    let gap = 1.0 - smoothstep(0.25, 0.45, a);
    c *= 1.0 - 0.75 * gap;
    let edge = (1.0 - smoothstep(0.0, 0.08, leaf.y)) * k;
    c *= 1.0 - 0.4 * edge;
    return DmS(c, 0.8, 0.0, clump * 0.05 + lit * 0.01 * k - gap * 0.03);
}

fn dm_cloth(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    // Coarse sacking: a weave of thick fibres, lumpy fill, dust and seams.
    let k = dm_fade(0.008, fw);
    let wx = abs(fract(uv.x * 60.0) - 0.5);
    let wy = abs(fract(uv.y * 60.0) - 0.5);
    let weave = mix(0.5, 0.5 + 0.8 * (wx - wy), k);
    let fibre = mix(0.5, dm_noise(vec2<f32>(uv.x * 140.0, uv.y * 12.0)), dm_fade(0.01, fw));
    let lump = dm_fbm(uv * 4.0, fw * 4.0);
    var c = tint * (0.75 + 0.35 * weave) * (0.85 + 0.3 * fibre) * (0.65 + 0.6 * lump);
    c = mix(c, DM_EARTH * 1.5, smoothstep(0.55, 0.8, dm_fbm(uv * 0.9 + 5.0, fw)) * 0.5);
    let seam = (1.0 - smoothstep(0.004, 0.01 + fw, abs(fract(uv.x / 0.5) - 0.5) * 0.5 - 0.235)) * dm_fade(0.01, fw);
    c *= 1.0 - 0.35 * seam;
    return DmS(c, 0.97, 0.0, (weave - 0.5) * 0.003 + lump * 0.02);
}

fn dm_glass(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    var c = tint * 0.6;
    let dust = dm_fbm(uv * 1.5, fw * 1.5);
    c += vec3<f32>(0.01, 0.01, 0.009) * dust;
    // Some panes cracked: bright lines, the rest whole but filthy.
    let pane = dm_h21(floor(uv / 1.1));
    let crack = dm_cracks(uv, 0.35, 0.004, fw, 0.6) * step(0.6, pane);
    c = mix(c, vec3<f32>(0.12, 0.12, 0.11), crack);
    return DmS(c, mix(0.04, 0.35, dust * dust) + crack * 0.4, 0.0, -crack * 0.002);
}

// Roof tiles in courses across the slope (uv.y runs down it), each course
// lapping the one below; tiles a shade apart, some slipped or gone, moss
// in the laps and soot near the ridge.
fn dm_tiles(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let course = 0.3;
    let tw = 0.24;
    let row = floor(uv.y / course);
    let fy = fract(uv.y / course);
    let qx = uv.x / tw + 0.5 * (row % 2.0);
    let col = floor(qx);
    let fx = fract(qx);
    let id = dm_h21(vec2<f32>(col, row));
    let k = dm_fade(0.05, fw);
    var c = tint * (0.8 + 0.4 * mix(0.5, id, k)) * (0.85 + 0.3 * dm_fbm(uv * 0.5, fw * 0.5));
    // Each tile rises from its top (under the course above) to its lip.
    let lip = smoothstep(0.82, 0.97, fy);
    let ramp = fy * 0.012 - lip * 0.012;
    // The gap between tiles of a course; the shadow under each lip.
    let gap = (1.0 - smoothstep(0.0, 0.06, min(fx, 1.0 - fx))) * k;
    let shadow = (1.0 - smoothstep(0.0, 0.15, fy)) * k;
    c *= 1.0 - 0.45 * gap - 0.5 * shadow;
    // Rounded profile across each tile (pantiles).
    let roll = sin(fx * 3.14159) * 0.006 * k;
    // Missing tiles: a dark hole onto the battens.
    let gone = step(0.993, dm_h21(vec2<f32>(col * 1.7 + 3.0, row * 2.3)));
    c = mix(c, vec3<f32>(0.02, 0.016, 0.012), gone);
    let moss = smoothstep(0.55, 0.8, dm_fbm(uv * 0.7 + 11.0, fw * 0.7)) * (0.5 + 0.5 * shadow);
    c = mix(c, vec3<f32>(0.05, 0.07, 0.025), moss * 0.55);
    let lichen = step(0.93, dm_noise(uv * 9.0 + 4.0)) * k;
    c = mix(c, vec3<f32>(0.25, 0.24, 0.16), lichen * 0.4);
    return DmS(c, 0.75 - 0.2 * id * k, 0.0, (ramp + roll) * (1.0 - gone) - gone * 0.01);
}

// Bark: ridges running up the trunk, furrows between, lichen in patches.
// uv.x runs round (along the face), uv.y up.
fn dm_bark(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let k = dm_fade(0.03, fw);
    let q = vec2<f32>(uv.x * 9.0 + dm_noise(uv * vec2<f32>(3.0, 0.6)) * 2.5, uv.y * 1.3);
    let ridge = dm_noise(q * vec2<f32>(1.0, 1.0)) * 0.7 + dm_noise(q * vec2<f32>(2.1, 3.0) + 5.0) * 0.3;
    let furrow = smoothstep(0.35, 0.6, ridge);
    var c = tint * mix(0.45, 1.15, mix(0.6, furrow, k));
    c *= 0.85 + 0.3 * dm_fbm(uv * 1.5, fw * 1.5);
    let lichen = smoothstep(0.62, 0.75, dm_fbm(uv * 1.2 + 21.0, fw * 1.2));
    c = mix(c, vec3<f32>(0.2, 0.24, 0.13), lichen * 0.5);
    return DmS(c, 0.92, 0.0, furrow * 0.012 * k);
}

// Floor tiles indoors: squares with grout, each a shade apart, cracked
// here and there, grimy at the edges of the room.
fn dm_floortile(uv: vec2<f32>, tint: vec3<f32>, fw: f32) -> DmS {
    let size = 0.33;
    let q = uv / size;
    let cell = floor(q);
    let f = fract(q);
    let k = dm_fade(0.03, fw);
    let id = dm_h21(cell + 7.0);
    var c = tint * (0.88 + 0.24 * mix(0.5, id, k));
    let e = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
    let grout = (1.0 - smoothstep(0.015, 0.04 + fw / size, e)) * k;
    c = mix(c, tint * 0.45, grout);
    let crack = dm_cracks(uv, 0.5, 0.003, fw, 0.25);
    c = mix(c, tint * 0.3, crack);
    let grime = smoothstep(0.45, 0.8, dm_fbm(uv * 0.8 + 3.0, fw * 0.8));
    c = mix(c, DM_EARTH, grime * 0.45);
    return DmS(c, mix(0.35, 0.8, max(grime, grout)), 0.0, -grout * 0.002 - crack * 0.002);
}

// Kinds from 64 up are the same materials indoors, under a roof that is
// gone: the sky that still gets in is mostly blocked by the walls and the
// ceiling no camera sees (lib/mat). Darker in every light, so a room is dark
// until a torch is in it.
const DM_INDOOR: f32 = 0.28;
// Bit 128: a room floor nobody has looked into yet (fog of war, main.luau).
// A hint, not a black box: a shade darker and greyer.
const DM_UNSEEN: f32 = 0.6;

fn game_surface(s: SurfaceIn) -> SurfaceOut {
    let kind = s.kind & 63u;
    let indoor = (s.kind & 64u) != 0u;
    let unseen = (s.kind & 128u) != 0u;
    let n = s.n;
    let an = abs(n);
    let top = an.y > max(an.x, an.z);
    // Along the face: x along a wall or the ground's x, y up a wall or the
    // ground's z.
    var t1 = vec3<f32>(1.0, 0.0, 0.0);
    var t2 = vec3<f32>(0.0, 0.0, 1.0);
    if !top {
        t2 = vec3<f32>(0.0, 1.0, 0.0);
        if an.x > an.z { t1 = vec3<f32>(0.0, 0.0, 1.0); }
    }
    let fw = max(max(length(s.dx), length(s.dy)), 0.0005);
    let ly = clamp(s.local.y + 0.5, 0.0, 1.0);
    let eps = max(0.006, fw * 0.75);
    var o = DmS(s.tint, s.rough, s.metal, 0.0);
    var hx = 0.0;
    var hy = 0.0;
    var bump = 1.0;
    for (var k = 0; k < 3; k++) {
        let q = s.p + t1 * select(0.0, eps, k == 1) + t2 * select(0.0, eps, k == 2);
        let uv = vec2<f32>(dot(q, t1), dot(q, t2));
        var r = DmS(s.tint, s.rough, s.metal, 0.0);
        switch kind {
            case 0u: { r = dm_asphalt(uv, s.tint, fw); }
            case 1u: { r = dm_pavement(uv, s.tint, fw); }
            case 2u: { if top { r = dm_grass(uv, s.tint, fw); } else { r = dm_dirt(uv, DM_EARTH * 1.6, fw); } }
            case 3u: { r = dm_dirt(uv, s.tint, fw); }
            case 4u: { if top { r = dm_concrete(uv, ly, false, vec3<f32>(dm_luma(s.tint)) * 1.3, fw); } else { r = dm_brick(uv, ly, s.tint, fw); } }
            case 5u: { if top { r = dm_concrete(uv, ly, false, s.tint * 0.8, fw); } else { r = dm_plaster(uv, ly, s.tint, fw); } }
            case 6u: { r = dm_concrete(uv, ly, !top, s.tint, fw); }
            case 7u: { r = dm_wood(uv, s.tint, fw); }
            case 8u: { r = dm_rusty(s.tint, uv, 0.12 + 0.3 * (1.0 - ly) * (1.0 - ly), fw); }
            case 9u: {
                // Burnt shells (dark tints) are charred with rust coming through.
                let burnt = 1.0 - smoothstep(0.01, 0.03, dm_luma(s.tint));
                r = dm_rusty(s.tint, uv, mix(0.62, 0.5, burnt), fw);
                r.albedo = mix(r.albedo, r.albedo * vec3<f32>(0.35, 0.3, 0.28), burnt);
            }
            case 10u: { if top { r = dm_roof(uv, s.tint, fw); } else { r = dm_concrete(uv, ly, true, s.tint * 1.4, fw); } }
            case 11u: { r = dm_foliage(q, uv, s.tint, fw); }
            case 12u: { r = dm_cloth(uv, s.tint, fw); }
            case 13u: {
                r = DmS(s.tint * (0.8 + 0.4 * dm_fbm(uv * 3.0, fw * 3.0)), 0.8, 0.0, 0.0);
            }
            case 14u: { r = dm_glass(uv, s.tint, fw); }
            case 15u: {
                // Car paint: gloss going dull and rusty towards the sills.
                r = dm_rusty(s.tint, uv, 0.04 + 0.35 * (1.0 - ly) * (1.0 - ly) * (1.0 - ly), fw);
                if top { r = dm_rusty(s.tint, uv, 0.05, fw); }
                let grime = (1.0 - ly) * 0.5 + 0.3 * dm_fbm(uv * 0.8, fw);
                r.albedo = mix(r.albedo, DM_EARTH, grime * 0.5);
                r.rough = mix(0.3, r.rough, grime);
            }
            case 16u: { if top { r = dm_wood(uv, s.tint, fw); } else { r = dm_logs(uv, s.tint, fw); } }
            case 19u: { r = dm_floortile(uv, s.tint, fw); }
            case 20u: { r = DmS(s.tint * (0.9 + 0.2 * dm_fbm(uv * 2.0, fw * 2.0)), s.rough, s.metal, 0.0); }
            case 18u: { if top { r = dm_wood(uv, s.tint, fw); } else { r = dm_bark(uv, s.tint, fw); } }
            case 17u: { if top { r = dm_tiles(select(uv, vec2<f32>(uv.x, -uv.y), n.z < 0.0), s.tint, fw); } else { r = DmS(s.tint * 0.7, 0.8, 0.0, 0.0); } }
            default: { bump = 0.0; }
        }
        if k == 0 { o = r; } else if k == 1 { hx = r.h; } else { hy = r.h; }
    }
    // The height's slope along the face tilts the normal.
    let gx = (hx - o.h) / eps;
    let gy = (hy - o.h) / eps;
    let nn = normalize(n - (t1 * gx + t2 * gy) * bump * 0.8);
    if indoor { o.albedo *= DM_INDOOR; }
    if unseen {
        let grey = dot(o.albedo, vec3<f32>(0.3, 0.59, 0.11));
        o.albedo = mix(vec3<f32>(grey), o.albedo, 0.45) * DM_UNSEEN;
    }
    return SurfaceOut(max(o.albedo, vec3<f32>(0.0)), nn, clamp(o.rough, 0.03, 1.0), clamp(o.metal, 0.0, 1.0), vec3<f32>(0.0));
}
