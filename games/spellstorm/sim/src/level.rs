//! One level of a biome (arena.gd): a winding path from the entrance at the
//! front (+z) through three zones, past two clearings where the fights
//! gather, to the boss's arena near the back and the gate behind it. The
//! walkable ground of a walled level is a band round the path, kept as a
//! signed distance field by 1 m cell. Everything here is laid out once per
//! seed; what breaks (props) keeps its state here too.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use glam::{Affine3A, Mat3, Vec2, Vec3, Vec4};

use crate::biome::{BIOMES, Biome, WallStyle};
use crate::collide::{Shape2, Statics};
use crate::el::El;
use crate::rng::{Pcg, Rand};

pub const LEVEL_HALF: Vec2 = Vec2::new(34.0, 105.0);
pub const PATH_W: f32 = 2.4;
pub const URN_EVERY: f32 = 28.0;

/// What a prop lets out when it breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spill {
    None,
    Souls,
    Globe,
    Blood,
    Fire,
}

#[derive(Clone, Debug)]
pub struct Prop {
    pub kind: &'static str,
    pub pos: Vec3,
    pub yaw: f32,
    /// Picks the look (which variant, its size and lean) on the app's side.
    pub seed: u32,
    pub hp: f32,
    /// Collision radius; 0 for walk-through.
    pub r: f32,
    pub spill: Spill,
    /// Takes hits and breaks (in Arena.props).
    pub breakable: bool,
    pub broken: bool,
    /// The collision shape in `Level::statics`, if any.
    pub shape: Option<usize>,
}

/// One piece of a wall for the renderer.
#[derive(Clone, Copy, Debug)]
pub enum WallMesh {
    /// A unit box scaled by the transform.
    Box(Vec3),
    /// One of six lumpy rocks (jagged for the pit's cliffs), unit sized.
    Crag(u8, bool),
    /// A small skull for the ossuary niches.
    Skull,
}

#[derive(Clone, Copy, Debug)]
pub struct WallPiece {
    pub mesh: WallMesh,
    pub xf: Affine3A,
    /// 0 lit stone, 1 dark stone, 2 darker, 3 bone.
    pub mat: u8,
    /// Wall chunk (20 m cells) for merging.
    pub chunk: (i32, i32),
}

/// Small things along the path drawn by the app.
#[derive(Clone, Copy, Debug)]
pub enum Deco {
    /// A post along a bridge.
    Post(Vec3),
    /// A kerb stone beside a trodden way, turned by degrees.
    Kerb(Vec3, f32),
}

#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub pos: Vec3,
    pub yaw: f32,
}

/// A field laid when the level is built (permanent).
#[derive(Clone, Copy, Debug)]
pub struct LaidField {
    pub at: Vec3,
    pub kind: El,
    pub radius: f32,
}

pub struct Level {
    pub biome_index: usize,
    pub half: Vec2,
    pub rng: Pcg,
    pub path: Vec<Vec2>,
    pub path_run: Vec<f32>,
    pub clearings: Vec<Vec3>,
    pub clearing_at: Vec<f32>,
    pub start: Vec2,
    pub portal_at: Vec2,
    pub exit_at: Vec2,
    pub isles: Vec<Vec4>,
    pub plazas: Vec<Vec3>,
    pub nave: Vec2,
    pub corridor: bool,
    pub sdf: Vec<f32>,
    pub sdf_o: Vec2,
    pub sdf_w: usize,
    pub sdf_h: usize,
    hw: Vec<f32>,
    pub ortho: bool,
    okeys: Vec<Vec2>,
    oseg: Vec<usize>,
    rim: Vec<Vec4>,
    props_at: HashMap<(i32, i32), Vec<Vec3>>,
    near_path: HashMap<(i32, i32), f32>,
    pub props: Vec<Prop>,
    pub walls: Vec<WallPiece>,
    pub torches: Vec<Vec3>,
    pub lamps: Vec<Lamp>,
    pub deco: Vec<Deco>,
    pub laid_fields: Vec<LaidField>,
    pub statics: Statics,
    next_post: f32,
    next_lamp: f32,
}

fn v3(p: Vec2) -> Vec3 {
    Vec3::new(p.x, 0.0, p.y)
}

