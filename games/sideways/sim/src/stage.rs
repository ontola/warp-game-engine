//! Deterministic rally layout and shared terrain/surface queries. Rendering,
//! placement and physics consume the same stage. The original layout keeps
//! its f64 scalar/f32 vector arithmetic; current terrain and road-surface
//! design intentionally evolve independently of the earlier engine version.

use crate::surfaces::Surface;
use gdcompat::math::{V2, V3, lerp, signf, smoothstep, v2, v3};
use gdcompat::{Noise, Pcg};
use std::collections::HashMap;

pub const STEP: f64 = 2.0;
pub const CELL: f64 = 32.0;
pub const REACH: f64 = 70.0;
pub const CHUNK: f64 = 64.0;
pub const GRID: f64 = 2.0;
pub const CORRIDOR: f64 = 230.0;
pub const RUNOUT: f64 = 140.0;
pub const RIVER_STEP: f64 = 4.0;

#[derive(Clone, Debug)]
pub struct River {
    pub pts: Vec<V2>,
    pub levels: Vec<f32>,
    pub wet: Vec<u8>,
    pub half: f64,
    pub depth: f64,
    pub cross: usize,
    pub ford: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Note {
    pub at: f64,
    pub kind: &'static str,
    pub grade: i32,
    pub end: f64,
}

/// Where a point is relative to the centre line.
#[derive(Clone, Copy, Debug)]
pub struct Located {
    pub index: i64,
    pub along: f64,
    pub offset: f64,
    pub height: f64,
    pub right: V3,
}

impl Located {
    const NONE: Located = Located { index: -1, along: 0.0, offset: f64::INFINITY, height: 0.0, right: V3::ZERO };
}

type Cell = (i32, i32);

fn cell_of(x: f64, z: f64) -> Cell {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

pub struct Stage {
    pub vegetation: crate::vegetation::Vegetation,
    pub points: Vec<V3>,
    pub dirs: Vec<V3>,
    pub widths: Vec<f32>,
    pub road_surfaces: Vec<u8>,
    pub curvature: Vec<f32>,
    pub length: f64,
    pub finish: f64,
    pub ledges: Vec<f32>,
    pub ledge_wall: Vec<f32>,
    pub ledge_drop: Vec<f32>,
    pub rivers: Vec<River>,
    pub notes: Vec<Note>,
    pub noise_seed: i32,
    pub detail_seed: i32,
    noise: Noise,
    pub detail: Noise,
    hash: HashMap<Cell, Vec<i32>>,
    locate_candidates: HashMap<Cell, Vec<i32>>,
    river_candidates: HashMap<Cell, Vec<(usize, usize)>>,
}

impl Stage {
    /// `Stage.generate(target_length)`, drawing from the global RNG.
    pub fn generate(rng: &mut Pcg, target_length: f64) -> Stage {
        // FastNoiseLite stores the seed as a 32-bit int.
        let noise_seed = rng.next_u32() as i32;
        let detail_seed = rng.next_u32() as i32;
        let mut s = Stage {
            vegetation: Default::default(),
            points: vec![],
            dirs: vec![],
            widths: vec![],
            road_surfaces: vec![],
            curvature: vec![],
            length: 0.0,
            finish: 0.0,
            ledges: vec![],
            ledge_wall: vec![],
            ledge_drop: vec![],
            rivers: vec![],
            notes: vec![],
            noise_seed,
            detail_seed,
            noise: Noise::new(noise_seed, 1.0 / 420.0, 3),
            detail: Noise::new(detail_seed, 1.0 / 70.0, 2),
            hash: HashMap::new(),
            locate_candidates: HashMap::new(),
            river_candidates: HashMap::new(),
        };
        s.layout(rng, target_length);
        s.profile(rng);
        s.make_ledges(rng);
        s.make_rivers(rng);
        s.index();
        s.pace_notes();
        s.make_road_surfaces();
        s
    }

    fn make_road_surfaces(&mut self) {
        // One continuous gravel road. Arbitrary asphalt/dirt/mud bands did
        // not correspond to the landscape or a change of road. Future types
        // belong to authored junctions (including width) or wet low ground.
        // The same rows feed rendering, tyre grip and AI look-ahead.
        self.road_surfaces = vec![Surface::Gravel as u8; self.points.len()];
    }

    pub fn road_surface_at(&self, along: f64) -> Surface {
        if self.road_surfaces.is_empty() {
            return Surface::Gravel;
        }
        let i = ((along / STEP).floor() as i64).clamp(0, self.road_surfaces.len() as i64 - 1);
        Surface::from_u8(self.road_surfaces[i as usize])
    }

    fn layout(&mut self, rng: &mut Pcg, target_length: f64) {
        self.points = vec![V3::ZERO];
        let mut remaining = ((target_length / STEP).ceil() as i64).max(0);
        let mut p = V2::default();
        let mut heading = 0.0f64;
        let mut straight_steps = remaining.min((120.0f64 / STEP).ceil() as i64);
        let mut fast_corners = 0;
        let step = STEP as f32;
        while remaining > 0 {
            for _ in 0..straight_steps.min(remaining) {
                p += v2(heading.sin() as f32, -heading.cos() as f32) * step;
                self.points.push(v3(p.x, 0.0, p.y));
                remaining -= 1;
            }
            if remaining == 0 {
                break;
            }
            let mut grade = pick_grade(rng);
            if fast_corners >= 2 {
                grade = grade.min(2);
            }
            fast_corners = if grade > 2 { fast_corners + 1 } else { 0 };
            let radius = [14.0, 24.0, 38.0, 60.0, 95.0, 150.0, 240.0][grade];
            let mut turn_sign = -signf(heading);
            if turn_sign == 0.0 {
                turn_sign = if rng.randf_global() < 0.5 { 1.0 } else { -1.0 };
            }
            let target_heading = turn_sign * rng.randf_range(20.0, 50.0).to_radians_godot();
            let delta = target_heading - heading;
            let turn_steps = ((1.5 * radius * delta.abs() / STEP).ceil() as i64).max(1);
            for st in 0..turn_steps.min(remaining) {
                let t = (st as f64 + 0.5) / turn_steps as f64;
                let eased = t * t * (3.0 - 2.0 * t);
                let direction = heading + delta * eased;
                p += v2(direction.sin() as f32, -direction.cos() as f32) * step;
                self.points.push(v3(p.x, 0.0, p.y));
                remaining -= 1;
            }
            heading = target_heading;
            if remaining > 0 {
                straight_steps = ((rng.randf_range(15.0, 155.0) / STEP).round() as i64).clamp(8, 77);
            }
        }
    }

    fn profile(&mut self, rng: &mut Pcg) {
        let n = self.points.len();
        let raw: Vec<f32> = self.points.iter().map(|p| self.ground(p.x as f64, p.z as f64) as f32).collect();
        let mut smooth = raw;
        for _ in 0..3 {
            let mut out = smooth.clone();
            let r = 15usize;
            for i in 0..n {
                let mut acc = 0.0f64;
                let mut cnt = 0;
                for j in i.saturating_sub(r)..(i + r + 1).min(n) {
                    acc += smooth[j] as f64;
                    cnt += 1;
                }
                out[i] = (acc / cnt as f64) as f32;
            }
            smooth = out;
        }
        let max_d = 0.11 * STEP;
        for i in 1..n {
            let prev = smooth[i - 1] as f64;
            smooth[i] = (smooth[i] as f64).clamp(prev - max_d, prev + max_d) as f32;
        }
        for i in (0..n.saturating_sub(1)).rev() {
            let next = smooth[i + 1] as f64;
            smooth[i] = (smooth[i] as f64).clamp(next - max_d, next + max_d) as f32;
        }
        self.dirs = (0..n)
            .map(|i| (self.points[(i + 1).min(n - 1)] - self.points[i.saturating_sub(1)]).normalized())
            .collect();
        self.curvature = (0..n)
            .map(|i| {
                let lo = i.saturating_sub(3);
                let hi = (i + 3).min(n - 1);
                let a = self.dirs[lo];
                let b = self.dirs[hi];
                let turn = (a.cross(b).y as f64).atan2(a.dot(b) as f64);
                (turn / (STEP * (hi - lo) as f64)) as f32
            })
            .collect();
        let mut i = 40usize;
        while n > 40 && i < n - 40 {
            let straight = (i - 15..i + 15).all(|j| (self.curvature[j] as f64).abs() <= 1.0 / 400.0);
            if straight && (rng.randf_global() as f64) < 0.35 {
                let height = rng.randf_range(0.6, 1.6);
                let half = rng.randf_range(5.0, 9.0);
                for j in i - 20..i + 20 {
                    let x = (j as f64 - i as f64) / half;
                    smooth[j] = (smooth[j] as f64 + height * (-x * x).exp()) as f32;
                }
                i += 60;
            } else {
                i += 5;
            }
        }
        self.widths = vec![0.0; n];
        for j in 0..n {
            self.points[j].y = smooth[j];
            self.widths[j] = (3.4 + 0.5 * self.detail.get_1d((j as f64 * 0.7) as f32) as f64) as f32;
        }
        self.length = (n - 1) as f64 * STEP;
        self.finish = self.length - RUNOUT;
    }

    fn make_ledges(&mut self, rng: &mut Pcg) {
        let n = self.points.len();
        self.ledges = vec![0.0; n];
        self.ledge_wall = vec![0.0; n];
        self.ledge_drop = vec![0.0; n];
        let mut i = 90usize;
        while n > 80 && i < n - 80 {
            if rng.randf_global() as f64 > 0.007 {
                i += 1;
                continue;
            }
            let len = rng.randi_range(60, 150) as usize;
            let end = (i + len).min(n - 60);
            let mid = (i + end) / 2;
            let right = v3(-self.dirs[mid].z, 0.0, self.dirs[mid].x);
            let p = self.points[mid];
            let (px, pz, rx, rz) = (p.x as f64, p.z as f64, right.x as f64, right.z as f64);
            let rise = self.ground(px + rx * 40.0, pz + rz * 40.0) - self.ground(px - rx * 40.0, pz - rz * 40.0);
            let side = if rise >= 0.0 { 1.0 } else { -1.0 };
            let wall = rng.randf_range(9.0, 22.0);
            let drop = rng.randf_range(14.0, 34.0);
            for j in i..=end {
                let ease = smoothstep(0.0, 15.0, (j - i) as f64).min(smoothstep(0.0, 15.0, (end - j) as f64));
                self.ledges[j] = (side * ease) as f32;
                self.ledge_wall[j] = wall as f32;
                self.ledge_drop[j] = drop as f32;
            }
            i = end + 100;
        }
    }

    fn make_rivers(&mut self, rng: &mut Pcg) {
        self.rivers.clear();
        let n = self.points.len();
        if n < 300 {
            self.index_rivers();
            return; // too short for a stream crossing (Godot would fail here)
        }
        let count = if self.length < 2500.0 { 1 } else { 2 };
        let mut tries = 0;
        while self.rivers.len() < count && tries < 40 {
            tries += 1;
            let c = rng.randi_range(150, n as i32 - 150) as usize;
            let mut clear = true;
            for j in c - 40..c + 40 {
                if self.ledges[j] != 0.0 || ((j as i64 - c as i64).abs() < 20 && (self.curvature[j] as f64).abs() > 1.0 / 90.0) {
                    clear = false;
                    break;
                }
            }
            for r in &self.rivers {
                if (r.cross as i64 - c as i64).abs() < 250 {
                    clear = false;
                }
            }
            if !clear {
                continue;
            }
            let mut ford = rng.randf_global() < 0.5;
            let half = rng.randf_range(2.5, 4.5);
            let depth = rng.randf_range(0.7, 1.3);
            let right = v2(-self.dirs[c].z, self.dirs[c].x);
            let mut land = f64::INFINITY;
            for k in -4i64..5 {
                let pk = self.points[(c as i64 + k) as usize];
                let q = v2(pk.x, pk.z);
                for sgn in [-1.0f32, 1.0] {
                    let b = q + (right * sgn) * ((self.widths[c] as f64 + 4.0) as f32);
                    land = land.min(self.ground(b.x as f64, b.y as f64));
                }
            }
            let road_h = self.points[c].y as f64;
            let mut level = if ford { (land - 0.3).min(road_h + 0.22) } else { (land - 0.5).min(road_h - 2.0) };
            if ford {
                let dip = road_h - (level - 0.22);
                if dip > 3.0 {
                    ford = false;
                    level = (land - 0.5).min(road_h - 2.0);
                } else {
                    let sigma = 6.0 + dip * 4.0;
                    let reach = (sigma * 2.5) as i64;
                    for j in c as i64 - reach..c as i64 + reach + 1 {
                        let x = (j - c as i64) as f64 / sigma;
                        let y = &mut self.points[j as usize].y;
                        *y = (*y as f64 - dip * (-x * x).exp()) as f32;
                    }
                }
            }
            let heading = right.angle() as f64 + rng.randf_range(-0.4, 0.4);
            let half_len = 75;
            let mut up_pts = Vec::new();
            let mut down_pts = Vec::new();
            for way in [1.0f64, -1.0] {
                let mut p = v2(self.points[c].x, self.points[c].z);
                let mut hd = if way > 0.0 { heading } else { heading + std::f64::consts::PI };
                for k in 0..half_len {
                    hd += self.detail.get_1d((k as f64 * 3.1 + way * 50.0 + c as f64) as f32) as f64 * 0.35;
                    p += v2(hd.cos() as f32, hd.sin() as f32) * RIVER_STEP as f32;
                    if way > 0.0 { down_pts.push(p) } else { up_pts.push(p) }
                }
            }
            let mut pts: Vec<V2> = up_pts.iter().rev().cloned().collect();
            pts.push(v2(self.points[c].x, self.points[c].z));
            pts.extend(down_pts.iter().cloned());
            let mid = up_pts.len();
            let mut levels = vec![0.0f32; pts.len()];
            levels[mid] = level as f32;
            let flat = 18;
            for k in (0..mid).rev() {
                let t = self.ground(pts[k].x as f64, pts[k].y as f64) - 0.6;
                let nx = levels[k + 1] as f64;
                levels[k] = if mid - k < flat { nx } else { nx.max(t.min(nx + 0.12 * RIVER_STEP)) } as f32;
            }
            for k in mid + 1..pts.len() {
                let t = self.ground(pts[k].x as f64, pts[k].y as f64) - 0.6;
                let pv = levels[k - 1] as f64;
                levels[k] = if k - mid < flat { pv } else { pv.min(t.max(pv - 0.12 * RIVER_STEP)) } as f32;
            }
            let mut wet = vec![0u8; pts.len()];
            for k in 0..pts.len() {
                let g = self.ground(pts[k].x as f64, pts[k].y as f64);
                let l = levels[k] as f64;
                wet[k] = (g > l - 0.3 && g < l + 9.0) as u8;
            }
            wet[mid] = 1;
            self.rivers.push(River { pts, levels, wet, half, depth, cross: c, ford });
        }
        self.index_rivers();
    }

    fn index_rivers(&mut self) {
        let mut h: HashMap<Cell, Vec<(usize, usize)>> = HashMap::new();
        for (ri, r) in self.rivers.iter().enumerate() {
            for (k, p) in r.pts.iter().enumerate() {
                h.entry(cell_of(p.x as f64, p.y as f64)).or_default().push((ri, k));
            }
        }
        self.river_candidates = neighbourhoods(&h);
    }

    /// Nearest stream point within `reach`: (river, point, distance, level).
    fn river_near(&self, x: f64, z: f64, reach: f64) -> Option<(usize, usize, f64, f64)> {
        if self.rivers.is_empty() {
            return None;
        }
        let q = v2(x as f32, z as f32);
        let mut best = None;
        let mut best_d = reach * reach;
        for &(ri, k) in self.river_candidates.get(&cell_of(x, z)).map(|v| v.as_slice()).unwrap_or(&[]) {
            let d = q.distance_squared_to(self.rivers[ri].pts[k]) as f64;
            if d < best_d {
                best_d = d;
                best = Some((ri, k));
            }
        }
        let (ri, k) = best?;
        let r = &self.rivers[ri];
        let mut bd = f64::INFINITY;
        let mut level = 0.0;
        for s in [k as i64 - 1, k as i64] {
            if s < 0 || s as usize + 1 >= r.pts.len() {
                continue;
            }
            let s = s as usize;
            let (a, b) = (r.pts[s], r.pts[s + 1]);
            let t = ((q - a).dot(b - a) as f64 / (b - a).length_squared() as f64).clamp(0.0, 1.0);
            let d = q.distance_to(a.lerp(b, t as f32)) as f64;
            if d < bd {
                bd = d;
                level = lerp(r.levels[s] as f64, r.levels[s + 1] as f64, t);
            }
        }
        Some((ri, k, bd, level))
    }

    /// Water surface height at (x, z), or -inf where it is dry.
    pub fn water_level(&self, x: f64, z: f64) -> f64 {
        match self.river_near(x, z, 12.0) {
            Some((ri, k, d, level)) => {
                let r = &self.rivers[ri];
                if r.wet[k] == 0 || d > r.half + 0.5 { f64::NEG_INFINITY } else { level }
            }
            None => f64::NEG_INFINITY,
        }
    }

    fn index(&mut self) {
        self.hash.clear();
        for (i, p) in self.points.iter().enumerate() {
            self.hash.entry(cell_of(p.x as f64, p.z as f64)).or_default().push(i as i32);
        }
        self.locate_candidates = neighbourhoods(&self.hash);
    }

    /// Nearest centre-line point to `p`, horizontally.
    pub fn locate(&self, p: V3, hint: i64) -> Located {
        let mut best = -1i64;
        let mut best_d = f64::INFINITY;
        let n = self.points.len() as i64;
        let d2 = |j: usize| {
            let q = self.points[j];
            v2((q.x as f64 - p.x as f64) as f32, (q.z as f64 - p.z as f64) as f32).length_squared() as f64
        };
        if hint >= 0 {
            for j in (hint - 30).max(0)..(hint + 30).min(n) {
                let d = d2(j as usize);
                if d < best_d {
                    best_d = d;
                    best = j;
                }
            }
            if best_d < 30.0 * 30.0 {
                return self.result(p, best as usize);
            }
        }
        if let Some(c) = self.locate_candidates.get(&cell_of(p.x as f64, p.z as f64)) {
            for &j in c {
                let d = d2(j as usize);
                if d < best_d {
                    best_d = d;
                    best = j as i64;
                }
            }
        }
        if best < 0 {
            return Located::NONE;
        }
        self.result(p, best as usize)
    }

    fn result(&self, p: V3, i: usize) -> Located {
        let d = self.dirs[i];
        let rel = p - self.points[i];
        let (rx, rz) = (rel.x as f64, rel.z as f64);
        let t = (rx * d.x as f64 + rz * d.z as f64).clamp(-STEP, STEP);
        let right = v3(-d.z, 0.0, d.x);
        let offset = rx * right.x as f64 + rz * right.z as f64;
        let j = (i as i64 + if t > 0.0 { 1 } else { -1 }).clamp(0, self.points.len() as i64 - 1) as usize;
        let h = lerp(self.points[i].y as f64, self.points[j].y as f64, t.abs() / STEP);
        Located { index: i as i64, along: i as f64 * STEP + t, offset, height: h, right }
    }

    pub fn point_at(&self, along: f64) -> V3 {
        let f = (along / STEP).clamp(0.0, self.points.len() as f64 - 1.001);
        let i = f as usize;
        self.points[i].lerp(self.points[i + 1], (f - i as f64) as f32)
    }

    fn row(&self, along: f64, len: usize) -> usize {
        ((along / STEP) as i64).clamp(0, len as i64 - 1) as usize
    }

    pub fn dir_at(&self, along: f64) -> V3 {
        self.dirs[self.row(along, self.dirs.len())]
    }

    pub fn width_at(&self, along: f64) -> f64 {
        self.widths[self.row(along, self.widths.len())] as f64
    }

    pub fn curvature_at(&self, along: f64) -> f64 {
        self.curvature[self.row(along, self.curvature.len())] as f64
    }

    pub fn surface_at(&self, p: V3) -> Surface {
        self.surface_near(p, -1).0
    }

    /// Surface at `p` and the road index found (pass it back as the hint).
    pub fn surface_near(&self, p: V3, hint: i64) -> (Surface, i64) {
        let r = self.locate(p, hint);
        (self.surface_from(&r), r.index)
    }

    fn surface_from(&self, r: &Located) -> Surface {
        let off = r.offset.abs();
        let w = self.width_at(r.along);
        if off < w - 0.9 {
            self.road_surface_at(r.along)
        } else if off < w + 0.6 {
            Surface::Loose
        } else {
            Surface::Grass
        }
    }

    /// Natural ground without the road.
    pub fn ground(&self, x: f64, z: f64) -> f64 {
        self.noise.get_2d(x as f32, z as f32) as f64 * 34.0 + self.detail.get_2d(x as f32, z as f32) as f64 * 3.0
    }

    fn carve(&self, x: f64, z: f64, h: f64, near: i64) -> f64 {
        let Some((ri, k, d, level)) = self.river_near(x, z, 40.0) else {
            return h;
        };
        let r = &self.rivers[ri];
        // The generated stream can meander back beside another road section.
        // `wet == 0` rejects portions whose level does not fit the terrain;
        // water_level already respects this mask. Carving those rejected
        // portions anyway dug deep, invisible channels under the road ribbon.
        if r.wet[k] == 0 {
            return h;
        }
        let half = r.half;
        let mut bed = level - r.depth * (1.0 - smoothstep(half * 0.45, half, d)) + 0.25 * smoothstep(half * 0.8, half, d);
        bed += (d - half).max(0.0) * 0.5;
        let carved = h.min(bed);
        // A river used to impose its 1:2 bank plane up to 40 m away,
        // including past an endpoint. That sliced nearby cut faces into
        // cones/wedges (the reported #4 crest lost almost 9 m). The wet
        // channel stays exact; only its dry outer bank joins back into the
        // authoritative terrain, with a compact, C1-continuous influence.
        if d <= half + 2.0 { return carved; }
        let road_weight = if near >= 0 {
            let at = self.result(v3(x as f32, 0.0, z as f32), near as usize);
            let edge = at.offset.abs() - self.widths[near as usize] as f64;
            // Preserve every native triangle touching road + 2 m, not only
            // sample points in that strip: one grid diagonal is its halo.
            let guard = 2.0 + GRID * std::f64::consts::SQRT_2;
            if edge <= guard { return carved; }
            smoothstep(guard, guard + 2.0, edge)
        } else { 1.0 };
        let bank_weight = 1.0 - smoothstep(half + 2.0, half + 12.0, d);
        // Round the terrain/bank intersection without an infinite-support
        // blur. The maximum extra incision is 0.1875 m and is zero where
        // the two surfaces differ by at least 0.75 m.
        let join = (1.0 - (h - bed).abs() / 0.75).max(0.0);
        let rounded = carved - 0.1875 * join * join * smoothstep(half + 2.0, half + 4.0, d);
        if bank_weight == 0.0 && road_weight == 1.0 { return h; }
        lerp(carved, lerp(h, rounded, bank_weight), road_weight)
    }

    pub fn height_near(&self, x: f64, z: f64, near: i64) -> f64 {
        self.carve(x, z, self.height_near_raw(x, z, near), near)
    }

    fn height_near_raw(&self, x: f64, z: f64, near: i64) -> f64 {
        let natural = self.ground(x, z);
        if near < 0 {
            return natural;
        }
        let near = near as usize;
        let r = self.result(v3(x as f32, 0.0, z as f32), near);
        let off = r.offset.abs();
        if off > REACH {
            return natural;
        }
        let w = self.widths[near] as f64;
        let road_h = r.height - 0.06 * (off / w).clamp(0.0, 1.0);
        if off < w + 0.8 {
            return road_h - 0.08;
        }
        // The old 55 cm Gaussian ditch was narrower than one native 2 m
        // terrain cell. Oblique roads sampled it at alternating phases,
        // creating repeated triangular pits instead of a continuous verge.
        // Carry the existing foundation inset into the surroundings over
        // two native cells, with zero endpoint slope. The road ribbon/ruts
        // and the foundation inside w + 0.8 remain exactly unchanged.
        let verge = -0.08 * (1.0 - smoothstep(w + 0.8, w + 0.8 + 2.0 * GRID, off));
        let reach = ((natural - road_h).abs() / 0.32).clamp(12.0, 60.0);
        let blend = smoothstep(w + 3.0, w + 3.0 + reach, off);
        let normal = lerp(road_h, natural, blend) + verge * (1.0 - blend * 0.6);
        // Ledge strength used to jump to the nearest 2 m road row. Near a
        // cut's ends that became a metre-scale stair even on a dense mesh.
        // Interpolate the signed wall/drop amplitudes, not the height and
        // envelope independently (which would attenuate the end twice).
        let row = (r.along / STEP).clamp(0.0, (self.points.len() - 1) as f64);
        let a = row.floor() as usize;
        let b = (a + 1).min(self.points.len() - 1);
        let t = row.fract();
        let ledge = lerp(self.ledges[a] as f64, self.ledges[b] as f64, t);
        if ledge == 0.0 {
            return normal;
        }
        // Generated cuts are separated, but keep edited/opposite-sign
        // neighbours bounded instead of dividing by their cancelled sign.
        let strength = lerp(self.ledges[a].abs() as f64, self.ledges[b].abs() as f64, t);
        let wall = lerp((self.ledge_wall[a] * self.ledges[a].abs()) as f64,
            (self.ledge_wall[b] * self.ledges[b].abs()) as f64, t) / strength;
        let drop = lerp((self.ledge_drop[a] * self.ledges[a].abs()) as f64,
            (self.ledge_drop[b] * self.ledges[b].abs()) as f64, t) / strength;
        let back = smoothstep(w + 14.0, REACH - 5.0, off);
        if signf(r.offset) == signf(ledge) {
            // A 4.6 m cut sampled on the shared 2 m grid had only two cells
            // across a 9-22 m face. Its grid diagonals formed regular toe
            // pyramids. Give the toe/crown a resolved, varying footprint;
            // keep the same high plateau and a steep central rock face.
            // World noise is coherent across road rows, chunks and physics.
            let toe = 2.4 + 0.95 * self.detail.get_2d((x * 2.3 + 113.0) as f32, (z * 2.3 - 71.0) as f32) as f64;
            let span = 7.6 + 1.6 * self.detail.get_2d((x * 1.7 - 37.0) as f32, (z * 1.7 + 159.0) as f32) as f64;
            let face = smoothstep(w + toe, w + toe + span, off);
            let mut rock = road_h
                + wall * face
                + self.detail.get_2d((x * 3.0) as f32, (z * 3.0) as f32) as f64 * 1.4 * face * (1.0 - face);
            rock = lerp(rock, natural, back);
            return lerp(normal, normal.max(rock), ledge.abs());
        }
        let mut fall = road_h - 0.1 - drop * smoothstep(w + 1.9, w + 11.0, off);
        fall = lerp(fall, natural, back);
        lerp(normal, normal.min(fall), ledge.abs())
    }

    pub fn height(&self, x: f64, z: f64) -> f64 {
        self.height_near(x, z, self.nearest_index(x, z))
    }

    pub fn nearest_index(&self, x: f64, z: f64) -> i64 {
        let c = cell_of(x, z);
        let mut best = -1i64;
        let mut best_d = REACH * REACH;
        for dx in -3..=3 {
            for dy in -3..=3 {
                if let Some(list) = self.hash.get(&(c.0 + dx, c.1 + dy)) {
                    for &j in list {
                        let p = self.points[j as usize];
                        let d = v2((p.x as f64 - x) as f32, (p.z as f64 - z) as f32).length_squared() as f64;
                        if d < best_d {
                            best_d = d;
                            best = j as i64;
                        }
                    }
                }
            }
        }
        best
    }

    /// Runs of consecutive road indices that may be nearest to anything in
    /// the square.
    pub fn runs_near(&self, x0: f64, z0: f64, size: f64, margin: f64) -> Vec<Vec<i32>> {
        let mut idx = Vec::new();
        let c0 = cell_of(x0 - margin, z0 - margin);
        let c1 = cell_of(x0 + size + margin, z0 + size + margin);
        for cx in c0.0..=c1.0 {
            for cy in c0.1..=c1.1 {
                if let Some(list) = self.hash.get(&(cx, cy)) {
                    for &j in list {
                        let p = self.points[j as usize];
                        let (px, pz) = (p.x as f64, p.z as f64);
                        if px > x0 - margin && px < x0 + size + margin && pz > z0 - margin && pz < z0 + size + margin {
                            idx.push(j);
                        }
                    }
                }
            }
        }
        idx.sort();
        let mut runs: Vec<Vec<i32>> = Vec::new();
        for j in idx {
            match runs.last_mut() {
                Some(r) if j - r[r.len() - 1] <= 3 => r.push(j),
                _ => runs.push(vec![j]),
            }
        }
        runs
    }

    fn pace_notes(&mut self) {
        self.notes.clear();
        let n = self.curvature.len();
        let mut i = 0;
        while i < n {
            let k = self.curvature[i] as f64;
            if k.abs() > 1.0 / 300.0 {
                let mut j = i;
                let mut peak = 0.0f64;
                let mut turn = 0.0f64;
                while j < n && (self.curvature[j] as f64).abs() > 1.0 / 400.0 && signf(self.curvature[j] as f64) == signf(k) {
                    peak = peak.max((self.curvature[j] as f64).abs());
                    turn += (self.curvature[j] as f64).abs() * STEP;
                    j += 1;
                }
                if turn > 12.0f64.to_radians_godot() {
                    let radius = 1.0 / peak;
                    let mut grade = 6;
                    for (lim, g) in [(20.0, 1), (32.0, 2), (50.0, 3), (80.0, 4), (125.0, 5)] {
                        if radius < lim {
                            grade = g;
                            break;
                        }
                    }
                    if radius < 17.0 {
                        grade = 0;
                    }
                    self.notes.push(Note { at: i as f64 * STEP, kind: if k > 0.0 { "left" } else { "right" }, grade, end: j as f64 * STEP });
                }
                i = j + 1;
            } else {
                i += 1;
            }
        }
        let np = self.points.len();
        let mut j = 10;
        while np >= 20 && j < np - 10 {
            let before = self.points[j].y as f64 - self.points[j - 6].y as f64;
            let after = self.points[j + 6].y as f64 - self.points[j].y as f64;
            if before - after > 0.9 {
                let far = self.notes.last().map(|n| (n.at - j as f64 * STEP).abs() > 40.0).unwrap_or(true);
                if far {
                    self.notes.push(Note { at: j as f64 * STEP, kind: "crest", grade: 0, end: j as f64 * STEP });
                }
            }
            j += 4;
        }
        // Godot's sort_custom is not stable; ties do not occur in practice
        // because corners and crests are 40 m apart.
        self.notes.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap());
    }
}

trait DegToRad {
    fn to_radians_godot(self) -> f64;
}

impl DegToRad for f64 {
    /// Godot's `deg_to_rad`: x * (PI / 180).
    fn to_radians_godot(self) -> f64 {
        self * (std::f64::consts::PI / 180.0)
    }
}

/// For every cell next to an occupied one: the 3x3 neighbourhood's entries,
/// in the order dx -1..1, dy -1..1 (the order Godot's loops visit them).
fn neighbourhoods<T: Clone>(index: &HashMap<Cell, Vec<T>>) -> HashMap<Cell, Vec<T>> {
    let mut out: HashMap<Cell, Vec<T>> = HashMap::new();
    for &(cx, cy) in index.keys() {
        for dx in -1..=1 {
            for dy in -1..=1 {
                let q = (cx + dx, cy + dy);
                if out.contains_key(&q) {
                    continue;
                }
                let mut list = Vec::new();
                for ex in -1..=1 {
                    for ey in -1..=1 {
                        if let Some(v) = index.get(&(q.0 + ex, q.1 + ey)) {
                            list.extend(v.iter().cloned());
                        }
                    }
                }
                out.insert(q, list);
            }
        }
    }
    out
}

fn pick_grade(rng: &mut Pcg) -> usize {
    let r = rng.randf_global() as f64;
    match r {
        r if r < 0.06 => 0,
        r if r < 0.20 => 1,
        r if r < 0.42 => 2,
        r if r < 0.64 => 3,
        r if r < 0.80 => 4,
        r if r < 0.92 => 5,
        _ => 6,
    }
}

#[cfg(test)]
mod road_surface_tests {
    use super::*;

