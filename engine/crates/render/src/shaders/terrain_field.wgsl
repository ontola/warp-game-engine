// The procedural terrain function. terrain_field.rs mirrors every line in
// Rust (physics, LOD bounds, tests); keep the two in step. Only +, -, *, /,
// sqrt, floor and integer hashing, so the CPU side is bit-exact on every
// platform. `cut` is the grid spacing the caller samples at: features
// shorter than about two cells are faded out, so coarse patches are
// filtered, not aliased, and neighbours sampling one edge at one cut agree.
//
// The ground is a stack of shapes: eroded hills and mountains, then rivers
// and streams that cut their own valleys, bedrock tors, boulders, cliff
// strata, and fine relief last.

struct TerrainSample {
    pos: vec3<f32>,
    // Up component of the large-scale surface normal.
    up: f32,
    // 0..1 exposed bedrock and boulders.
    rock: f32,
    // 0..1 loose stone: scree under cliffs, gravel in stream beds.
    scree: f32,
    // 0..1 damp ground near water.
    wet: f32,
    // Metres of water above the bed (0 on dry land).
    water: f32,
    // 0..1 vegetation kind: 0 short grass, 1 heather and moss.
    moor: f32,
};

const TF_BASE: f32 = 2048.0;

fn tf_hash(ix: i32, iz: i32, seed: u32) -> u32 {
    let h = (bitcast<u32>(ix) * 0x8da6b343u) ^ (bitcast<u32>(iz) * 0xd8163841u) ^ (seed * 0xcb1ab31fu);
    let s = h * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

fn tf_grad(ix: i32, iz: i32, seed: u32) -> vec2<f32> {
    let h = tf_hash(ix, iz, seed);
    return vec2<f32>(f32(h & 0xffffu) / 32767.5 - 1.0, f32(h >> 16u) / 32767.5 - 1.0);
}

// Gradient noise with its analytic derivative: (value, d/dx, d/dz).
fn tf_noised(p: vec2<f32>, seed: u32) -> vec3<f32> {
    let fl = floor(p);
    let ix = i32(fl.x);
    let iz = i32(fl.y);
    let f = p - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let ga = tf_grad(ix, iz, seed);
    let gb = tf_grad(ix + 1, iz, seed);
    let gc = tf_grad(ix, iz + 1, seed);
    let gd = tf_grad(ix + 1, iz + 1, seed);
    let va = ga.x * f.x + ga.y * f.y;
    let vb = gb.x * (f.x - 1.0) + gb.y * f.y;
    let vc = gc.x * f.x + gc.y * (f.y - 1.0);
    let vd = gd.x * (f.x - 1.0) + gd.y * (f.y - 1.0);
    let k = va - vb - vc + vd;
    let v = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    let dx = ga.x + u.x * (gb.x - ga.x) + u.y * (gc.x - ga.x) + u.x * u.y * (ga.x - gb.x - gc.x + gd.x) + du.x * (u.y * k + vb - va);
    let dz = ga.y + u.x * (gb.y - ga.y) + u.y * (gc.y - ga.y) + u.x * u.y * (ga.y - gb.y - gc.y + gd.y) + du.y * (u.x * k + vc - va);
    return vec3<f32>(v, dx, dz);
}

fn tf_smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Weight of a feature `size` metres long at grid spacing `cut`.
fn tf_fade(size: f32, cut: f32) -> f32 {
    return clamp(size / (2.0 * cut) - 1.0, 0.0, 1.0);
}

// Metres to the zero line of a noise field sampled at p * freq (p in
// TF_BASE units): |value| / |gradient|.
fn tf_line(n: vec3<f32>, freq: f32) -> f32 {
    let g = sqrt(n.y * n.y + n.z * n.z) * (freq / TF_BASE) + 1e-6;
    return min(abs(n.x) / g, 1000.0);
}

fn tf_rotate(q: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(1.6 * q.x - 1.2 * q.y, 1.2 * q.x + 1.6 * q.y);
}

// Half-buried boulders on a jittered 7 m grid, flat-topped and a little
// elongated: (height, coverage).
fn tf_boulders(x: f32, z: f32, density: f32, cut: f32, seed: u32) -> vec2<f32> {
    let cell = 7.0;
    let cx = i32(floor(x / cell));
    let cz = i32(floor(z / cell));
    var best = 0.0;
    var cover = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let h = tf_hash(cx + i, cz + j, seed + 77u);
            let keep = f32(h & 1023u) / 1023.0;
            if keep >= density { continue; }
            let h2 = tf_hash(cx + i, cz + j, seed + 78u);
            let ox = (f32(cx + i) + 0.15 + 0.7 * f32((h >> 10u) & 255u) / 255.0) * cell;
            let oz = (f32(cz + j) + 0.15 + 0.7 * f32((h >> 18u) & 255u) / 255.0) * cell;
            let k = f32((h >> 26u) & 63u) / 63.0;
            let r = 0.4 + 1.5 * k * k;
            let w = tf_fade(r * 2.0, cut);
            if w <= 0.0 { continue; }
            let dx = (x - ox) * (0.75 + 0.5 * f32(h2 & 255u) / 255.0);
            let dz = (z - oz) * (0.75 + 0.5 * f32((h2 >> 8u) & 255u) / 255.0);
            let s = (dx * dx + dz * dz) / (r * r);
            if s < 1.0 {
                let b = r * (0.4 + 0.3 * f32((h2 >> 16u) & 255u) / 255.0) * (1.0 - s * s) * w;
                best = max(best, b);
                cover = max(cover, tf_smooth(0.0, 0.3, 1.0 - s) * w);
            }
        }
    }
    return vec2<f32>(best, cover);
}

