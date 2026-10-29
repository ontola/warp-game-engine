//! The game drawn by Warp: the level built into a scene (floor, walls,
//! props, torches), the wizards and monsters posed from the sim every frame
//! into a pool of instances, the Godot camera, moon, ambient and fog.
use crate::anim;
use crate::kits::Kits;
use crate::look::{self, linear3, push_f32, surface_layer, Look, Piece, FLOOR};
use glam::{Affine3A, Mat3, Mat4, Quat, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::headlights::LocalLight;
use render::irradiance_volume::{Grid, Probe};
use render::scene::{Material, NO_SHADOW, SURFACE};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use spellstorm::level::{Level, WallMesh, PATH_W};
use spellstorm::World;
use std::collections::HashMap;
use std::f32::consts::PI;

/// Instances kept for whatever moves (wizards, monsters, corpses, spells).
const POOL: usize = 4096;

/// Godot's light energy 1 lights white albedo like this many lux.
const LIGHT: f32 = PI;

/// A biome's mist (FogVolume density 1) as even fog density per metre.
const MIST_DEPTH: f32 = 0.02;

/// Godot's ZONE_TINTS: the level's three zones, front to back.
const ZONE_TINTS: [[f32; 3]; 3] = [[1.0, 1.0, 1.0], [0.82, 0.92, 0.8], [1.08, 0.93, 0.88]];

/// A Godot colour (sRGB, may exceed 1).
#[derive(Clone, Copy, Debug)]
pub struct Col(pub [f32; 3]);

impl Col {
    pub fn darkened(self, k: f32) -> Col {
        Col(self.0.map(|c| c * (1.0 - k)))
    }
    pub fn lightened(self, k: f32) -> Col {
        Col(self.0.map(|c| c + (1.0 - c) * k))
    }
    pub fn lerp(self, to: Col, t: f32) -> Col {
        Col([0, 1, 2].map(|i| self.0[i] + (to.0[i] - self.0[i]) * t))
    }
    pub fn hsv(self) -> (f32, f32, f32) {
        let [r, g, b] = self.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let v = max;
        if delta == 0.0 {
            return (0.0, 0.0, v);
        }
        let s = delta / max;
        let mut h = if r == max {
            (g - b) / delta
        } else if g == max {
            2.0 + (b - r) / delta
        } else {
            4.0 + (r - g) / delta
        };
        h /= 6.0;
        if h < 0.0 {
            h += 1.0;
        }
        (h, s, v)
    }
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Col {
        if s == 0.0 {
            return Col([v, v, v]);
        }
        let h6 = (h * 6.0) % 6.0;
        let i = h6.floor() as i32;
        let f = h6 - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        Col(match i {
            0 => [v, t, p],
            1 => [q, v, p],
            2 => [p, v, t],
            3 => [p, q, v],
            4 => [t, p, v],
            _ => [v, p, q],
        })
    }
    pub fn linear(self) -> Vec3 {
        linear3(self.0)
    }
}

/// The cloth colours of a wizard in a robe of `robe` (Art.wizard), by the
/// weave each cloth has there.
pub fn wizard_cloth(robe: [f32; 3]) -> HashMap<u32, Vec3> {
    let (h, s, v) = Col(robe).hsv();
    let robe = Col::from_hsv(h, s * 0.7, v * 0.6);
    let dark = robe.darkened(0.45);
    HashMap::from([
        (90, robe.linear()),
        (70, robe.darkened(0.15).lerp(Col([0.25, 0.22, 0.2]), 0.2).linear()),
        (120, dark.darkened(0.3).linear()),
        (200, robe.darkened(0.3).lerp(Col([0.2, 0.17, 0.14]), 0.2).linear()),
    ])
}

/// A light the level keeps (torches, lamps, candles, braziers).
#[derive(Clone, Copy, Debug)]
pub struct Lamp {
    pub pos: Vec3,
    pub light: LocalLight,
}

pub struct View {
    pub kits: Kits,
    look: Look,
    pub scene: Scene,
    pub light: Lighting,
    level_key: u64,
    pool_first: usize,
    /// Scene instance ranges of each prop (hidden once broken).
    prop_ranges: Vec<std::ops::Range<usize>>,
    prop_broken: Vec<bool>,
    pub lamps: Vec<Lamp>,
    /// Camera-facing cards in the level: instance, mesh, centre, scale.
    cards: Vec<(usize, usize, Vec3, Vec3)>,
    probes: Vec<Probe>,
    /// Wizard kit pieces per seat (their robes recoloured).
    wizard_pieces: HashMap<usize, Vec<Piece>>,
    wizard_anims: Vec<anim::WizardAnim>,
    enemy_anims: HashMap<hecs::Entity, anim::EnemyAnim>,
    /// Pieces of dead monsters lying about.
    debris: Vec<Debris>,
    cam_center: Vec3,
    cam_height: f32,
    pub camera: Camera,
    pub fresh: bool,
    pub fx: crate::fx::Fx,
    pub marks: crate::marks::Marks,
    pub shrines: crate::shrines::Shrines,
    /// How long each wizard has lain fallen.
    wizard_fallen: Vec<f32>,
}

fn level_key(l: &Level) -> u64 {
    let mut h: u64 = 1469598103934665603;
    let mut eat = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(1099511628211);
    };
    eat(l.biome_index as u64);
    eat(l.path.len() as u64);
    for p in l.path.iter().take(4) {
        eat(p.x.to_bits() as u64);
        eat(p.y.to_bits() as u64);
    }
    eat(l.props.len() as u64);
    h
}

