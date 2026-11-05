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
    // 0..1 share of the ground round here under water at full detail,
    // for drawing (terrain_field.rs Sample::film).
    film: f32,
    // 0..1 vegetation kind: 0 short grass, 1 heather and moss.
    moor: f32,
    // 0..1 lying snow.
    snow: f32,
};

// A game designer's choices for one world (terrain_field.rs Style). 0.5 is
// the reference look for the 0..1 amounts; multipliers default to 1.
struct TerrainStyle {
    mountains: f32,
    peaks: f32,
    snow_line: f32,
    hills: f32,
    rivers: f32,
    streams: f32,
    rocks: f32,
    boulders: f32,
    moor: f32,
    grass: f32,
    dunes: f32,
    ditches: f32,
    ponds: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
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

// Weight of a feature `size` metres long at grid spacing `cut`. Cut 0 is
// full detail; x / 0 is not portable WGSL (Metal gave other ground).
fn tf_fade(size: f32, cut: f32) -> f32 {
    if cut <= 0.0 { return 1.0; }
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
// Share of ground at distance `d` from a line that a strip |d| < w covers,
// seen at grid spacing `cut` (terrain_field.rs seen).
fn tf_seen(d: f32, w: f32, cut: f32) -> f32 {
    let hw = max(w, cut);
    let e = 0.5 * cut + 0.01;
    return w / hw * tf_smooth(hw + e, hw - e, d);
}

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
            // Skewed, half-buried and rounded: a low cap with a soft foot,
            // not a bell with walls the texture smears down.
            let skew = f32((h2 >> 24u) & 255u) / 255.0 - 0.5;
            let d2 = dx * dx + dz * dz;
            let s = d2 / (r * r) * (1.0 + 0.6 * skew * dx * dz / (d2 + 1e-6));
            if s < 1.0 {
                let b = r * (0.22 + 0.2 * f32((h2 >> 16u) & 255u) / 255.0) * sqrt(1.0 - s) * tf_smooth(0.0, 0.25, 1.0 - s) * w;
                best = max(best, b);
                cover = max(cover, tf_smooth(0.0, 0.3, 1.0 - s) * w);
            }
        }
    }
    return vec2<f32>(best, cover);
}