    #[test]
    fn generated_road_has_one_surface_for_rendering_ai_and_wheel_queries() {
        for (seed, length) in [(1, 8000.0), (73, 3000.0), (9021, 800.0)] {
            let stage = Stage::generate(&mut Pcg::new(seed), length + RUNOUT);
            assert_eq!(stage.road_surfaces.len(), stage.points.len());
            for (index, point) in stage.points.iter().enumerate() {
                assert_eq!(stage.road_surfaces[index], Surface::Gravel as u8,
                    "render row changed surface at seed {seed}, row {index}");
                assert_eq!(stage.road_surface_at(index as f64 * STEP), Surface::Gravel);
                assert_eq!(stage.surface_near(*point, index as i64).0, Surface::Gravel,
                    "wheel query disagrees at seed {seed}, row {index}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn reported_round_two_stage() -> Stage {
    // F12 sideways-1792785290021-227228-2: seed 1, round 2, 3 km,
    // one human seat, no rivals. Replay only generation, not the race.
    let mut rng=Pcg::new(1);
    rng.skip(835);
    let stage=Stage::generate(&mut rng,3000.0+RUNOUT);
    assert_eq!((stage.noise_seed,stage.detail_seed),(370671355,-305494282));
    stage
}

#[cfg(test)]
mod cut_bank_tests {
    use super::*;

    fn foundation_digest(s: &Stage) -> (usize, u64) {
        let mut digest=0xcbf29ce484222325u64;let mut samples=0usize;
        for row in 0..s.points.len() {let p=s.points[row];let d=s.dirs[row];
            for side in [-1.0,1.0] {for step in 0..=24 {
                let off=(s.widths[row]as f64+2.0)*step as f64/24.0;
                let x=p.x as f64-d.z as f64*side*off;let z=p.z as f64+d.x as f64*side*off;
                for byte in s.height_near(x,z,row as i64).to_le_bytes(){digest=(digest^byte as u64).wrapping_mul(0x100000001b3);}
                samples+=1;
            }}
        }
        (samples,digest)
    }

    #[test]
    #[ignore = "recorded-camera shape diagnostic; run explicitly with --nocapture"]
    fn reported_fourth_capture_shape_metrics() {
        let s=reported_stage();
        let eye=v3(-18.910427,1.5267234,-1957.8578);
        let target=v3(-21.110065,0.37672335,-1964.1864);
        let f=(target-eye).normalized();let right=f.cross(V3::UP).normalized();let up=right.cross(f);
        let scale=(1.1170107126235962f32*0.5).tan();
        let mut hits=Vec::new();
        for (name,u,v) in [("left toe",0.29f32,0.43f32),("upper crest",0.48,0.21),("far rail",0.67,0.34)] {
            let d=(f+right*((2.0*u-1.0)*scale*16.0/9.0)+up*((1.0-2.0*v)*scale)).normalized();
            let mut previous=0.0;
            for k in 1..=2400 {
                let distance=k as f32*0.25;let p=eye+d*distance;
                if p.y as f64<=grid_height(&s,p.x as f64,p.z as f64) {
                    let(mut a,mut b)=(previous,distance);
                    for _ in 0..16 {let m=(a+b)*0.5;let q=eye+d*m;
                        if q.y as f64<=grid_height(&s,q.x as f64,q.z as f64){b=m;}else{a=m;}}
                    let q=eye+d*((a+b)*0.5);let r=s.locate(q,-1);
                    println!("F12 #4 {name}: world={q:?}, row={}, along={:.3}, edge={:.3}, ledge={:.4}, wall={:.3}, height={:.5}, raw={:.5}, grid={:.5}, river={:?}",
                        r.index,r.along,r.offset.abs()-s.width_at(r.along),s.ledges[r.index as usize],s.ledge_wall[r.index as usize],s.height(q.x as f64,q.z as f64),
                        s.height_near_raw(q.x as f64,q.z as f64,s.nearest_index(q.x as f64,q.z as f64)),q.y,s.river_near(q.x as f64,q.z as f64,40.0));
                    hits.push((name,r.index));break;
                }
                previous=distance;
            }
        }
        let mut sections=Vec::new();
        for (name,row) in hits {
            for row in (row-8).max(0)..=(row+8).min(s.points.len()as i64-1) {
                let p=s.points[row as usize];let d=s.dirs[row as usize];
                for side in [-1.0,1.0] {
                    for k in 0..=80 {let edge=2.0+k as f64*0.25;let off=s.widths[row as usize]as f64+edge;
                        let x=p.x as f64-d.z as f64*side*off;let z=p.z as f64+d.x as f64*side*off;
                        sections.push(serde_json::json!({"hit":name,"row":row,"side":side,"edge":edge,"x":x,"z":z,
                            "height":s.height(x,z),"raw":s.height_near_raw(x,z,s.nearest_index(x,z)),"grid":grid_height(&s,x,z),"natural":s.ground(x,z),"river":s.river_near(x,z,40.0)}));
                    }
                }
            }
        }
        let (samples,digest)=foundation_digest(&s);
        println!("road+2m foundation: {samples} samples, digest={digest:016x}");
        if let Ok(path)=std::env::var("WARP_MACROTERRAIN_METRICS") {
            std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"foundation_samples":samples,"foundation_digest":format!("{digest:016x}"),"sections":sections})).unwrap()).unwrap();
        }
    }