impl View {
    pub fn new(kits: Kits) -> View {
        let scene = Scene::default();
        let look = Look::empty();
        View {
            kits,
            look,
            scene,
            light: Lighting::default(),
            level_key: 0,
            pool_first: 0,
            prop_ranges: vec![],
            prop_broken: vec![],
            lamps: vec![],
            cards: vec![],
            probes: vec![],
            wizard_pieces: HashMap::new(),
            wizard_anims: vec![],
            wizard_fallen: vec![],
            enemy_anims: HashMap::new(),
            debris: Vec::new(),
            cam_center: Vec3::ZERO,
            cam_height: 18.5,
            camera: Camera { pos: Vec3::new(0.0, 20.0, 12.0), target: Vec3::ZERO, fov_y: 42f32.to_radians(), near: 0.1 },
            fresh: true,
            fx: crate::fx::Fx::default(),
            marks: crate::marks::Marks::default(),
            shrines: crate::shrines::Shrines::default(),
        }
    }

    /// The level changed (a new one was built): the scene must be made again.
    pub fn stale(&self, w: &World) -> bool {
        level_key(&w.level) != self.level_key
    }

    /// Build the scene for the world's level.
    pub fn build(&mut self, w: &World) -> Result<(), String> {
        let mut scene = Scene::default();
        scene.sky = Some((4, 2, vec![[0.0, 0.0, 0.0, 1.0]; 8]));
        let mut look = Look::new(&self.kits, &mut scene)?;
        let l = &w.level;
        let biome = l.biome();
        self.lamps.clear();
        self.cards.clear();
        self.prop_ranges.clear();
        self.prop_broken.clear();
        self.debris.clear();
        self.wizard_pieces.clear();

        // The floor, in 10 m chunks, each with only the bits of path,
        // islands and plazas that reach it (Arena._floor).
        let level_rec = floor_record(&mut scene, l);
        let chunk = 10.0;
        let reach = PATH_W + 1.0;
        let wide = if l.corridor { 24.0 } else { 0.0 };
        let plane = |m: u32| {
            let h = chunk * 0.5;
            let v = |x: f32, z: f32| Vertex { pos: [x, 0.0, z], normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0] };
            let verts = [v(-h, -h), v(h, -h), v(h, h), v(-h, h)];
            ClusterMesh::build(&[Part { vertices: &verts, indices: &[0, 2, 1, 0, 3, 2], material: m }])
        };
        let mut x = -l.half.x - 5.0 - wide;
        while x < l.half.x + 5.0 + wide {
            let mut z = -l.half.y - 5.0;
            while z < l.half.y + 8.0 {
                let lo = Vec2::new(x, z);
                let hi = lo + Vec2::splat(chunk);
                if l.corridor && l.bound(lo + Vec2::splat(chunk * 0.5)) > 7.1 + 24.0 {
                    z += chunk;
                    continue;
                }
                let o = scene.aux.len();
                scene.aux.push(level_rec as u32);
                let counts = scene.aux.len();
                scene.aux.extend([0u32; 3]);
                let mut segs = 0u32;
                for i in 0..l.path.len().saturating_sub(1) {
                    let (a, b) = (l.path[i], l.path[i + 1]);
                    if a.x.min(b.x) - reach < hi.x && a.x.max(b.x) + reach > lo.x && a.y.min(b.y) - reach < hi.y && a.y.max(b.y) + reach > lo.y && segs < 32 {
                        for v in [a.x, a.y, b.x, b.y, l.path_run[i]] {
                            push_f32(&mut scene.aux, v);
                        }
                        segs += 1;
                    }
                }
                let mut isles = 0u32;
                for e in &l.isles {
                    if e.x - e.z - 7.0 < hi.x && e.x + e.z + 7.0 > lo.x && e.y - e.w - 7.0 < hi.y && e.y + e.w + 7.0 > lo.y && isles < 16 {
                        for v in e.to_array() {
                            push_f32(&mut scene.aux, v);
                        }
                        isles += 1;
                    }
                }
                let mut plazas = 0u32;
                for c in &l.plazas {
                    if c.x - c.z - 1.0 < hi.x && c.x + c.z + 1.0 > lo.x && c.y - c.z - 1.0 < hi.y && c.y + c.z + 1.0 > lo.y && plazas < 4 {
                        for v in c.to_array() {
                            push_f32(&mut scene.aux, v);
                        }
                        plazas += 1;
                    }
                }
                scene.aux[counts] = segs;
                scene.aux[counts + 1] = isles;
                scene.aux[counts + 2] = plazas;
                let m = scene.add_material(Material { tint: [1.0; 3], layer: surface_layer(FLOOR, o), roughness: 0.95, flags: SURFACE, scale: 1.0, metal: 0.0 });
                let mesh = scene.add_mesh(&plane(m));
                scene.add_instance(mesh, Affine3A::from_translation(Vec3::new(x + chunk * 0.5, 0.0, z + chunk * 0.5)));
                // Nothing lies under the floor to shade: it casts no shadow.
                scene.mark_no_shadow();
                z += chunk;
            }
            x += chunk;
        }