fn terrain_eval(x: f32, z: f32, cut: f32, seed: u32, st: TerrainStyle) -> TerrainSample {
    let p = vec2<f32>(x, z) / TF_BASE;
    // Regions, kilometres across: lowland or mountain, soil or bedrock, and
    // high massifs whose peaks rise kilometres above the valleys.
    let region = tf_noised(p * 0.25 + vec2<f32>(3.1, 7.7), seed + 1u).x + 0.5 * tf_noised(p * 0.5 + vec2<f32>(1.3, 2.9), seed + 1u).x;
    let alpine = tf_smooth(0.0, 0.45, tf_noised(p * 0.15 + vec2<f32>(8.3, 1.9), seed + 18u).x + 0.3 * region + 0.8 * (st.peaks - 0.5))
        * tf_smooth(0.0, 0.1, st.peaks);
    let mountains = max(tf_smooth(-0.1, 0.45, region + 1.2 * (st.mountains - 0.5)), alpine);
    let bedrock = tf_smooth(-0.1, 0.3, tf_noised(p * 0.6 + vec2<f32>(5.5, 0.4), seed + 2u).x);
    let moor = tf_smooth(-0.2, 0.25, tf_noised(p * 0.9 + vec2<f32>(2.2, 8.1), seed + 6u).x + 1.2 * (st.moor - 0.5));
    let warp = vec2<f32>(tf_noised(p * 1.5 + vec2<f32>(5.2, 1.3), seed + 3u).x, tf_noised(p * 1.5 + vec2<f32>(1.7, 9.2), seed + 3u).x);
    var q = p + warp * 0.12;

    // Rivers (about 3 km apart) run on a flat valley floor, level across,
    // with walls that scale with the relief they cut through, and meander
    // inside it. The floor is the kilometre shapes at the centre line;
    // in the high mountains these are glacial troughs.
    let river_n = tf_noised(p * 0.7 + warp * 0.3 + vec2<f32>(9.1, 4.4), seed + 4u);
    // Only some rivers flow; `rivers` sets how many.
    let river_on = max(tf_smooth(-0.15, 0.15, 2.2 * st.rivers - 1.1 + tf_noised(p * 0.35 + vec2<f32>(2.7, 5.1), seed + 20u).x),
        tf_smooth(0.3, 0.7, alpine));
    let rl = tf_line(river_n, 0.7);
    let river = mix(1000.0, rl, river_on);
    let rn = river_n.yz / (sqrt(river_n.y * river_n.y + river_n.z * river_n.z) + 1e-6) * select(-1.0, 1.0, river_n.x >= 0.0);
    // Distance from the trough with a rounded centre: a crease there would
    // kink the slope, and stream masks read the slope.
    let ra = sqrt(rl * rl + 2500.0);
    let rs = clamp(mix(1000.0, ra, river_on) / 1400.0, 0.0, 1.0);
    let drs = rn * (river_on * rl / ra / 1400.0);
    // High peaks stand back from the troughs: the relief grows away from them.
    let pt = clamp((rs - 0.3) / 0.7, 0.0, 1.0);
    // Eroded fBm: accumulated slope damps finer octaves, so peaks stay
    // sharp and valleys smooth. 2 km down to 32 m.
    let amp = 100.0 * st.hills + 640.0 * mountains - 340.0 * alpine + 1100.0 * alpine * pt * pt * (3.0 - 2.0 * pt);
    let damp = drs * (alpine * 1100.0 * 6.0 * pt * (1.0 - pt) / 0.7);
    var h = 0.0;
    var low = 0.0;
    var low1 = 0.0;
    var low_slope = vec2<f32>(0.0);
    var low_slope1 = vec2<f32>(0.0);
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
        low1 += b * n.x * e * w * select(0.0, 1.0, i <= 1);
        if i == 1 {
            low_slope1 = slope;
        }
        if i == 2 {
            low_slope = slope;
        }
        b *= 0.5;
        m = vec4<f32>(1.6 * m.x - 1.2 * m.z, 1.6 * m.y - 1.2 * m.w, 1.2 * m.x + 1.6 * m.z, 1.2 * m.y + 1.6 * m.w);
        wl *= 0.5;
        q = tf_rotate(q);
    }
    // Massifs rise between the trough valleys, with long ridges a few
    // kilometres apart on top. Folds and rises are eased, so the slope has
    // no kink anywhere.
    let big = tf_noised(p * 0.5 + vec2<f32>(4.4, 7.3), seed + 21u);
    let fold = sqrt(big.x * big.x + 0.01);
    let fu = clamp((0.55 - fold) / 0.45, 0.0, 1.0);
    let uplift = alpine * (300.0 + 700.0 * rs * rs * (3.0 - 2.0 * rs) + 800.0 * fu * fu * (3.0 - 2.0 * fu));
    let duplift = big.yz * (-alpine * 800.0 * 6.0 * fu * (1.0 - fu) / 0.45 * big.x / fold * (0.5 / TF_BASE))
        + drs * (alpine * 700.0 * 6.0 * rs * (1.0 - rs));
    var y = h * amp + uplift;
    var g = slope * (amp / TF_BASE) + damp * h + duplift;
    let g0 = g;
    let up0 = 1.0 / sqrt(1.0 + g.x * g.x + g.y * g.y);

    // Crags: bedrock mountainsides break into cliff bands and grassy
    // benches, about 24 m apart. Bands come and go along the slope, wander
    // in height and sink under the snow, so they never read as stairs.
    let crag = min(2.0 * st.rocks, 1.0) * bedrock * tf_smooth(0.2, 0.7, mountains) * tf_smooth(0.97, 0.85, up0) * tf_fade(24.0, cut)
        * tf_smooth(-0.15, 0.35, tf_noised(p * 3.0 + warp, seed + 22u).x) * (1.0 - 0.8 * tf_smooth(st.snow_line - 300.0, st.snow_line, y));
    if crag > 0.0 {
        let s = y / 24.0 + 1.4 * tf_noised(p * 2.5 + warp, seed + 12u).x + 0.4 * tf_noised(p * 9.0, seed + 23u).x;
        let fl = floor(s);
        let t = s - fl;
        let tb = clamp((t - 0.6) / 0.4, 0.0, 1.0);
        let k = 0.35 + 0.65 * 6.0 * tb * (1.0 - tb) / 0.4;
        y += crag * 24.0 * (fl + 0.35 * t + 0.65 * tb * tb * (3.0 - 2.0 * tb) - s);
        g *= 1.0 + crag * (k - 1.0);
    }

    // High troughs keep only the broadest shapes, so their floors run level.
    let gl = mix(low_slope, low_slope1, alpine) * (amp / TF_BASE);
    let floor_y = (mix(low, low1, alpine) - 0.1) * amp + uplift
        // rn flips sign on the centre line, so extrapolate by the true
        // distance there: a faded river must not step the floor.
        - river_on * min(rl, 600.0) * (gl.x * rn.x + gl.y * rn.y);
    let relief = max(y - floor_y, 0.0);
    // Glaciers carved the high valleys into troughs: a broad flat floor,
    // walls that rise steeply and ease into shoulders (Lauterbrunnen).
    let floor_w = mix(20.0 + 0.12 * relief, 80.0 + 0.3 * relief, alpine);
    let wall = mix(50.0 + 2.0 * relief, 200.0 + 1.4 * relief, alpine);
    let rt = clamp((river - floor_w) / wall, 0.0, 1.0);
    let ru = 1.0 - rt;
    let rv = mix(rt * rt * (3.0 - 2.0 * rt), 1.0 - ru * ru * ru, alpine);
    // The floor is flat: no slope where rt is clamped at 0.
    let drv = select(0.0, mix(6.0 * rt * ru, 3.0 * ru * ru, alpine) / wall, rt > 0.0);
    y = floor_y + (y - floor_y) * rv;
    g = g * rv + rn * (relief * drv);
    let meander = tf_noised(p * 9.0 + warp, seed + 13u).x * 0.7 * floor_w;
    // Across the river by the true signed distance, so the channel has no
    // seam where the sign flips on a faded river.
    let ch = abs(min(rl, 600.0) * select(-1.0, 1.0, river_n.x >= 0.0) - meander) + 1000.0 * (1.0 - river_on);
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
    let streams_here = tf_smooth(0.4 - 1.5 * st.streams, 0.65 - 1.5 * st.streams, tf_noised(p * 0.8 + vec2<f32>(7.4, 2.5), seed + 8u).x)
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
    // Banks too narrow for this cell would leave single wet vertices.
    let near_water = min(max(ch - ch_w, 0.0), stream);
    let rf = tf_fade(2.0 * ch_w, cut);
    let near_wet = min(max(ch - ch_w, 0.0) + 1000.0 * (1.0 - rf), stream + 1000.0 * (1.0 - notch));

    // Bedrock tors: blocky outcrops with cliff sides, in groups on the
    // higher ground, away from water.
    let tor_n = tf_noised(p * 55.0 + warp * 2.0, seed + 9u);
    let tor_zone = tf_smooth(0.65 - st.rocks, 0.9 - st.rocks, tf_noised(p * 4.0 + vec2<f32>(6.3, 2.2), seed + 17u).x) * tf_smooth(5.0, 40.0, relief);
    // A tor too small for this cell blurs into a lower, broader mound (its
    // edge widens with the cell) rather than vanishing and popping back.
    let tor_fade = tf_fade(12.0, cut);
    let tor_amp = (5.0 + 9.0 * mountains) * bedrock * tor_zone * tf_smooth(30.0, 80.0, near_water);
    let tor_h = tor_amp * tf_fade(48.0, cut);
    let tor_w = 0.3 + 0.054 * cut;
    let tt = clamp((tor_n.x - 0.15) / tor_w, 0.0, 1.0);
    let tor = tt * tt * (3.0 - 2.0 * tt);
    // Horizontal sheets about 2.5 m thick and vertical joints split the
    // outcrop into blocks.
    var th = tor_h * (tor + 0.5 * max(tor_n.x - 0.3, 0.0) * tor_fade);
    if th > 0.0 {
        let s = th / 2.5;
        let fl = floor(s);
        let tb = clamp((s - fl - 0.5) / 0.5, 0.0, 1.0);
        th = 2.5 * (fl + 0.5 * (s - fl) + 0.5 * tb * tb * (3.0 - 2.0 * tb)) * tf_fade(5.0, cut) + th * (1.0 - tf_fade(5.0, cut));
        let joint = min(tf_line(tf_noised(p * 110.0, seed + 14u), 110.0), tf_line(tf_noised(p * 70.0 + vec2<f32>(4.1, 0.7), seed + 15u), 70.0));
        th *= 1.0 - 0.6 * (1.0 - tf_smooth(0.3, 1.6, joint)) * tf_fade(3.0, cut);
    }
    y += th;
    g += tor_n.yz * (tor_h * 6.0 * tt * (1.0 - tt) / tor_w * (55.0 / TF_BASE));
    // Dunes: crests across the wind, a long gentle windward slope and a
    // steep lee, wandering with noise; whole fields of them on low ground.
    if st.dunes > 0.0 {
        let wind = vec2<f32>(0.8, 0.6);
        let field = (0.45 + 0.55 * tf_smooth(-0.4, 0.3, tf_noised(p * 0.8 + vec2<f32>(1.7, 4.2), seed + 25u).x)) * (1.0 - mountains);
        let dh = st.dunes * 18.0 * field * tf_fade(40.0, cut);
        if dh > 0.0 {
            let u = (x * wind.x + z * wind.y) / 200.0 + 0.6 * tf_noised(p * 6.0 + vec2<f32>(8.8, 3.1), seed + 26u).x;
            let s = u - floor(u);
            var f: f32;
            var df: f32;
            if s < 0.8 { f = tf_smooth(0.0, 0.8, s); df = 6.0 * (s / 0.8) * (1.0 - s / 0.8) / 0.8; }
            else { let t = (1.0 - s) / 0.2; f = t * t * (3.0 - 2.0 * t); df = -6.0 * t * (1.0 - t) / 0.2; }
            y += dh * f;
            g += wind * (dh * df / 200.0);
        }
    }
    let steep = sqrt(g.x * g.x + g.y * g.y);
    let up = 1.0 / sqrt(1.0 + steep * steep);

    var rock = clamp(tf_smooth(0.8, 0.6, up) + 0.7 * bedrock * tf_smooth(0.95, 0.85, up) + 0.5 * mountains * tf_smooth(0.85, 0.7, up)
        + tf_smooth(st.snow_line - 500.0, st.snow_line, y) * tf_smooth(0.93, 0.8, up) + tf_smooth(0.0, 0.15, tor * tor_h), 0.0, 1.0);
    let scree = tf_smooth(0.85, 0.72, up) * (1.0 - tf_smooth(0.65, 0.55, up)) * (0.4 + 0.6 * bedrock) * mountains
        + 0.9 * tf_smooth(2.0, 0.6, stream) * notch + 0.8 * rf * tf_smooth(ch_w + 2.5, ch_w, ch) * tf_smooth(-0.1, 0.2, tf_noised(p * 40.0, seed + 16u).x);

    // Boulders: at the foot of tors and crags, in stream and river beds.
    let tor_foot = tf_smooth(0.05, 0.15, tor_n.x) * (1.0 - tor) * bedrock;
    let boulder_density = clamp(0.02 + 0.45 * tor_foot + 0.2 * crag + 0.3 * tf_smooth(5.0, 1.0, stream) + 0.25 * tf_smooth(ch_w + 3.0, ch_w, ch), 0.0, 0.6);
    let bo = tf_boulders(x, z, min(boulder_density * st.boulders, 0.8), cut, seed);
    y += bo.x;
    rock = max(rock, bo.y);

    // Fine relief, far rougher on rock; calm on valley floors.
    let rough = 0.15 + 0.85 * rock;
    // The smooth ground, before its fine relief: still water stands level on it.
    let y_base = y;
    var db = b * amp;
    // Ground keeps getting rougher the closer you look, whatever the relief:
    // metres of roughness per octave fall off as wavelength^0.6, from
    // 0.8 m at 16 m: hummocks a hand high on pasture. Rock breaks into
    // sharp creases, only where the mesh has the vertices to hold them
    // (a crease between vertices shows as a dotted grid).
    var da = 0.8 * mix(0.6 + 0.8 * tf_smooth(0.97, 0.8, up0) + 0.3 * moor, 2.5, rock);
    for (var i = 7; i < 17; i++) {
        let w = tf_fade(wl, cut);
        if w <= 0.0 { break; }
        let n = tf_noised(q, seed + 10u + u32(i));
        d += n.yz;
        let e = 1.0 / (1.0 + 0.25 * (d.x * d.x + d.y * d.y));
        y += (db * n.x * rough + da * mix(n.x, 0.45 - abs(n.x), rock * tf_fade(0.5 * wl, cut))) * e * w * (0.35 + 0.65 * open);
        db *= 0.5;
        da *= 0.6597539;
        wl *= 0.5;
        q = tf_rotate(q);
    }

    // Water: the bed stays where it is; the surface is level.
    // Ditches round long parcels on flat land, on a grid turned by the seed.
    let flat = tf_smooth(0.97, 0.995, up0);
    var ditch = 1000.0;
    var water_d = 0.0;
    var wet_d = 0.0;
    if st.ditches > 0.0 && flat > 0.0 {
        let a = f32(seed % 97u) * 0.0647;
        let u = x * cos(a) + z * sin(a);
        let v = z * cos(a) - x * sin(a);
        let across = abs((u / 60.0 + 0.5) - floor(u / 60.0 + 0.5) - 0.5) * 60.0;
        let along = abs((v / 400.0 + 0.5) - floor(v / 400.0 + 0.5) - 0.5) * 400.0;
        ditch = min(across, along);
        let on = st.ditches * flat * tf_fade(3.0, cut);
        y -= on * 1.3 * (1.0 - tf_smooth(0.6, 1.8, ditch));
        if ditch < 1.6 { water_d = max((y_base - 0.5) - y, 0.0) * on; }
        wet_d = st.ditches * flat * mix(tf_seen(ditch, 3.25, cut), tf_smooth(5.0, 1.5, ditch), tf_fade(13.0, cut));
    }
    // Ponds in flat hollows, their water level with the ground round them.
    var water_p = 0.0;
    if st.ponds > 0.0 && flat > 0.0 {
        let pond = st.ponds * flat * tf_smooth(0.12, 0.2, tf_noised(p * 14.0 + vec2<f32>(2.6, 9.4), seed + 27u).x) * tf_fade(30.0, cut);
        y -= pond * 1.6;
        water_p = max((y_base - 0.35) - y, 0.0) * min(pond, 1.0);
    }
    let water_r = select(0.0, max(river_level - y, 0.0), ch < ch_w);
    let water_s = select(0.0, max(stream_level - y, 0.0) * notch, stream_line < 1.2 && notch > 0.0);
    let water = max(max(max(water_r, water_s) * channel, water_d), water_p);
    // The water a fine mesh would hold, as a share of each cell: ditches,
    // stream beds and the river's channel (half-widths where the carved
    // beds hold water). Wet banks narrower than the cell blend the same way.
    let film = max(max(st.ditches * flat * tf_seen(ditch, 1.2, cut), streams_here * tf_seen(stream_line, 1.0, cut)), tf_seen(ch, 0.5 * ch_w, cut));
    let wet_far = max((1.0 - rf) * tf_seen(ch, ch_w + 4.5, cut), streams_here * (1.0 - tf_fade(sw, cut)) * tf_seen(stream_line, 4.5, cut));

    // Bedrock strata: ledges stick out of steep rock and overhang a little.
    var pos = vec3<f32>(x, y + water, z);
    if rock > 0.0 && steep > 0.0 {
        let layer = y / 5.5 + 2.0 * tf_noised(p * 5.0, seed + 5u).x + 0.8 * tf_noised(p * 24.0, seed + 5u).x;
        let t = layer - floor(layer);
        let ledge = tf_smooth(0.0, 0.18, t) * (1.0 - tf_smooth(0.45, 1.0, t)) - 0.35;
        let push = 1.8 * tf_smooth(0.8, 0.6, up) * ledge * tf_fade(5.5, cut) * tf_smooth(-0.3, 0.3, tf_noised(p * 8.0, seed + 24u).x) * (1.0 - bo.y);
        pos.x -= g.x / steep * push;
        pos.z -= g.y / steep * push;
    }
    var o: TerrainSample;
    o.pos = pos;
    o.up = up;
    o.rock = rock;
    o.scree = clamp(scree, 0.0, 1.0);
    o.wet = max(max(max(tf_smooth(8.0, 1.0, near_wet), wet_d), tf_smooth(0.0, 0.3, water_p)), wet_far);
    o.water = water;
    o.film = film;
    o.moor = moor;
    // Snow lies above the snow line on all but steep faces, in drifts.
    // Snow follows the broad slope, so ledges and steps do not stripe it;
    // only sheer walls stay bare.
    o.snow = tf_smooth(st.snow_line - 150.0, st.snow_line + 150.0, y + 120.0 * tf_noised(p * 12.0, seed + 19u).x)
        * tf_smooth(0.5, 0.7, up0) * tf_smooth(0.35, 0.55, up);
    return o;
}