    fn reported_stage() -> Stage {
        let mut rng=Pcg::new(1);rng.next_u32();
        let stage=Stage::generate(&mut rng,3000.0+RUNOUT);
        assert_eq!((stage.noise_seed,stage.detail_seed),(683407368,2033395789));
        stage
    }

    #[test]
    fn remote_river_bank_does_not_slice_the_reported_cliff_crests() {
        let s=reported_stage();
        for (row,edge,previous_cut) in [(1142,11.0,6.80036),(1174,12.0,17.97660)] {
            let p=s.points[row];let d=s.dirs[row];let off=s.widths[row]as f64+edge;
            let x=p.x as f64+d.z as f64*off;let z=p.z as f64-d.x as f64*off;
            let near=s.nearest_index(x,z);let raw=s.height_near_raw(x,z,near);
            let (ri,_,distance,_)=s.river_near(x,z,40.0).unwrap();
            assert!(distance>s.rivers[ri].half+12.0,"fixture moved into the river's bank");
            assert_eq!(s.height(x,z).to_bits(),raw.to_bits(),"remote river still clips row {row}");
            assert!(raw-previous_cut>4.0,"fixture no longer exercises the recorded large cliff loss");
        }
        // This fingerprint includes the intentionally changed r52 off-road
        // verge (old narrow-ditch value: 0xabbf09378d960281). The generation
        // seed and road vertical profile are unchanged; builder's separate
        // bit-identity test preserves the original drivable ribbon/ruts.
        assert_eq!(foundation_digest(&s),(78_550,0x7fd617ff94b4b62b));
    }