        // Walls: blocks and rocks in the biome's stone.
        let wall_mats: Vec<usize> = self.kits.entry(&format!("walls/{}", biome.id))["materials"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect()).unwrap_or_default();
        let box_mesh = self.kits.entry("box")["mesh"].as_u64().unwrap_or(0) as usize;
        for piece in &l.walls {
            let kit_mesh = match piece.mesh {
                WallMesh::Box(_) => box_mesh,
                WallMesh::Crag(i, jag) => self.kits.entry(&format!("crag/{i}/{}", jag as u8))["mesh"].as_u64().unwrap_or(0) as usize,
                WallMesh::Skull => self.kits.entry("skull")["mesh"].as_u64().unwrap_or(0) as usize,
            };
            // The outer walls of an open level are lit and dark (Arena._walls).
            let mi = if !l.corridor && piece.mat == 1 { 4 } else { piece.mat as usize };
            let Some(&mat) = wall_mats.get(mi).or(wall_mats.first()) else { continue };
            if let Some((mesh, _)) = look.mesh(&self.kits, &mut scene, kit_mesh, Some(mat)) {
                scene.add_instance(mesh, piece.xf);
            }
        }

        // Props: one of the biome's looks of each, picked by its seed.
        for p in &l.props {
            let variants = self.kits.prop_variants(biome.id, p.kind);
            let first = scene.instances.len();
            if variants > 0 {
                let name = format!("prop/{}/{}/{}", biome.id, p.kind, p.seed as usize % variants);
                let at = Affine3A::from_rotation_translation(Quat::from_rotation_y(p.yaw), p.pos);
                self.place(&mut look, &mut scene, &name, at);
            }
            self.prop_ranges.push(first..scene.instances.len());
            self.prop_broken.push(p.broken);
        }
        for t in &l.torches {
            self.place(&mut look, &mut scene, "torch", Affine3A::from_translation(*t));
        }
        for lamp in &l.lamps {
            self.place(&mut look, &mut scene, "lamp", Affine3A::from_rotation_translation(Quat::from_rotation_y(lamp.yaw), lamp.pos));
        }