fn xz(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// How much a prop of `kind` takes before it breaks; a bolt does 2.
pub fn prop_hp(kind: &str) -> f32 {
    match kind {
        "grave" | "dead_tree" | "ice" | "coffin" => 6.0,
        "rock" | "sarcophagus" | "statue" => 16.0,
        "pillar" => 22.0,
        "stalagmite" => 12.0,
        "basalt" => 20.0,
        "brazier" => 8.0,
        "urn" | "oil_jar" | "blood_urn" => 2.0,
        "pew" => 5.0,
        _ => 8.0,
    }
}

/// The collision radius of a prop kind (arena.gd's `r` and Dressing's),
/// None for one you walk through (and that never breaks).
pub fn prop_radius(kind: &str) -> Option<f32> {
    Some(match kind {
        "grave" => 0.4,
        "dead_tree" => 0.35,
        "rock" => 0.8,
        "pillar" => 0.55,
        "sarcophagus" => 0.9,
        "stalagmite" => 0.55,
        "ice" => 0.5,
        "basalt" => 0.6,
        "brazier" => 0.5,
        "urn" | "blood_urn" => 0.3,
        "oil_jar" => 0.35,
        "coffin" => 0.6,
        "statue" => 0.55,
        "pew" => 0.0,
        "fence" => 0.3,
        "mausoleum" => 1.4,
        "cross" => 0.3,
        "ruin_wall" => 0.8,
        "skull_niche" => 0.6,
        "bone_pile" => 0.5,
        "altar" => 0.8,
        "piling" => 0.25,
        "boat" => 0.9,
        "sunken_ruin" => 0.7,
        "hanging_moss_tree" => 0.35,
        "crystal" => 0.5,
        "mine_support" => 0.3,
        "mine_cart" => 0.7,
        "fungus_shelf" => 0.4,
        "banner" => 0.25,
        "weapon_rack" => 0.6,
        "frozen_statue" => 0.55,
        "barricade" => 0.8,
        "icicle_rock" => 0.6,
        "spikes" => 0.5,
        "cage" => 0.6,
        "skull_pile" => 0.5,
        "obsidian_spire" => 0.6,
        "lava_vent" => 0.5,
        _ => return None,
    })
}

fn spill_of(kind: &str) -> Spill {
    match kind {
        "urn" => Spill::Souls,
        "oil_jar" => Spill::Fire,
        "blood_urn" => Spill::Blood,
        "coffin" => Spill::Globe,
        _ => Spill::None,
    }
}

impl Level {
    pub fn biome(&self) -> &'static Biome {
        &BIOMES[self.biome_index % BIOMES.len()]
    }

    pub fn built(&self) -> bool {
        self.biome().nave
    }

    pub fn build(biome_index: usize, seed: u64) -> Level {
        let biome = &BIOMES[biome_index % BIOMES.len()];
        let mut l = Level {
            biome_index: biome_index % BIOMES.len(),
            half: LEVEL_HALF,
            rng: Pcg::new(seed),
            path: Vec::new(),
            path_run: Vec::new(),
            clearings: Vec::new(),
            clearing_at: Vec::new(),
            start: Vec2::ZERO,
            portal_at: Vec2::ZERO,
            exit_at: Vec2::ZERO,
            isles: Vec::new(),
            plazas: Vec::new(),
            nave: Vec2::new(-1.0, -1.0),
            corridor: biome.corridor,
            sdf: Vec::new(),
            sdf_o: Vec2::ZERO,
            sdf_w: 0,
            sdf_h: 0,
            hw: Vec::new(),
            ortho: false,
            okeys: Vec::new(),
            oseg: Vec::new(),
            rim: Vec::new(),
            props_at: HashMap::new(),
            near_path: HashMap::new(),
            props: Vec::new(),
            walls: Vec::new(),
            torches: Vec::new(),
            lamps: Vec::new(),
            deco: Vec::new(),
            laid_fields: Vec::new(),
            statics: Statics::default(),
            next_post: 0.0,
            next_lamp: 3.0,
        };
        l.plan();
        l.index_path();
        if l.corridor {
            l.band();
        }
        if l.corridor {
            l.band_walls();
            l.outside();
        } else {
            l.box_walls();
        }
        l.path_dressing();
        l.arena_ring();
        if l.built() {
            l.nave_props();
        }
        let mut count = (l.half.x * l.half.y * 4.0 / 46.0) as usize;
        if l.corridor {
            count = (l.inside_area() / 33.0 * biome.density) as usize;
        }
        for _ in 0..count {
            let Some(spot) = l.free_spot(2.6) else {
                continue;
            };
            let zone = biome.zones[l.zone_at(spot)];
            let kind = zone[l.rng.pick(zone.len())];
            l.prop(kind, Some(spot), None);
        }
        if !biome.water_props.is_empty() {
            let mut placed = 0;
            for _ in 0..600 {
                if placed >= 70 {
                    break;
                }
                let q = Vec2::new(
                    l.rng.range(-l.half.x + 2.0, l.half.x - 2.0),
                    l.rng.range(-l.half.y + 2.0, l.half.y - 2.0),
                );
                if l.shore(q) < 0.9 || l.path_dist(q) < PATH_W * 0.5 + 1.2 || l.crowded(v3(q), 2.0)
                {
                    continue;
                }
                let kind = biome.water_props[l.rng.pick(biome.water_props.len())];
                l.prop(kind, Some(v3(q)), None);
                placed += 1;
            }
        }
        let urns = (l.length() / URN_EVERY) as usize;
        for u in 0..urns {
            let at = l.urn_spot((u as f32 + 0.5) * l.length() / urns as f32);
            l.prop("blood_urn", at, None);
        }
        for _ in 0..(l.half.y / 9.0) as usize {
            for &k in biome.fields {
                let Some(fp) = l.free_spot(3.0) else { continue };
                let radius = l.rng.range(1.6, 3.0);
                l.laid_fields.push(LaidField {
                    at: fp,
                    kind: k,
                    radius,
                });
            }
        }
        l.statics.finish();
        l
    }

    /// Which of the three zones `at` is in, by how far along the path.
    pub fn zone_at(&self, at: Vec3) -> usize {
        let k = self.progress_of(xz(at)) / self.length().max(0.001);
        ((k * 3.0) as i32).clamp(0, 2) as usize
    }

    fn plan(&mut self) {
        let built = self.built();
        let corridor = self.corridor;
        self.ortho = built && corridor && self.biome().id != "chapel";
        let half = self.half;
        self.start = Vec2::new(self.rng.range(-6.0, 6.0), half.y - 7.0);
        let arena = Vec3::new(0.0, -half.y + 24.0, 14.0);
        self.portal_at = Vec2::new(arena.x, arena.y - 6.0);
        self.exit_at = Vec2::new(0.0, -half.y + 1.6);
        let mut keys = vec![Vec2::new(self.start.x, half.y + 1.5), self.start];
        let reach = half.x - 9.0;
        let mut side = if self.rng.f() < 0.5 { 1.0 } else { -1.0 };
        let mut z = self.start.y - 16.0;
        let mut marks: Vec<Vec2> = Vec::new();
        let mut last_x = self.start.x;
        let mut straight = false;
        while z > arena.y + 22.0 {
            let mut x = side * self.rng.range(reach * 0.15, reach);
            if straight {
                let wobble = if self.ortho {
                    0.0
                } else {
                    self.rng.range(-1.5, 1.5)
                };
                x = (last_x + wobble).clamp(-reach, reach);
            } else if (x - last_x).abs() < 6.0 {
                x = (last_x + side * 8.0).clamp(-reach, reach);
            }
            marks.push(Vec2::new(x, z));
            last_x = x;
            if self.rng.f() < 0.75 {
                side = -side;
            }
            straight = corridor && !straight && self.rng.f() < 0.35;
            z -= if straight {
                self.rng.range(20.0, 28.0)
            } else {
                self.rng.range(12.0, 24.0)
            };
        }
        let c1 = (marks.len() as f32 * 0.3) as usize;
        let c2 = (marks.len() as f32 * 0.68) as usize;
        for i in 0..marks.len() {
            let mut m = marks[i];
            if built && i > c1 && (i < c2 || (self.ortho && i == c2)) {
                m.x = marks[c1].x;
            }
            keys.push(m);
            if i == c1 || i == c2 {
                let r = self.rng.range(8.0, 9.5);
                self.clearings.push(Vec3::new(m.x, m.y, r));
            }
        }
        if built && c2 < marks.len() {
            marks[c2].x = marks[c1].x;
        }
        keys.push(Vec2::new(arena.x, arena.y + arena.z + 3.0));
        keys.push(Vec2::new(arena.x, arena.y));
        keys.push(self.portal_at);
        keys.push(Vec2::new(0.0, -half.y - 1.0));
        self.clearings.push(arena);
        if self.ortho {
            self.orthogonal(&keys);
        } else {
            self.path = smooth(&keys, 7);
        }
        if built {
            for c in self.clearings.clone() {
                self.plazas.push(Vec3::new(c.x, c.y, c.z + 1.0));
            }
        }
        if !self.biome().isles {
            return;
        }
        self.isles
            .push(Vec4::new(self.start.x, self.start.y, 8.0, 5.0));
        for c in self.clearings.clone() {
            self.isles.push(Vec4::new(c.x, c.y, c.z + 1.5, c.z + 0.5));
        }
        for m in &marks {
            let rx = self.rng.range(3.0, 4.5);
            let rz = self.rng.range(2.6, 3.6);
            self.isles.push(Vec4::new(m.x, m.y, rx, rz));
        }
        for _ in 0..30 {
            let c = Vec2::new(
                self.rng.range(-half.x + 4.0, half.x - 4.0),
                self.rng.range(-half.y + 4.0, half.y - 4.0),
            );
            let clear = self
                .isles
                .iter()
                .all(|e| c.distance(Vec2::new(e.x, e.y)) >= e.z.max(e.w) + 4.0);
            if clear {
                let rx = self.rng.range(2.0, 4.0);
                let rz = self.rng.range(1.8, 3.2);
                self.isles.push(Vec4::new(c.x, c.y, rx, rz));
            }
        }
    }

    /// Straight halls between the keys, turning square.
    fn orthogonal(&mut self, keys: &[Vec2]) {
        self.okeys = vec![keys[0]];
        for &q in &keys[1..] {
            let p = *self.okeys.last().unwrap();
            if (p.x - q.x).abs() > 0.01 && (p.y - q.y).abs() > 0.01 {
                self.okeys.push(Vec2::new(p.x, q.y));
            }
            self.okeys.push(q);
        }
        self.path.clear();
        self.oseg.clear();
        for i in 0..self.okeys.len() - 1 {
            let a = self.okeys[i];
            let b = self.okeys[i + 1];
            let n = ((a.distance(b) / 2.0) as usize).max(1);
            for k in 0..n {
                self.path.push(a.lerp(b, k as f32 / n as f32));
                self.oseg.push(i);
            }
        }
        self.path.push(*self.okeys.last().unwrap());
        self.oseg.push(self.okeys.len() - 2);
    }

    fn index_path(&mut self) {
        self.path_run.clear();
        let mut run = 0.0;
        for i in 0..self.path.len() {
            if i > 0 {
                run += self.path[i].distance(self.path[i - 1]);
            }
            self.path_run.push(run);
        }
        let at: Vec<f32> = self
            .clearings
            .iter()
            .map(|c| self.progress_of(Vec2::new(c.x, c.y)))
            .collect();
        self.clearing_at = at;
        if self.built() && self.clearing_at.len() >= 2 {
            self.nave = Vec2::new(self.clearing_at[0] + 9.0, self.clearing_at[1] - 9.0);
        }
        let mut x = -self.half.x;
        while x <= self.half.x {
            let mut z = -self.half.y;
            while z <= self.half.y {
                let d = self.path_dist(Vec2::new(x + 1.0, z + 1.0));
                self.near_path
                    .insert(((x / 2.0).floor() as i32, (z / 2.0).floor() as i32), d);
                z += 2.0;
            }
            x += 2.0;
        }
    }

    fn sdf_alloc(&mut self) {
        let pad = 16.0;
        self.sdf_o = Vec2::new(-self.half.x - pad, -self.half.y - pad);
        self.sdf_w = ((self.half.x + pad) * 2.0) as usize + 1;
        self.sdf_h = ((self.half.y + pad) * 2.0) as usize + 1;
        self.sdf = vec![30.0; self.sdf_w * self.sdf_h];
    }

    fn band(&mut self) {
        if self.ortho {
            self.band_ortho();
            return;
        }
        let knots: Vec<f32> = (0..(self.length() / 20.0) as usize + 3)
            .map(|_| self.rng.range(5.0, 9.0))
            .collect();
        self.hw.clear();
        for i in 0..self.path.len() {
            let run = self.path_run[i];
            let f = run / 20.0;
            let j = f as usize;
            let mut t = f - j as f32;
            t = t * t * (3.0 - 2.0 * t);
            let mut w = lerp(knots[j], knots[(j + 1).min(knots.len() - 1)], t);
            if self.nave.x >= 0.0 && run > self.nave.x - 6.0 && run < self.nave.y + 6.0 {
                w = 13.5;
            }
            w = lerp(
                w,
                2.3,
                ((self.portal_at.y - 3.0 - self.path[i].y) / 5.0).clamp(0.0, 1.0),
            );
            self.hw.push(w);
        }
        self.sdf_alloc();
        for i in 0..self.path.len() - 1 {
            self.raster_seg(self.path[i], self.path[i + 1], self.hw[i], self.hw[i + 1]);
        }
        self.raster_seg(self.start, self.start, 7.0, 7.0);
        for c in self.clearings.clone() {
            let cc = Vec2::new(c.x, c.y);
            self.raster_seg(cc, cc, c.z + 3.0, c.z + 3.0);
        }
        let noise = gdcompat::Noise::with_defaults(self.rng.next_u32() as i32, 0.08);
        for iz in 0..self.sdf_h {
            for ix in 0..self.sdf_w {
                let q = self.sdf_o + Vec2::new(ix as f32, iz as f32);
                let i2 = iz * self.sdf_w + ix;
                let mut d = self.sdf[i2] + noise.get_2d(q.x, q.y) * 1.6;
                d = d.max(q.y - (self.half.y + 0.6));
                d = d.max(-q.y - (self.half.y + 0.6));
                self.sdf[i2] = d;
            }
        }
    }

    fn band_ortho(&mut self) {
        self.sdf_alloc();
        let mut segw = Vec::new();
        for i in 0..self.okeys.len() - 1 {
            let a = self.okeys[i];
            let b = self.okeys[i + 1];
            let mut w = self.rng.range(3.2, 4.6);
            let run = self.progress_of((a + b) * 0.5);
            if self.nave.x >= 0.0
                && run > self.nave.x - 6.0
                && run < self.nave.y + 6.0
                && (a.x - b.x).abs() < 0.01
            {
                w = 13.5;
            }
            if a.y <= self.portal_at.y + 0.1 && b.y <= self.portal_at.y + 0.1 {
                w = 2.3;
            }
            segw.push(w);
            self.raster_box((a + b) * 0.5, (b - a).abs() * 0.5 + Vec2::splat(w));
            if w < 6.0 && w > 3.0 && a.distance(b) > 14.0 && self.rng.f() < 0.55 {
                let along = (b - a).normalize();
                let across = Vec2::new(along.y, -along.x) * self.rng.sign();
                let room = Vec2::new(self.rng.range(3.0, 5.0), self.rng.range(3.0, 5.0));
                let depth = across.x.abs() * room.x + across.y.abs() * room.y;
                let t = self.rng.range(0.3, 0.7);
                let c = a.lerp(b, t) + across * (w + 1.0 + depth);
                if c.x.abs() + room.x < self.half.x - 2.0 {
                    self.raster_box(c, room);
                    let door =
                        along.x.abs() * Vec2::new(1.4, 0.0) + along.y.abs() * Vec2::new(0.0, 1.4);
                    let he = door
                        + across.x.abs() * Vec2::new(1.2, 0.0)
                        + across.y.abs() * Vec2::new(0.0, 1.2);
                    self.raster_box(c - across * depth - across * 0.5, he);
                }
            }
        }
        self.hw = (0..self.path.len()).map(|i| segw[self.oseg[i]]).collect();
        self.raster_box(self.start, Vec2::splat(6.0));
        for c in self.clearings.clone() {
            self.raster_box(Vec2::new(c.x, c.y), Vec2::splat(c.z + 2.0));
        }
        for iz in 0..self.sdf_h {
            for ix in 0..self.sdf_w {
                let q = self.sdf_o + Vec2::new(ix as f32, iz as f32);
                let i2 = iz * self.sdf_w + ix;
                let d = self.sdf[i2].max(q.y - (self.half.y + 0.6));
                self.sdf[i2] = d.max(-q.y - (self.half.y + 0.6));
            }
        }
    }

    fn raster_box(&mut self, c: Vec2, he: Vec2) {
        let x0 = ((c.x - he.x - 9.0 - self.sdf_o.x) as i64).max(0) as usize;
        let x1 = (((c.x + he.x + 9.0 - self.sdf_o.x) as i64 + 1).min(self.sdf_w as i64 - 1)).max(0)
            as usize;
        let z0 = ((c.y - he.y - 9.0 - self.sdf_o.y) as i64).max(0) as usize;
        let z1 = (((c.y + he.y + 9.0 - self.sdf_o.y) as i64 + 1).min(self.sdf_h as i64 - 1)).max(0)
            as usize;
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                let q = (self.sdf_o + Vec2::new(ix as f32, iz as f32) - c).abs() - he;
                let d = Vec2::new(q.x.max(0.0), q.y.max(0.0)).length() + q.x.max(q.y).min(0.0);
                let i = iz * self.sdf_w + ix;
                if d < self.sdf[i] {
                    self.sdf[i] = d;
                }
            }
        }
    }

    fn raster_seg(&mut self, a: Vec2, b: Vec2, wa: f32, wb: f32) {
        let reach = wa.max(wb) + 8.0;
        let x0 = ((a.x.min(b.x) - reach - self.sdf_o.x) as i64).max(0) as usize;
        let x1 = (((a.x.max(b.x) + reach - self.sdf_o.x) as i64 + 1).min(self.sdf_w as i64 - 1))
            .max(0) as usize;
        let z0 = ((a.y.min(b.y) - reach - self.sdf_o.y) as i64).max(0) as usize;
        let z1 = (((a.y.max(b.y) + reach - self.sdf_o.y) as i64 + 1).min(self.sdf_h as i64 - 1))
            .max(0) as usize;
        let ab = b - a;
        let l2 = ab.length_squared().max(0.0001);
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                let q = self.sdf_o + Vec2::new(ix as f32, iz as f32);
                let t = ((q - a).dot(ab) / l2).clamp(0.0, 1.0);
                let d = q.distance(a + ab * t) - lerp(wa, wb, t);
                let i = iz * self.sdf_w + ix;
                if d < self.sdf[i] {
                    self.sdf[i] = d;
                }
            }
        }
    }

    /// How far `p` (xz) is outside the walkable ground: negative inside.
    pub fn bound(&self, p: Vec2) -> f32 {
        if self.sdf.is_empty() {
            return f32::NEG_INFINITY;
        }
        let f = p - self.sdf_o;
        let x = f.x.clamp(0.0, self.sdf_w as f32 - 1.001);
        let z = f.y.clamp(0.0, self.sdf_h as f32 - 1.001);
        let ix = x as usize;
        let iz = z as usize;
        let tx = x - ix as f32;
        let tz = z - iz as f32;
        let i = iz * self.sdf_w + ix;
        let s = &self.sdf;
        lerp(
            lerp(s[i], s[i + 1], tx),
            lerp(s[i + self.sdf_w], s[i + self.sdf_w + 1], tx),
            tz,
        )
    }

    /// The way out of the walkable ground at `p`.
    pub fn bound_out(&self, p: Vec2) -> Vec2 {
        let g = Vec2::new(
            self.bound(p + Vec2::new(0.5, 0.0)) - self.bound(p - Vec2::new(0.5, 0.0)),
            self.bound(p + Vec2::new(0.0, 0.5)) - self.bound(p - Vec2::new(0.0, 0.5)),
        );
        if g.length() > 0.0001 {
            g.normalize()
        } else {
            Vec2::new(0.0, -1.0)
        }
    }

    fn inside_area(&self) -> f32 {
        self.sdf.iter().filter(|v| **v < 0.0).count() as f32
    }

    /// Half the band's width `run` metres along the path.
    pub fn hw_at(&self, run: f32) -> f32 {
        if self.hw.is_empty() {
            return 9.0;
        }
        for i in 0..self.path.len() - 1 {
            if self.path_run[i + 1] >= run {
                let span = (self.path_run[i + 1] - self.path_run[i]).max(0.001);
                return lerp(
                    self.hw[i],
                    self.hw[i + 1],
                    ((run - self.path_run[i]) / span).clamp(0.0, 1.0),
                );
            }
        }
        *self.hw.last().unwrap()
    }

    /// Contour points where the band's field crosses zero, one every 1.4 m.
    fn contour(&self) -> Vec<Vec2> {
        let mut pts = Vec::new();
        let mut taken: HashMap<(i32, i32), Vec2> = HashMap::new();
        for iz in 0..self.sdf_h - 1 {
            for ix in 0..self.sdf_w - 1 {
                let i = iz * self.sdf_w + ix;
                let v = self.sdf[i];
                for nb in [(1usize, 0usize), (0, 1)] {
                    let w = self.sdf[i + nb.0 + nb.1 * self.sdf_w];
                    if (v < 0.0) == (w < 0.0) {
                        continue;
                    }
                    let t = v / (v - w);
                    let q = self.sdf_o
                        + Vec2::new(ix as f32 + nb.0 as f32 * t, iz as f32 + nb.1 as f32 * t);
                    let key = ((q.x / 1.4).floor() as i32, (q.y / 1.4).floor() as i32);
                    if taken.contains_key(&key) {
                        continue;
                    }
                    let mut near = false;
                    for dx in -1..=1 {
                        for dz in -1..=1 {
                            if let Some(o) = taken.get(&(key.0 + dx, key.1 + dz)) {
                                if o.distance(q) < 1.25 {
                                    near = true;
                                }
                            }
                        }
                    }
                    if near {
                        continue;
                    }
                    taken.insert(key, q);
                    pts.push(q);
                }
            }
        }
        pts
    }

    /// Walls along the edge of the band, low where they face the camera.
    fn band_walls(&mut self) {
        let pts = self.contour();
        let mut chains: Vec<Vec<Vec2>> = Vec::new();
        let mut used = vec![false; pts.len()];
        for start in 0..pts.len() {
            if used[start] {
                continue;
            }
            let mut chain = vec![pts[start]];
            used[start] = true;
            for _ in 0..2 {
                loop {
                    let tip = *chain.last().unwrap();
                    let mut best = None;
                    let mut bd = 2.4;
                    for (k, p) in pts.iter().enumerate() {
                        if used[k] {
                            continue;
                        }
                        let d = p.distance(tip);
                        if d < bd {
                            bd = d;
                            best = Some(k);
                        }
                    }
                    match best {
                        Some(k) => {
                            used[k] = true;
                            chain.push(pts[k]);
                        }
                        None => break,
                    }
                }
                chain.reverse();
            }
            chains.push(chain);
        }
        let mut ends = Vec::new();
        for ch in &chains {
            ends.push(ch[0]);
            ends.push(*ch.last().unwrap());
        }
        for ci in 0..chains.len() {
            for at_end in 0..2 {
                let ch = &chains[ci];
                let e = if at_end == 1 {
                    *ch.last().unwrap()
                } else {
                    ch[0]
                };
                let mut best_p = None;
                let mut best_d = 3.2;
                for (ei, p) in ends.iter().enumerate() {
                    if ei / 2 == ci {
                        continue;
                    }
                    let d = p.distance(e);
                    if d > 0.05 && d < best_d {
                        best_d = d;
                        best_p = Some(*p);
                    }
                }
                if let Some(p) = best_p {
                    if at_end == 1 {
                        chains[ci].push(p);
                    } else {
                        chains[ci].insert(0, p);
                    }
                }
            }
        }
        // Any stretch of edge still without a wall in front gets a stub.
        let mut segs: Vec<(Vec2, Vec2)> = Vec::new();
        for ch in &chains {
            for k in 0..ch.len().saturating_sub(1) {
                segs.push((ch[k], ch[k + 1]));
            }
        }
        for iz in 0..self.sdf_h - 1 {
            for ix in 0..self.sdf_w - 1 {
                let i = iz * self.sdf_w + ix;
                for nb in [(1usize, 0usize), (0, 1)] {
                    let v = self.sdf[i];
                    let w = self.sdf[i + nb.0 + nb.1 * self.sdf_w];
                    if (v < 0.0) == (w < 0.0) {
                        continue;
                    }
                    let q = self.sdf_o
                        + Vec2::new(
                            ix as f32 + nb.0 as f32 * v / (v - w),
                            iz as f32 + nb.1 as f32 * v / (v - w),
                        );
                    let covered = segs.iter().any(|(a2, b2)| {
                        let ab = *b2 - *a2;
                        let tt =
                            ((q - *a2).dot(ab) / ab.length_squared().max(0.0001)).clamp(0.0, 1.0);
                        (*a2 + ab * tt).distance(q) < 0.8
                    });
                    if covered {
                        continue;
                    }
                    let out = self.bound_out(q);
                    let tan = Vec2::new(-out.y, out.x) * 0.7;
                    chains.push(vec![q - tan, q + tan]);
                    segs.push((q - tan, q + tan));
                }
            }
        }
        let style = self.biome().wall_style;
        let natural = matches!(style, WallStyle::Rock | WallStyle::Cliff);
        let wall_props = self.biome().wall_props;
        let every = self.biome().wall_every;
        let mut last_torch: Vec<Vec2> = Vec::new();
        for chain in &chains {
            if chain.len() < 2 {
                continue;
            }
            let drift = self.rng.f() * 10.0;
            let mut since = self.rng.f() * every;
            for k in 0..chain.len() - 1 {
                let a = chain[k];
                let b = chain[k + 1];
                let mid = (a + b) * 0.5;
                if mid.y < -self.half.y + 1.5 && mid.x.abs() < 2.8 {
                    continue;
                }
                let out = self.bound_out(mid);
                let along = b - a;
                let len = along.length();
                if len < 0.05 {
                    continue;
                }
                let face = smoothstep(0.25, 0.8, out.y);
                let kf = k as f32;
                let wave =
                    0.88 + 0.12 * (drift + kf * 0.45).sin() + 0.06 * (drift * 1.7 + kf * 1.3).sin();
                let tall = match style {
                    WallStyle::Castle => 3.3,
                    WallStyle::Cliff => 2.0,
                    _ => 2.7,
                };
                let h = lerp(tall, 0.75, face) * wave;
                let yaw = along.x.atan2(along.y) + PI * 0.5;
                let basis = Mat3::from_rotation_y(yaw);
                let centre = Vec3::new(mid.x + out.x * 0.45, 0.0, mid.y + out.y * 0.45);
                let ck = ((mid.x / 20.0).floor() as i32, (mid.y / 20.0).floor() as i32);
                if natural {
                    for j in 0..2 {
                        let w = (len + 0.9) * self.rng.range(0.65, 0.85);
                        let tall_one = if k % 4 == 0 && j == 0 { 1.35 } else { 1.0 };
                        let ch = h * self.rng.range(0.85, 1.25) * tall_one;
                        let off = (j as f32 - 0.5) * len * 0.45;
                        let lean_y = self.rng.range(-0.4, 0.4);
                        let lean_x = self.rng.range(-0.12, 0.12);
                        let depth = self.rng.range(1.3, 1.9);
                        let rb =
                            basis * Mat3::from_rotation_y(lean_y) * Mat3::from_rotation_x(lean_x);
                        let at = centre + basis * Vec3::new(off, 0.0, 0.0) + Vec3::Y * ch * 0.38;
                        let crag = (self.rng.next_u32() % 6) as u8;
                        self.walls.push(WallPiece {
                            mesh: WallMesh::Crag(crag, style == WallStyle::Cliff),
                            xf: Affine3A::from_mat3_translation(
                                rb * Mat3::from_diagonal(Vec3::new(w, ch, depth)),
                                at,
                            ),
                            mat: if j == 0 { 0 } else { 1 },
                            chunk: ck,
                        });
                    }
                } else {
                    let piece = |walls: &mut Vec<WallPiece>,
                                 mat: u8,
                                 size: Vec3,
                                 at: Vec3,
                                 b: Mat3| {
                        walls.push(WallPiece {
                            mesh: WallMesh::Box(size),
                            xf: Affine3A::from_mat3_translation(b * Mat3::from_diagonal(size), at),
                            mat,
                            chunk: ck,
                        });
                    };
                    piece(
                        &mut self.walls,
                        0,
                        Vec3::new(len + 0.35, h, 0.8),
                        centre + Vec3::Y * h * 0.5,
                        basis,
                    );
                    piece(
                        &mut self.walls,
                        1,
                        Vec3::new(len + 0.4, 0.14, 1.0),
                        centre + Vec3::Y * (h + 0.07),
                        basis,
                    );
                    piece(
                        &mut self.walls,
                        1,
                        Vec3::new(len + 0.4, 0.3, 1.05),
                        centre + Vec3::Y * 0.15,
                        basis,
                    );
                    if style == WallStyle::Castle && h > 1.9 {
                        let n = ((len / 0.9).round() as usize).max(1);
                        for m in 0..n {
                            let o = -len * 0.5 + (m as f32 + 0.5) * len / n as f32;
                            let at = centre + basis * Vec3::new(o, 0.0, 0.0) + Vec3::Y * (h + 0.36);
                            piece(
                                &mut self.walls,
                                0,
                                Vec3::new(len / n as f32 * 0.55, 0.45, 0.9),
                                at,
                                basis,
                            );
                        }
                    }
                    if style == WallStyle::Ossuary && h > 2.2 && k % 2 == 1 && len > 0.9 {
                        let inner = centre - Vec3::new(out.x, 0.0, out.y) * 0.4;
                        piece(
                            &mut self.walls,
                            2,
                            Vec3::new(len * 0.7, 0.75, 0.06),
                            inner + Vec3::Y * 1.35,
                            basis,
                        );
                        piece(
                            &mut self.walls,
                            1,
                            Vec3::new(len * 0.8, 0.08, 0.2),
                            inner + Vec3::Y * 0.94,
                            basis,
                        );
                        let sk = ((len * 0.7 / 0.24) as usize).max(1);
                        for m in 0..sk {
                            for row in 0..2 {
                                let o2 = -len * 0.33
                                    + (m as f32 + 0.5) * len * 0.66 / sk as f32
                                    + if row == 1 { 0.06 } else { 0.0 };
                                let at = inner - Vec3::new(out.x, 0.0, out.y) * 0.04
                                    + basis * Vec3::new(o2, 0.0, 0.0)
                                    + Vec3::Y * (1.08 + row as f32 * 0.25);
                                let turn = self.rng.range(-0.4, 0.4);
                                let face_yaw = (-out.x).atan2(-out.y);
                                self.walls.push(WallPiece {
                                    mesh: WallMesh::Skull,
                                    xf: Affine3A::from_mat3_translation(
                                        Mat3::from_rotation_y(face_yaw + turn),
                                        at,
                                    ),
                                    mat: 3,
                                    chunk: ck,
                                });
                            }
                        }
                    }
                    if k % 3 == 0 {
                        let ph = h + 0.35;
                        let foot = Vec3::new(a.x + out.x * 0.45, 0.0, a.y + out.y * 0.45);
                        piece(
                            &mut self.walls,
                            0,
                            Vec3::new(0.75, ph, 1.15),
                            foot + Vec3::Y * ph * 0.5,
                            basis,
                        );
                        piece(
                            &mut self.walls,
                            1,
                            Vec3::new(0.95, 0.2, 1.35),
                            foot + Vec3::Y * (ph + 0.1),
                            basis,
                        );
                        if ph > 2.4 && last_torch.iter().all(|o| o.distance(a) >= 13.0) {
                            last_torch.push(a);
                            self.torches.push(Vec3::new(
                                a.x - out.x * 0.25,
                                2.2,
                                a.y - out.y * 0.25,
                            ));
                        }
                    }
                }
                since += len;
                if h > 1.7 && !wall_props.is_empty() && since >= every {
                    let spot = mid - out * 0.85;
                    if self.path_dist(spot) > PATH_W * 0.5 + 1.2 && !self.crowded(v3(spot), 1.4) {
                        since = 0.0;
                        let kind = wall_props[self.rng.pick(wall_props.len())];
                        self.prop(kind, Some(v3(spot)), Some((-out.x).atan2(-out.y)));
                    }
                }
                let xa = basis * Vec3::X;
                self.statics.add(Shape2::Box {
                    c: Vec2::new(centre.x, centre.z),
                    half: Vec2::new((len + 0.5) * 0.5, 0.6),
                    ax: Vec2::new(xa.x, xa.z),
                    top: 4.0,
                });
                self.rim.push(Vec4::new(
                    mid.x - out.x * 1.4,
                    mid.y - out.y * 1.4,
                    -out.x,
                    -out.y,
                ));
            }
        }
        self.gate();
    }

    /// Beyond the walls: dark ground thick with the biome's things.
    fn outside(&mut self) {
        let kinds = self.biome().outside;
        if kinds.is_empty() {
            return;
        }
        let mut n = 0;
        for _ in 0..900 {
            if n >= 170 {
                break;
            }
            let p = Vec2::new(
                self.rng.range(-self.half.x - 12.0, self.half.x + 12.0),
                self.rng.range(-self.half.y, self.half.y + 6.0),
            );
            let d = self.bound(p);
            if !(1.8..=11.0).contains(&d) {
                continue;
            }
            let kind = kinds[self.rng.pick(kinds.len())];
            if let Some(i) = self.prop(kind, Some(v3(p)), None) {
                // Out of reach: no collision, nothing to break.
                let pr = &mut self.props[i];
                pr.breakable = false;
                if let Some(s) = pr.shape.take() {
                    self.statics.remove(s);
                }
            }
            n += 1;
        }
    }

    /// A walled level that is one big rectangle (the isles).
    fn box_walls(&mut self) {
        let half = self.half;
        let low = self.biome().low_walls;
        let segs = [
            (
                Vec3::new(-(half.x + 1.2 + 1.25) * 0.5, 0.0, -half.y - 0.6),
                Vec3::new(half.x + 1.2 - 1.25, 3.4, 1.2),
            ),
            (
                Vec3::new((half.x + 1.2 + 1.25) * 0.5, 0.0, -half.y - 0.6),
                Vec3::new(half.x + 1.2 - 1.25, 3.4, 1.2),
            ),
            (
                Vec3::new(-half.x - 0.6, 0.0, 0.0),
                Vec3::new(1.2, if low { 1.1 } else { 3.0 }, half.y * 2.0 + 2.4),
            ),
            (
                Vec3::new(half.x + 0.6, 0.0, 0.0),
                Vec3::new(1.2, if low { 1.1 } else { 3.0 }, half.y * 2.0 + 2.4),
            ),
            (
                Vec3::new(0.0, 0.0, half.y + 0.6),
                Vec3::new(half.x * 2.0 + 2.4, 0.7, 1.2),
            ),
        ];
        for (at, size) in segs {
            self.statics.add(Shape2::Box {
                c: Vec2::new(at.x, at.z),
                half: Vec2::new(size.x, size.z) * 0.5,
                ax: Vec2::X,
                top: 4.0,
            });
            let h = size.y;
            let along_x = size.x > size.z;
            let length = if along_x { size.x } else { size.z };
            let n = (length / 1.6) as usize;
            let ck = ((at.x / 20.0).floor() as i32, (at.z / 20.0).floor() as i32);
            for i in 0..n {
                let off = -length * 0.5 + (i as f32 + 0.5) * length / n as f32;
                let bh = h * self.rng.range(0.85, 1.1);
                let s = if along_x {
                    Vec3::new(length / n as f32 - 0.06, bh, size.z)
                } else {
                    Vec3::new(size.x, bh, length / n as f32 - 0.06)
                };
                let p = if along_x {
                    Vec3::new(off, bh * 0.5, 0.0)
                } else {
                    Vec3::new(0.0, bh * 0.5, off)
                };
                self.walls.push(WallPiece {
                    mesh: WallMesh::Box(s),
                    xf: Affine3A::from_scale_rotation_translation(s, glam::Quat::IDENTITY, at + p),
                    mat: if i % 3 != 0 { 0 } else { 1 },
                    chunk: ck,
                });
            }
            if h > 2.0 {
                let tn = (length / 14.0) as usize;
                for i in 0..tn {
                    let off2 = -length * 0.5 + (i as f32 + 0.5) * length / tn as f32;
                    let inward = if along_x {
                        Vec3::new(0.0, 0.0, -at.z.signum() * 0.75)
                    } else {
                        Vec3::new(-at.x.signum() * 0.75, 0.0, 0.0)
                    };
                    let tp = if along_x {
                        Vec3::new(off2, 2.2, 0.0)
                    } else {
                        Vec3::new(0.0, 2.2, off2)
                    } + inward;
                    self.torches.push(at + tp);
                }
            }
        }
        self.gate();
    }

    /// An open archway in the back wall where the path leads out: its
    /// pieces are drawn by the app; here only the block that stops you.
    fn gate(&mut self) {
        let z = -self.half.y - 0.6;
        self.statics.add(Shape2::Box {
            c: Vec2::new(0.0, z),
            half: Vec2::new(1.7, 0.6),
            ax: Vec2::X,
            top: 4.0,
        });
    }

    /// Where the gate stands (its middle at the floor).
    pub fn gate_at(&self) -> Vec3 {
        Vec3::new(0.0, 0.0, -self.half.y - 0.6)
    }

    pub fn path_dist(&self, p: Vec2) -> f32 {
        line_dist(&self.path, p)
    }

    pub fn progress_of(&self, p: Vec2) -> f32 {
        let mut best = f32::INFINITY;
        let mut at = 0.0;
        for i in 0..self.path.len().saturating_sub(1) {
            let a = self.path[i];
            let ab = self.path[i + 1] - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
            let d = p.distance(a + ab * t);
            if d < best {
                best = d;
                at = self.path_run[i] + t * ab.length();
            }
        }
        at
    }

    pub fn point_at(&self, run: f32) -> Vec2 {
        if self.path.is_empty() {
            return Vec2::ZERO;
        }
        for i in 0..self.path.len() - 1 {
            if self.path_run[i + 1] >= run {
                let span = self.path_run[i + 1] - self.path_run[i];
                return self.path[i].lerp(
                    self.path[i + 1],
                    ((run - self.path_run[i]) / span.max(0.001)).clamp(0.0, 1.0),
                );
            }
        }
        *self.path.last().unwrap()
    }

    pub fn length(&self) -> f32 {
        self.path_run.last().copied().unwrap_or(0.0)
    }

    pub fn in_clearing(&self, p: Vec2, pad: f32) -> bool {
        self.clearings
            .iter()
            .any(|c| p.distance(Vec2::new(c.x, c.y)) < c.z + pad)
    }

    fn arena_ring(&mut self) {
        let a = *self.clearings.last().unwrap();
        let kind = match self.biome().id {
            "chapel" | "catacombs" | "keep" => "pillar",
            "caves" => "stalagmite",
            "pit" => "basalt",
            _ => "rock",
        };
        let n = 18;
        for i in 0..n {
            let ang = TAU * i as f32 / n as f32;
            let at = Vec2::new(a.x + ang.cos() * (a.z + 1.2), a.y + ang.sin() * (a.z + 1.2));
            if self.path_dist(at) < PATH_W + 1.0 {
                continue;
            }
            self.prop(kind, Some(v3(at)), Some(0.0));
        }
        // Blood and mana round the inside of the ring: who keeps moving
        // can heal and fill up during the boss (Joep: the first boss was
        // too hard to outlast).
        let n = 6;
        for i in 0..n {
            let ang = TAU * (i as f32 + 0.5) / n as f32;
            let at = Vec2::new(a.x + ang.cos() * (a.z - 1.6), a.y + ang.sin() * (a.z - 1.6));
            if self.path_dist(at) < PATH_W * 0.5 + 0.8 {
                continue;
            }
            let kind = if i % 2 == 0 { "blood_urn" } else { "urn" };
            self.prop(kind, Some(v3(at)), None);
        }
    }

    fn nave_props(&mut self) {
        if self.nave.x < 0.0 || self.nave.y <= self.nave.x {
            return;
        }
        let a = self.point_at(self.nave.x);
        let b = self.point_at(self.nave.y);
        let x0 = a.x;
        let mut z = a.y.min(b.y);
        let z_end = a.y.max(b.y);
        match self.biome().id {
            "chapel" => {
                let mut zs = z;
                while z < z_end {
                    for x in [3.2, 6.0] {
                        for sgn in [-1.0f32, 1.0] {
                            if self.rng.f() < 0.5 {
                                let px = x0 + sgn * (x + 0.4) + self.rng.range(-0.2, 0.2);
                                let tilt = self.rng.range(-0.15, 0.15);
                                let knock = if self.rng.f() < 0.12 { 0.6 } else { 0.0 };
                                self.prop(
                                    "pew",
                                    Some(Vec3::new(px, 0.0, z)),
                                    Some(PI + tilt + knock),
                                );
                            }
                        }
                    }
                    if self.rng.f() < 0.22 {
                        let sgn2 = self.rng.sign();
                        let kind = if self.rng.f() < 0.6 { "urn" } else { "oil_jar" };
                        let px = x0 + sgn2 * self.rng.range(8.6, 10.0);
                        self.prop(kind, Some(Vec3::new(px, 0.0, z + 1.2)), None);
                    }
                    z += 2.2;
                }
                while zs < z_end {
                    for sgn in [-1.0f32, 1.0] {
                        self.prop(
                            "statue",
                            Some(Vec3::new(x0 + sgn * 11.4, 0.0, zs + 1.0)),
                            Some(-PI * 0.5 * sgn),
                        );
                    }
                    zs += 9.0;
                }
            }
            "catacombs" => {
                let mut zp = z;
                while zp < z_end {
                    for sgn in [-1.0f32, 1.0] {
                        self.prop(
                            "pillar",
                            Some(Vec3::new(x0 + sgn * 2.9, 0.0, zp)),
                            Some(0.0),
                        );
                    }
                    zp += 3.4;
                }
                while z < z_end {
                    for x in [7.0, 10.0] {
                        for sgn in [-1.0f32, 1.0] {
                            if self.rng.f() < 0.7 {
                                self.prop(
                                    "sarcophagus",
                                    Some(Vec3::new(x0 + sgn * x, 0.0, z)),
                                    Some(0.0),
                                );
                            }
                        }
                    }
                    z += 3.2;
                }
            }
            _ => {
                while z < z_end {
                    for sgn in [-1.0f32, 1.0] {
                        self.prop("pillar", Some(Vec3::new(x0 + sgn * 3.1, 0.0, z)), Some(0.0));
                    }
                    z += 4.0;
                }
            }
        }
    }

    /// Signed distance to the nearest island shore, negative on land.
    pub fn shore(&self, p: Vec2) -> f32 {
        if self.isles.is_empty() {
            return f32::NEG_INFINITY;
        }
        let mut d = f32::INFINITY;
        for e in &self.isles {
            let q = Vec2::new((p.x - e.x) / e.z, (p.y - e.y) / e.w);
            d = d.min((q.length() - 1.0) * e.z.min(e.w));
        }
        d
    }

    /// Off every island and off the bridges: wading.
    pub fn in_water(&self, at: Vec3) -> bool {
        if self.isles.is_empty() {
            return false;
        }
        let p = xz(at);
        self.shore(p) > 0.15 && self.path_dist(p) > PATH_W * 0.5
    }

    fn path_dressing(&mut self) {
        self.next_post = 0.0;
        self.next_lamp = 3.0;
        let mut run = 0.0;
        for i in 0..self.path.len() - 1 {
            let (a, b) = (self.path[i], self.path[i + 1]);
            self.dress_span(a, b, run);
            run += a.distance(b);
        }
    }

    fn dress_span(&mut self, a: Vec2, b: Vec2, run: f32) {
        let built = self.built();
        let len = a.distance(b);
        if len <= 0.0 {
            return;
        }
        let dir = (b - a) / len;
        let nrm = Vec2::new(-dir.y, dir.x);
        let mut t = 0.0;
        while t < len {
            let c = a + dir * t;
            let at = run + t;
            let inside = c.x.abs() < self.half.x - 0.5 && c.y.abs() < self.half.y - 0.5;
            if inside && at >= self.next_post {
                if !self.isles.is_empty() && self.shore(c) > 0.1 {
                    self.next_post = at + 1.6;
                    for sgn in [-1.0f32, 1.0] {
                        let q = c + nrm * sgn * (PATH_W * 0.5 - 0.1);
                        self.deco.push(Deco::Post(Vec3::new(q.x, 0.2, q.y)));
                    }
                } else if self.isles.is_empty() && !built {
                    self.next_post = at + 1.1;
                    for sgn in [-1.0f32, 1.0] {
                        let q2 = c + nrm * sgn * (PATH_W * 0.5 + 0.15);
                        if self.rng.f() < 0.55 && self.path_dist(q2) > PATH_W * 0.5 {
                            let turn = self.rng.f() * 60.0;
                            self.deco
                                .push(Deco::Kerb(Vec3::new(q2.x, 0.06, q2.y), turn));
                        }
                    }
                }
            }
            if inside
                && at >= self.next_lamp
                && (self.isles.is_empty() || self.shore(c) < -0.5)
                && !self.in_clearing(c, 1.0)
            {
                if built {
                    self.next_lamp = at + 11.0;
                    for sgn in [-1.0f32, 1.0] {
                        self.lamp(c + nrm * sgn * (PATH_W * 0.5 + 0.5), Some(-nrm * sgn));
                    }
                } else {
                    self.next_lamp = at + 15.0;
                    let sgn2 = self.rng.sign();
                    self.lamp(c + nrm * sgn2 * (PATH_W * 0.5 + 0.6), Some(-nrm * sgn2));
                }
            }
            t += 0.25;
        }
    }

    /// A post with a hanging lantern beside the path.
    fn lamp(&mut self, at: Vec2, toward: Option<Vec2>) {
        let yaw = match toward {
            Some(t) if t != Vec2::ZERO => (-t.y).atan2(t.x),
            _ => self.rng.f() * TAU,
        };
        self.lamps.push(Lamp { pos: v3(at), yaw });
        self.mark(v3(at));
    }

    fn urn_spot(&mut self, run: f32) -> Option<Vec3> {
        for _ in 0..30 {
            let r = (run + self.rng.range(-6.0, 6.0)).clamp(8.0, self.length() - 4.0);
            let c = self.point_at(r);
            let dir = (self.point_at(r + 1.0) - self.point_at(r - 1.0)).normalize_or_zero();
            let side = self.rng.sign();
            let p2 = c + Vec2::new(-dir.y, dir.x)
                * side
                * self.rng.range(PATH_W * 0.5 + 1.2, PATH_W * 0.5 + 4.0);
            if self.bound(p2) > -1.2 || self.in_clearing(p2, 1.0) || p2.distance(self.start) < 7.0 {
                continue;
            }
            if !self.isles.is_empty() && self.shore(p2) > -0.8 {
                continue;
            }
            let p = v3(p2);
            if self.crowded(p, 1.6) {
                continue;
            }
            return Some(p);
        }
        None
    }

    fn free_spot(&mut self, clear: f32) -> Option<Vec3> {
        for _ in 0..40 {
            let mut p = Vec3::new(
                self.rng.range(-self.half.x + 2.0, self.half.x - 2.0),
                0.0,
                self.rng.range(-self.half.y + 2.0, self.half.y - 2.0),
            );
            if self.corridor {
                let run = self.rng.range(0.0, self.length());
                let c = self.point_at(run);
                let dir = (self.point_at(run + 1.0) - self.point_at(run - 1.0)).normalize_or_zero();
                let off =
                    Vec2::new(-dir.y, dir.x) * self.rng.range(-1.0, 1.0) * (self.hw_at(run) + 4.0);
                p = Vec3::new(c.x + off.x, 0.0, c.y + off.y);
                if self.bound(xz(p)) > -1.2 {
                    continue;
                }
            }
            let p2 = xz(p);
            if p2.distance(self.start) < 7.0 || self.in_clearing(p2, 1.0) {
                continue;
            }
            if self.off_path(p2) < PATH_W * 0.5 + 1.0 {
                continue;
            }
            if !self.isles.is_empty() && self.shore(p2) > -0.8 {
                continue;
            }
            if self.crowded(p, clear) {
                continue;
            }
            self.mark(p);
            return Some(p);
        }
        None
    }

    fn off_path(&self, p: Vec2) -> f32 {
        let coarse = self
            .near_path
            .get(&((p.x / 2.0).floor() as i32, (p.y / 2.0).floor() as i32))
            .copied()
            .unwrap_or(f32::INFINITY);
        if coarse > PATH_W + 3.0 {
            coarse
        } else {
            self.path_dist(p)
        }
    }

    fn cell(p: Vec3) -> (i32, i32) {
        ((p.x / 3.0).floor() as i32, (p.z / 3.0).floor() as i32)
    }

    fn mark(&mut self, p: Vec3) {
        self.props_at.entry(Self::cell(p)).or_default().push(p);
    }

    fn crowded(&self, p: Vec3, clear: f32) -> bool {
        let c = Self::cell(p);
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(v) = self.props_at.get(&(c.0 + dx, c.1 + dz)) {
                    if v.iter().any(|q| p.distance(*q) < clear) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// A prop of `kind` at a free spot, or at `at` facing `yaw`. Returns
    /// its index in `props`, None when it is not a prop (a lantern, or no
    /// room was found).
    pub fn prop(
        &mut self,
        kind: &'static str,
        at: Option<Vec3>,
        yaw: Option<f32>,
    ) -> Option<usize> {
        let p = match at {
            Some(a) => a,
            None => self.free_spot(2.6)?,
        };
        if at.is_some() {
            self.mark(p);
        }
        if kind == "lantern" {
            self.lamp(xz(p), None);
            return None;
        }
        let yaw = yaw.unwrap_or_else(|| self.rng.f() * TAU);
        let seed = self.rng.next_u32();
        let r = prop_radius(kind);
        let mut prop = Prop {
            kind,
            pos: p,
            yaw,
            seed,
            hp: prop_hp(kind),
            r: r.unwrap_or(0.0),
            spill: spill_of(kind),
            breakable: r.is_some(),
            broken: false,
            shape: None,
        };
        if kind == "pew" {
            let ax = Mat3::from_rotation_y(yaw) * Vec3::X;
            prop.shape = Some(self.statics.add(Shape2::Box {
                c: xz(p),
                half: Vec2::new(1.2, 0.275),
                ax: Vec2::new(ax.x, ax.z),
                top: 3.0,
            }));
        } else if let Some(r) = r {
            prop.shape = Some(self.statics.add(Shape2::Circle {
                c: xz(p),
                r,
                top: 3.0,
            }));
        }
        self.props.push(prop);
        let i = self.props.len() - 1;
        self.statics.owner(self.props[i].shape, i);
        Some(i)
    }

    /// Keep `p` on the walkable ground, 0.8 m from its edge.
    pub fn clamp_inside(&self, p: Vec3) -> Vec3 {
        let mut q = Vec3::new(
            p.x.clamp(-self.half.x + 0.8, self.half.x - 0.8),
            0.0,
            p.z.clamp(-self.half.y + 0.8, self.half.y - 0.8),
        );
        if self.corridor {
            let q2 = xz(q);
            let d = self.bound(q2);
            if d > -0.8 {
                let out = self.bound_out(q2);
                q.x -= out.x * (d + 0.8);
                q.z -= out.y * (d + 0.8);
            }
        }
        q
    }

    /// Where a monster of an encounter walks in: between `near` and `far`
    /// metres further along the path than `lead`, off to either side, out
    /// of the water when it can (`water` 1 for in it).
    pub fn spawn_ahead(
        &mut self,
        lead: f32,
        near: f32,
        far: f32,
        away_from: &[Vec3],
        water: f32,
    ) -> Vec3 {
        let mut best = Vec3::ZERO;
        let mut best_d = f32::NEG_INFINITY;
        for _ in 0..14 {
            let run = (lead + self.rng.range(near, far)).clamp(0.0, self.length());
            let c = self.point_at(run);
            let dir = (self.point_at(run + 1.0) - self.point_at(run - 1.0)).normalize_or_zero();
            let wide = if self.corridor {
                9.0f32.min(self.hw_at(run) - 1.5)
            } else {
                9.0
            };
            let side = Vec2::new(-dir.y, dir.x) * self.rng.range(-wide, wide);
            let p = self.clamp_inside(Vec3::new(c.x + side.x, 0.0, c.y + side.y));
            let mut d = f32::INFINITY;
            for w in away_from {
                d = d.min(p.distance(*w));
            }
            let wet = if self.in_water(p) { 3.0 * water } else { 0.0 };
            let score = d.min(9.0) + wet + self.rng.f();
            if score > best_d {
                best_d = score;
                best = p;
            }
        }
        best
    }

    /// Where a monster can walk in from: over the walls near the wizards.
    pub fn spawn_point(&mut self, away_from: &[Vec3]) -> Vec3 {
        let mut best = Vec3::ZERO;
        let mut best_d = -1.0;
        if self.corridor && !self.rim.is_empty() {
            for _ in 0..24 {
                let r = self.rim[self.rng.pick(self.rim.len())];
                let p2 = Vec3::new(r.x, 0.0, r.y);
                let mut d2 = f32::INFINITY;
                for w in away_from {
                    d2 = d2.min(p2.distance(*w));
                }
                if d2 < 30.0 && d2 > best_d {
                    best_d = d2;
                    best = p2;
                }
            }
            if best_d > 0.0 {
                return best;
            }
        }
        let half = self.half;
        for _ in 0..12 {
            let side = self.rng.pick(3);
            let p = match side {
                0 => Vec3::new(
                    self.rng.range(-half.x + 1.5, half.x - 1.5),
                    0.0,
                    -half.y + 1.2,
                ),
                1 => Vec3::new(
                    -half.x + 1.2,
                    0.0,
                    self.rng.range(-half.y + 1.5, half.y - 1.5),
                ),
                _ => Vec3::new(
                    half.x - 1.2,
                    0.0,
                    self.rng.range(-half.y + 1.5, half.y - 1.5),
                ),
            };
            let mut d = f32::INFINITY;
            for w in away_from {
                d = d.min(p.distance(*w));
            }
            if d > best_d {
                best_d = d;
                best = p;
            }
        }
        best
    }

    /// A prop takes `amount` (`Prop.damage`). Returns true when it broke.
    pub fn damage_prop(&mut self, i: usize, amount: f32) -> bool {
        let p = &mut self.props[i];
        if p.broken || !p.breakable {
            return false;
        }
        p.hp -= amount;
        if p.hp <= 0.0 {
            p.broken = true;
            p.breakable = false;
            if let Some(s) = p.shape.take() {
                self.statics.remove(s);
            }
            return true;
        }
        false
    }

    /// The props within `r` of `at` (blasts), unbroken ones only.
    pub fn props_near(&self, at: Vec3, r: f32) -> Vec<usize> {
        let mut out = Vec::new();
        for (i, p) in self.props.iter().enumerate() {
            if p.breakable
                && !p.broken
                && Vec2::new(p.pos.x - at.x, p.pos.z - at.z).length() < r + 0.4
            {
                out.push(i);
            }
        }
        out
    }
}

pub fn line_dist(line: &[Vec2], p: Vec2) -> f32 {
    let mut best = f32::INFINITY;
    for i in 0..line.len().saturating_sub(1) {
        let a = line[i];
        let ab = line[i + 1] - a;
        let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
        best = best.min(p.distance(a + ab * t));
    }
    best
}

/// A Catmull-Rom curve through `keys`, `steps` points per span.
pub fn smooth(keys: &[Vec2], steps: usize) -> Vec<Vec2> {
    let mut out = Vec::new();
    for i in 0..keys.len() - 1 {
        let p0 = keys[i.saturating_sub(1)];
        let p1 = keys[i];
        let p2 = keys[i + 1];
        let p3 = keys[(i + 2).min(keys.len() - 1)];
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let t2 = t * t;
            let t3 = t2 * t;
            out.push(
                0.5 * ((2.0 * p1)
                    + (-p0 + p2) * t
                    + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                    + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3),
            );
        }
    }
    out.push(*keys.last().unwrap());
    out
}