    #[test]
    fn river_beds_water_and_native_road_triangles_keep_their_old_foundation() {
        let s=reported_stage();let mut wet=0usize;let mut guarded=0usize;
        let legacy=|x:f64,z:f64,h:f64| {
            let Some((ri,k,d,level))=s.river_near(x,z,40.0) else{return h};let r=&s.rivers[ri];
            if r.wet[k]==0{return h;}
            let bed=level-r.depth*(1.0-smoothstep(r.half*0.45,r.half,d))
                +0.25*smoothstep(r.half*0.8,r.half,d)+(d-r.half).max(0.0)*0.5;
            h.min(bed)
        };
        for r in &s.rivers {for k in 1..r.pts.len()-1 {
            let p=r.pts[k];let direction=(r.pts[k+1]-r.pts[k-1]).normalized();
            for side in [-1.0,0.0,1.0] {for edge in [0.0,r.half,r.half+1.9] {
                let x=p.x as f64-direction.y as f64*side*edge;let z=p.y as f64+direction.x as f64*side*edge;
                let near=s.nearest_index(x,z);let raw=s.height_near_raw(x,z,near);
                let Some((ri,j,d,level))=s.river_near(x,z,40.0) else{continue};let river=&s.rivers[ri];
                if river.wet[j]==0||d>river.half+2.0{continue;}
                assert_eq!(s.height_near(x,z,near).to_bits(),legacy(x,z,raw).to_bits());wet+=1;
                if d<=river.half+0.5 {assert_eq!(s.water_level(x,z).to_bits(),level.to_bits());}
            }}
        }}
        // Every corner of each native grid cell touched by road+2m is
        // unchanged. This is stronger than checking the continuous field.
        for row in 0..s.points.len() {let p=s.points[row];let direction=s.dirs[row];
            for side in [-1.0,1.0] {let off=s.widths[row]as f64+2.0;
                let x=p.x as f64-direction.z as f64*side*off;let z=p.z as f64+direction.x as f64*side*off;
                let gx=(x/GRID).floor();let gz=(z/GRID).floor();
                for dx in [0.0,1.0] {for dz in [0.0,1.0] {
                    let x=(gx+dx)*GRID;let z=(gz+dz)*GRID;let near=s.nearest_index(x,z);let raw=s.height_near_raw(x,z,near);
                    assert_eq!(s.height_near(x,z,near).to_bits(),legacy(x,z,raw).to_bits(),"road guard grid corner ({x},{z})");guarded+=1;
                }}
            }
        }
        println!("unchanged river/road foundations: wet={wet}, native road+2m corners={guarded}");
        assert!(wet>500);assert_eq!(guarded,s.points.len()*8);
    }

