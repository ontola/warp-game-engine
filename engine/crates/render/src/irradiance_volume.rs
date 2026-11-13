//! Indoor diffuse light: a regular grid of irradiance probes the game bakes
//! (or updates) and the shade pass interpolates, instead of sky light and
//! the outdoor bounce heuristic. For rooms: a lobby lit by lamps and neon,
//! where light reaches the floor via walls and the ceiling.
//!
//! Each probe is an ambient cube: irradiance (lux) on the six axis
//! directions, blended by the squared normal components. Unlike a linear
//! (L1) fit it keeps strongly one-sided light (a lamp above, a dark floor). Inside the grid the shade pass uses
//! it for diffuse ambient and rough reflections, fading to the outdoor model
//! over the outer half cell. Probes live in the scene's `aux` buffer, so
//! `Renderer::set_irradiance_volume` updates them without a rebuild.
//!
//! `BoxRoom` is a small CPU baker for box-shaped geometry (walls, floor,
//! furniture, emissive tubes) and the scene's local lights, with shadow rays
//! and as many diffuse bounces as `bake` passes.
use crate::headlights::LocalLight;
use glam::Vec3;
use std::f32::consts::PI;

/// Header words in `aux` before the probes: origin xyz, cell, dims xyz, 0.
pub const HEADER_WORDS: usize = 8;
pub const PROBE_WORDS: usize = 18;
pub const MAX_PROBES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid { pub origin: Vec3, pub cell: f32, pub dims: [u32; 3] }

impl Grid {
    pub fn count(&self) -> usize { self.dims.iter().map(|&d| d as usize).product() }
    /// The grid of `cell` spacing covering the box `min`..`max`, one cell
    /// beyond it on every side: the volume fades out over half a cell past
    /// its outer probes (shading's `weight`), and that fade must lie outside
    /// what it covers, not on the floor. Probes may land in walls or on
    /// surfaces: `room_bake::probe_positions` moves them out, so the grid
    /// need not be placed around the geometry.
    pub fn covering(min: Vec3, max: Vec3, cell: f32) -> Self {
        let n = ((max - min) / cell).ceil() + 2.0;
        Self { origin: min - Vec3::splat(cell), cell, dims: [n.x as u32 + 1, n.y as u32 + 1, n.z as u32 + 1] }
    }
    pub fn position(&self, i: usize) -> Vec3 {
        let [x, y, _] = self.dims.map(|d| d as usize);
        self.origin + Vec3::new((i % x) as f32, ((i / x) % y) as f32, (i / (x * y)) as f32) * self.cell
    }
    pub fn validate(&self) -> Result<(), String> {
        if !self.origin.is_finite() || !(self.cell.is_finite() && self.cell > 0.01) { return Err("irradiance grid origin/cell invalid".into()); }
        if self.dims.iter().any(|&d| d < 2) || self.count() > MAX_PROBES { return Err(format!("irradiance grid needs 2..{MAX_PROBES} probes, at least 2 per axis")); }
        Ok(())
    }
}

/// One probe: irradiance facing +x, -x, +y, -y, +z, -z.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Probe { pub axes: [Vec3; 6] }

impl Probe {
    pub fn irradiance(&self, n: Vec3) -> Vec3 {
        let n2 = n * n;
        let pick = |c: f32, pos: usize| if c >= 0.0 { self.axes[pos] } else { self.axes[pos + 1] };
        (pick(n.x, 0) * n2.x + pick(n.y, 2) * n2.y + pick(n.z, 4) * n2.z) / n2.element_sum().max(1e-12)
    }
    fn words(&self) -> [u32; PROBE_WORDS] {
        let mut w = [0u32; PROBE_WORDS];
        for (i, v) in self.axes.iter().enumerate() { for c in 0..3 { w[i * 3 + c] = v[c].to_bits(); } }
        w
    }
}

/// `aux` words for a grid header followed by its probes.
pub fn pack(grid: &Grid, probes: &[Probe]) -> Result<Vec<u32>, String> {
    grid.validate()?;
    if probes.len() != grid.count() { return Err("probe count differs from the grid".into()); }
    if probes.iter().any(|p| !p.axes.iter().all(|a| a.is_finite() && a.min_element() >= 0.0)) { return Err("non-finite irradiance probe".into()); }
    let mut words = vec![grid.origin.x.to_bits(), grid.origin.y.to_bits(), grid.origin.z.to_bits(), grid.cell.to_bits(), grid.dims[0], grid.dims[1], grid.dims[2], 0];
    for p in probes { words.extend(p.words()); }
    Ok(words)
}

