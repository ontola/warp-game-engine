//! Terrain chunks, the road ribbon, barriers and where things grow
//! (src/stage_builder.gd), as plain data: heights, collision triangles and
//! placements. The renderer and the physics both read from this.

use crate::stage::{self, GRID, Stage, STEP};
use gdcompat::math::{V2, V3, signf, v2, v3};
use gdcompat::{Noise, Pcg, Var, hash};
use std::f64::consts::TAU;

/// A placed thing: position, yaw, uniform and vertical scale.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Spot {
    pub pos: V3Ser,
    pub yaw: f32,
    pub scale: f32,
    pub scale_y: f32,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct V3Ser(pub f32, pub f32, pub f32);

impl From<V3> for V3Ser {
    fn from(v: V3) -> Self {
        V3Ser(v.x, v.y, v.z)
    }
}

impl Spot {
    pub fn origin(&self) -> V3 {
        v3(self.pos.0, self.pos.1, self.pos.2)
    }
}

pub struct Chunk {
    pub cell: (i32, i32),
    pub x0: f64,
    pub z0: f64,
    /// n x n heights on the 2 m grid (n = 33).
    pub heights: Vec<f32>,
    /// metres from the road edge per vertex.
    pub road_d: Vec<f32>,
    pub trees: Vec<Spot>,
    pub rocks: Vec<Spot>,
    pub grass: Vec<Spot>,
    pub ferns: Vec<Spot>,
    pub stumps: Vec<Spot>,
}

pub const N: usize = (stage::CHUNK / GRID) as usize + 1;

impl Chunk {
    pub fn vertex(&self, i: usize, j: usize) -> V3 {
        v3((self.x0 + i as f64 * GRID) as f32, self.heights[j * N + i], (self.z0 + j as f64 * GRID) as f32)
    }

    /// Collision triangles, two per grid cell, wound as Godot's.
    pub fn triangles(&self) -> (Vec<V3>, Vec<[u32; 3]>) {
        let mut verts = Vec::with_capacity(N * N);
        for j in 0..N {
            for i in 0..N {
                verts.push(self.vertex(i, j));
            }
        }
        let mut idx = Vec::with_capacity((N - 1) * (N - 1) * 2);
        for j in 0..N - 1 {
            for i in 0..N - 1 {
                let a = (j * N + i) as u32;
                let n = N as u32;
                idx.push([a, a + 1, a + n]);
                idx.push([a + 1, a + n + 1, a + n]);
            }
        }
        (verts, idx)
    }
}

/// The chunks of terrain within CORRIDOR of the road, sorted.
pub fn chunk_cells(s: &Stage) -> Vec<(i32, i32)> {
    let mut seen = std::collections::HashSet::new();
    let reach = (stage::CORRIDOR / stage::CHUNK).ceil() as i32;
    for i in (0..s.points.len()).step_by(8) {
        let p = s.points[i];
        let c = ((p.x as f64 / stage::CHUNK).floor() as i32, (p.z as f64 / stage::CHUNK).floor() as i32);
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                let k = (c.0 + dx, c.1 + dy);
                if seen.contains(&k) {
                    continue;
                }
                let centre = (v2(k.0 as f32, k.1 as f32) + v2(0.5, 0.5)) * stage::CHUNK as f32;
                if (centre.distance_to(v2(p.x, p.z)) as f64) < stage::CORRIDOR {
                    seen.insert(k);
                }
            }
        }
    }
    let mut out: Vec<_> = seen.into_iter().collect();
    out.sort();
    out
}

