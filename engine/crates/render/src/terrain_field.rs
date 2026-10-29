//! The procedural terrain function, line for line the same as
//! `shaders/terrain_field.wgsl`. The GPU builds the visible mesh from the
//! WGSL copy; this copy answers physics, LOD bounds and tests. Only basic
//! IEEE operations (no sin, exp or pow), so results are bit-exact on every
//! CPU. The GPU copy agrees to within float rounding.

use glam::{Vec2, Vec3};

pub const WGSL: &str = include_str!("shaders/terrain_field.wgsl");

pub const BASE: f32 = 2048.0;
/// Largest height the function can reach above or below zero (metres),
/// with margin. Used for conservative bounds before the GPU measures.
pub const MAX_ABS_HEIGHT: f32 = 4500.0;
/// Most the strata can move a point sideways (metres).
pub const MAX_PUSH: f32 = 1.3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub pos: Vec3,
    /// Up component of the large-scale surface normal.
    pub up: f32,
    /// 0..1 exposed bedrock and boulders.
    pub rock: f32,
    /// 0..1 loose stone: scree under cliffs, gravel in stream beds.
    pub scree: f32,
    /// 0..1 damp ground near water.
    pub wet: f32,
    /// Metres of water above the bed (0 on dry land).
    pub water: f32,
    /// 0..1 vegetation kind: 0 short grass, 1 heather and moss.
    pub moor: f32,
    /// 0..1 lying snow.
    pub snow: f32,
    /// Height scale of the large shapes here (metres).
    pub amp: f32,
}

impl Sample {
    /// Rough bound on how far a surface sampled at grid spacing `cell`
    /// strays from the full-detail one (metres): the dropped octaves near
    /// two to four cells dominate, and their amplitude grows with length.
    pub fn error(&self, cell: f32) -> f32 {
        let rough = 0.15 + 0.85 * self.rock.max(((cell - 2.0) / 6.0).clamp(0.0, 1.0));
        let ledges = if cell * 4.0 > 5.5 { 0.0 } else { 1.2 * self.rock };
        0.5 * cell * self.amp / BASE * rough + ledges * (cell / 1.4).min(1.0)
    }
}

/// A game designer's choices for one world. The 0..1 amounts read 0 as
/// none, 0.5 as the reference look and 1 as the most; the multipliers
/// default to 1. Physics and the GPU must use the same style.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Style {
    /// How much of the land is mountains rather than lowland.
    pub mountains: f32,
    /// How much rises into high massifs, kilometres above the valleys.
    pub peaks: f32,
    /// Height (metres) above which snow lies.
    pub snow_line: f32,
    /// Relief of the lowland hills (multiplier).
    pub hills: f32,
    /// How many of the rivers flow.
    pub rivers: f32,
    /// How many streams run down the hills.
    pub streams: f32,
    /// Tors and crag bands.
    pub rocks: f32,
    /// Boulders (multiplier).
    pub boulders: f32,
    /// Heather and moss rather than short grass.
    pub moor: f32,
    /// Grass cover for the meadow; the terrain itself ignores it.
    pub grass: f32,
    pub _pad: [f32; 2],
}

impl Default for Style {
    fn default() -> Self {
        Style { mountains: 0.5, peaks: 0.5, snow_line: 1500.0, hills: 1.0, rivers: 0.5, streams: 0.5, rocks: 0.5, boulders: 1.0, moor: 0.5, grass: 1.0, _pad: [0.0; 2] }
    }
}

impl Style {
    /// Rolling farmland: low hills, rivers, no high ground.
    pub fn lowland() -> Self { Style { mountains: 0.0, peaks: 0.0, hills: 1.2, rivers: 0.8, rocks: 0.2, moor: 0.3, ..Style::default() } }
    /// High snowy massifs over green valleys.
    pub fn alpine() -> Self { Style { mountains: 0.8, peaks: 1.0, snow_line: 1700.0, rocks: 0.8, moor: 0.2, ..Style::default() } }
    /// A style by name: default, lowland or alpine.
    pub fn named(name: &str) -> Option<Self> {
        match name { "default" => Some(Style::default()), "lowland" => Some(Style::lowland()), "alpine" => Some(Style::alpine()), _ => None }
    }
}