        self.shrines.build(&mut scene);
        // The pool for whatever moves: every kit it may show is built now.
        for (i, &id) in w.wizards.iter().enumerate() {
            let robe = w.wiz(id).robe;
            let pieces = look.wizard(&self.kits, &mut scene, i, &wizard_cloth([robe[0], robe[1], robe[2]]));
            self.wizard_pieces.insert(i, pieces);
        }
        for k in spellstorm::enemy::KINDS.iter() {
            look.pieces(&self.kits, &mut scene, &format!("enemy/{}", k.key));
        }
        self.pool_first = scene.instances.len();
        let any = scene.meshes.len() - 1;
        for _ in 0..POOL {
            scene.add_instance(any, Affine3A::ZERO);
            let i = scene.instances.len() - 1;
            scene.instances[i].cluster_count = 0;
        }

        // Ambient light: Godot's flat ambient colour, as an even irradiance
        // over the level (one probe every 8 m).
        let amb = linear3(biome.ambient) * 0.45 * LIGHT;
        let lo = Vec3::new(-l.half.x - 40.0, -2.0, -l.half.y - 20.0);
        let cell = 8.0;
        let dims = [((l.half.x * 2.0 + 80.0) / cell) as u32 + 2, 3, ((l.half.y * 2.0 + 40.0) / cell) as u32 + 2];
        let grid = Grid { origin: lo, cell, dims };
        scene.reserve_irradiance_volume(grid)?;
        self.probes = vec![Probe { axes: [amb; 6] }; grid.count()];