// RGB9E5 shared-exponent radiance, retaining fractional lux at night and
// up to 65408 lux by day. Six packed colours leave room for separation planes.
fn pack_emission(rgb: Vec3) -> u32 {
    let rgb = rgb.min(Vec3::splat(65408.0));
    let maximum = rgb.max_element();
    let mut exponent = ((maximum.max(2f32.powi(-16)).log2().floor() as i32) + 16).max(0);
    let mut scale = 2f32.powi(exponent - 24);
    if (maximum/scale).round() >= 512.0 { exponent += 1; scale *= 2.0; }
    let q = (rgb/scale).round().as_uvec3();
    q.x | (q.y << 9) | (q.z << 18) | ((exponent as u32) << 27)
}

/// One axis of sky/sun transport in a word: three 9-bit mantissas with a
/// shared 4-bit exponent (values 2^-23 to about 2, a constant ~0.2 %
/// relative step) and the invalid flag (z above 1.5) in the top bit. A dark
/// room lit only by light through a doorway has transport around 1e-5,
/// where the earlier 10-bit square-root code stepped by 75 % from probe to
/// probe (blocks on the walls once the eye adapts to the dark).
pub fn pack_transport(v: Vec3) -> u32 {
    let invalid = u32::from(v.z > 1.5) << 31;
    let v = v.clamp(Vec3::ZERO, Vec3::splat(1.0));
    let maximum = v.max_element();
    let mut exponent = ((maximum.max(2f32.powi(-15)).log2().floor() as i32) + 15).clamp(0, 15);
    let mut scale = 2f32.powi(exponent - 23);
    if (maximum / scale).round() >= 512.0 && exponent < 15 { exponent += 1; scale *= 2.0; }
    let q = (v / scale).round().min(Vec3::splat(511.0)).as_uvec3();
    q.x | (q.y << 9) | (q.z << 18) | ((exponent as u32) << 27) | invalid
}

/// The shader's decode of [`pack_transport`].
pub fn unpack_transport(word: u32) -> Vec3 {
    let scale = 2f32.powi(((word >> 27) & 15) as i32 - 23);
    Vec3::new((word & 511) as f32, ((word >> 9) & 511) as f32, ((word >> 18) & 511) as f32) * scale
}

/// Mode 7: sky/sun transport plus static RGB emission, in the existing 18
/// words per probe. Optional planes use the spare third words of four axes.
pub fn pack_sky_emission(grid: &Grid, sky: &[Probe], emission: &[Probe], planes: &[[f32;4]]) -> Result<Vec<u32>,String> {
    let mut words=pack(grid,sky)?;
    if emission.len()!=sky.len() || emission.iter().any(|p| p.axes.iter().any(|v| !v.is_finite() || v.min_element()<0.0))
        || (!planes.is_empty() && planes.len()!=sky.len()) || planes.iter().flatten().any(|v| !v.is_finite()) {
        return Err("invalid static emission probes or separation planes".into());
    }
    words[7]=7;
    for (i,p) in sky.iter().enumerate() {
        for (a,v) in p.axes.iter().enumerate() {
            let at=8+i*18+a*3;
            words[at]=pack_transport(*v);
            words[at+1]=pack_emission(emission[i].axes[a]);
            words[at+2]=if a<4 && !planes.is_empty() { planes[i][a].to_bits() } else { 0 };
        }
    }
    Ok(words)
}

/// Words per probe of [`pack_gradients`]: per axis (6) the gradient (x, y,
/// z per metre) of the sky transport's three channels and the emission's
/// three, as half floats (6 * 6 * 3 halves).
pub const GRADIENT_WORDS: usize = 54;

/// The bit of `room_bake` neighbour masks (26 neighbours, x outermost, then
/// y, then z, the probe itself left out) for the neighbour at `o`.
pub fn neighbour_bit(o: [i32; 3]) -> u32 {
    let k = (o[0] + 1) * 9 + (o[1] + 1) * 3 + (o[2] + 1);
    1 << (if k > 13 { k - 1 } else { k })
}

