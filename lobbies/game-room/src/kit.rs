//! The props the live lobby moves around (view.gd's `_make_item`, shot
//! cores, particle quads, room.gd's fireballs and the laser beam), built
//! once into the room as hidden instances. The live view copies their
//! meshes into instance slots each frame, so the room renderer never has
//! to be rebuilt for a new item.

use crate::godot::{rgba, Color};
use crate::room::{mat, Mat, RoomBuild};
use game_room_sim::world::Kind;
use glam::{Affine3A, Mat3A, Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

/// Blaster glows and shot cores come in this many hues.
pub const HUES: usize = 12;
/// Fireballs of each kind that can burn at once.
pub const FIREBALLS: usize = 4;
/// Samples taken from each particle colour ramp.
const RAMP: usize = 5;

/// One mesh of an item: its dump instance, and whether it hangs under the
/// item's turning node (the saber's blade, the ball's spinner).
#[derive(Clone, Copy, Debug)]
pub struct Part { pub instance: usize, pub pivot: bool, pub local: Affine3A }

/// Particle colours per burst kind (view.gd's `burst` calls).
pub const PALETTES: [(&str, &[&str]); 8] = [
    ("fire", &["ffd27a", "ff7a3c", "fff3c8"]),
    ("smoke", &["4a4458", "2b2838"]),
    ("hit", &["ffffff", "ffe066"]),
    ("popcorn", &["fff1c4"]),
    ("puff", &["6a6880", "4a4458"]),
    ("slash", &["38d6ff", "f2f1f8"]),
    ("dust", &["8c80b0"]),
    ("spawn", &["7c5cff", "ffb454", "f2f1f8"]),
];

/// room.gd's two fireballs per explosion: colour and radius.
pub const FIREBALL_KINDS: [(&str, f32); 2] = [("ff6a2a", 1.4), ("ffd27a", 0.7)];

pub struct Kit {
    /// Item parts by `item_key`.
    pub items: HashMap<String, Vec<Part>>,
    /// Where the item's turning node sits.
    pub pivots: HashMap<String, Vec3>,
    /// A shot core by `shot_key`.
    pub shots: HashMap<String, usize>,
    /// Unit particle quads by palette name: one per ramp sample.
    pub sparks: HashMap<&'static str, Vec<usize>>,
    /// Per fireball kind: (instance, material) per pool member.
    pub fireballs: Vec<Vec<(usize, usize)>>,
    /// The laser beam, a unit-long box.
    pub beam: usize,
}

/// The parts for an item: blasters by hue, sabers by blade colour.
pub fn item_key(kind: Kind, id: u32) -> String {
    match kind {
        Kind::Blaster => format!("blaster{}", hue_index((id as f64 * 0.37).rem_euclid(1.0))),
        Kind::Saber => format!("saber{}", id % 4),
        k => format!("{k:?}").to_lowercase(),
    }
}

/// The core for a shot of `kind` with `hue`.
pub fn shot_key(kind: Kind, hue: f64) -> String {
    match kind {
        Kind::Minigun | Kind::Bazooka | Kind::Popcorn => format!("{kind:?}").to_lowercase(),
        _ => format!("hue{}", hue_index(hue)),
    }
}

fn hue_index(h: f64) -> usize { (h * HUES as f64).round() as usize % HUES }

/// The hue a key's variant stands for.
fn hue(i: usize) -> f32 { i as f32 / HUES as f32 }

pub const SABER_BLADES: [&str; 4] = ["38d6ff", "ff4f4f", "3ddc97", "7c5cff"];

/// A part's transform relative to its item (or its pivot).
fn local(build: &RoomBuild, part: usize) -> Affine3A {
    let (s, r, t) = Mat4::from_cols_array(&build.dump.instances[part].transform).to_scale_rotation_translation();
    Affine3A::from_scale_rotation_translation(s, r, t)
}

/// Linear particle colour as an sRGB albedo the dump decodes back to it
/// (Godot uses vertex colours as they are, without decoding).
fn raw(c: Vec3) -> Color {
    let e = |v: f32| if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    rgba(e(c.x), e(c.y), e(c.z), 1.0)
}

/// Godot's Gradient with evenly spaced offsets, sampled at `t`.
pub fn ramp(colors: &[&str], t: f32) -> Color {
    if colors.len() == 1 { return Color::hex(colors[0]); }
    let x = t.clamp(0.0, 1.0) * (colors.len() - 1) as f32;
    let i = (x.floor() as usize).min(colors.len() - 2);
    Color::hex(colors[i]).lerp(Color::hex(colors[i + 1]), x - i as f32)
}

fn m(c: &str, roughness: f32) -> Mat { mat(Color::hex(c), roughness, 0.0, Color::BLACK, 0.0) }
fn mm(c: &str, roughness: f32, metallic: f32) -> Mat { mat(Color::hex(c), roughness, metallic, Color::BLACK, 0.0) }
fn lit(c: Color, roughness: f32, energy: f32) -> Mat { mat(c, roughness, 0.0, c, energy) }

struct Item<'a> { r: &'a mut RoomBuild, parts: Vec<Part>, pivot: bool }