/// One terrain chunk: heights, road distance and what grows on it, with
/// its own random numbers so chunks can be built in parallel.
pub fn terrain_chunk(s: &Stage, c: (i32, i32)) -> Chunk {
    let mut rng = Pcg::new(hash(&Var::Array(&[Var::Int(s.noise_seed as i64), Var::Int(c.0 as i64), Var::Int(c.1 as i64)])) as u64);
    let n = N;
    let x0 = c.0 as f64 * stage::CHUNK;
    let z0 = c.1 as f64 * stage::CHUNK;
    let m = n + 2;
    let mut hs = vec![0.0f32; m * m];
    let mut road_d = vec![0.0f32; m * m];
    for j in 0..m {
        for i in 0..m {
            let x = x0 + (i as f64 - 1.0) * GRID;
            let z = z0 + (j as f64 - 1.0) * GRID;
            // A winding run is not distance-monotonic: hill climbing from
            // yesterday's guess can settle on the wrong road section. Use
            // the same exact spatial lookup as Stage::height so terrain,
            // road shoulders, vegetation roots and colliders agree.
            let near = s.nearest_index(x, z);
            let near_d = if near >= 0 {
                let p = s.points[near as usize];
                v2((p.x as f64 - x) as f32, (p.z as f64 - z) as f32).length_squared() as f64
            } else { f64::INFINITY };
            hs[j * m + i] = s.height_near(x, z, near) as f32;
            // This also becomes a terrain vertex attribute and is linearly
            // interpolated by grass sampling. Infinity produces NaNs at a
            // zero barycentric weight; 100 m is beyond every verge threshold.
            road_d[j * m + i] = if near >= 0 {
                (near_d.sqrt() - s.widths[near as usize] as f64) as f32
            } else { 100.0 };
        }
    }
    let mut trees = Vec::new();
    let mut rocks = Vec::new();
    let mut grass = Vec::new();
    let mut ferns = Vec::new();
    let mut stumps = Vec::new();
    let gridf = GRID as f32;
    // What grows on this land, relative to Sideways' own forest; the RNG
    // draws stay the same, only the odds change.
    let (tree_odds, grass_odds, fern_odds) = s.land.as_ref().map_or((1.0, 1.0, 1.0), |l| (l.trees as f64, l.grass as f64, l.ferns as f64));
    let tree_scale = s.land.as_ref().map_or(1.0, |l| l.tree_scale);
    for j in 0..n {
        for i in 0..n {
            let k = (j + 1) * m + (i + 1);
            let x = x0 + i as f64 * GRID;
            let z = z0 + j as f64 * GRID;
            let dx = hs[k + 1] as f64 - hs[k - 1] as f64;
            let dz = hs[k + m] as f64 - hs[k - m] as f64;
            let normal = v3(-dx as f32, (2.0 * GRID) as f32, -dz as f32).normalized();
            let steep = (normal.y as f64) < 0.75 || (!s.rivers.is_empty() && s.water_level(x, z) > hs[k] as f64 - 0.4);
            let edge = road_d[k] as f64;
            if i < n - 1 && j < n - 1 && edge > 0.7 && edge < 26.0 && !steep {
                let tufts = if edge < 9.0 { 7 } else if rng.randf() < 0.5 { 2 } else { 0 };
                let tufts = if grass_odds >= 1.0 { tufts } else { (tufts as f64 * grass_odds).round() as i32 };
                let cx = x + (rng.randf() * gridf) as f64;
                let cz = z + (rng.randf() * gridf) as f64;
                for _ in 0..tufts {
                    let gx = (cx + rng.randf_range_f32(-0.6, 0.6) as f64).clamp(x, x + GRID);
                    let gz = (cz + rng.randf_range_f32(-0.6, 0.6) as f64).clamp(z, z + GRID);
                    let gs = rng.randf_range_f32(0.6, 1.2) as f64 * if edge < 9.0 { 1.0 } else { 0.75 };
                    let fx = (gx - x) / GRID;
                    let fz = (gz - z) / GRID;
                    let l = |a: f32, b: f32, t: f64| a as f64 + (b as f64 - a as f64) * t;
                    let gy = { let a = l(hs[k], hs[k + 1], fx); let b = l(hs[k + m], hs[k + m + 1], fx); a + (b - a) * fz };
                    let yaw = (rng.randf() as f64 * TAU) as f32;
                    let sy = gs as f32 * rng.randf_range_f32(0.8, 1.3);
                    grass.push(Spot { pos: v3(gx as f32, (gy - 0.03) as f32, gz as f32).into(), yaw, scale: gs as f32, scale_y: sy });
                }
            }
            if i < n - 1 && j < n - 1 && (i + j * 3) % 2 == 0 {
                let tx = x + rng.randf_range_f32(0.0, gridf * 2.0) as f64;
                let tz = z + rng.randf_range_f32(0.0, gridf * 2.0) as f64;
                let clear = road_d[k] as f64 - rng.randf_range_f32(4.5, 9.0) as f64;
                let thin = s.detail.get_2d((tx * 0.6) as f32, (tz * 0.6) as f32) as f64;
                if clear > 0.0 && !steep && thin > -0.25 && (rng.randf() as f64) < (0.28 + 0.25 * thin) * tree_odds {
                    let ty = s.height(tx, tz);
                    let mut foot = ty;
                    for (ox, oz) in [(0.7f32, 0.0f32), (-0.7, 0.0), (0.0, 0.7), (0.0, -0.7)] {
                        foot = foot.min(s.height(tx + ox as f64, tz + oz as f64));
                    }
                    let sc = rng.randf_range_f32(0.75, 1.25);
                    let yaw = (rng.randf() as f64 * TAU) as f32;
                    let sy = sc * rng.randf_range_f32(0.9, 1.15);
                    let phenotype = s.vegetation.tree(s.noise_seed as u64, tx as f32, tz as f32);
                    trees.push(Spot { pos: v3(tx as f32, (foot - 0.25) as f32, tz as f32).into(), yaw,
                        scale: sc * phenotype.scale[0] * tree_scale, scale_y: sy * phenotype.scale[1] * tree_scale });
                } else if clear > 4.0 && !steep && rng.randf() < 0.004 {
                    let sy = s.height(tx, tz);
                    let ss = rng.randf_range_f32(0.8, 1.3);
                    let yaw = (rng.randf() as f64 * TAU) as f32;
                    stumps.push(Spot { pos: v3(tx as f32, (sy - 0.05) as f32, tz as f32).into(), yaw, scale: ss, scale_y: ss });
                } else if clear > 3.0 && !steep && (rng.randf() as f64) < (0.05 + 0.08 * thin.max(0.0)) * fern_odds {
                    let fy = s.height(tx, tz);
                    let fs = rng.randf_range_f32(0.7, 1.3);
                    let yaw = (rng.randf() as f64 * TAU) as f32;
                    ferns.push(Spot { pos: v3(tx as f32, (fy - 0.05) as f32, tz as f32).into(), yaw, scale: fs, scale_y: fs });
                } else if clear > -3.0 && clear < 6.0 && rng.randf() < 0.012 {
                    let ry = s.height(tx, tz);
                    let rs = rng.randf_range_f32(0.4, 1.3);
                    // Axis (three draws) then angle: the rock's tumble is
                    // only for looks; keep the draws so the stream matches.
                    let (_ax, _ay, _az) = (rng.randf(), rng.randf(), rng.randf());
                    let yaw = (rng.randf() as f64 * TAU) as f32;
                    rocks.push(Spot { pos: v3(tx as f32, (ry - rs as f64 * 0.2) as f32, tz as f32).into(), yaw, scale: rs, scale_y: rs * 0.7 });
                }
            }
        }
    }
    let mut heights = vec![0.0f32; n * n];
    let mut rd = vec![0.0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            heights[j * n + i] = hs[(j + 1) * m + i + 1];
            rd[j * n + i] = road_d[(j + 1) * m + i + 1];
        }
    }
    Chunk { cell: c, x0, z0, heights, road_d: rd, trees, rocks, grass, ferns, stumps }
}