fn hash(ix: i32, iz: i32, seed: u32) -> u32 {
    let h = (ix as u32).wrapping_mul(0x8da6b343) ^ (iz as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f);
    let s = h.wrapping_mul(747796405).wrapping_add(2891336453);
    let w = ((s >> ((s >> 28) + 4)) ^ s).wrapping_mul(277803737);
    (w >> 22) ^ w
}

fn grad(ix: i32, iz: i32, seed: u32) -> Vec2 {
    let h = hash(ix, iz, seed);
    Vec2::new((h & 0xffff) as f32 / 32767.5 - 1.0, (h >> 16) as f32 / 32767.5 - 1.0)
}

/// Gradient noise and its derivative: (value, d/dx, d/dz).
pub fn noised(p: Vec2, seed: u32) -> Vec3 {
    let fl = Vec2::new(p.x.floor(), p.y.floor());
    let (ix, iz) = (fl.x as i32, fl.y as i32);
    let f = p - fl;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let ga = grad(ix, iz, seed);
    let gb = grad(ix + 1, iz, seed);
    let gc = grad(ix, iz + 1, seed);
    let gd = grad(ix + 1, iz + 1, seed);
    let va = ga.x * f.x + ga.y * f.y;
    let vb = gb.x * (f.x - 1.0) + gb.y * f.y;
    let vc = gc.x * f.x + gc.y * (f.y - 1.0);
    let vd = gd.x * (f.x - 1.0) + gd.y * (f.y - 1.0);
    let k = va - vb - vc + vd;
    let v = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    let dx = ga.x + u.x * (gb.x - ga.x) + u.y * (gc.x - ga.x) + u.x * u.y * (ga.x - gb.x - gc.x + gd.x) + du.x * (u.y * k + vb - va);
    let dz = ga.y + u.x * (gb.y - ga.y) + u.y * (gc.y - ga.y) + u.x * u.y * (ga.y - gb.y - gc.y + gd.y) + du.y * (u.x * k + vc - va);
    Vec3::new(v, dx, dz)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn fade(size: f32, cut: f32) -> f32 {
    (size / (2.0 * cut) - 1.0).clamp(0.0, 1.0)
}

fn line(n: Vec3, freq: f32) -> f32 {
    let g = (n.y * n.y + n.z * n.z).sqrt() * (freq / BASE) + 1e-6;
    (n.x.abs() / g).min(1000.0)
}

fn rotate(q: Vec2) -> Vec2 { Vec2::new(1.6 * q.x - 1.2 * q.y, 1.2 * q.x + 1.6 * q.y) }

fn mix(a: f32, b: f32, t: f32) -> f32 { a * (1.0 - t) + b * t }

fn boulders(x: f32, z: f32, density: f32, cut: f32, seed: u32) -> (f32, f32) {
    let cell = 7.0f32;
    let cx = (x / cell).floor() as i32;
    let cz = (z / cell).floor() as i32;
    let mut best = 0.0f32;
    let mut cover = 0.0f32;
    for j in -1..=1 {
        for i in -1..=1 {
            let h = hash(cx + i, cz + j, seed.wrapping_add(77));
            let keep = (h & 1023) as f32 / 1023.0;
            if keep >= density { continue; }
            let h2 = hash(cx + i, cz + j, seed.wrapping_add(78));
            let ox = ((cx + i) as f32 + 0.15 + 0.7 * ((h >> 10) & 255) as f32 / 255.0) * cell;
            let oz = ((cz + j) as f32 + 0.15 + 0.7 * ((h >> 18) & 255) as f32 / 255.0) * cell;
            let k = ((h >> 26) & 63) as f32 / 63.0;
            let r = 0.4 + 1.5 * k * k;
            let w = fade(r * 2.0, cut);
            if w <= 0.0 { continue; }
            let dx = (x - ox) * (0.75 + 0.5 * (h2 & 255) as f32 / 255.0);
            let dz = (z - oz) * (0.75 + 0.5 * ((h2 >> 8) & 255) as f32 / 255.0);
            let s = (dx * dx + dz * dz) / (r * r);
            if s < 1.0 {
                let b = r * (0.4 + 0.3 * ((h2 >> 16) & 255) as f32 / 255.0) * (1.0 - s * s) * w;
                best = best.max(b);
                cover = cover.max(smooth(0.0, 0.3, 1.0 - s) * w);
            }
        }
    }
    (best, cover)
}

fn sign(c: bool) -> f32 { if c { 1.0 } else { -1.0 } }

/// The surface point for ground coordinates (x, z), keeping only features
/// longer than about two `cut`-sized cells.
pub fn eval(x: f32, z: f32, cut: f32, seed: u32, st: &Style) -> Sample {
    let sd = |k: u32| seed.wrapping_add(k);
    let p = Vec2::new(x, z) / BASE;
    let region = noised(p * 0.25 + Vec2::new(3.1, 7.7), sd(1)).x + 0.5 * noised(p * 0.5 + Vec2::new(1.3, 2.9), sd(1)).x;
    let alpine = smooth(0.0, 0.45, noised(p * 0.15 + Vec2::new(8.3, 1.9), sd(18)).x + 0.3 * region + 0.8 * (st.peaks - 0.5))
        * smooth(0.0, 0.1, st.peaks);
    let mountains = smooth(-0.1, 0.45, region + 1.2 * (st.mountains - 0.5)).max(alpine);
    let bedrock = smooth(-0.1, 0.3, noised(p * 0.6 + Vec2::new(5.5, 0.4), sd(2)).x);
    let moor = smooth(-0.2, 0.25, noised(p * 0.9 + Vec2::new(2.2, 8.1), sd(6)).x + 1.2 * (st.moor - 0.5));
    let warp = Vec2::new(noised(p * 1.5 + Vec2::new(5.2, 1.3), sd(3)).x, noised(p * 1.5 + Vec2::new(1.7, 9.2), sd(3)).x);
    let mut q = p + warp * 0.12;

    let amp = 100.0 * st.hills + 640.0 * mountains + 700.0 * alpine;
    let mut h = 0.0f32;
    let mut low = 0.0f32;
    let mut low_slope = Vec2::ZERO;
    let mut b = 1.0f32;
    let mut d = Vec2::ZERO;
    let mut slope = Vec2::ZERO;
    let mut m = [1.0f32, 0.0, 0.0, 1.0];
    let mut wl = BASE;
    for i in 0..7u32 {
        let w = fade(wl, cut);
        let n = noised(q, sd(10 + i));
        d += Vec2::new(n.y, n.z);
        let e = 1.0 / (1.0 + d.x * d.x + d.y * d.y);
        slope += Vec2::new(m[0] * n.y + m[2] * n.z, m[1] * n.y + m[3] * n.z) * (b * e * w);
        let k = if (1..=4).contains(&i) { 0.8 * mountains } else { 0.0 };
        h += b * (n.x + k * (0.4 - n.x.abs() - n.x)) * e * w;
        if i <= 2 { low += b * n.x * e * w; }
        if i == 2 {
            low_slope = slope;
        }
        b *= 0.5;
        m = [1.6 * m[0] - 1.2 * m[2], 1.6 * m[1] - 1.2 * m[3], 1.2 * m[0] + 1.6 * m[2], 1.2 * m[1] + 1.6 * m[3]];
        wl *= 0.5;
        q = rotate(q);
    }
    let big = noised(p * 0.5 + Vec2::new(4.4, 7.3), sd(21));
    let fold = (big.x * big.x + 0.01).sqrt();
    let fu = ((0.55 - fold) / 0.45).clamp(0.0, 1.0);
    let uplift = alpine * (300.0 + 1500.0 * fu * fu * (3.0 - 2.0 * fu));
    let duplift = Vec2::new(big.y, big.z) * (-alpine * 1500.0 * 6.0 * fu * (1.0 - fu) / 0.45 * big.x / fold * (0.5 / BASE));
    let mut y = h * amp + uplift;
    let mut g = slope * (amp / BASE) + duplift;
    let g0 = g;
    let up0 = 1.0 / (1.0 + g.x * g.x + g.y * g.y).sqrt();

    // Crag bands come and go along the slope, wander in height and sink
    // under the snow, so they never read as stairs.
    let crag = (2.0 * st.rocks).min(1.0) * bedrock * smooth(0.2, 0.7, mountains) * smooth(0.97, 0.85, up0) * fade(24.0, cut)
        * smooth(-0.15, 0.35, noised(p * 3.0 + warp, sd(22)).x) * (1.0 - 0.8 * smooth(st.snow_line - 300.0, st.snow_line, y));
    if crag > 0.0 {
        let s = y / 24.0 + 1.4 * noised(p * 2.5 + warp, sd(12)).x + 0.4 * noised(p * 9.0, sd(23)).x;
        let fl = s.floor();
        let t = s - fl;
        let tb = ((t - 0.6) / 0.4).clamp(0.0, 1.0);
        let k = 0.35 + 0.65 * 6.0 * tb * (1.0 - tb) / 0.4;
        y += crag * 24.0 * (fl + 0.35 * t + 0.65 * tb * tb * (3.0 - 2.0 * tb) - s);
        g *= 1.0 + crag * (k - 1.0);
    }

    let river_n = noised(p * 0.7 + warp * 0.3 + Vec2::new(9.1, 4.4), sd(4));
    let river_on = smooth(-0.15, 0.15, 2.2 * st.rivers - 1.1 + noised(p * 0.35 + Vec2::new(2.7, 5.1), sd(20)).x);
    let river = mix(1000.0, line(river_n, 0.7), river_on);
    let rn = Vec2::new(river_n.y, river_n.z) / ((river_n.y * river_n.y + river_n.z * river_n.z).sqrt() + 1e-6) * sign(river_n.x >= 0.0);
    let gl = low_slope * (amp / BASE);
    let floor_y = (low - 0.1) * amp + uplift - river.min(600.0) * (gl.x * rn.x + gl.y * rn.y);
    let relief = (y - floor_y).max(0.0);
    let floor_w = 20.0 + 0.12 * relief;
    let wall = 50.0 + 2.0 * relief;
    let rv = smooth(floor_w, floor_w + wall, river);
    let rt = ((river - floor_w) / wall).clamp(0.0, 1.0);
    let drv = 6.0 * rt * (1.0 - rt) / wall;
    y = floor_y + (y - floor_y) * rv;
    g = g * rv + rn * (relief * drv);
    let meander = noised(p * 9.0 + warp, sd(13)).x * 0.7 * floor_w;
    let ch = (river.min(600.0) * sign(river_n.x >= 0.0) - meander).abs();
    let ch_w = 3.5 + 0.02 * relief;
    let rb = smooth(0.0, ch_w, ch);
    let channel = fade(1.6, cut);
    y = mix(y, y.min(floor_y - 1.3 * (1.0 - rb * rb)), channel * (1.0 - rb));
    let river_level = floor_y - 0.3;

    let stream_n = noised(p * 2.1 + warp * 0.6 + Vec2::new(3.3, 6.6), sd(7));
    let sn = Vec2::new(stream_n.y, stream_n.z) / ((stream_n.y * stream_n.y + stream_n.z * stream_n.z).sqrt() + 1e-6);
    let stream_line = line(stream_n, 2.1);
    let depth = (0.05 * relief + 0.8).clamp(0.8, 4.0);
    let sw = 3.0 + 5.0 * depth;
    let across = (g0.x * sn.x + g0.y * sn.y).abs();
    let streams_here = smooth(0.4 - 1.5 * st.streams, 0.65 - 1.5 * st.streams, noised(p * 0.8 + Vec2::new(7.4, 2.5), sd(8)).x)
        * smooth(0.85, 0.93, up0) * smooth(0.6 * depth / sw, 0.3 * depth / sw, across) * smooth(floor_w * 0.5, floor_w, ch);
    let stream = mix(1000.0, stream_line, streams_here);
    let notch = fade(sw, cut) * streams_here;
    let sv = smooth(1.0, sw, stream_line);
    let yc = y.min(y - stream_line.min(50.0) * (g0.x * sn.x + g0.y * sn.y) * sign(stream_n.x >= 0.0));
    let stream_level = yc - depth * notch;
    y = mix(y, y.min(yc - depth), notch * (1.0 - sv));
    let sb = smooth(0.0, 1.2, stream_line);
    y = mix(y, y.min(stream_level - 0.35 * (1.0 - sb * sb)), channel * notch * (1.0 - sb));
    let open = rv * (1.0 - notch * (1.0 - sv));
    // Banks too narrow for this cell would leave single wet vertices.
    let near_water = (ch - ch_w).max(0.0).min(stream);
    let rf = fade(2.0 * ch_w, cut);
    let near_wet = ((ch - ch_w).max(0.0) + 1000.0 * (1.0 - rf)).min(stream + 1000.0 * (1.0 - notch));

    let tor_n = noised(p * 30.0 + warp * 2.0, sd(9));
    let tor_zone = smooth(0.65 - st.rocks, 0.9 - st.rocks, noised(p * 4.0 + Vec2::new(6.3, 2.2), sd(17)).x) * smooth(5.0, 40.0, relief);
    let tor_fade = fade(20.0, cut);
    let tor_h = (8.0 + 14.0 * mountains) * bedrock * tor_zone * smooth(30.0, 80.0, near_water) * tor_fade;
    let tt = ((tor_n.x - 0.22) / 0.08).clamp(0.0, 1.0);
    let tor = tt * tt * (3.0 - 2.0 * tt);
    let mut th = tor_h * (tor + 0.5 * (tor_n.x - 0.3).max(0.0));
    if th > 0.0 {
        let s = th / 2.5;
        let fl = s.floor();
        let tb = ((s - fl - 0.5) / 0.5).clamp(0.0, 1.0);
        th = 2.5 * (fl + 0.25 * (s - fl) + 0.75 * tb * tb * (3.0 - 2.0 * tb)) * fade(5.0, cut) + th * (1.0 - fade(5.0, cut));
        let joint = line(noised(p * 110.0, sd(14)), 110.0).min(line(noised(p * 70.0 + Vec2::new(4.1, 0.7), sd(15)), 70.0));
        th *= 1.0 - 0.6 * (1.0 - smooth(0.3, 1.6, joint)) * fade(3.0, cut);
    }
    y += th;
    g += Vec2::new(tor_n.y, tor_n.z) * (tor_h * 6.0 * tt * (1.0 - tt) / 0.08 * (30.0 / BASE));
    let steep = (g.x * g.x + g.y * g.y).sqrt();
    let up = 1.0 / (1.0 + steep * steep).sqrt();

    let mut rock = (smooth(0.8, 0.6, up) + 0.7 * bedrock * smooth(0.95, 0.85, up) + smooth(0.0, 0.15, tor * tor_h) * tor_fade).clamp(0.0, 1.0);
    let scree = smooth(0.85, 0.72, up) * (1.0 - smooth(0.65, 0.55, up)) * (0.4 + 0.6 * bedrock) * mountains
        + 0.9 * smooth(2.0, 0.6, stream) * notch + 0.8 * rf * smooth(ch_w + 2.5, ch_w, ch) * smooth(-0.1, 0.2, noised(p * 40.0, sd(16)).x);

    let tor_foot = smooth(0.12, 0.2, tor_n.x) * (1.0 - tor) * bedrock;
    let boulder_density = (0.02 + 0.45 * tor_foot + 0.2 * crag + 0.3 * smooth(5.0, 1.0, stream) + 0.25 * smooth(ch_w + 3.0, ch_w, ch)).clamp(0.0, 0.6);
    let (bh, bc) = boulders(x, z, (boulder_density * st.boulders).min(0.8), cut, seed);
    y += bh;
    rock = rock.max(bc);

    let rough = 0.15 + 0.85 * rock;
    let mut db = b * amp;
    for i in 7..17u32 {
        let w = fade(wl, cut);
        if w <= 0.0 { break; }
        let n = noised(q, sd(10 + i));
        d += Vec2::new(n.y, n.z);
        let e = 1.0 / (1.0 + 0.25 * (d.x * d.x + d.y * d.y));
        y += db * n.x * e * w * rough * (0.35 + 0.65 * open);
        db *= 0.5;
        wl *= 0.5;
        q = rotate(q);
    }

    let water_r = if ch < ch_w { (river_level - y).max(0.0) } else { 0.0 };
    let water_s = if stream_line < 1.2 && notch > 0.0 { (stream_level - y).max(0.0) * notch } else { 0.0 };
    let water = water_r.max(water_s) * channel;

    let mut pos = Vec3::new(x, y + water, z);
    if rock > 0.0 && steep > 0.0 {
        let layer = y / 5.5 + 2.0 * noised(p * 5.0, sd(5)).x + 0.8 * noised(p * 24.0, sd(5)).x;
        let t = layer - layer.floor();
        let ledge = smooth(0.0, 0.18, t) * (1.0 - smooth(0.45, 1.0, t)) - 0.35;
        let push = 1.8 * smooth(0.8, 0.6, up) * ledge * fade(5.5, cut) * smooth(-0.3, 0.3, noised(p * 8.0, sd(24)).x);
        pos.x -= g.x / steep * push;
        pos.z -= g.y / steep * push;
    }
    // Snow follows the broad slope, so ledges and steps do not stripe it;
    // only sheer walls stay bare.
    let snow = smooth(st.snow_line - 150.0, st.snow_line + 150.0, y + 120.0 * noised(p * 12.0, sd(19)).x)
        * smooth(0.62, 0.8, up0) * smooth(0.35, 0.55, up);
    Sample { pos, up, rock, scree: scree.clamp(0.0, 1.0), wet: smooth(8.0, 1.0, near_wet), water, moor, snow, amp }
}

/// Ground height under world (x, z) at full detail, for physics. The strata
/// move points sideways, so the surface point above (x, z) is found by a
/// few fixed-point steps on the ground coordinates.
pub fn height(x: f32, z: f32, seed: u32, style: &Style) -> f32 {
    let (mut u, mut v) = (x, z);
    let mut s = eval(u, v, 0.0, seed, style);
    for _ in 0..4 {
        u += x - s.pos.x;
        v += z - s.pos.z;
        s = eval(u, v, 0.0, seed, style);
    }
    s.pos.y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_finite() {
        for i in 0..2000 {
            let x = (i as f32 * 37.3) % 20000.0 - 10000.0;
            let z = (i as f32 * 91.7) % 20000.0 - 10000.0;
            let a = eval(x, z, 0.0, 7, &Style::default());
            assert_eq!(a, eval(x, z, 0.0, 7, &Style::default()));
            assert!(a.pos.is_finite() && a.pos.y.abs() < MAX_ABS_HEIGHT, "{a:?}");
            assert!((a.pos.x - x).abs() <= MAX_PUSH && (a.pos.z - z).abs() <= MAX_PUSH);
        }
    }

    #[test]
    fn coarse_cut_drops_detail_smoothly() {
        let fine = eval(123.4, -567.8, 0.0, 3, &Style::default());
        let coarse = eval(123.4, -567.8, 64.0, 3, &Style::default()).pos.y;
        assert!((fine.pos.y - coarse).abs() < 0.1 * fine.amp);
    }
}