/// Per probe, how the sky transport and the emission change along x, y and
/// z: central differences to the neighbours it sees (`neighbours`, the
/// bake's masks), one-sided where it sees one of the two, none where it sees
/// neither or is invalid. Shading continues each probe's value along its
/// gradient towards the shaded point and blends with C1 weights, so the
/// light has no kinks at the probe spacing (shade.wgsl
/// `irradiance_sample_at`). A header word (1), then GRADIENT_WORDS per probe.
pub fn pack_gradients(grid: &Grid, sky: &[Probe], emission: &[Probe], neighbours: &[u32]) -> Result<Vec<u32>, String> {
    let n = grid.count();
    if sky.len() != n || emission.len() != n || neighbours.len() != n { return Err("one sky probe, emission probe and neighbour mask per probe".into()); }
    let [dx, dy, dz] = grid.dims.map(|d| d as i32);
    let mut out = vec![0u32; 1 + n * GRADIENT_WORDS];
    out[0] = 1;
    let value = |i: usize, a: usize| -> [f32; 6] { let s = sky[i].axes[a]; let e = emission[i].axes[a]; [s.x, s.y, s.z, e.x, e.y, e.z] };
    for i in 0..n {
        if sky[i].axes[0].z >= 1.5 { continue; }
        let (x, y, z) = (i as i32 % dx, (i as i32 / dx) % dy, i as i32 / (dx * dy));
        let at = |o: [i32; 3]| ((x + o[0]) + (y + o[1]) * dx + (z + o[2]) * dx * dy) as usize;
        let mut halves = [0u16; GRADIENT_WORDS * 2];
        for d in 0..3 {
            let mut plus = [0; 3];
            plus[d] = 1;
            let minus = plus.map(|v| -v);
            let sees = |o: [i32; 3]| neighbours[i] & neighbour_bit(o) != 0;
            let (p, m) = (sees(plus), sees(minus));
            for a in 0..6 {
                let f = value(i, a);
                let g: [f32; 6] = match (p, m) {
                    (true, true) => { let (fp, fm) = (value(at(plus), a), value(at(minus), a)); std::array::from_fn(|q| (fp[q] - fm[q]) / (2.0 * grid.cell)) }
                    (true, false) => { let fp = value(at(plus), a); std::array::from_fn(|q| (fp[q] - f[q]) / grid.cell) }
                    (false, true) => { let fm = value(at(minus), a); std::array::from_fn(|q| (f[q] - fm[q]) / grid.cell) }
                    (false, false) => [0.0; 6],
                };
                for q in 0..6 { halves[(a * 6 + q) * 3 + d] = f16_signed(g[q]); }
            }
        }
        for w in 0..GRADIENT_WORDS { out[1 + i * GRADIENT_WORDS + w] = halves[2 * w] as u32 | ((halves[2 * w + 1] as u32) << 16); }
    }
    Ok(out)
}

/// IEEE half bits of a finite value, rounded to nearest; tiny values flush
/// to zero, large ones saturate at 65504.
fn f16_signed(v: f32) -> u16 {
    let v = if v.is_finite() { v.clamp(-65504.0, 65504.0) } else { 0.0 };
    let sign = if v < 0.0 { 0x8000u16 } else { 0 };
    let a = v.abs();
    if a < 6.1e-5 { return sign; }
    let bits = a.to_bits();
    let exponent = ((bits >> 23) & 255) as i32 - 127 + 15;
    let mantissa = (bits & 0x7f_ffff) + 0x1000;
    let (exponent, mantissa) = if mantissa & 0x80_0000 != 0 { (exponent + 1, 0) } else { (exponent, mantissa >> 13) };
    sign | ((exponent.clamp(1, 30) as u16) << 10) | (mantissa & 0x3ff) as u16
}

/// Trilinear lookup on the CPU (the shade pass does the same).
pub fn sample(grid: &Grid, probes: &[Probe], p: Vec3, n: Vec3) -> Vec3 {
    let f = ((p - grid.origin) / grid.cell).clamp(Vec3::ZERO, Vec3::new(grid.dims[0] as f32 - 1.0, grid.dims[1] as f32 - 1.0, grid.dims[2] as f32 - 1.0));
    let i = f.floor().min(Vec3::new(grid.dims[0] as f32 - 2.0, grid.dims[1] as f32 - 2.0, grid.dims[2] as f32 - 2.0));
    let t = f - i;
    let mut e = Vec3::ZERO;
    for corner in 0..8 {
        let o = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let w = (if o[0] == 1 { t.x } else { 1.0 - t.x }) * (if o[1] == 1 { t.y } else { 1.0 - t.y }) * (if o[2] == 1 { t.z } else { 1.0 - t.z });
        let idx = (i.x as usize + o[0]) + (i.y as usize + o[1]) * grid.dims[0] as usize + (i.z as usize + o[2]) * (grid.dims[0] * grid.dims[1]) as usize;
        e += probes[idx].irradiance(n) * w;
    }
    e
}