impl Item<'_> {
    fn keep(&mut self, instance: usize) -> usize {
        self.r.hidden.push(instance);
        self.parts.push(Part { instance, pivot: self.pivot, local: Affine3A::IDENTITY });
        instance
    }
    fn box_(&mut self, size: Vec3, pos: Vec3, material: &Mat) -> usize { let i = self.r.box_(size, pos, material); self.keep(i) }
    fn sphere(&mut self, radius: f32, pos: Vec3, material: &Mat) -> usize { let i = self.r.sphere(radius, pos, material); self.keep(i) }
    fn cylinder(&mut self, radius: f32, height: f32, pos: Vec3, material: &Mat) -> usize { let i = self.r.cylinder(radius, height, pos, material); self.keep(i) }
    /// A cylinder laid along x (`rotation.z = PI / 2`).
    fn barrel(&mut self, radius: f32, height: f32, pos: Vec3, material: &Mat) {
        let i = self.cylinder(radius, height, pos, material);
        self.r.rotate(i, Mat3A::from_rotation_z(PI / 2.0));
    }
}

/// Add every prop to `r`, hidden.
pub fn add(r: &mut RoomBuild) -> Kit {
    let mut kit = Kit { items: HashMap::new(), pivots: HashMap::new(), shots: HashMap::new(), sparks: HashMap::new(), fireballs: vec![], beam: 0 };
    let mut items = HashMap::new();
    let mut item = |r: &mut RoomBuild, key: String, make: &dyn Fn(&mut Item)| {
        r.group = format!("kit/{key}");
        let mut it = Item { r, parts: vec![], pivot: false };
        make(&mut it);
        it.r.group.clear();
        items.insert(key, it.parts);
    };
    item(r, "bomb".into(), &|it| {
        it.sphere(0.27, Vec3::new(0.0, 0.27, 0.0), &mm("1b1a26", 0.25, 0.6));
        it.sphere(0.07, Vec3::new(0.1, 0.38, 0.2), &mm("6a6880", 0.2, 0.3));
        it.cylinder(0.09, 0.12, Vec3::new(0.0, 0.56, 0.0), &mm("3a3848", 0.4, 0.7));
        it.cylinder(0.015, 0.14, Vec3::new(0.03, 0.68, 0.0), &m("c8a46a", 0.8));
        it.sphere(0.04, Vec3::new(0.05, 0.76, 0.0), &mat(Color::hex("ffd27a"), 0.2, 0.0, Color::hex("ffb454"), 6.0));
    });
    for h in 0..HUES {
        item(r, format!("blaster{h}"), &|it| {
            let glow = Color::from_hsv(hue(h), 0.75, 1.0);
            it.box_(Vec3::new(0.55, 0.18, 0.16), Vec3::new(0.12, 0.1, 0.0), &m("e9e6f5", 0.3));
            it.box_(Vec3::new(0.12, 0.25, 0.12), Vec3::new(-0.08, -0.08, 0.0), &m("2b2b3d", 0.4));
            it.barrel(0.07, 0.05, Vec3::new(0.42, 0.1, 0.0), &lit(glow, 0.3, 4.0));
            it.box_(Vec3::new(0.4, 0.04, 0.17), Vec3::new(0.12, 0.17, 0.0), &lit(glow, 0.3, 3.0));
        });
    }
    item(r, "minigun".into(), &|it| {
        let metal = mm("3a3848", 0.35, 0.8);
        it.box_(Vec3::new(0.4, 0.24, 0.22), Vec3::new(-0.02, 0.1, 0.0), &m("ffb454", 0.4));
        it.box_(Vec3::new(0.12, 0.22, 0.12), Vec3::new(-0.12, -0.1, 0.0), &m("2b2b3d", 0.4));
        for i in 0..6 {
            let a = i as f32 * TAU / 6.0;
            it.barrel(0.025, 0.55, Vec3::new(0.45, 0.1 + a.sin() * 0.06, a.cos() * 0.06), &metal);
        }
        it.barrel(0.1, 0.05, Vec3::new(0.68, 0.1, 0.0), &metal);
    });
    item(r, "bazooka".into(), &|it| {
        it.barrel(0.12, 1.0, Vec3::new(0.1, 0.18, 0.0), &m("5d6b3a", 0.6));
        it.barrel(0.15, 0.08, Vec3::new(-0.4, 0.18, 0.0), &m("2b2b3d", 0.4));
        it.box_(Vec3::new(0.1, 0.24, 0.1), Vec3::new(-0.05, -0.02, 0.0), &m("2b2b3d", 0.4));
        it.sphere(0.1, Vec3::new(0.62, 0.18, 0.0), &mat(Color::hex("ff4f4f"), 0.4, 0.0, Color::hex("ff4f4f"), 1.5));
    });
    item(r, "laser".into(), &|it| {
        let cyan = Color::hex("38d6ff");
        it.box_(Vec3::new(0.5, 0.14, 0.14), Vec3::new(0.1, 0.1, 0.0), &mm("f2f1f8", 0.2, 0.3));
        it.box_(Vec3::new(0.1, 0.22, 0.1), Vec3::new(-0.08, -0.06, 0.0), &m("2b2b3d", 0.4));
        it.barrel(0.05, 0.3, Vec3::new(0.42, 0.1, 0.0), &lit(cyan, 0.2, 5.0));
        for i in 0..3 { it.box_(Vec3::new(0.04, 0.17, 0.16), Vec3::new(-0.05 + i as f32 * 0.1, 0.1, 0.0), &lit(cyan, 0.2, 3.0)); }
    });
    for (k, blade) in SABER_BLADES.iter().enumerate() {
        item(r, format!("saber{k}"), &|it| {
            let blade = Color::hex(blade);
            it.pivot = true;
            it.cylinder(0.05, 0.26, Vec3::ZERO, &mm("c9c4d8", 0.25, 0.8));
            it.cylinder(0.035, 1.05, Vec3::new(0.0, 0.65, 0.0), &mat(blade.lightened(0.6), 0.2, 0.0, blade, 6.0));
        });
    }
    item(r, "popcorn".into(), &|it| {
        let (red, white) = (m("e2574c", 0.6), m("f2f1f8", 0.6));
        for i in 0..8 {
            let a = i as f32 * TAU / 8.0;
            let b = it.box_(Vec3::new(0.09, 0.38, 0.03), Vec3::new(a.sin() * 0.15, 0.19, a.cos() * 0.15), if i % 2 == 0 { &red } else { &white });
            it.r.rotate(b, Mat3A::from_rotation_y(a));
        }
        it.cylinder(0.15, 0.36, Vec3::new(0.0, 0.19, 0.0), &m("2b2b3d", 0.6));
        for i in 0..9 {
            let (col, row) = ((i % 3) as f32 - 1.0, (i / 3) as f32);
            it.sphere(0.07, Vec3::new(col * 0.09, 0.4 + row * 0.04, (row - 1.0) * 0.07), &m("fff1c4", 0.9));
        }
    });
    let stripes = ["ff4f8b", "f2f1f8", "38d6ff", "f2f1f8", "ffb454", "f2f1f8"];
    let img = image::RgbaImage::from_fn(64, 32, |x, _| {
        let c = Color::hex(stripes[(x as usize * 6 / 64) % 6]);
        let b = |v: f32| (v * 255.0) as u8;
        image::Rgba([b(c.r), b(c.g), b(c.b), 255])
    });
    let texture = r.image(img);
    item(r, "ball".into(), &|it| {
        let mut m = mat(Color::WHITE, 0.35, 0.0, Color::BLACK, 0.0);
        m.texture = texture;
        it.pivot = true;
        it.sphere(0.45, Vec3::ZERO, &m);
    });
    for parts in items.values_mut() { for p in parts { p.local = local(r, p.instance); } }
    kit.items = items;
    kit.pivots = (0..SABER_BLADES.len()).map(|k| (format!("saber{k}"), Vec3::ZERO)).chain([("ball".to_string(), Vec3::new(0.0, 0.45, 0.0))]).collect();

    // Shot cores: a white-hot sphere glowing in the shot's colour.
    r.group = "kit/shots".into();
    let mut core = |r: &mut RoomBuild, key: String, c: Color, energy: f32| {
        let i = r.sphere(0.09, Vec3::ZERO, &mat(Color::WHITE, 0.2, 0.0, c, energy));
        r.hidden.push(i);
        kit.shots.insert(key, i);
    };
    for h in 0..HUES { core(r, format!("hue{h}"), Color::from_hsv(hue(h), 0.8, 1.0), 8.0); }
    core(r, "minigun".into(), Color::hex("ffd27a"), 8.0);
    core(r, "bazooka".into(), Color::hex("ff7a3c"), 8.0);
    core(r, "popcorn".into(), Color::hex("fff1c4"), 0.4);

    // Particle quads, unshaded in the ramp's colours (times 2.2, view.gd).
    r.group = "kit/sparks".into();
    for (name, colors) in PALETTES {
        let samples = if colors.len() == 1 { 1 } else { RAMP };
        let quads = (0..samples).map(|k| {
            let c = ramp(colors, k as f32 / (samples - 1).max(1) as f32);
            let mut m = mat(raw(Vec3::new(c.r, c.g, c.b) * 2.2), 1.0, 0.0, Color::BLACK, 0.0);
            m.unshaded = true;
            m.cull_disabled = true;
            let i = r.quad(Vec2::ONE, Vec3::ZERO, &m);
            r.hidden.push(i);
            i
        }).collect();
        kit.sparks.insert(name, quads);
    }

    // Fireballs: each its own material, so each fades on its own.
    r.group = "kit/fireballs".into();
    for (c, radius) in FIREBALL_KINDS {
        let c = Color::hex(c);
        kit.fireballs.push((0..FIREBALLS).map(|_| {
            let mut m = mat(rgba(c.r * 1.8, c.g * 1.8, c.b * 1.8, 0.75), 0.7, 0.0, Color::BLACK, 0.0);
            m.unshaded = true;
            m.transparent = true;
            let i = r.sphere(radius, Vec3::ZERO, &m);
            r.hidden.push(i);
            (i, r.material_of(i))
        }).collect());
    }

    r.group = "kit/beam".into();
    let cyan = Color::hex("38d6ff");
    kit.beam = r.box_flat(Vec3::new(1.0, 0.12, 0.12), Vec3::ZERO, &mat(cyan.lightened(0.5), 0.2, 0.0, cyan, 9.0));
    r.hidden.push(kit.beam);
    r.group.clear();
    kit
}