    #[test]
    fn compact_river_banks_join_continuously_without_a_forty_metre_ring() {
        let mut s=reported_stage();
        s.rivers=vec![River {pts:vec![v2(0.0,-100.0),v2(0.0,0.0),v2(0.0,100.0)],
            levels:vec![0.0;3],wet:vec![1;3],half:3.0,depth:1.0,cross:0,ford:false}];
        s.index_rivers();
        for radius in [5.0,15.0,40.0] {for offset in [-10.0,-0.4,0.4,10.0] {
            let h=(radius-3.0)*0.5+0.25+offset;
            let sample=|x:f64|s.carve(x,0.0,h,-1);
            assert!((sample(radius-0.0001)-sample(radius+0.0001)).abs()<0.0005,
                "bank discontinuity at {radius}m, height {h}");
            let left=(sample(radius)-sample(radius-0.01))/0.01;
            let right=(sample(radius+0.01)-sample(radius))/0.01;
            assert!((left-right).abs()<0.03,"bank derivative jump at {radius}m: {left} vs {right}");
            if radius>=15.0 {assert_eq!(sample(radius).to_bits(),h.to_bits());}
        }}
        // This is the old hard min intersection, away from protected water.
        let h=2.75;let sample=|x:f64|s.carve(x,0.0,h,-1);
        let left=(sample(8.0)-sample(7.99))/0.01;let right=(sample(8.01)-sample(8.0))/0.01;
        assert!((left-right).abs()<0.03,"unrounded bank/terrain intersection");
    }