/// An axis-aligned box: walls, floors, furniture, light tubes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block { pub min: Vec3, pub max: Vec3, pub albedo: Vec3, /// Radiance in nits.
    pub emission: Vec3 }

/// Box geometry and local lights for baking. Rays that hit nothing see `sky`
/// radiance (zero for a closed room).
#[derive(Clone, Debug, Default)]
pub struct BoxRoom { pub blocks: Vec<Block>, pub lights: Vec<LocalLight>, pub sky: Vec3 }

impl BoxRoom {
    /// Nearest hit along `dir` beyond `t_min`: (distance, block, outward normal).
    pub fn hit(&self, origin: Vec3, dir: Vec3, t_min: f32, t_max: f32) -> Option<(f32, usize, Vec3)> {
        let inv = dir.recip();
        let mut best: Option<(f32, usize, Vec3)> = None;
        for (i, b) in self.blocks.iter().enumerate() {
            let t0 = (b.min - origin) * inv;
            let t1 = (b.max - origin) * inv;
            let (near, far) = (t0.min(t1), t0.max(t1));
            let enter = near.max_element();
            let exit = far.min_element();
            if exit < enter || exit < t_min { continue; }
            // From outside the entry face; from inside (a probe in a wall) the exit.
            let (t, from_inside) = if enter >= t_min { (enter, false) } else { (exit, true) };
            if t > t_max || best.is_some_and(|(bt, _, _)| t >= bt) { continue; }
            let axis = if from_inside { if far.x <= far.y && far.x <= far.z { 0 } else if far.y <= far.z { 1 } else { 2 } }
                else if near.x >= near.y && near.x >= near.z { 0 } else if near.y >= near.z { 1 } else { 2 };
            let mut n = Vec3::ZERO;
            n[axis] = if from_inside { dir[axis].signum() } else { -dir[axis].signum() };
            best = Some((t, i, n));
        }
        best
    }

    fn visible(&self, from: Vec3, to: Vec3) -> bool {
        let d = to - from;
        let len = d.length();
        len < 1e-4 || self.hit(from, d / len, 1e-3, len - 1e-3).is_none()
    }

    /// Direct irradiance at `p` facing `n` from the local lights (shadowed).
    pub fn direct(&self, p: Vec3, n: Vec3) -> Vec3 {
        let lifted = p + n * 2e-3;
        self.lights.iter().map(|lamp| {
            let Some((to_light, e)) = crate::sprites::light_at(lamp, p) else { return Vec3::ZERO };
            let cos = n.dot(to_light);
            if cos <= 0.0 || !self.visible(lifted, lamp.position) { return Vec3::ZERO; }
            e * cos
        }).sum()
    }

    /// Radiance arriving at `origin` from `dir`: emission plus diffuse
    /// reflection of direct light, and of `bounce` (the previous pass).
    pub fn radiance(&self, origin: Vec3, dir: Vec3, bounce: Option<(&Grid, &[Probe])>) -> Vec3 {
        let Some((t, i, n)) = self.hit(origin, dir, 1e-3, 1e4) else { return self.sky };
        let b = &self.blocks[i];
        let p = origin + dir * t;
        let mut e = self.direct(p, n);
        if let Some((grid, probes)) = bounce { e += sample(grid, probes, p + n * grid.cell * 0.5, n); }
        b.emission + b.albedo * e / PI
    }