fn terrain_eval(x: f32, z: f32, cut: f32, seed: u32) -> TerrainSample {
    let p = vec2<f32>(x, z) / TF_BASE;
    // Regions, kilometres across: lowland or mountain, soil or bedrock.
    let region = tf_noised(p * 0.25 + vec2<f32>(3.1, 7.7), seed + 1u).x + 0.5 * tf_noised(p * 0.5 + vec2<f32>(1.3, 2.9), seed + 1u).x;
    let mountains = tf_smooth(-0.1, 0.45, region);
    let bedrock = tf_smooth(-0.1, 0.3, tf_noised(p * 0.6 + vec2<f32>(5.5, 0.4), seed + 2u).x);
    let moor = tf_smooth(-0.2, 0.25, tf_noised(p * 0.9 + vec2<f32>(2.2, 8.1), seed + 6u).x);
    let warp = vec2<f32>(tf_noised(p * 1.5 + vec2<f32>(5.2, 1.3), seed + 3u).x, tf_noised(p * 1.5 + vec2<f32>(1.7, 9.2), seed + 3u).x);
    var q = p + warp * 0.12;

    // Eroded fBm: accumulated slope damps finer octaves, so peaks stay
    // sharp and valleys smooth. 2 km down to 32 m.
    let amp = 60.0 + 640.0 * mountains;
    var h = 0.0;
    var low = 0.0;
    var low_slope = vec2<f32>(0.0);
    var b = 1.0;
    var d = vec2<f32>(0.0);
    var slope = vec2<f32>(0.0);
    // q = m * p (plus warp): the octave's slope is m^T times its gradient.
    var m = vec4<f32>(1.0, 0.0, 0.0, 1.0);
    var wl = TF_BASE;
    for (var i = 0; i < 7; i++) {
        let w = tf_fade(wl, cut);
        let n = tf_noised(q, seed + 10u + u32(i));
        // Erosion and the slope estimate use the unfolded gradient: the fold
        // kinks it, and a kink there would step the height.
        d += n.yz;
        let e = 1.0 / (1.0 + d.x * d.x + d.y * d.y);
        slope += vec2<f32>(m.x * n.y + m.z * n.z, m.y * n.y + m.w * n.z) * (b * e * w);
        // Mountains fold the middle octaves into sharp ridges and aretes.
        let k = select(0.0, 0.8 * mountains, i >= 1 && i <= 4);
        h += b * (n.x + k * (0.4 - abs(n.x) - n.x)) * e * w;
        // Valley floors follow the unfolded kilometre shapes, which match
        // the slope estimate.
        low += b * n.x * e * w * select(0.0, 1.0, i <= 2);
        if i == 2 {
            low_slope = slope;
        }
        b *= 0.5;
        m = vec4<f32>(1.6 * m.x - 1.2 * m.z, 1.6 * m.y - 1.2 * m.w, 1.2 * m.x + 1.6 * m.z, 1.2 * m.y + 1.6 * m.w);
        wl *= 0.5;
        q = tf_rotate(q);
    }
    var y = h * amp;
    var g = slope * (amp / TF_BASE);
    let g0 = g;
    let up0 = 1.0 / sqrt(1.0 + g.x * g.x + g.y * g.y);

    // Crags: bedrock mountainsides break into cliff bands and grassy
    // benches, about 24 m apart.
    let crag = bedrock * tf_smooth(0.2, 0.7, mountains) * tf_smooth(0.97, 0.85, up0) * tf_fade(24.0, cut);
    if crag > 0.0 {
        let s = y / 24.0 + 0.6 * tf_noised(p * 6.0 + warp, seed + 12u).x;
        let fl = floor(s);
        let t = s - fl;
        let tb = clamp((t - 0.6) / 0.4, 0.0, 1.0);
        let k = 0.35 + 0.65 * 6.0 * tb * (1.0 - tb) / 0.4;
        y += crag * 24.0 * (fl + 0.35 * t + 0.65 * tb * tb * (3.0 - 2.0 * tb) - s);
        g *= 1.0 + crag * (k - 1.0);
    }

    // Rivers (about 3 km apart) run on a flat valley floor, level across,
    // with walls that scale with the relief they cut through, and meander
    // inside it. The floor is the kilometre shapes at the centre line.
    let river_n = tf_noised(p * 0.7 + warp * 0.3 + vec2<f32>(9.1, 4.4), seed + 4u);
    let river = tf_line(river_n, 0.7);
    let rn = river_n.yz / (sqrt(river_n.y * river_n.y + river_n.z * river_n.z) + 1e-6) * select(-1.0, 1.0, river_n.x >= 0.0);
    let gl = low_slope * (amp / TF_BASE);
    let floor_y = (low - 0.1) * amp - min(river, 600.0) * (gl.x * rn.x + gl.y * rn.y);
    let relief = max(y - floor_y, 0.0);
    let floor_w = 20.0 + 0.12 * relief;
    let wall = 50.0 + 2.0 * relief;
    let rv = tf_smooth(floor_w, floor_w + wall, river);
    let rt = clamp((river - floor_w) / wall, 0.0, 1.0);
    let drv = 6.0 * rt * (1.0 - rt) / wall;
    y = floor_y + (y - floor_y) * rv;
    g = g * rv + rn * (relief * drv);
    let meander = tf_noised(p * 9.0 + warp, seed + 13u).x * 0.7 * floor_w;
    let ch = abs(min(river, 600.0) * select(-1.0, 1.0, river_n.x >= 0.0) - meander);
    let ch_w = 3.5 + 0.02 * relief;
    let rb = tf_smooth(0.0, ch_w, ch);
    let channel = tf_fade(1.6, cut);
    y = mix(y, min(y, floor_y - 1.3 * (1.0 - rb * rb)), channel * (1.0 - rb));
    let river_level = floor_y - 0.3;

    // Streams (about 1 km apart) only where they can run downhill: in
    // gentle ground, not across a slope. A local notch, deeper in high
    // ground, with a narrow bed and water level across.
    let stream_n = tf_noised(p * 2.1 + warp * 0.6 + vec2<f32>(3.3, 6.6), seed + 7u);
    let sn = stream_n.yz / (sqrt(stream_n.y * stream_n.y + stream_n.z * stream_n.z) + 1e-6);
    let stream_line = tf_line(stream_n, 2.1);
    let depth = clamp(0.05 * relief + 0.8, 0.8, 4.0);
    let sw = 3.0 + 5.0 * depth;
    let across = abs(g0.x * sn.x + g0.y * sn.y);
    let streams_here = tf_smooth(-0.35, -0.1, tf_noised(p * 0.8 + vec2<f32>(7.4, 2.5), seed + 8u).x)
        * tf_smooth(0.85, 0.93, up0) * tf_smooth(0.6 * depth / sw, 0.3 * depth / sw, across) * tf_smooth(floor_w * 0.5, floor_w, ch);
    let stream = mix(1000.0, stream_line, streams_here);
    let notch = tf_fade(sw, cut) * streams_here;
    let sv = tf_smooth(1.0, sw, stream_line);
    // Height at the centre line, but never above the bank here: the slope
    // estimate is rough, and water must not stand above the ground.
    let yc = min(y, y - min(stream_line, 50.0) * (g0.x * sn.x + g0.y * sn.y) * select(-1.0, 1.0, stream_n.x >= 0.0));
    let stream_level = yc - depth * notch;
    y = mix(y, min(y, yc - depth), notch * (1.0 - sv));
    let sb = tf_smooth(0.0, 1.2, stream_line);
    y = mix(y, min(y, stream_level - 0.35 * (1.0 - sb * sb)), channel * notch * (1.0 - sb));
    let open = rv * (1.0 - notch * (1.0 - sv));
    let near_water = min(max(ch - ch_w, 0.0), stream);

    // Bedrock tors: blocky outcrops with cliff sides, in groups on the
    // higher ground, away from water.
    let tor_n = tf_noised(p * 30.0 + warp * 2.0, seed + 9u);
    let tor_zone = tf_smooth(0.15, 0.4, tf_noised(p * 4.0 + vec2<f32>(6.3, 2.2), seed + 17u).x) * tf_smooth(5.0, 40.0, relief);
    let tor_fade = tf_fade(20.0, cut);
    let tor_h = (8.0 + 14.0 * mountains) * bedrock * tor_zone * tf_smooth(30.0, 80.0, near_water) * tor_fade;
    let tt = clamp((tor_n.x - 0.22) / 0.08, 0.0, 1.0);
    let tor = tt * tt * (3.0 - 2.0 * tt);
    // Horizontal sheets about 2.5 m thick and vertical joints split the
    // outcrop into blocks.
    var th = tor_h * (tor + 0.5 * max(tor_n.x - 0.3, 0.0));
    if th > 0.0 {
        let s = th / 2.5;
        let fl = floor(s);
        let tb = clamp((s - fl - 0.5) / 0.5, 0.0, 1.0);
        th = 2.5 * (fl + 0.25 * (s - fl) + 0.75 * tb * tb * (3.0 - 2.0 * tb)) * tf_fade(5.0, cut) + th * (1.0 - tf_fade(5.0, cut));
        let joint = min(tf_line(tf_noised(p * 110.0, seed + 14u), 110.0), tf_line(tf_noised(p * 70.0 + vec2<f32>(4.1, 0.7), seed + 15u), 70.0));
        th *= 1.0 - 0.6 * (1.0 - tf_smooth(0.3, 1.6, joint)) * tf_fade(3.0, cut);
    }
    y += th;
    g += tor_n.yz * (tor_h * 6.0 * tt * (1.0 - tt) / 0.08 * (30.0 / TF_BASE));
    let steep = sqrt(g.x * g.x + g.y * g.y);
    let up = 1.0 / sqrt(1.0 + steep * steep);

    var rock = clamp(tf_smooth(0.8, 0.6, up) + 0.7 * bedrock * tf_smooth(0.95, 0.85, up) + tf_smooth(0.0, 0.15, tor * tor_h) * tor_fade, 0.0, 1.0);
    let scree = tf_smooth(0.85, 0.72, up) * (1.0 - tf_smooth(0.65, 0.55, up)) * (0.4 + 0.6 * bedrock) * mountains
        + 0.9 * tf_smooth(2.0, 0.6, stream) + 0.8 * tf_smooth(ch_w + 2.5, ch_w, ch) * tf_smooth(-0.1, 0.2, tf_noised(p * 40.0, seed + 16u).x);

    // Boulders: at the foot of tors and crags, in stream and river beds.
    let tor_foot = tf_smooth(0.12, 0.2, tor_n.x) * (1.0 - tor) * bedrock;
    let boulder_density = clamp(0.02 + 0.45 * tor_foot + 0.2 * crag + 0.3 * tf_smooth(5.0, 1.0, stream) + 0.25 * tf_smooth(ch_w + 3.0, ch_w, ch), 0.0, 0.6);
    let bo = tf_boulders(x, z, boulder_density, cut, seed);
    y += bo.x;
    rock = max(rock, bo.y);

    // Fine relief, far rougher on rock; calm on valley floors.
    let rough = 0.15 + 0.85 * rock;
    var db = b * amp;
    for (var i = 7; i < 17; i++) {
        let w = tf_fade(wl, cut);
        if w <= 0.0 { break; }
        let n = tf_noised(q, seed + 10u + u32(i));
        d += n.yz;
        let e = 1.0 / (1.0 + 0.25 * (d.x * d.x + d.y * d.y));
        y += db * n.x * e * w * rough * (0.35 + 0.65 * open);
        db *= 0.5;
        wl *= 0.5;
        q = tf_rotate(q);
    }

    // Water: the bed stays where it is; the surface is level.
    let water_r = select(0.0, max(river_level - y, 0.0), ch < ch_w);
    let water_s = select(0.0, max(stream_level - y, 0.0), stream_line < 1.2 && notch > 0.0);
    let water = max(water_r, water_s) * channel;

    // Bedrock strata: ledges stick out of steep rock and overhang a little.
    var pos = vec3<f32>(x, y + water, z);
    if rock > 0.0 && steep > 0.0 {
        let layer = y / 5.5 + 0.8 * tf_noised(p * 24.0, seed + 5u).x;
        let t = layer - floor(layer);
        let ledge = tf_smooth(0.0, 0.18, t) * (1.0 - tf_smooth(0.45, 1.0, t)) - 0.35;
        let push = 1.8 * tf_smooth(0.8, 0.6, up) * ledge * tf_fade(5.5, cut);
        pos.x -= g.x / steep * push;
        pos.z -= g.y / steep * push;
    }
    var o: TerrainSample;
    o.pos = pos;
    o.up = up;
    o.rock = rock;
    o.scree = clamp(scree, 0.0, 1.0);
    o.wet = tf_smooth(8.0, 1.0, near_water);
    o.water = water;
    o.moor = moor;
    return o;
}