    fn grid_height(s:&Stage,x:f64,z:f64)->f64 {
        let gx=(x/GRID).floor();let gz=(z/GRID).floor();
        let (u,v)=(x/GRID-gx,z/GRID-gz);
        let h=|i:f64,j:f64|s.height((gx+i)*GRID,(gz+j)*GRID);
        if u+v<=1.0 {h(0.,0.)*(1.-u-v)+h(1.,0.)*u+h(0.,1.)*v}
        else {h(1.,1.)*(u+v-1.)+h(1.,0.)*(1.-v)+h(0.,1.)*(1.-u)}
    }

    #[test]
    fn reported_cut_bank_keeps_its_height_and_steep_face_with_less_grid_error() {
        let s=reported_stage();let mut errors=Vec::new();let mut steep=0usize;
        for row in 778..=868 {
            let p=s.points[row];let d=s.dirs[row];let side=s.ledges[row].signum()as f64;
            let position=|off:f64|(p.x as f64-d.z as f64*side*off,p.z as f64+d.x as f64*side*off);
            for k in 0..=100 {
                let off=s.widths[row]as f64+1.0+k as f64*0.1;
                let (x,z)=position(off);errors.push((grid_height(&s,x,z)-s.height(x,z)).abs());
            }
            if s.ledges[row].abs()==1.0 {
                let (x,z)=position(s.widths[row]as f64+12.0);
                assert!((s.height_near_raw(x,z,row as i64)-(p.y as f64-0.06+s.ledge_wall[row]as f64)).abs()<0.15,
                    "cliff plateau was lowered at row {row}");
                let peak=(0..90).map(|k| {
                    let off=s.widths[row]as f64+1.0+k as f64*0.1;
                    let a=position(off);let b=position(off+0.1);
                    (s.height_near_raw(b.0,b.1,row as i64)-s.height_near_raw(a.0,a.1,row as i64))/0.1
                }).fold(0.0f64,f64::max);
                assert!(peak>1.5,"cut became a shallow hillside: row {row}, slope {peak}");steep+=1;
            }
        }
        errors.sort_by(f64::total_cmp);
        // Actual pre-fix F12 corridor: p95=0.96884 m, max=1.54492 m.
        assert!(errors[errors.len()*95/100]<0.50);
        assert!(*errors.last().unwrap()<0.80);
        assert!(steep>50);
    }