pub const ACROSS: [f64; 23] = [
    -1.25, -1.0, -0.86, -0.72, -0.58, -0.46, -0.36, -0.28, -0.21, -0.14, -0.07, 0.0, 0.07, 0.14, 0.21, 0.28, 0.36, 0.46, 0.58, 0.72, 0.86, 1.0, 1.25,
];

/// The road ribbon: wheel tracks, the ridge between them and the windrows
/// are real shape, and this is the road collider too.
pub struct Road {
    pub verts: Vec<V3>,
    pub tris: Vec<[u32; 3]>,
    /// Road-space coordinates, including additional shoulder seam vertices.
    pub uvs: Vec<[f32; 2]>,
    /// The original rectangular ribbon occupies the first rows * cols vertices.
    pub rows: usize,
    pub cols: usize,
}

pub fn road(s: &Stage) -> Road {
    let cols = ACROSS.len();
    let n = s.points.len();
    let wob = Noise::with_defaults(11, 1.0 / 18.0);
    let mut verts = Vec::with_capacity(n * cols);
    let mut uvs = Vec::with_capacity(n * cols);
    let mut heights = std::collections::HashMap::new();
    let mut ground = |x, z| terrain_triangle_height(x, z, &mut |i, j| {
        *heights.entry((i, j)).or_insert_with(|| s.height(i as f64 * GRID, j as f64 * GRID) as f32)
    });
    for i in 0..n {
        let p = s.points[i];
        let d = s.dirs[i];
        let right = v3(-d.z, 0.0, d.x);
        let w = s.widths[i] as f64;
        let wander = wob.get_1d((i as f64 * STEP) as f32) as f64 * 0.08;
        let wear = 0.6 + 0.4 * wob.get_1d((i as f64 * STEP * 0.3 + 100.0) as f32) as f64;
        for &a in ACROSS.iter() {
            let mut off = a * w;
            if a.abs() > 1.1 {
                off = signf(a) * (w + 0.8);
            }
            let q = p + right * off as f32;
            let mut y = p.y as f64 - 0.06 * a.abs().min(1.0);
            let rut = (-(((a + wander).abs() - 0.25) / 0.085).powi(2)).exp() * wear;
            let ridge = (-((a + wander) / 0.07).powi(2)).exp() * wear;
            let windrow = (-((a.abs() - 0.8) / 0.08).powi(2)).exp();
            y += -0.045 * rut + 0.025 * ridge + 0.04 * windrow;
            if a.abs() > 1.1 {
                // Join the actual 2 m terrain triangles, not the analytic
                // height function between their samples. A tiny overlap
                // avoids a coplanar seam; the drivable road stays unchanged.
                y = ground(q.x, q.z) as f64 - SHOULDER_OVERLAP as f64;
            }
            verts.push(v3(q.x, y as f32, q.z));
            uvs.push([a as f32, (i as f64 * STEP) as f32]);
        }
    }
    let mut tris = Vec::with_capacity((n - 1) * (cols - 1) * 2);
    for i in 0..n - 1 {
        for c in 1..cols - 2 {
            let a = (i * cols + c) as u32;
            let cl = cols as u32;
            tris.push([a, a + cl, a + 1]);
            tris.push([a + 1, a + cl, a + cl + 1]);
        }
        // Each outer 80 cm shoulder must follow every terrain-triangle
        // boundary it crosses, including the diagonal inside each grid cell.
        // Snapping only the two row endpoints still leaves visible gaps.
        for (outer, inner) in [(0, 1), (cols - 1, cols - 2)] {
            let outer_a = verts[i * cols + outer];
            let outer_b = verts[(i + 1) * cols + outer];
            let inner_a = verts[i * cols + inner];
            let inner_b = verts[(i + 1) * cols + inner];
            let cuts = terrain_edge_cuts(outer_a, outer_b);
            let mut previous = (i * cols + outer, i * cols + inner);
            for &t in cuts.iter().skip(1) {
                let next = if t == 1.0 {
                    ((i + 1) * cols + outer, (i + 1) * cols + inner)
                } else {
                    let q = outer_a.lerp(outer_b, t as f32);
                    let k = verts.len();
                    verts.push(v3(q.x, ground(q.x, q.z) - SHOULDER_OVERLAP, q.z));
                    verts.push(inner_a.lerp(inner_b, t as f32));
                    let along = ((i as f64 + t) * STEP) as f32;
                    uvs.push([ACROSS[outer] as f32, along]);
                    uvs.push([ACROSS[inner] as f32, along]);
                    (k, k + 1)
                };
                let (a, b, c, d) = if outer == 0 {
                    (previous.0, next.0, previous.1, next.1)
                } else {
                    (previous.1, next.1, previous.0, next.0)
                };
                tris.push([a as u32, b as u32, c as u32]);
                tris.push([c as u32, b as u32, d as u32]);
                previous = next;
            }
        }
    }
    Road { verts, tris, uvs, rows: n, cols }
}