        self.light = lighting(w, amb);
        self.scene = scene;
        self.look = look;
        self.level_key = level_key(l);
        self.fresh = true;
        Ok(())
    }

    /// Kit `name` at `at` as static instances, its lights kept.
    fn place(&mut self, look: &mut Look, scene: &mut Scene, name: &str, at: Affine3A) {
        let pieces = look.pieces(&self.kits, scene, name);
        let Some(kit) = self.kits.kit(name) else { return };
        let at = Mat4::from(at);
        for p in &pieces {
            let xf = at * kit.model_xf(p.node);
            scene.add_instance(p.mesh, look::affine(xf));
            if !p.shadow {
                scene.mark_no_shadow();
            }
            if p.billboard {
                let (scale, _, pos) = xf.to_scale_rotation_translation();
                self.cards.push((scene.instances.len() - 1, p.mesh, pos, scale));
            }
        }
        for (i, n) in kit.nodes.iter().enumerate() {
            if let Some(l) = n.light {
                let pos = (at * kit.model_xf(i)).w_axis.truncate();
                self.lamps.push(Lamp { pos, light: omni(pos, l) });
            }
        }
    }

    /// After the renderer was made from `scene`: ambient, glow and lights.
    pub fn upload(&self, gpu: &Gpu, renderer: &Renderer) -> Result<(), String> {
        renderer.set_irradiance_volume(gpu, &self.probes)?;
        let mut o = renderer.debug_options();
        // Godot's glow (intensity 0.6, threshold 1), matched like the lobby's.
        o.bloom_screen = true;
        o.bloom_strength = 0.6 * 0.36;
        o.bloom_threshold = 1.0;
        renderer.set_debug_options(o)?;
        renderer.set_sprite_image(gpu, crate::fx::IMAGE, &crate::fx::atlas(), render::hud::ImageFilter::Linear);
        Ok(())
    }

    /// Pose everything that moves, the camera and the nearest lights.
    pub fn pose(&mut self, gpu: &Gpu, renderer: &Renderer, w: &World, dt: f32) -> Result<(), String> {
        let mut out: Vec<(usize, Affine3A, bool)> = Vec::new();
        let time = w.time as f32;
        self.wizard_anims.resize_with(w.wizards.len(), Default::default);
        // Where each staff's gem is as the staff moves: its light and orbs sit there.
        let mut gems = vec![None; w.wizards.len()];
        for (i, &id) in w.wizards.iter().enumerate() {
            let a = w.a(id);
            let wz = w.wiz(id);
            let Some(pieces) = self.wizard_pieces.get(&i) else { continue };
            let Some(kit) = self.kits.kit("wizard") else { continue };
            let pose = self.wizard_anims[i].pose(kit, &a, &wz, dt);
            // A fallen wizard lies down beside the grave.
            self.wizard_fallen.resize(w.wizards.len(), 0.0);
            let f = &mut self.wizard_fallen[i];
            *f = if a.alive { 0.0 } else { *f + dt };
            let k = (*f / 0.5).min(1.0);
            let k = k * k * (3.0 - 2.0 * k);
            let at = Mat4::from_translation(a.pos + Vec3::Y * 0.12 * k) * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2 * k);
            for p in pieces {
                out.push((p.mesh, look::affine(at * anim::posed(kit, &pose, p.node)), p.shadow));
            }
            if let Some(&g) = kit.named.get("gem") {
                gems[i] = Some((at * anim::posed(kit, &pose, g)).w_axis.truncate());
            }
        }
        self.marks.gems = gems.clone();
        self.shrines.gems = gems;
        let mut seen = Vec::new();
        for &id in w.enemies.iter().chain(&w.corpses) {
            let Some(a) = w.actor(id) else { continue };
            let Ok(e) = w.ecs.get::<&spellstorm::enemy::Enemy>(id) else { continue };
            let name = format!("enemy/{}", e.cfg().key);
            let pieces = self.look.pieces(&self.kits, &mut self.scene, &name);
            let Some(kit) = self.kits.kit(&name) else { continue };
            let seed = id.id();
            let ea = self.enemy_anims.entry(id).or_default();
            let model = ea.model(kit.base_scale, &a, &e, time, seed, dt);
            seen.push(id);
            let at = Mat4::from_rotation_translation(Quat::from_rotation_y(a.yaw), a.pos) * model;
            // Bodies that fall apart go to pieces once (a blow-up after its fuse).
            if let Some(t) = ea.dead_t.filter(|_| !ea.scattered) {
                let c = e.cfg();
                let big = if c.model.is_some() { c.size as f32 } else { 1.0 };
                let how = match anim::fall(c.model.unwrap_or(c.key), big) {
                    f @ anim::Fall::Scatter { .. } => Some(f),
                    anim::Fall::BlowUp if t >= e.fuse as f32 => Some(anim::BLOWN),
                    _ => None,
                };
                if let Some(anim::Fall::Scatter { power, up, life }) = how {
                    ea.scattered = true;
                    let parts = pieces.iter().map(|p| (p.mesh, at * kit.model_xf(p.node), p.shadow)).collect();
                    self.debris.push(Debris::new(&self.scene, parts, a.pos, power, up, life, seed));
                }
            }
            if !ea.scattered {
                // Its eyes swell while it winds up a blow (Enemy._tell).
                let eyes = if e.wind > 0.0 { 1.0 + 1.5 * (1.0 - e.wind / e.wind_time()) as f32 } else { 1.0 };
                for p in &pieces {
                    let mut xf = at * kit.model_xf(p.node);
                    if eyes != 1.0 && kit.eyes.contains(&p.node) {
                        xf *= Mat4::from_scale(Vec3::splat(eyes));
                    }
                    out.push((p.mesh, look::affine(xf), p.shadow));
                }
            }
        }
        self.enemy_anims.retain(|id, _| seen.contains(id));
        self.debris.retain_mut(|d| d.step(dt));
        for d in &self.debris {
            d.push(&mut out);
        }
        self.shrines.update(w, time, &mut out);
        let first = self.pool_first;
        for i in 0..POOL {
            let slot = first + i;
            match out.get(i) {
                Some(&(mesh, xf, shadow)) => {
                    let flags = if shadow { 0 } else { NO_SHADOW };
                    self.scene.update_instance(slot, mesh, xf);
                    self.scene.instances[slot].flags = flags;
                }
                None => self.scene.instances[slot].cluster_count = 0,
            }
        }
        // Props broken since the scene was made disappear.
        for (i, p) in w.level.props.iter().enumerate() {
            if p.broken && !self.prop_broken.get(i).copied().unwrap_or(true) {
                self.prop_broken[i] = true;
                let r = self.prop_ranges[i].clone();
                for j in r.clone() {
                    self.scene.instances[j].cluster_count = 0;
                }
                if !r.is_empty() {
                    renderer.update_instances(gpu, r.start, &self.scene.instances[r])?;
                }
            }
        }
        renderer.update_instances(gpu, first, &self.scene.instances[first..first + POOL])?;
        self.follow(w, dt);
        self.fx.update(w, dt);
        self.marks.update(w, self.fx.time, dt);
        self.marks.torches(&w.level.torches, self.camera.target, self.fx.time);
        let shake = self.fx.shake_offset();
        self.camera.pos += shake;
        self.camera.target += shake;
        // Cards turn to the camera (halo.gdshader's billboard).
        let face = facing(&self.camera);
        for &(i, mesh, pos, scale) in &self.cards {
            if self.scene.instances[i].cluster_count == 0 {
                continue;
            }
            self.scene.update_instance(i, mesh, Affine3A::from_mat3_translation(face * Mat3::from_diagonal(scale), pos));
            renderer.update_instances(gpu, i, &self.scene.instances[i..i + 1])?;
        }
        // The 32 lights nearest the middle of the view.
        let mid = self.camera.target;
        let mut near: Vec<(usize, &Lamp)> = self.lamps.iter().enumerate().collect();
        near.sort_by(|a, b| a.1.pos.distance_squared(mid).total_cmp(&b.1.pos.distance_squared(mid)));
        // Spell lights first, the level's lamps after.
        let mut locals: Vec<LocalLight> = self.fx.lights();
        locals.sort_by(|a, b| b.intensity_cd.max_element().total_cmp(&a.intensity_cd.max_element()));
        locals.truncate(12);
        // Every wizard's staff light, always.
        locals.splice(0..0, self.marks.spill.iter().chain(&self.shrines.lights).copied());
        let room = render::headlights::MAX_LOCAL_LIGHTS - locals.len();
        // Every lamp flickers a little (Arena's torch loop).
        let t = w.time as f32;
        locals.extend(near.iter().take(room).map(|&(i, l)| {
            let i = i as f32;
            let k = 0.85 + 0.1 * (t * 9.0 + i * 1.7).sin() + 0.05 * (t * 23.0 + i).sin();
            LocalLight { intensity_cd: l.light.intensity_cd * k, ..l.light }
        }));
        renderer.set_local_lights(&locals)?;
        let spots = &self.marks.spots[..self.marks.spots.len().min(render::headlights::MAX_LIGHTS)];
        renderer.set_headlights(spots)?;
        renderer.set_spot_falloff(&vec![crate::marks::STAFF_FALLOFF; spots.len()]);
        renderer.set_spot_softness(spellstorm::knobs::shadow_softness() as f32);
        // Lamps light the haze (Godot's light_volumetric_fog_energy is 1).
        if self.light.volumetric.is_some() {
            // The haze takes each gem's light whole (the spots do not scatter).
            let n = self.marks.spill.len();
            let mut beams = locals.clone();
            beams.splice(0..n, self.marks.lights.iter().copied());
            renderer.set_volumetric_beams(&beams)?;
        }
        self.light.time = w.time as f32;
        let mut sprites = self.marks.sprites.clone();
        sprites.extend_from_slice(&self.shrines.sprites);
        sprites.extend(self.fx.sprites(&self.camera));
        sprites.truncate(render::sprites::MAX_SPRITES);
        crate::effects::sync(gpu);
        crate::effects::tag(&mut sprites, self.fx.time);
        renderer.set_sprites(gpu, &sprites)?;
        let t = self.fx.time;
        self.fx.blasts.flush(gpu, renderer, t)?;
        Ok(())
    }

    /// main.gd _update_camera: over the wizards, higher as they spread.
    fn follow(&mut self, w: &World, dt: f32) {
        let mut pts: Vec<Vec3> = w.wizards.iter().filter(|&&id| w.alive(id)).map(|&id| w.pos(id)).collect();
        if pts.is_empty() {
            pts = w.wizards.iter().map(|&id| w.pos(id)).collect();
        }
        let mut center = Vec3::ZERO;
        let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in &pts {
            center += *p;
            lo = lo.min(Vec2::new(p.x, p.z));
            hi = hi.max(Vec2::new(p.x, p.z));
        }
        if !pts.is_empty() {
            center /= pts.len() as f32;
        }
        let spread = if pts.is_empty() { 0.0 } else { ((hi.x - lo.x) * 0.56).max(hi.y - lo.y) };
        let want_h = (13.5 + spread * 0.85).clamp(16.5, 30.0);
        if self.fresh {
            self.cam_center = center;
            self.cam_height = 18.5;
            self.fresh = false;
        }
        let k = 1.0 - (-3.0 * dt).exp();
        self.cam_center = self.cam_center.lerp(center, k);
        self.cam_height += (want_h - self.cam_height) * k;
        let c = self.cam_center + Vec3::new(0.0, 0.0, -1.5);
        self.camera = Camera { pos: c + Vec3::new(0.0, self.cam_height, self.cam_height * 0.55), target: c, fov_y: 42f32.to_radians(), near: 0.1 };
    }
}