    #[test]
    fn ledge_envelope_is_continuous_when_the_nearest_road_row_changes() {
        let mut s=reported_stage();let n=20;
        s.points=(0..n).map(|i|v3(0.0,100.0,-(i as f32)*STEP as f32)).collect();
        s.dirs=vec![v3(0.0,0.0,-1.0);n];s.widths=vec![3.4;n];
        s.ledges=(0..n).map(|i|i as f32/(n-1)as f32).collect();
        s.ledge_wall=vec![12.0;n];s.ledge_drop=vec![20.0;n];s.rivers.clear();s.index();
        for row in 2..n-3 {
            let z=-(row as f64+0.5)*STEP;
            let before=s.height(8.0,z+0.001);let after=s.height(8.0,z-0.001);
            assert!((after-before).abs()<0.01,"nearest-row ledge discontinuity: {before} -> {after}");
        }
        s.ledges[7]=1.0;s.ledges[8]=-1.0;
        for delta in [-0.0001,0.0,0.0001] {for side in [-1.0,1.0] {
            let height=s.height(8.0*side,-15.0+delta);
            assert!(height.is_finite()&&(70.0..=113.0).contains(&height),"opposite ledges exploded: {height}");
        }}
    }

    #[test]
    fn drivable_foundation_retains_its_exact_original_height() {
        for seed in [1,7,9001] {
            let s=Stage::generate(&mut Pcg::new(seed),3000.0+RUNOUT);
            for row in 0..s.points.len() {
                let p=s.points[row];let d=s.dirs[row];let right=v3(-d.z,0.,d.x);
                for fraction in [-1.0,-0.5,0.0,0.5,1.0] {
                    let q=p+right*(fraction*s.widths[row]);let r=s.result(q,row);
                    let expected=r.height-0.06*(r.offset.abs()/s.widths[row]as f64).clamp(0.0,1.0)-0.08;
                    assert_eq!(s.height_near_raw(q.x as f64,q.z as f64,row as i64).to_bits(),expected.to_bits());
                }
            }
        }
    }
}

#[cfg(test)]
mod roadside_grid_audit {
    use super::*;

    fn mesh_height(s: &Stage, x: f64, z: f64) -> f64 {
        let (i, j) = ((x / GRID).floor(), (z / GRID).floor());
        let (u, v) = (x / GRID - i, z / GRID - j);
        // Same f32 vertex storage and diagonal as the collision/render mesh.
        let h = |a, b| s.height((i + a) * GRID, (j + b) * GRID) as f32 as f64;
        if u + v <= 1.0 { h(0., 0.) * (1. - u - v) + h(1., 0.) * u + h(0., 1.) * v }
        else { h(1., 1.) * (u + v - 1.) + h(1., 0.) * (1. - v) + h(0., 1.) * (1. - u) }
    }

