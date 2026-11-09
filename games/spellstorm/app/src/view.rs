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
use spellstorm::fx::Death;
use spellstorm::World;
use std::collections::HashMap;
use std::f32::consts::PI;

/// Instances kept for whatever moves (wizards, monsters, corpses, spells).
const POOL: usize = 4096;
/// The steel's length as exported (Godot: SWING_REACH 1.7 - SWORD_HAND 0.62
/// - Art.BLADE_AT 0.12).
const STEEL_LEN: f32 = 0.96;
const BLADE_AT: f32 = 0.12;

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
    /// Its kit's instances (start, count): they cast no shadow from it.
    pub owner: (u32, u32),
}

pub struct View {
    pub spell_effects:app::editor::SceneTuning,
    pub kits: Kits,
    look: Look,
    pub scene: Scene,
    pub light: Lighting,
    level_key: u64,
    pool_first: usize,
    /// Scene instance ranges of each prop (hidden once broken).
    prop_ranges: Vec<std::ops::Range<usize>>,
    prop_broken: Vec<bool>,
    /// Each prop cut into chunks (mesh, where, shadow), thrown when it breaks.
    prop_chunks: Vec<Vec<(usize, Mat4, bool)>>,
    pub lamps: Vec<Lamp>,
    /// Camera-facing cards in the level: instance, mesh, centre, scale.
    cards: Vec<(usize, usize, Vec3, Vec3)>,
    probes: Vec<Probe>,
    /// Wizard kit pieces per seat (their robes recoloured).
    wizard_pieces: HashMap<usize, Vec<Piece>>,
    wizard_anims: Vec<anim::WizardAnim>,
    enemy_anims: HashMap<hecs::Entity, anim::EnemyAnim>,
    /// The monsters' motion-captured clips, and each kit's rig.
    rig: Option<(body::Library, crate::rig::Clips)>,
    bounds: HashMap<String, Option<body::Bound>>,
    /// Monsters with real skeletons.
    skins: crate::skins::Skins,
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
    /// The fallen wizards as ragdolls.
    dolls: crate::ragdoll::Dolls,
    /// Fallen monsters as ragdolls (Joep: they collapse, not stand and sink).
    enemy_dolls: crate::ragdoll::Dolls,
    /// The level's even ambient light (lux), for what is lit game-side.
    ambient: Vec3,
    /// Each wizard's hurt_t last frame: a new hit spurts blood.
    wizard_hurt: Vec<f64>,
    /// Each wizard's blade length as drawn: it grows to the enchanted reach.
    blade_len: Vec<f32>,
    blade_sparks: f32,
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
            spell_effects:app::editor::SceneTuning::new("spellstorm","Spell VFX",&[
                app::editor::SceneControl{label:"Particle size",default:1.,min:0.25,max:3.,step:0.05},
                app::editor::SceneControl{label:"Spell light intensity",default:1.,min:0.,max:3.,step:0.05},
            ]),
            kits,
            look,
            scene,
            light: Lighting::default(),
            level_key: 0,
            pool_first: 0,
            prop_ranges: vec![],
            prop_broken: vec![],
            prop_chunks: vec![],
            lamps: vec![],
            cards: vec![],
            probes: vec![],
            wizard_pieces: HashMap::new(),
            wizard_anims: vec![],
            dolls: Default::default(),
            enemy_dolls: Default::default(),
            ambient: Vec3::ZERO,
            wizard_hurt: vec![],
            blade_len: vec![],
            blade_sparks: 0.0,
            enemy_anims: HashMap::new(),
            rig: match body::Library::builtin() {
                Ok(lib) => crate::rig::Clips::new(&lib).map(|c| (lib, c)),
                Err(e) => {
                    eprintln!("spellstorm: no monster animations: {e}");
                    None
                }
            },
            bounds: HashMap::new(),
            skins: crate::skins::Skins::load(),
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
        // Every lamp shadowed, every spell light lit (checked on an RTX 5070
        // Ti, 2026-11-09: chapel frame 5.94 -> 5.54 ms). KERN_CLUSTERED_LIGHTS=0
        // goes back to the 32 nearest lights.
        scene.graphics.clustered_lights = true;
        scene.sky = Some((4, 2, vec![[0.0, 0.0, 0.0, 1.0]; 8]));
        let mut look = Look::new(&self.kits, &mut scene)?;
        let l = &w.level;
        let biome = l.biome();
        self.lamps.clear();
        self.cards.clear();
        self.prop_ranges.clear();
        self.prop_broken.clear();
        self.prop_chunks.clear();
        self.debris.clear();
        self.wizard_pieces.clear();
        self.dolls = Default::default();
        self.enemy_dolls = Default::default();
        self.enemy_anims.clear();

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
            let mut chunks = Vec::new();
            if variants > 0 {
                let name = format!("prop/{}/{}/{}", biome.id, p.kind, p.seed as usize % variants);
                let at = Affine3A::from_rotation_translation(Quat::from_rotation_y(p.yaw), p.pos);
                self.place(&mut look, &mut scene, &name, at);
                // What it breaks into (prop.gd): each piece cut in chunks.
                if p.breakable {
                    if let Some(kit) = self.kits.kit(&name) {
                        let at = Mat4::from(at);
                        for (i, n) in kit.nodes.iter().enumerate() {
                            let Some(mesh) = n.mesh else { continue };
                            if !kit.shown(i) {
                                continue;
                            }
                            let xf = at * kit.model_xf(i);
                            for (m, mid) in look.chunks(&self.kits, &mut scene, mesh, n.material) {
                                chunks.push((m, xf * Mat4::from_translation(mid), n.shadow));
                            }
                        }
                    }
                }
            }
            self.prop_chunks.push(chunks);
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
            let mut pieces = look.wizard(&self.kits, &mut scene, i, &wizard_cloth([robe[0], robe[1], robe[2]]));
            // The staff's head casts no shadow: its prongs sit right by the
            // gem's light and threw big claws over the floor (Joep).
            if let Some(kit) = self.kits.kit("wizard") {
                let staff = kit.tree.node("staff");
                for p in &mut pieces {
                    if kit.tree.ancestors(p.node).any(|j| Some(j) == staff) {
                        p.shadow = false;
                    }
                }
            }
            self.wizard_pieces.insert(i, pieces);
        }
        for k in spellstorm::enemy::KINDS.iter() {
            let name = format!("enemy/{}", k.key);
            look.pieces(&self.kits, &mut scene, &name);
            // What it shatters into, cut now: meshes added during play never
            // reach the renderer.
            if let Some(kit) = self.kits.kit(&name) {
                for (i, n) in kit.nodes.iter().enumerate() {
                    if let (Some(mesh), true) = (n.mesh, kit.shown(i)) {
                        look.chunks(&self.kits, &mut scene, mesh, n.material);
                    }
                }
            }
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
        self.ambient = amb;
        let lo = Vec3::new(-l.half.x - 40.0, -2.0, -l.half.y - 20.0);
        let cell = 8.0;
        let dims = [((l.half.x * 2.0 + 80.0) / cell) as u32 + 2, 3, ((l.half.y * 2.0 + 40.0) / cell) as u32 + 2];
        let grid = Grid { origin: lo, cell, dims };
        scene.reserve_irradiance_volume(grid)?;
        self.probes = vec![Probe { axes: [amb; 6] }; grid.count()];

        self.light = lighting(w, amb);
        self.skins.reserve(&mut scene);
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
        let first = scene.instances.len() as u32;
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
                self.lamps.push(Lamp { pos, light: omni(pos, l), owner: (first, scene.instances.len() as u32 - first) });
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
        // Bodies that flash white from a hit or are bloodied: (out range, flash, wounds).
        let mut marked: Vec<(std::ops::Range<usize>, f32, f32)> = Vec::new();
        let time = w.time as f32;
        self.wizard_hurt.resize(w.wizards.len(), 0.0);
        self.blade_len.resize(w.wizards.len(), STEEL_LEN);
        self.wizard_anims.resize_with(w.wizards.len(), Default::default);
        // Where each staff's gem is as the staff moves: its light and orbs sit there.
        let mut gems = vec![None; w.wizards.len()];
        let doll_alpha = self.dolls.step(dt);
        for (i, &id) in w.wizards.iter().enumerate() {
            let a = w.a(id);
            let wz = w.wiz(id);
            let Some(pieces) = self.wizard_pieces.get(&i) else { continue };
            let Some(kit) = self.kits.kit("wizard") else { continue };
            let pose = self.wizard_anims[i].pose(kit, &a, &wz, dt);
            let at = Mat4::from_translation(a.pos);
            // A fallen wizard goes limp: a ragdoll thrown as it stood, with
            // its momentum and the killing blow (Joep).
            let fallen = self.dolls.dolls.get(i).is_some_and(Option::is_some);
            if !a.alive && !fallen {
                let xfs: Vec<Mat4> = pieces.iter().map(|p| at * anim::posed(kit, &pose, p.node)).collect();
                let force = 3.0 + (a.overkill as f32 * 0.08).min(4.0);
                let blow = if a.last_dir.length_squared() > 1e-4 { a.last_dir } else { a.hurt_dir };
                let node = |n: usize| at * anim::posed(kit, &pose, n);
                self.dolls.fall(i, kit, &node, pieces, &xfs, &self.scene.meshes, a.vel, blow, force);
            } else if a.alive && fallen {
                self.dolls.clear(i);
            }
            // The blade grows with the enchantment, as far as the swing then
            // reaches: what you see is what it hits (Wizard._size_sword).
            let enchanted = wz.enchant_time > 0.0;
            let reach = if enchanted { wz.enchant_reach() } else { spellstorm::wizard::swing_reach() };
            let want = (reach - 0.62 - BLADE_AT).max(0.2);
            let len = &mut self.blade_len[i];
            *len += (want - *len) * (1.0 - (-10.0 * dt).exp());
            let stretch = *len / STEEL_LEN;
            let gem = kit.tree.node("gem");
            let steel = kit.tree.node("steel");
            let from = out.len();
            let mut steel_xf = None;
            for (j, p) in pieces.iter().enumerate() {
                let xf = if a.alive { None } else { self.dolls.piece(i, j, doll_alpha) };
                let mut xf = xf.unwrap_or_else(|| at * anim::posed(kit, &pose, p.node));
                if a.alive && Some(p.node) == steel {
                    xf *= Mat4::from_scale(Vec3::new(1.0, stretch, 1.0));
                    steel_xf = Some(xf);
                }
                if Some(p.node) == gem { gems[i] = Some(xf.w_axis.truncate()); }
                out.push((p.mesh, look::affine(xf), p.shadow));
            }
            // The element runs off the enchanted blade along its whole
            // length: fire rises, water drips, the rest drifts.
            if let (true, Some(xf), Some(el)) = (enchanted, steel_xf, wz.enchant.as_ref().and_then(|e| e.primary)) {
                let n: u32 = wz.enchant.as_ref().map(|e| e.counts.iter().map(|c| c.1 as u32).sum()).unwrap_or(1);
                let c = el.color();
                let mut em = crate::fx::particles([c[0], c[1], c[2], 0.9], 1, 0.45, 0.07 + 0.012 * n as f32);
                em.gravity = match el {
                    spellstorm::El::Fire => Vec3::Y * 2.5,
                    spellstorm::El::Water | spellstorm::El::Poison => Vec3::Y * -6.0,
                    _ => Vec3::Y * 0.4,
                };
                em.v = (0.1, 0.5);
                em.spread = 180.0;
                let k = (dt * 60.0 * (0.6 + 0.35 * n as f32)).max(0.0);
                self.blade_sparks += k;
                while self.blade_sparks >= 1.0 {
                    self.blade_sparks -= 1.0;
                    let t = self.fx.f();
                    self.fx.emit(em, xf.transform_point3(Vec3::Y * t * STEEL_LEN), 1);
                }
            }
            // Low on health the robe gets bloodier; a new hit spurts blood.
            let life = if a.alive { (a.hp / a.max_hp.max(1e-6)) as f32 } else { 0.0 };
            let wounds = ((0.5 - life) / 0.4).clamp(0.0, 1.0);
            marked.push((from..out.len(), hit_flash(&a), wounds));
            if a.alive && a.hurt_t > self.wizard_hurt[i] + 1e-6 {
                self.fx.spurt(a.pos + Vec3::Y * 1.1, a.hurt_dir);
            }
            self.wizard_hurt[i] = a.hurt_t;
            if gems[i].is_none() {
                gems[i] = gem.map(|g| (at * anim::posed(kit, &pose, g)).w_axis.truncate());
            }
        }
        let mut seen = Vec::new();
        let enemy_alpha = self.enemy_dolls.step(dt);
        for &id in w.enemies.iter().chain(&w.corpses) {
            let Some(a) = w.actor(id) else { continue };
            let Ok(e) = w.ecs.get::<&spellstorm::enemy::Enemy>(id) else { continue };
            let name = format!("enemy/{}", e.cfg().key);
            let pieces = self.look.pieces(&self.kits, &mut self.scene, &name);
            let Some(kit) = self.kits.kit(&name) else { continue };
            if !self.bounds.contains_key(&name) {
                let b = self.rig.as_ref().and(crate::rig::rig(e.cfg().key)).map(|d| crate::rig::bind(d, kit, &self.kits));
                self.bounds.insert(name.clone(), b);
            }
            let bound = self.bounds.get(&name).and_then(|b| b.as_ref()).zip(self.rig.as_ref());
            let seed = id.id();
            let ea = self.enemy_anims.entry(id).or_default();
            let model = ea.model(kit.base_scale, &a, &e, time, seed, dt, bound.is_some());
            // Its bones under the motion-captured clips (rig.rs).
            let bones = bound.map(|(b, (lib, clips))| {
                let wind = if e.wind > 0.0 {
                    Some((clips.action(lib, e.cfg().attack), 1.0 - (e.wind / e.wind_time()) as f32))
                } else if e.boss_wind > 0.0 {
                    Some((clips.action(lib, spellstorm::enemy::Attack::Slam), 1.0 - (e.boss_wind / e.boss_wind_time.max(1e-3)) as f32))
                } else {
                    None
                };
                let doing = body::Doing {
                    speed: a.vel.with_y(0.0).length(),
                    wind,
                    hit: a.hurt_t > ea.hurt_seen + 1e-6,
                    dead: !a.alive,
                    held: a.frozen > 0.0 || a.stun > 0.0,
                };
                ea.hurt_seen = a.hurt_t;
                let layers = ea.body.step(&clips.set, b.hip() * kit.base_scale, &doing, dt);
                (b.pose(lib, &layers), b)
            });
            let part = |node: usize| match &bones {
                Some((m, b)) => m[b.bone_of[node] as usize] * kit.model_xf(node),
                None => kit.model_xf(node),
            };
            seen.push(id);
            // Dust kicked up over a burrower tunnelling (the mound's dust).
            if e.under && a.alive {
                ea.dust -= dt;
                if ea.dust <= 0.0 {
                    ea.dust = 0.12 - 0.06 * e.heave;
                    self.fx.puff(a.pos + Vec3::Y * 0.1, [0.35, 0.27, 0.18, 0.6], 0.18 + 0.2 * e.heave);
                }
            }
            let at = Mat4::from_rotation_translation(Quat::from_rotation_y(a.yaw), a.pos) * model;
            // Bodies that fall apart go to pieces once (a blow-up after its fuse).
            if let Some(t) = ea.dead_t.filter(|_| !ea.scattered) {
                let c = e.cfg();
                let big = if c.model.is_some() { c.size as f32 } else { 1.0 };
                // Frozen it stands until it shatters; a huge blow bursts it
                // at once: both into chunks of its body (remains.gd).
                let shatter = match e.how {
                    Some(Death::Freeze) if t >= 0.7 => Some((3.5, 3.5, 2.5)),
                    Some(Death::Gore) => Some((5.0 * big.sqrt(), 5.0, 2.5)),
                    _ => None,
                };
                let how = match anim::fall(c.model.unwrap_or(c.key), big) {
                    _ if matches!(e.how, Some(Death::Freeze | Death::Gore | Death::Shock | Death::Void | Death::Blown)) => None,
                    // A real skeleton plays its own death.
                    _ if self.skins.has(c) => None,
                    f @ anim::Fall::Scatter { .. } => Some(f),
                    anim::Fall::BlowUp if t >= e.fuse as f32 => Some(anim::BLOWN),
                    _ => None,
                };
                if let Some((power, up, life)) = shatter {
                    ea.scattered = true;
                    self.skins.shed(&mut self.scene, id);
                    let mut parts = Vec::new();
                    for p in &pieces {
                        let n = &kit.nodes[p.node];
                        let Some(mesh) = n.mesh else { continue };
                        let xf = at * part(p.node);
                        for (m, mid) in self.look.chunks(&self.kits, &mut self.scene, mesh, n.material) {
                            parts.push((m, xf * Mat4::from_translation(mid), p.shadow));
                        }
                    }
                    if !parts.is_empty() {
                        self.debris.push(Debris::new(&self.scene, parts, a.pos - a.last_dir.with_y(0.0) * 0.5, power, up, life, seed));
                    }
                } else if let Some(anim::Fall::Scatter { power, up, life }) = how {
                    ea.scattered = true;
                    let parts = pieces.iter().map(|p| (p.mesh, at * part(p.node), p.shadow)).collect();
                    self.debris.push(Debris::new(&self.scene, parts, a.pos, power, up, life, seed));
                }
            }
            // The rest go limp where they stood and fall the way the blow
            // threw them (Joep), with a flash where it struck.
            if let Some(t) = ea.dead_t.filter(|_| !ea.scattered && !ea.fell) {
                ea.fell = true;
                let c = e.cfg();
                let big = if c.model.is_some() { c.size as f32 } else { 1.0 };
                let limp = matches!(anim::fall(c.model.unwrap_or(c.key), big), anim::Fall::Topple | anim::Fall::Slump { .. })
                    && !matches!(e.how, Some(Death::Freeze | Death::Gore | Death::Void))
                    && !self.skins.has(c)
                    && t < 0.5;
                let slot = if limp { self.enemy_dolls.free(ENEMY_DOLLS) } else { None };
                if let Some(slot) = slot {
                    let xfs: Vec<Mat4> = pieces.iter().map(|p| at * part(p.node)).collect();
                    let joint = |bone: usize| bones.as_ref().and_then(|(m, b)| b.rest[bone].map(|r| (at * m[bone]).transform_point3(r)));
                    let (segs, parts) = limp_body(&xfs, &pieces, &self.scene.meshes, a.pos, &joint, bones.as_ref().map(|(_, b)| &b.bone_of[..]));
                    let blow = if a.last_dir.length_squared() > 1e-4 { a.last_dir } else { a.hurt_dir };
                    let force = death_force(&a, e.how) / big.max(0.6).sqrt();
                    if self.enemy_dolls.throw(slot, &segs, parts, &xfs, a.vel, blow, force) {
                        ea.doll = Some(slot);
                    }
                }
                death_hit(&mut self.fx, &a, big);
            }
            if let Some(slot) = ea.doll.filter(|_| !ea.scattered) {
                // Lies, then sinks away before it is gone.
                let sink = Mat4::from_translation(Vec3::NEG_Y * (1.0 - e.gone as f32).clamp(0.0, 1.0) * 1.6);
                let flash = hit_flash(&a);
                marked.push((out.len()..out.len() + pieces.len(), flash, 0.0));
                for (j, p) in pieces.iter().enumerate() {
                    let xf = self.enemy_dolls.piece(slot, j, enemy_alpha).unwrap_or_else(|| at * part(p.node));
                    out.push((p.mesh, look::affine(sink * xf), p.shadow));
                }
                continue;
            }
            if !ea.scattered {
                // Its eyes swell while it winds up a blow (Enemy._tell).
                let eyes = if e.wind > 0.0 { 1.0 + 1.5 * (1.0 - e.wind / e.wind_time()) as f32 } else { 1.0 };
                // Frozen to death it stands white as ice until it shatters.
                let flash = if e.how == Some(Death::Freeze) { 0.6 } else { hit_flash(&a) };
                let kind = e.cfg();
                if self.skins.pose(&mut self.scene, id, kind, &a, &e, ea.dead_t, at, kit.base_scale, flash, dt) {
                    continue;
                }
                marked.push((out.len()..out.len() + pieces.len(), flash, 0.0));
                for p in &pieces {
                    let mut xf = at * part(p.node);
                    if eyes != 1.0 && kit.eyes.contains(&p.node) {
                        xf *= Mat4::from_scale(Vec3::splat(eyes));
                    }
                    out.push((p.mesh, look::affine(xf), p.shadow));
                }
            }
        }
        for (id, ea) in &self.enemy_anims {
            if let Some(slot) = ea.doll.filter(|_| !seen.contains(id)) {
                self.enemy_dolls.clear(slot);
            }
        }
        self.enemy_anims.retain(|id, _| seen.contains(id));
        self.skins.release(&mut self.scene, &seen);
        self.debris.retain_mut(|d| d.step(dt));
        for d in &self.debris {
            d.push(&mut out);
        }
        self.shrines.update(w, &gems, time, &mut out);
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
        for (r, flash, wounds) in marked {
            for i in r.filter(|&i| i < POOL) {
                let inst = &mut self.scene.instances[first + i];
                inst.set_hit_flash(flash);
                inst.set_wounds(wounds);
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
                // Its chunks fly off from the blow, tumble and settle.
                let chunks = self.prop_chunks.get(i).cloned().unwrap_or_default();
                if !chunks.is_empty() {
                    let (from, power) = self.fx.prop_hits.remove(&i).unwrap_or((p.pos - Vec3::Y, 2.0));
                    let from = Vec3::new(from.x, p.pos.y + 0.3, from.z);
                    self.debris.push(Debris::new(&self.scene, chunks, from, 1.5 + power * 0.45, 3.0 + power * 0.25, 5.0, i as u32 * 7919 + 13));
                }
            }
        }
        renderer.update_instances(gpu, first, &self.scene.instances[first..first + POOL])?;
        if !self.skins.instances.is_empty() {
            let r = self.skins.instances.clone();
            renderer.update_instances(gpu, r.start, &self.scene.instances[r])?;
            self.skins.flush(gpu, renderer)?;
        }
        self.follow(w, dt);
        self.fx.update(w, dt);
        self.marks.update(w, &gems, self.fx.time, dt);
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
        for light in &mut locals {light.intensity_cd*=self.spell_effects.value(1) as f32;}
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
        if renderer.clustered_lights_enabled() {
            // Clustered lights (KERN_CLUSTERED_LIGHTS=0 turns them off): every staff, shrine
            // and spell light, and every lamp, the lamps shadowed (the
            // renderer shadows the ones nearest the cameras within budget).
            use render::clustered_lights::{ClusteredLight, LightShadow};
            let mut spells = self.fx.lights();
            for light in &mut spells { light.intensity_cd *= self.spell_effects.value(1) as f32; }
            let mut all: Vec<ClusteredLight> = self.marks.spill.iter().chain(&self.shrines.lights).chain(&spells)
                .map(|l| ClusteredLight::unshadowed(*l)).collect();
            all.extend(self.lamps.iter().enumerate().map(|(i, l)| {
                let k = 0.85 + 0.1 * (t * 9.0 + i as f32 * 1.7).sin() + 0.05 * (t * 23.0 + i as f32).sin();
                ClusteredLight { light: LocalLight { intensity_cd: l.light.intensity_cd * k, ..l.light },
                    shadow: Some(LightShadow { id: i as u64, exclude: Some(l.owner) }) }
            }));
            all.truncate(render::clustered_lights::MAX_CLUSTERED_LIGHTS);
            renderer.set_clustered_lights(&all)?;
            renderer.set_local_lights(&[])?;
        } else {
            renderer.set_local_lights(&locals)?;
        }
        // Blood is lit by these next frame, as the floor is (Fx::lit_at).
        self.fx.env_lights = locals.clone();
        self.fx.env_light = self.ambient + self.light.sun_color * self.light.sun_lux * self.light.sun_dir.y.max(0.0);
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
        // Round glows (the staff's gem) face the camera fully, as Fx's do: a
        // square that keeps world up flattens to an oval from the high
        // camera. Taller sprites (flames) stay standing.
        let face = facing(&self.camera);
        for sp in &mut sprites {
            if sp.facing == render::sprites::Facing::Billboard && sp.size[0] == sp.size[1] {
                sp.facing = render::sprites::Facing::Fixed { right: face.x_axis, up: face.y_axis };
            }
        }
        let size=self.spell_effects.value(0) as f32;
        sprites.extend(self.fx.sprites(&self.camera).into_iter().map(|mut s|{s.size=s.size.map(|v|v*size);s}));
        sprites.truncate(render::sprites::MAX_SPRITES);
        crate::effects::sync(gpu);
        crate::effects::tag(&mut sprites, self.fx.time);
        renderer.set_sprites(gpu, &sprites)?;
        let t = self.fx.time;
        self.fx.blasts.flush(gpu, renderer, t)?;
        self.fx.sparks.flush(gpu, renderer, t)?;
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
        // Shut in the boss's arena the party may spread over all of it: the
        // camera rises to keep everyone in view.
        let top = if w.run.sealed { 44.0 } else { 30.0 };
        let want_h = (13.5 + spread * 0.85).clamp(16.5, top);
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
/// The white a hit leaves on a body (Actor's overlay: 0.8 k², gone in a blink).
fn hit_flash(a: &spellstorm::actor::Actor) -> f32 {
    if a.hurt_t <= 0.0 || !a.alive {
        return 0.0;
    }
    let k = (a.hurt_t / spellstorm::actor::HURT_TIME) as f32;
    0.8 * k * k
}

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

/// How many fallen monsters lie as ragdolls at once; the rest fall their
/// own way.
const ENEMY_DOLLS: usize = 24;

/// A fallen monster's capsules and which one each piece rides. With bones:
/// body, head, the arms and the legs, from where its joints are now;
/// without: one capsule round the whole of it.
fn limp_body(xfs: &[Mat4], pieces: &[Piece], meshes: &[render::scene::MeshRef], base: Vec3, joint: &dyn Fn(usize) -> Option<Vec3>, bone_of: Option<&[u8]>) -> (Vec<crate::ragdoll::Segment>, Vec<usize>) {
    use crate::ragdoll::Segment;
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for (p, x) in pieces.iter().zip(xfs) {
        let Some(m) = meshes.get(p.mesh) else { continue };
        let c = x.transform_point3(Vec3::new(m.sphere[0], m.sphere[1], m.sphere[2]));
        lo = lo.min(c);
        hi = hi.max(c);
    }
    if !lo.is_finite() {
        lo = base;
        hi = base + Vec3::Y * 1.6;
    }
    let tall = (hi.y - base.y).max(0.5);
    let wide = (hi.x - lo.x).max(hi.z - lo.z).max(0.3);
    let whole = || {
        let r = (wide * 0.35).clamp(0.12, 0.9).min(tall * 0.45);
        let mid = Vec3::new((lo.x + hi.x) * 0.5, 0.0, (lo.z + hi.z) * 0.5);
        let seg = Segment { start: mid.with_y(base.y + 0.05), end: mid.with_y(base.y + tall), radius: r, parent: None, anchor: base };
        (vec![seg], vec![0; pieces.len()])
    };
    let Some(bone_of) = bone_of else { return whole() };
    // body::Bone order: pelvis, spine x3, neck, head, then each arm (clavicle,
    // upper, lower, hand) left and right, then each leg (thigh, calf, foot,
    // ball) left and right.
    let group = |b: usize| match b {
        4 | 5 => 1,
        7..=9 => 2,
        11..=13 => 3,
        14..=17 => 4,
        18..=21 => 5,
        _ => 0,
    };
    let (Some(pelvis), Some(chest)) = (joint(0), joint(3).or(joint(2)).or(joint(1))) else { return whole() };
    let s = (chest.distance(pelvis) / 0.45).clamp(0.4, 3.0);
    let mut segs = vec![Segment { start: pelvis - (chest - pelvis).normalize_or(Vec3::Y) * 0.08 * s, end: chest + (chest - pelvis).normalize_or(Vec3::Y) * 0.1 * s, radius: 0.16 * s, parent: None, anchor: pelvis }];
    let mut seg_of = [0usize; 6];
    let limbs: [(usize, usize, usize, f32); 5] = [(1, 4, 5, 0.12), (2, 7, 9, 0.06), (3, 11, 13, 0.06), (4, 14, 16, 0.08), (5, 18, 20, 0.08)];
    for (g, from, to, r) in limbs {
        let (Some(a), Some(b)) = (joint(from), joint(to)) else { continue };
        let end = if g == 1 { b + (b - a).normalize_or(Vec3::Y) * 0.15 * s } else { b };
        if end.distance(a) < 0.05 {
            continue;
        }
        seg_of[g] = segs.len();
        segs.push(Segment { start: a, end, radius: r * s, parent: Some(0), anchor: a });
    }
    let parts = pieces.iter().map(|p| seg_of[group(bone_of.get(p.node).copied().unwrap_or(0) as usize)]).collect();
    (segs, parts)
}

/// How hard the killing blow throws a monster: harder the more it overkilled,
/// hardest by wind or earth, softly by fire.
fn death_force(a: &spellstorm::actor::Actor, how: Option<Death>) -> f32 {
    use spellstorm::El;
    let base = 2.5 + (a.overkill as f32 * 0.06).min(5.0);
    let by = match a.last_el {
        Some(El::Air | El::Storm) => 1.8,
        Some(El::Earth | El::Mud | El::Magma) => 1.5,
        Some(El::Fire | El::Steam) => 0.7,
        Some(El::Water) => 1.2,
        None => 1.1,
        _ => 1.0,
    };
    base * by * if how == Some(Death::Blown) { 1.8 } else { 1.0 }
}

/// Where the killing blow struck: a flash and a spray in its element's
/// colour, blood for a plain blow.
fn death_hit(fx: &mut crate::fx::Fx, a: &spellstorm::actor::Actor, big: f32) {
    let at = a.pos + Vec3::Y * (0.9 * big.sqrt()) - a.last_dir.with_y(0.0).normalize_or_zero() * 0.3;
    match a.last_el {
        Some(el) => {
            let c = el.color();
            fx.flash(at, [c[0], c[1], c[2], 1.0], 1.4 * big.sqrt(), 0.25);
            fx.burst(at, [c[0], c[1], c[2], 0.95], 14, 4.5, 0.12, Some(el));
        }
        None => {
            fx.flash(at, [1.0, 0.9, 0.8, 1.0], 1.0 * big.sqrt(), 0.18);
            fx.spurt(at, a.last_dir);
        }
    }
}