const SHOULDER_OVERLAP: f32 = 0.003;

/// Interpolate the same diagonal and f32 samples as Chunk::triangles().
fn terrain_triangle_height(x: f32, z: f32, sample: &mut impl FnMut(i32, i32) -> f32) -> f32 {
    let gx = x as f64 / GRID;
    let gz = z as f64 / GRID;
    let (i, j) = (gx.floor() as i32, gz.floor() as i32);
    let (x, z) = (gx - i as f64, gz - j as f64);
    if x + z <= 1.0 {
        (sample(i, j) as f64 * (1.0 - x - z)
            + sample(i + 1, j) as f64 * x + sample(i, j + 1) as f64 * z) as f32
    } else {
        (sample(i + 1, j + 1) as f64 * (x + z - 1.0)
            + sample(i, j + 1) as f64 * (1.0 - x)
            + sample(i + 1, j) as f64 * (1.0 - z)) as f32
    }
}

/// Grid edges are x = k*GRID, z = k*GRID and x+z = k*GRID.
fn terrain_edge_cuts(a: V3, b: V3) -> Vec<f64> {
    let mut cuts = vec![0.0, 1.0];
    for (start, end) in [(a.x as f64, b.x as f64), (a.z as f64, b.z as f64),
        (a.x as f64 + a.z as f64, b.x as f64 + b.z as f64)] {
        if (end - start).abs() < 1e-12 { continue; }
        let lo = (start.min(end) / GRID).floor() as i32 + 1;
        let hi = (start.max(end) / GRID).floor() as i32;
        for k in lo..=hi {
            let t = (k as f64 * GRID - start) / (end - start);
            if t > 0.0 && t < 1.0 { cuts.push(t); }
        }
    }
    cuts.sort_by(f64::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    cuts
}

/// A box collider: centre, half extents and a horizontal facing.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub centre: V3,
    pub half: V3,
    /// Unit direction the box's length (local Z) runs along.
    pub along: V3,
}