    fn metrics(samples: &[(f64, f64)]) -> serde_json::Value {
        let mut errors: Vec<_> = samples.iter().map(|(raw, grid)| (raw - grid).abs()).collect();
        errors.sort_by(f64::total_cmp);
        let (lo, hi) = samples.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, h)| (lo.min(*h), hi.max(*h)));
        serde_json::json!({"samples": samples.len(), "grid_range_m": hi - lo,
            "grid_min_m": lo, "grid_max_m": hi, "error_p95_m": errors[errors.len() * 95 / 100],
            "error_max_m": errors.last().unwrap()})
    }

    fn flat_stage(degrees: f64, phase: f32) -> Stage {
        let mut s = Stage::generate(&mut Pcg::new(7), 200.0);
        let d = v3(degrees.to_radians().sin() as f32, 0., -degrees.to_radians().cos() as f32);
        s.points = (0..80).map(|i| d * ((i as f32 - 40.) * STEP as f32) + v3(phase, 0., phase * 0.7)).collect();
        s.dirs = vec![d; 80]; s.widths = vec![3.4; 80]; s.ledges = vec![0.; 80];
        s.ledge_wall = vec![0.; 80]; s.ledge_drop = vec![0.; 80]; s.rivers.clear();
        s.noise = Noise::with_defaults(7, 0.0); s.detail = Noise::with_defaults(9, 0.0); s.index();
        s
    }

    #[test]
    fn resolved_verge_has_no_large_direction_or_grid_phase_waves() {
        let mut worst = 0.0f64;
        for degrees in [0f64, 15., 30., 45., 60., 75., 90.] { for phase in [0., 0.37, 1.11] {
            let s = flat_stage(degrees, phase); let d = s.dirs[40];
            for side in [-1., 1.] { for edge in [0.8, 1.2, 2.2, 3.2, 4.8, 6.8] {
                let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                for k in 0..=256 {
                    let q = s.point_at(64. + k as f64 * 0.125) + v3(-d.z, 0., d.x) * (side * (3.4 + edge)) as f32;
                    let h = mesh_height(&s, q.x as f64, q.z as f64);
                    lo = lo.min(h); hi = hi.max(h);
                }
                worst = worst.max(hi - lo);
                // Native triangle geometry, not just the analytic profile.
                // Old profile reached 344.8 mm on these same flat roads.
                assert!(hi - lo < 0.015, "verge wave {}m at {degrees}deg, phase {phase}, side {side}, edge {edge}", hi - lo);
            }}
        }}
        println!("maximum flat-road verge wave: {worst:.9} m");
    }

    #[test]
    fn shallow_verge_joins_continuously_and_preserves_drivable_foundation() {
        let s = flat_stage(0., 0.37); let p = s.points[40]; let w = s.widths[40] as f64;
        for side in [-1., 1.] {
            let h = |edge: f64| s.height_near_raw(p.x as f64 + side * (w + edge), p.z as f64, 40);
            for edge in [0.8, 0.8 + 2. * GRID] {
                assert!((h(edge - 0.0001) - h(edge + 0.0001)).abs() < 0.00001);
                let left = (h(edge) - h(edge - 0.01)) / 0.01;
                let right = (h(edge + 0.01) - h(edge)) / 0.01;
                assert!((left - right).abs() < 0.001, "verge derivative discontinuity at {edge}: {left}/{right}");
            }
            for k in 0..=128 {
                let x = p.x as f64 + side * (w + 0.79) * k as f64 / 128.;
                let r = s.result(v3(x as f32, 0., p.z), 40);
                let expected = r.height - 0.06 * (r.offset.abs() / w).clamp(0., 1.) - 0.08;
                assert_eq!(s.height_near_raw(x, p.z as f64, 40).to_bits(), expected.to_bits());
            }
        }
    }

    #[test]
    fn reported_roadside_native_triangle_error_is_bounded() {
        let s = reported_round_two_stage();
        let at = s.locate(v3(44.444496, 2.2734036, -1263.9277), -1);
        let (mut worst_verge, mut worst_outer) = (0.0f64, 0.0f64);
        for side in [-1., 1.] { for edge in [0.8, 1.2, 2.2, 3.2] { for k in 0..=320 {
            let along = at.along - 20. + k as f64 * 0.125;
            let p = s.point_at(along);
            let d = (s.point_at(along + 0.25) - s.point_at(along - 0.25)).normalized();
            let off = s.width_at(along) + edge;
            let (x, z) = (p.x as f64 - d.z as f64 * side * off, p.z as f64 + d.x as f64 * side * off);
            let error = (s.height(x, z) - mesh_height(&s, x, z)).abs();
            if edge <= 2.2 { worst_verge = worst_verge.max(error); }
            else { worst_outer = worst_outer.max(error); }
        }}}
        // Previously 475 mm in the ditch zone beside the recorded road.
        // Beyond edge=3m the unchanged natural hillside blend contributes
        // additional curvature; constrain it separately from the verge.
        assert!(worst_verge < 0.03, "reported verge native-grid error {worst_verge}m");
        assert!(worst_outer < 0.10, "reported outer hillside native-grid error {worst_outer}m");
        println!("reported native-grid error: verge {worst_verge:.9} m, outer hillside {worst_outer:.9} m");
    }

    #[test]
    #[ignore = "bounded CPU geometry diagnosis; optional SIDEWAYS_ROADSIDE_AUDIT JSON output"]
    fn reported_roadside_grid_audit() {
        let actual = reported_round_two_stage();
        let at = actual.locate(v3(44.444496, 2.2734036, -1263.9277), -1);
        let mut actual_rows = Vec::new();
        for side in [-1., 1.] { for edge in [0.8, 1.2, 2.2, 3.2] {
            let mut samples = Vec::new();
            let mut points = Vec::new();
            for k in 0..=320 {
                let along = at.along - 20. + k as f64 * 0.125;
                let p = actual.point_at(along);
                let d = (actual.point_at(along + 0.25) - actual.point_at(along - 0.25)).normalized();
                let off = actual.width_at(along) + edge;
                let (x, z) = (p.x as f64 - d.z as f64 * side * off, p.z as f64 + d.x as f64 * side * off);
                let raw = actual.height(x, z); let grid = mesh_height(&actual, x, z);
                samples.push((raw, grid));
                points.push([x, z, raw, grid]);
            }
            actual_rows.push(serde_json::json!({"side": side, "edge_m": edge,
                "metrics": metrics(&samples), "points": points}));
        }}
        let mut synthetic_rows = Vec::new();
        for degrees in [0f64, 15., 30., 45., 60., 75., 90.] { for phase in [0., 0.37, 1.11] {
            let s = flat_stage(degrees, phase); let d = s.dirs[40];
            for edge in [0.8, 1.2, 2.2, 3.2] {
                let samples: Vec<_> = (0..=256).map(|k| {
                    let p = s.point_at(64. + k as f64 * 0.125);
                    let q = p + v3(-d.z, 0., d.x) * (3.4 + edge) as f32;
                    (s.height(q.x as f64, q.z as f64), mesh_height(&s, q.x as f64, q.z as f64))
                }).collect();
                synthetic_rows.push(serde_json::json!({"degrees": degrees, "phase_m": phase,
                    "edge_m": edge, "metrics": metrics(&samples)}));
            }
        }}
        let report = serde_json::json!({"capture": "sideways-1793039836887-117876-1", "noise_seed": actual.noise_seed,
            "detail_seed": actual.detail_seed, "road_row": at.index, "along_m": at.along,
            "native_cell_m": GRID, "actual": actual_rows, "synthetic": synthetic_rows});
        if let Ok(path) = std::env::var("SIDEWAYS_ROADSIDE_AUDIT") {
            std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
        println!("roadside grid: row {} along {:.3}; {} actual / {} synthetic profiles", at.index, at.along, actual_rows.len(), synthetic_rows.len());
    }
}

#[cfg(test)]
mod foundation_tests {
    use super::*;

    #[test]
    fn rejected_dry_river_does_not_excavate_reported_hairpin_foundation() {
        let stage=reported_round_two_stage();
        let (x,z)=(-57.27709198,-662.73962402);
        let near=stage.nearest_index(x,z);
        let (ri,k,_,_)=stage.river_near(x,z,40.0).expect("recorded dry river beside road");
        assert_eq!((ri,k,stage.rivers[ri].wet[k]),(1,138,0));
        let raw=stage.height_near_raw(x,z,near);
        assert!((raw-11.04958583).abs()<1e-5);
        assert_eq!(stage.height(x,z),raw,"the old carve lowered this point to 4.897 m");
        for row in 370..=403 {
            let p=stage.points[row];let d=stage.dirs[row];let right=v3(-d.z,0.0,d.x);
            for side in [-1.0,0.0,1.0] {
                let q=p+right*(side*(stage.widths[row]+0.8));
                let raw=stage.height_near_raw(q.x as f64,q.z as f64,row as i64);
                let actual=stage.height_near(q.x as f64,q.z as f64,row as i64);
                assert!((actual-raw).abs()<1e-6,"dry river still undercuts row {row}, side {side}: {actual} vs {raw}");
            }
        }
    }

    #[test]
    fn valid_ford_and_bridge_beds_still_carve_below_water() {
        let mut stage=reported_round_two_stage();
        let row=stage.rivers[1].cross;let p=stage.points[row];
        let (ri,k,d,level)=stage.river_near(p.x as f64,p.z as f64,40.0).unwrap();
        assert_eq!((ri,k,stage.rivers[ri].wet[k]),(1,75,1));
        assert!(d<1e-4);
        let raw=stage.height_near_raw(p.x as f64,p.z as f64,row as i64);
        let bed=level-stage.rivers[ri].depth;
        assert!(raw-bed>0.5,"fixture must test a real river cut");
        for ford in [true,false] {
            stage.rivers[ri].ford=ford;
            let actual=stage.height_near(p.x as f64,p.z as f64,row as i64);
            assert!((actual-bed).abs()<1e-5,"valid crossing lost its river bed (ford={ford})");
            assert!((stage.water_level(p.x as f64,p.z as f64)-level).abs()<1e-5);
        }
    }
}