    /// Bake `bounces` passes (1: direct light and emission seen from each
    /// probe; each further pass adds one more diffuse bounce). `rays` per
    /// axis hemisphere (cosine-weighted, stratified, deterministic).
    pub fn bake(&self, grid: &Grid, rays: u32, bounces: u32) -> Result<Vec<Probe>, String> {
        grid.validate()?;
        let rays = rays.clamp(4, 4096);
        let mut previous: Option<Vec<Probe>> = None;
        for _ in 0..bounces.max(1) {
            let count = grid.count();
            let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(16);
            let per = count.div_ceil(threads).max(1);
            let prev = previous.as_deref();
            let probes: Vec<Probe> = crate::par::scope(|scope| {
                let handles: Vec<_> = (0..count).step_by(per).map(|start| scope.spawn(move || {
                    (start..(start + per).min(count)).map(|i| {
                        let p = grid.position(i);
                        let axes = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z].map(|axis| {
                            let (t, bt) = axis.any_orthonormal_pair();
                            let side = (rays as f32).sqrt().ceil() as u32;
                            let mut sum = Vec3::ZERO;
                            for k in 0..side * side {
                                let (u, v) = (((k % side) as f32 + 0.5) / side as f32, ((k / side) as f32 + 0.5) / side as f32);
                                let (r, phi) = (u.sqrt(), 2.0 * PI * v);
                                let d = (t * (r * phi.cos()) + bt * (r * phi.sin()) + axis * (1.0 - u).max(0.0).sqrt()).normalize();
                                sum += self.radiance(p, d, prev.map(|probes| (grid, probes)));
                            }
                            // Cosine-weighted mean radiance times pi is irradiance.
                            sum / (side * side) as f32 * PI
                        });
                        Probe { axes }
                    }).collect::<Vec<_>>()
                })).collect();
                handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
            });
            previous = Some(probes);
        }
        Ok(previous.unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> BoxRoom {
        let wall = Vec3::splat(0.6);
        let t = 0.2;
        let (lo, hi) = (Vec3::new(-3.0, 0.0, -2.0), Vec3::new(3.0, 3.0, 2.0));
        let mut blocks = vec![];
        for axis in 0..3 {
            for side in [false, true] {
                let (mut min, mut max) = (lo - Vec3::splat(t), hi + Vec3::splat(t));
                if side { min[axis] = hi[axis]; } else { max[axis] = lo[axis]; }
                blocks.push(Block { min, max, albedo: wall, emission: Vec3::ZERO });
            }
        }
        // A neon tube under the ceiling.
        blocks.push(Block { min: Vec3::new(-1.0, 2.8, -0.05), max: Vec3::new(1.0, 2.9, 0.05), albedo: Vec3::ZERO, emission: Vec3::new(400.0, 100.0, 400.0) });
        BoxRoom { blocks, lights: vec![], sky: Vec3::splat(1000.0) }
    }

    #[test]
    fn packed_probes_round_trip_and_sample_trilinearly() {
        let grid = Grid { origin: Vec3::ZERO, cell: 1.0, dims: [2, 2, 2] };
        let probes: Vec<Probe> = (0..8).map(|i| Probe { axes: [Vec3::splat(i as f32); 6] }).collect();
        let words = pack(&grid, &probes).unwrap();
        assert_eq!(words.len(), HEADER_WORDS + 8 * PROBE_WORDS);
        assert_eq!(f32::from_bits(words[HEADER_WORDS + PROBE_WORDS]), 1.0);
        assert_eq!(sample(&grid, &probes, Vec3::splat(0.5), Vec3::Y), Vec3::splat(3.5));
        assert_eq!(sample(&grid, &probes, Vec3::splat(9.0), Vec3::Y), Vec3::splat(7.0), "clamped");
        let cube = Probe { axes: [Vec3::splat(4.0), Vec3::ZERO, Vec3::splat(2.0), Vec3::splat(1.0), Vec3::ZERO, Vec3::ZERO] };
        assert_eq!(cube.irradiance(Vec3::X), Vec3::splat(4.0));
        assert_eq!(cube.irradiance(Vec3::NEG_Y), Vec3::splat(1.0));
        assert!((cube.irradiance(Vec3::new(1.0, 1.0, 0.0).normalize()) - Vec3::splat(3.0)).length() < 1e-5);
        assert!(pack(&Grid { dims: [1, 2, 2], ..grid }, &probes[..4]).is_err());
    }

    #[test]
    fn a_closed_room_is_lit_by_its_tube_not_by_the_sky_and_bounces_add_light() {
        let room = room();
        let grid = Grid { origin: Vec3::new(-2.5, 0.5, -1.5), cell: 1.0, dims: [6, 3, 4] };
        let one = room.bake(&grid, 64, 1).unwrap();
        let two = room.bake(&grid, 64, 2).unwrap();
        let floor_up = sample(&grid, &one, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
        let floor_down = sample(&grid, &one, Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Y);
        assert!(floor_up.x > 10.0 && floor_up.x > 3.0 * floor_up.y, "magenta tube light from above: {floor_up}");
        assert!(floor_down.x < 1e-3, "no sky leaks in and one pass sees no floor bounce: {floor_down}");
        let bounced = sample(&grid, &two, Vec3::new(0.0, 0.5, 0.0), Vec3::NEG_Y);
        assert!(bounced.x > 0.5, "the second pass lights the floor-facing side: {bounced}");
        // A local lamp in a corner, shadowed by nothing, lights the opposite wall side.
        let mut lamp_room = room.clone();
        lamp_room.blocks.pop();
        lamp_room.lights.push(LocalLight { position: Vec3::new(2.5, 2.5, 1.5), direction: Vec3::NEG_Y, intensity_cd: Vec3::splat(500.0), range_m: 10.0,
            inner_cos: -0.9, outer_cos: -1.0, source_radius_m: 0.05, falloff: 2.0 });
        let lit = lamp_room.bake(&grid, 64, 1).unwrap();
        assert!(sample(&grid, &lit, Vec3::new(-2.0, 0.5, -1.0), Vec3::new(1.0, 0.0, 0.0)).x > 1.0, "wall reflection of the lamp reaches the far corner");
    }

    /// The same closed room rendered dark, then with its baked probes: the
    /// floor away from any direct light becomes visibly lit by the tube.
    #[test]
    fn gpu_shade_pass_uses_the_volume_inside_the_room() {
        use crate::scene::{Material, EMISSIVE};
        use glam::Affine3A;
        let room = room();
        let gpu = crate::Gpu::new().unwrap();
        let mut scene = crate::Scene::default();
        scene.sky = Some((4, 2, vec![[0.0, 0.0, 0.0, 1.0]; 8]));
        for block in &room.blocks {
            let flags = if block.emission.max_element() > 0.0 { EMISSIVE } else { 0 };
            let tint = if flags == EMISSIVE { (block.emission / 400.0).to_array() } else { block.albedo.to_array() };
            let m = scene.add_material(Material { tint, layer: -1, roughness: 0.9, flags, scale: 400.0, metal: 0.0 });
            let mesh = scene.add_mesh(&crate::primitives::cube(m).mesh());
            scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(block.max - block.min, glam::Quat::IDENTITY, (block.min + block.max) * 0.5));
        }
        let grid = Grid { origin: Vec3::new(-2.5, 0.5, -1.5), cell: 1.0, dims: [6, 3, 4] };
        scene.reserve_irradiance_volume(grid).unwrap();
        let mut light = crate::Lighting::default();
        light.sun_lux = 0.0;
        light.sky_lux = 0.0;
        light.ev100 = 4.0;
        let renderer = crate::Renderer::new(&gpu, &scene, &light);
        let camera = crate::Camera { pos: Vec3::new(0.0, 2.0, 1.8), target: Vec3::new(0.0, 0.0, -1.0), fov_y: 60f32.to_radians(), near: 0.05 };
        let floor = |rgba: &[u8]| { let (w, h) = (64u32, 48u32); let i = (((h * 3 / 4) * w + w / 2) * 4) as usize; rgba[i] as u32 + rgba[i + 1] as u32 + rgba[i + 2] as u32 };
        let (dark, _) = renderer.render(&gpu, &camera, &light, 64, 48);
        renderer.set_irradiance_volume(&gpu, &room.bake(&grid, 64, 2).unwrap()).unwrap();
        let (lit, _) = renderer.render(&gpu, &camera, &light, 64, 48);
        assert!(floor(&lit) > floor(&dark) + 60, "probe light on the floor: dark {} lit {}", floor(&dark), floor(&lit));
    }
}

#[cfg(test)]
mod emission_pack_tests {
    use super::*;
    #[test]
    fn packed_emission_preserves_hdr_and_fractional_lux() {
        for rgb in [Vec3::ZERO,Vec3::new(0.1,0.02,0.001),Vec3::new(25000.0,4000.0,300.0),Vec3::splat(65408.0)] {
            let w=pack_emission(rgb);
            let scale=2f32.powi((w>>27) as i32-24);
            let decoded=Vec3::new((w&511) as f32,((w>>9)&511) as f32,((w>>18)&511) as f32)*scale;
            assert!(decoded.abs_diff_eq(rgb,scale*0.501));
        }
        let grid=Grid {origin:Vec3::ZERO,cell:1.0,dims:[2,2,2]};
        let sky=vec![Probe {axes:[Vec3::new(0.5,0.2,2.0);6]};8];
        let emission=vec![Probe {axes:[Vec3::splat(1000.0);6]};8];
        let planes=vec![[1.0,0.0,0.0,-2.0];8];
        let words=pack_sky_emission(&grid,&sky,&emission,&planes).unwrap();
        assert_eq!(words.len(),8+18*8); assert_eq!(words[7],7);
        assert_ne!(words[8]&0x80000000,0);
        for j in 0..4 { assert_eq!(f32::from_bits(words[8+j*3+2]),planes[0][j]); }
    }
}