/// The camera's rotation: a card with it faces the camera.
pub fn facing(cam: &Camera) -> Mat3 {
    let back = (cam.pos - cam.target).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
    let up = back.cross(right);
    Mat3::from_cols(right, up, back)
}

/// A Godot OmniLight3D as a Warp local light.
pub fn omni(pos: Vec3, l: crate::kits::Light) -> LocalLight {
    LocalLight {
        position: pos,
        direction: Vec3::NEG_Y,
        intensity_cd: linear3(l.color) * l.energy * LIGHT,
        range_m: l.range.clamp(0.2, 200.0),
        inner_cos: -0.999,
        outer_cos: -1.0,
        source_radius_m: 0.05,
        falloff: l.attenuation.max(0.05),
    }
}

/// The level's numbers for floor.gdshader (surface.wgsl FL_*), and the
/// bounds field after them. Returns where they start in `aux`.
fn floor_record(scene: &mut Scene, l: &Level) -> usize {
    let b = l.biome();
    if scene.aux.is_empty() {
        scene.aux.push(0);
    }
    let o = scene.aux.len();
    let a = &mut scene.aux;
    let v3 = |a: &mut Vec<u32>, v: Vec3| {
        for x in v.to_array() {
            push_f32(a, x);
        }
    };
    v3(a, linear3(b.floor_a));
    v3(a, linear3(b.floor_b));
    push_f32(a, b.tiles);
    push_f32(a, l.half.x);
    push_f32(a, l.half.y);
    push_f32(a, b.wet);
    push_f32(a, PATH_W);
    push_f32(a, b.path_style as f32);
    v3(a, linear3(b.path_a));
    v3(a, linear3(b.path_b));
    let len = l.length();
    push_f32(a, l.point_at(len / 3.0).y);
    push_f32(a, l.point_at(len * 2.0 / 3.0).y);
    for t in ZONE_TINTS {
        v3(a, Vec3::from(t));
    }
    let flags = (!l.isles.is_empty()) as u32 | (l.corridor as u32) << 1 | (l.ortho as u32) << 2 | (b.outside_lava as u32) << 3;
    a.push(flags);
    push_f32(a, l.sdf_o.x - 0.5);
    push_f32(a, l.sdf_o.y - 0.5);
    push_f32(a, l.sdf_w as f32);
    push_f32(a, l.sdf_h as f32);
    let sdf_at = a.len() + 1;
    a.push(sdf_at as u32);
    for v in &l.sdf {
        push_f32(a, v.clamp(-40.0, 40.0));
    }
    o
}