/// Walls and Armco along the drop of every ledge, and bridge rails.
pub fn barriers(s: &Stage) -> Vec<Block> {
    let mut out = Vec::new();
    let n = s.points.len();
    let mut i = 0;
    while i < n {
        if (s.ledges[i] as f64).abs() < 0.35 {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n - 1 && (s.ledges[j + 1] as f64).abs() >= 0.35 {
            j += 1;
        }
        let side = -signf(s.ledges[i] as f64) as f32;
        let mut line: Vec<V3> = Vec::new();
        for k in i..=j {
            let right = v3(-s.dirs[k].z, 0.0, s.dirs[k].x);
            line.push(s.points[k] + right * (side * (s.widths[k] as f64 + 1.3) as f32));
        }
        for k in 0..line.len().saturating_sub(1) {
            let (a, b) = (line[k], line[k + 1]);
            let len = (b - a).length();
            out.push(Block { centre: (a + b) * 0.5 + V3::UP * 0.35, half: v3(0.2, 0.5, (len + 0.3) * 0.5), along: (b - a).normalized() });
        }
        i = j + 1;
    }
    for r in &s.rivers {
        if r.ford {
            continue;
        }
        let c = r.cross as i64;
        let span = ((r.half + 6.0) / STEP).ceil() as i64;
        for k in c - span..=c + span {
            let k = k as usize;
            let (p, f) = (s.points[k], s.dirs[k]);
            let right = v3(-f.z, 0.0, f.x);
            let w = s.widths[k] as f64 + 0.5;
            for sgn in [-1.0f32, 1.0] {
                let edge = p + right * (sgn * (w + 0.1) as f32);
                out.push(Block { centre: edge + V3::UP * 0.55, half: v3(0.125, 0.55, ((STEP + 0.2) * 0.5) as f32), along: f });
            }
        }
    }
    out
}

/// The visible barriers: per ledge, the line along the drop and whether it
/// is an Armco rail (else a dry-stone wall), as stage_builder.gd draws them.
pub struct BarrierLine {
    pub points: Vec<V3>,
    pub rail: bool,
}

pub fn barrier_lines(s: &Stage) -> Vec<BarrierLine> {
    let mut out = Vec::new();
    let n = s.points.len();
    let mut i = 0;
    while i < n {
        if (s.ledges[i] as f64).abs() < 0.35 {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n - 1 && (s.ledges[j + 1] as f64).abs() >= 0.35 {
            j += 1;
        }
        let side = -signf(s.ledges[i] as f64) as f32;
        let points = (i..=j)
            .map(|k| {
                let right = v3(-s.dirs[k].z, 0.0, s.dirs[k].x);
                s.points[k] + right * (side * (s.widths[k] as f64 + 1.3) as f32)
            })
            .collect();
        out.push(BarrierLine { points, rail: hash(&Var::Int(i as i64)) % 2 == 0 });
        i = j + 1;
    }
    out
}

/// Ground map for anything that needs heights without the physics: the
/// heights of all chunks on one grid.
pub struct GroundMap {
    pub origin: V2,
    pub width: usize,
    pub height: usize,
    pub heights: Vec<f32>,
}

#[cfg(test)]
mod road_seam_tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    fn reported_round_three() -> Stage {
        // F12 sideways-1792818090962-266592-4: seed 1, round 3, 3 km.
        let mut rng = Pcg::new(1);
        rng.skip(1806);
        let s = Stage::generate(&mut rng, 3000.0 + stage::RUNOUT);
        assert_eq!((s.noise_seed, s.detail_seed), (-1661450362, -518345863));
        s
    }

    /// Evaluate an actual terrain mesh triangle plane, independently of the
    /// shoulder's interpolation helper (including negative chunk positions).
    fn mesh_height(s: &Stage, chunks: &mut HashMap<(i32, i32), Chunk>, p: V3) -> f32 {
        let cell = ((p.x as f64 / stage::CHUNK).floor() as i32,
            (p.z as f64 / stage::CHUNK).floor() as i32);
        let c = chunks.entry(cell).or_insert_with(|| terrain_chunk(s, cell));
        let x = (p.x as f64 - c.x0) / GRID;
        let z = (p.z as f64 - c.z0) / GRID;
        let (i, j) = (x.floor() as usize, z.floor() as usize);
        let native_corners = if x.fract() + z.fract() <= 1.0 {
            [j * N + i, j * N + i + 1, (j + 1) * N + i]
        } else {
            [(j + 1) * N + i + 1, (j + 1) * N + i, j * N + i + 1]
        };
        for index in native_corners {
            assert!(c.road_d[index] <= 5.0,
                "terrain CLOD's 5 m vertex-lock band must preserve every seam triangle: {} at {p:?}", c.road_d[index]);
        }
        let (a, b, d) = if x.fract() + z.fract() <= 1.0 {
            (c.vertex(i, j), c.vertex(i + 1, j), c.vertex(i, j + 1))
        } else {
            (c.vertex(i + 1, j + 1), c.vertex(i, j + 1), c.vertex(i + 1, j))
        };
        let normal = (b - a).cross(d - a);
        a.y - (normal.x * (p.x - a.x) + normal.z * (p.z - a.z)) / normal.y
    }

    fn outer_edges(r: &Road) -> Vec<(usize, usize)> {
        let mut found = HashSet::new();
        for tri in &r.tris { for e in 0..3 {
            let (a, b) = (tri[e] as usize, tri[(e + 1) % 3] as usize);
            if r.uvs[a][0].abs() > 1.1 && r.uvs[a][0] == r.uvs[b][0] {
                assert!(found.insert((a.min(b), a.max(b))), "outer seam must be a boundary, not overlapping faces");
            }
        }}
        found.into_iter().collect()
    }

    #[test]
    fn shoulder_boundary_matches_actual_terrain_triangles_across_stages() {
        let mut stages = vec![reported_round_three(), stage::reported_round_two_stage()];
        for seed in [0, 7, 42] {
            stages.push(Stage::generate(&mut Pcg::new(seed), 800.0));
        }
        let mut checked = 0;
        for s in stages {
            let r = road(&s);
            let mut chunks = HashMap::new();
            for (a, b) in outer_edges(&r) {
                for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
                    let p = r.verts[a].lerp(r.verts[b], t);
                    let gap = p.y - mesh_height(&s, &mut chunks, p);
                    assert!((-0.0036..=-0.0024).contains(&gap),
                        "shoulder gap {gap} at {p:?}, stage {}, edge {a}..{b}", s.noise_seed);
                    checked += 1;
                }
            }
        }
        assert!(checked > 100_000, "check full stages, not only the reported point");
    }

    #[test]
    fn reported_stage_main_road_geometry_remains_bit_identical() {
        let r = road(&reported_round_three());
        let mut hash = 0xcbf29ce484222325u64;
        for i in 0..r.rows { for c in 1..r.cols - 1 {
            let p = r.verts[i * r.cols + c];
            for v in [p.x, p.y, p.z] { for byte in v.to_bits().to_le_bytes() {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }}
        }}
        // Captured from unmodified HEAD c8dddd8 builder, with a separate
        // comparison of all 62,800 original interior collision triangles.
        assert_eq!(hash, 0xbcf201770a17e336);
        let interior: Vec<_> = r.tris.iter().filter(|tri| tri.iter().all(|v| {
            let k = *v as usize;
            k < r.rows * r.cols && k % r.cols > 0 && k % r.cols < r.cols - 1
        })).collect();
        let expected: Vec<_> = (0..r.rows - 1).flat_map(|i| (1..r.cols - 2).flat_map(move |c| {
            let a = (i * r.cols + c) as u32; let cols = r.cols as u32;
            [[a, a + cols, a + 1], [a + 1, a + cols, a + cols + 1]]
        })).collect();
        assert_eq!(interior.len(), 62_800);
        assert_eq!(interior.into_iter().copied().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn reported_shoulder_has_no_hidden_collision_step() {
        use crate::physics::{Physics, ROAD};
        use rapier3d::{glamx::Vec3, prelude::{QueryFilter, Ray}};
        let s = reported_round_three();
        let r = road(&s);
        let mut chunks = HashMap::new();
        let edges: Vec<_> = outer_edges(&r).into_iter().filter(|(a, b)| {
            let along = (r.uvs[*a][1] + r.uvs[*b][1]) * 0.5;
            (1670.0..1684.0).contains(&along)
        }).collect();
        let mut probes = Vec::new();
        for (a, b) in edges {
            for t in [0.2, 0.5, 0.8] {
                let p = r.verts[a].lerp(r.verts[b], t);
                let h = mesh_height(&s, &mut chunks, p);
                probes.push((p, h));
            }
        }
        let mut physics = Physics::new();
        for c in chunks.values() { physics.add_chunk(c); }
        physics.add_road(&r);
        physics.step();
        for (p, h) in &probes {
            let ray = Ray::new(Vec3::new(p.x, h + 2.0, p.z), Vec3::NEG_Y);
            let (handle, toi) = physics.world.cast_ray(&ray, 4.0, true, QueryFilter::default()).expect("terrain seam collision");
            assert_ne!(physics.world.colliders[handle].user_data, ROAD,
                "road shoulder must end under terrain, not leave an invisible step at {p:?}");
            assert!((toi - 2.0).abs() < 0.001, "collider/visual ground mismatch at {p:?}");
        }
        assert!(probes.len() >= 100);
    }
}

#[cfg(test)]
mod foundation_tests {
    use super::*;

    #[test]
    fn vegetation_growth_preserves_roots_population_and_rng_consumption() {
        use crate::vegetation::{Season,Vegetation};
        let mut s=Stage::generate(&mut Pcg::new(7),340.0);
        let cells=chunk_cells(&s);
        let grown=Vegetation::new(91,1.0,Season::Summer).unwrap();
        let young=Vegetation::new(91,0.25,Season::Winter).unwrap();
        let mut checked=0;
        for key in cells.into_iter().take(12) {
            s.vegetation=grown; let mature=terrain_chunk(&s,key);
            s.vegetation=young; let juvenile=terrain_chunk(&s,key);
            s.vegetation=Vegetation::new(92,0.25,Season::Winter).unwrap(); let reseeded=terrain_chunk(&s,key);
            assert_eq!(mature.heights,juvenile.heights);assert_eq!(mature.road_d,juvenile.road_d);
            assert_eq!(mature.trees.len(),juvenile.trees.len());
            assert_eq!(mature.trees.len(),reseeded.trees.len());
            for (a,b) in mature.trees.iter().zip(&reseeded.trees) {
                assert_eq!([a.pos.0,a.pos.1,a.pos.2,a.yaw],[b.pos.0,b.pos.1,b.pos.2,b.yaw]);
            }
            for (a,b) in mature.trees.iter().zip(&juvenile.trees) {
                assert_eq!([a.pos.0,a.pos.1,a.pos.2,a.yaw],[b.pos.0,b.pos.1,b.pos.2,b.yaw]);
                let old=grown.tree(s.noise_seed as u64,a.pos.0,a.pos.2);
                let new=young.tree(s.noise_seed as u64,a.pos.0,a.pos.2);
                assert_eq!((old.archetype,old.juvenile),(new.archetype,new.juvenile));
                assert!((b.scale-a.scale*new.scale[0]/old.scale[0]).abs()<1e-6);
                assert!((b.scale_y-a.scale_y*new.scale[1]/old.scale[1]).abs()<1e-6);
                checked+=1;
            }
            // Every later RNG-dependent decoration must remain bit-identical.
            for (a,b) in [(&mature.rocks,&juvenile.rocks),(&mature.grass,&juvenile.grass),(&mature.ferns,&juvenile.ferns),(&mature.stumps,&juvenile.stumps)] {
                assert_eq!(serde_json::to_vec(a).unwrap(),serde_json::to_vec(b).unwrap());
            }
            for (a,b) in [(&mature.rocks,&reseeded.rocks),(&mature.grass,&reseeded.grass),(&mature.ferns,&reseeded.ferns),(&mature.stumps,&reseeded.stumps)] {
                assert_eq!(serde_json::to_vec(a).unwrap(),serde_json::to_vec(b).unwrap());
            }
        }
        assert!(checked>0,"the fixture must actually contain trees");
    }

    #[test]
    fn tree_colliders_follow_the_same_family_and_juvenile_instance_scales() {
        let vegetation=crate::vegetation::Vegetation::default();
        let mut seen=[[false;2];3];
        for i in 0..128 {
            let phenotype=vegetation.tree(7,i as f32,11.5);
            let t=Spot { pos:V3Ser(i as f32,2.0,11.5),yaw:0.7,
                scale:phenotype.scale[0],scale_y:phenotype.scale[1] };
            let mut physics=crate::physics::Physics::new();
            physics.add_trunk(&t);
            let (_,collider)=physics.world.colliders.iter().next().unwrap();
            let cylinder=collider.shape().as_cylinder().unwrap();
            assert!((cylinder.radius-0.22*t.scale).abs()<1e-6);
            assert!((cylinder.half_height-3.0*t.scale_y).abs()<1e-6);
            assert!((collider.translation().y-(t.pos.1+2.5*t.scale_y)).abs()<1e-6);
            seen[phenotype.archetype.index()][usize::from(phenotype.juvenile)]=true;
        }
        assert!(seen.into_iter().flatten().all(|present|present),"fixture must cover all three families and ages");
    }

    #[test]
    fn road_distance_remains_finite_outside_carved_corridor() {
        let s=stage::reported_round_two_stage();
        let mut absent=0;
        for key in [(-4,-11),(-1,-11)] {
            let chunk=terrain_chunk(&s,key);
            assert!(chunk.road_d.iter().all(|d|d.is_finite()));
            for j in 0..N {for i in 0..N {
                let x=chunk.x0+i as f64*GRID;let z=chunk.z0+j as f64*GRID;
                let near=s.nearest_index(x,z);let actual=chunk.road_d[j*N+i];
                if near<0 {
                    absent+=1;assert_eq!(actual,100.0);
                    assert!((actual*0.0+actual*1.0).is_finite(),"grid-boundary interpolation must stay finite");
                } else {
                    let p=s.points[near as usize];
                    let distance=(v2((p.x as f64-x) as f32,(p.z as f64-z) as f32).length_squared() as f64).sqrt();
                    let expected=(distance-s.widths[near as usize] as f64) as f32;
                    assert_eq!(actual.to_bits(),expected.to_bits(),"finite road distances must remain exact");
                }
            }}
        }
        assert!(absent>0,"fixture must include terrain outside the road lookup reach");
    }

    #[test]
    fn winding_route_chunk_samples_match_authoritative_stage_height() {
        let s=stage::reported_round_two_stage();
        // The old local hill climb differed by 3.68 m at (-68,-472).
        // Include the adjoining chunk to catch chunk-order/seam differences.
        for key in [(-2,-8),(-1,-8),(-1,-11)] {
            let chunk=terrain_chunk(&s,key);
            for j in 0..N {for i in 0..N {
                let x=chunk.x0+i as f64*GRID;let z=chunk.z0+j as f64*GRID;
                let expected=s.height(x,z) as f32;
                assert_eq!(chunk.heights[j*N+i].to_bits(),expected.to_bits(),
                    "terrain/height lookup disagree at ({x},{z})");
            }}
        }
    }

    #[test]
    fn reported_raised_road_has_matching_real_terrain_and_road_colliders() {
        use crate::physics::{Physics,ROAD};
        use rapier3d::{glamx::Vec3,prelude::{QueryFilter,Ray}};
        let s=stage::reported_round_two_stage();
        let chunk=terrain_chunk(&s,(-1,-11));
        let mut physics=Physics::new();physics.add_chunk(&chunk);physics.step();
        let points=[(-57.27709198f32,-662.73962402f32),(-40.47281265,-699.24645996)];
        let mut terrain_hits=Vec::new();
        for (x,z) in points {
            let ray=Ray::new(Vec3::new(x,60.0,z),Vec3::NEG_Y);
            let (_,toi)=physics.world.cast_ray(&ray,100.0,true,QueryFilter::default()).expect("terrain collider hit");
            let hit=60.0-toi;
            assert!((hit-s.height(x as f64,z as f64) as f32).abs()<0.12,
                "terrain collision does not match repaired foundation at ({x},{z}): {hit}");
            terrain_hits.push(hit);
        }
        physics.add_road(&road(&s));physics.step();
        for ((x,z),terrain_y) in points.into_iter().zip(terrain_hits) {
            let ray=Ray::new(Vec3::new(x,60.0,z),Vec3::NEG_Y);
            let (handle,toi)=physics.world.cast_ray(&ray,100.0,true,QueryFilter::default()).expect("road collider hit");
            assert_eq!(physics.world.colliders[handle].user_data,ROAD);
            let gap=60.0-toi-terrain_y;
            assert!((0.0..0.3).contains(&gap),"road still floats over terrain at ({x},{z}): {gap} m");
        }
    }
}