/// main.gd _world and apply_biome, in Warp's units.
fn lighting(w: &World, ambient: Vec3) -> Lighting {
    let b = w.level.biome();
    // The moon: rotation_degrees (-62, 28, 0), shining down its -z.
    let basis = Mat3::from_euler(glam::EulerRot::YXZ, 28f32.to_radians(), -62f32.to_radians(), 0.0);
    let sun_dir = basis * Vec3::Z;
    let ev = (1.0 / (1.2 * 1.5f32)).log2() + 0.43;
    // The global fog (density 0.004) and the biome's ground mist (a fog
    // volume of density `mist`, thick at the ankles, gone by the waist, in
    // patches) as one even medium: about the mist's optical depth along a
    // view ray from the camera's height.
    let mist = b.mist * MIST_DEPTH;
    let albedo = linear3([0.5, 0.55, 0.6]).lerp(linear3(b.mist_c), mist / (mist + 0.004));
    let emission = linear3(b.fog) * 0.15 + linear3(b.mist_c) * 0.05 * mist / (mist + 0.004);
    Lighting {
        sun_dir,
        sun_lux: (b.moon_energy * 1.8 + 0.15) * LIGHT,
        sun_color: linear3(b.moon),
        sky_lux: 0.0,
        ev100: ev,
        fog_density: 0.0,
        fog_start: 1000.0,
        fog_end: 2000.0,
        contrast: 1.08,
        saturation: 0.8,
        taa: 0.0,
        sharpen: 0.3,
        volumetric: Some(render::volumetric::Volumetric {
            density: 0.004 + mist,
            color: albedo,
            anisotropy: 0.4,
            intensity: 0.6,
            extinction: 1.0,
            ambient: albedo * ambient * 0.3 / (4.0 * PI) + emission,
            length: 80.0,
        }),
        ..Default::default()
    }
}

/// Pieces of a dead monster with their own little physics (debris.gd): they
/// fly, fall, bounce and tumble on the floor, settle, lie a while and sink.
struct Debris {
    parts: Vec<Bit>,
    life: f32,
    t: f32,
}

struct Bit {
    mesh: usize,
    pos: Vec3,
    rot: Quat,
    scale: Vec3,
    vel: Vec3,
    spin: Vec3,
    rest: f32,
    shadow: bool,
}

impl Debris {
    const GRAVITY: f32 = 20.0;

    fn new(scene: &Scene, parts: Vec<(usize, Mat4, bool)>, from: Vec3, power: f32, up: f32, life: f32, seed: u32) -> Debris {
        let mut r = seed.wrapping_mul(2654435761).max(1);
        let mut f = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            (r as f32) / (u32::MAX as f32)
        };
        let parts = parts
            .into_iter()
            .map(|(mesh, xf, shadow)| {
                let (scale, rot, pos) = xf.to_scale_rotation_translation();
                let mut away = pos - from;
                away.y = 0.0;
                if away.length() < 0.05 {
                    let a = f() * std::f32::consts::TAU;
                    away = Vec3::new(a.cos(), 0.0, a.sin());
                }
                let vel = away.normalize() * power * (0.5 + 0.7 * f()) + Vec3::Y * up * (0.6 + 0.6 * f());
                let spin = Vec3::new(f() * 2.0 - 1.0, f() * 2.0 - 1.0, f() * 2.0 - 1.0) * (4.0 + 8.0 * f());
                let radius = scene.meshes.get(mesh).map_or(0.1, |m| m.sphere[3]);
                let rest = (radius * scale.min_element() * 0.4).clamp(0.03, 0.3);
                Bit { mesh, pos, rot, scale, vel, spin, rest, shadow }
            })
            .collect();
        Debris { parts, life, t: 0.0 }
    }

    /// False once it has sunk away.
    fn step(&mut self, dt: f32) -> bool {
        self.t += dt;
        let sink = (self.t - self.life).max(0.0);
        for b in &mut self.parts {
            if sink > 0.0 {
                b.pos.y -= dt * 0.6;
                continue;
            }
            b.vel.y -= Self::GRAVITY * dt;
            let mut at = b.pos + b.vel * dt;
            if at.y < b.rest {
                at.y = b.rest;
                b.vel.y = if b.vel.y < -2.0 { -b.vel.y * 0.35 } else { 0.0 };
                // Rolling on the floor slows a piece and its tumbling down.
                b.vel.x *= 0.7;
                b.vel.z *= 0.7;
                b.spin *= 0.6;
            }
            b.pos = at;
            let w = b.spin.length();
            if w > 0.05 {
                b.rot = (Quat::from_axis_angle(b.spin / w, w * dt) * b.rot).normalize();
            }
        }
        sink <= 1.6
    }

    fn push(&self, out: &mut Vec<(usize, Affine3A, bool)>) {
        for b in &self.parts {
            out.push((b.mesh, Affine3A::from_scale_rotation_translation(b.scale, b.rot, b.pos), b.shadow));
        }
    }
}
