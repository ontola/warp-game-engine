//! The Game Room built in Rust: a line-by-line port of the Godot lobby's
//! room.gd construction (shell, window, logo, TV, game shelf, up next,
//! shelves, couch, jukebox, exit door, plaque, lights, foreground, string
//! lights). It produces the same scene description the Godot exporter
//! writes (tools/warp_export.gd), so `dump::Room` turns either into a Warp
//! scene, and a test holds the two to the same instances, materials and
//! lights.
//!
//! Things that move or change at run time are named: `group` prefixes
//! (`mover0/`, `disco/`, `pad1/`, `featured/`, ...), screens (`tv`,
//! `queue0`, `prompt_shelf`, ...) and the neon and LED strip materials.

use crate::dump::{Dump, InstanceDump, MeshDump, SurfaceDump, TextureDump};
use crate::godot::{self, euler, looking_at, rgb, rgba, xf, Arrays, Color};
use gdcompat::Pcg;
use glam::{Affine3A, Mat3A, Vec2, Vec3};
use game_room_sim::layout as l;

const BACK: f32 = l::BACK as f32;
fn v3(a: [f64; 3]) -> Vec3 { Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32) }
use serde_json::{json, Value};
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

pub const AMBER: Color = rgb(1.0, 0.705_882_4, 0.329_411_77);
pub const VIOLET: Color = rgb(0.486_274_5, 0.360_784_32, 1.0);
pub const NIGHT_LILAC: Color = rgb(0.654_902, 0.545_098_07, 0.980_392_16);
pub const GREEN: Color = rgb(0.239_215_69, 0.862_745_1, 0.592_156_9);
pub const PINK: Color = rgb(1.0, 0.309_803_93, 0.545_098_07);
pub const CYAN: Color = rgb(0.219_607_84, 0.839_215_7, 1.0);
pub const BG: Color = rgb(0.043_137_256, 0.043_137_256, 0.070_588_24);
pub const TEXT: Color = rgb(0.949_019_6, 0.945_098_04, 0.972_549);

pub const CAMERA_BASE: Vec3 = Vec3::new(0.0, 4.7, 20.4);
pub const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 4.55, 0.0);
pub const NEXT_SIZE: Vec2 = Vec2::new(2.7, 1.77);
pub const NEXT_POSITION: Vec3 = Vec3::new(-4.95, 2.38, (l::BACK + 0.92) as f32);
pub const TITLES_POSITION: Vec3 = Vec3::new(-4.95, 0.8, (l::BACK + 0.96) as f32);
pub const QUEUE_TITLE_ROWS: usize = 4;

/// A Godot StandardMaterial3D, as the exporter describes it.
#[derive(Clone, Debug)]
pub struct Mat {
    pub albedo: Color,
    pub texture: i64,
    pub roughness: f32,
    pub metallic: f32,
    pub emission: Option<(Color, f32)>,
    pub emission_texture: i64,
    pub unshaded: bool,
    pub transparent: bool,
    pub cull_disabled: bool,
    /// 0 nearest, 1 linear, 3 linear with mipmaps.
    pub filter: i64,
    pub uv_scale: [f32; 2],
    pub disable_fog: bool,
}

pub fn mat(color: Color, roughness: f32, metallic: f32, emission: Color, energy: f32) -> Mat {
    Mat {
        albedo: color,
        texture: -1,
        roughness,
        metallic,
        emission: (energy > 0.0).then_some((emission, energy)),
        emission_texture: -1,
        unshaded: false,
        transparent: false,
        cull_disabled: false,
        filter: 3,
        uv_scale: [1.0, 1.0],
        disable_fog: false,
    }
}

fn m(color: Color, roughness: f32) -> Mat {
    mat(color, roughness, 0.0, Color::BLACK, 0.0)
}

fn mm(color: Color, roughness: f32, metallic: f32) -> Mat {
    mat(color, roughness, metallic, Color::BLACK, 0.0)
}

fn glow(color: Color, roughness: f32, emission: Color, energy: f32) -> Mat {
    mat(color, roughness, 0.0, emission, energy)
}

impl Mat {
    fn value(&self) -> Value {
        let (emission, energy) = self.emission.unwrap_or((Color::BLACK, 1.0));
        json!({
            "type": "StandardMaterial3D",
            "albedo": self.albedo.array(),
            "albedo_texture": self.texture,
            "roughness": self.roughness,
            "metallic": self.metallic,
            "emission_enabled": self.emission.is_some(),
            "emission": emission.array(),
            "emission_energy": energy,
            "emission_texture": if self.emission.is_some() { self.emission_texture } else { -1 },
            "unshaded": self.unshaded,
            "transparency": if self.transparent { 1 } else { 0 },
            "cull": if self.cull_disabled { 2 } else { 0 },
            "filter": self.filter,
            "uv1_scale": [self.uv_scale[0], self.uv_scale[1], 1.0],
            "uv1_offset": [0.0, 0.0, 0.0],
            "vertex_color": false,
            "disable_fog": self.disable_fog,
        })
    }
}

/// A screen: a texture the lobby paints at run time (TV, up next, prompts).
#[derive(Clone, Debug)]
pub struct ScreenDef {
    pub name: String,
    pub size: [u32; 2],
    pub texture: usize,
    pub transparent: bool,
}

/// A neon tube or glowing part whose energy pulses (room.gd's `_neon`).
#[derive(Clone, Debug)]
pub struct Neon {
    pub material: usize,
    pub base: f64,
    pub phase: f32,
    pub speed: f32,
    pub flicker: bool,
}

/// A floating row of controller buttons over a station.
#[derive(Clone, Debug)]
pub struct PromptDef {
    pub name: String,
    pub items: Vec<(String, String)>,
    pub pos: Vec3,
    pub instance: usize,
    pub material: usize,
}

#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Box(Vec3),
    Cylinder { top: f32, bottom: f32, height: f32, radial: i32, rings: i32 },
    Sphere { radius: f32, height: f32, radial: i32, rings: i32 },
    Plane(Vec2),
    Quad(Vec2),
}

impl Shape {
    fn key(&self) -> String {
        format!("{self:?}")
    }

    fn type_name(&self) -> &'static str {
        match self {
            Shape::Box(_) => "BoxMesh",
            Shape::Cylinder { .. } => "CylinderMesh",
            Shape::Sphere { .. } => "SphereMesh",
            Shape::Plane(_) => "PlaneMesh",
            Shape::Quad(_) => "QuadMesh",
        }
    }

    pub fn arrays(&self) -> Arrays {
        match *self {
            Shape::Box(size) => godot::box_mesh(size),
            Shape::Cylinder { top, bottom, height, radial, rings } => godot::cylinder_mesh(top, bottom, height, radial, rings),
            Shape::Sphere { radius, height, radial, rings } => godot::sphere_mesh(radius, height, radial, rings),
            Shape::Plane(s) => godot::plane_mesh(s.x, s.y, false),
            Shape::Quad(s) => godot::plane_mesh(s.x, s.y, true),
        }
    }
}

fn cyl(radius: f32, height: f32) -> Shape {
    Shape::Cylinder { top: radius, bottom: radius, height, radial: 64, rings: 4 }
}

fn sph(radius: f32) -> Shape {
    Shape::Sphere { radius, height: radius * 2.0, radial: 64, rings: 32 }
}

/// Everything room.gd adds, in its order.
pub struct RoomBuild {
    pub dump: Dump,
    pub bin: Vec<u8>,
    pub images: Vec<image::RgbaImage>,
    pub screens: Vec<ScreenDef>,
    pub neon: Vec<Neon>,
    pub prompts: Vec<PromptDef>,
    /// Instances that start hidden (prompts, the featured box, bumpers).
    pub hidden: Vec<usize>,
    /// Logo pads: group name, position, yaw, body material.
    pub pads: Vec<(String, Vec3, f32, usize)>,
    /// Instance shapes, for run-time copies (spines, items).
    pub shapes: Vec<Shape>,
    /// Each pad's pose as last posed.
    pad_pose: Vec<Affine3A>,
    meshes: HashMap<String, usize>,
    group: String,
    rng: Pcg,
}

impl RoomBuild {
    fn new() -> RoomBuild {
        RoomBuild {
            dump: Dump {
                meshes: vec![],
                materials: vec![],
                textures: vec![],
                instances: vec![],
                lights: vec![],
                labels: vec![],
                sprites: vec![],
                particles: vec![],
                camera: Value::Null,
                environment: Value::Null,
            },
            bin: vec![],
            images: vec![],
            screens: vec![],
            neon: vec![],
            prompts: vec![],
            hidden: vec![],
            pads: vec![],
            shapes: vec![],
            pad_pose: vec![],
            meshes: HashMap::new(),
            group: String::new(),
            rng: Pcg::new(1),
        }
    }

    fn floats(&mut self, v: impl IntoIterator<Item = f32>) -> [usize; 2] {
        let at = self.bin.len();
        for f in v {
            self.bin.extend_from_slice(&f.to_le_bytes());
        }
        [at, self.bin.len() - at]
    }

    fn mesh(&mut self, shape: Shape) -> usize {
        let key = shape.key();
        if let Some(&i) = self.meshes.get(&key) {
            return i;
        }
        let a = shape.arrays();
        let positions = self.floats(a.positions.iter().flatten().copied());
        let normals = self.floats(a.normals.iter().flatten().copied());
        let uvs = self.floats(a.uvs.iter().flatten().copied());
        // Counter-clockwise, as the exporter writes them.
        let at = self.bin.len();
        for t in a.indices.chunks_exact(3) {
            for i in [t[0], t[2], t[1]] {
                self.bin.extend_from_slice(&i.to_le_bytes());
            }
        }
        let indices = [at, self.bin.len() - at];
        self.dump.meshes.push(MeshDump {
            kind: shape.type_name().into(),
            surfaces: vec![SurfaceDump { positions, normals, uvs, indices, material: -1 }],
        });
        self.meshes.insert(key, self.dump.meshes.len() - 1);
        self.dump.meshes.len() - 1
    }

    pub fn add_material(&mut self, material: &Mat) -> usize {
        self.dump.materials.push(material.value());
        self.dump.materials.len() - 1
    }

    fn image(&mut self, img: image::RgbaImage) -> i64 {
        let (w, h) = img.dimensions();
        self.dump.textures.push(TextureDump { file: String::new(), width: w, height: h, kind: "image".into(), screen: None });
        self.images.push(img);
        self.dump.textures.len() as i64 - 1
    }

    /// A texture the lobby paints (Godot's SubViewport).
    fn screen(&mut self, name: &str, size: [u32; 2], transparent: bool) -> i64 {
        self.dump.textures.push(TextureDump { file: String::new(), width: size[0], height: size[1], kind: "viewport".into(), screen: Some(name.into()) });
        self.images.push(image::RgbaImage::new(1, 1));
        let texture = self.dump.textures.len() - 1;
        self.screens.push(ScreenDef { name: name.into(), size, texture, transparent });
        texture as i64
    }

    /// Adds a mesh instance; returns its index.
    fn node(&mut self, name: &str, shape: Shape, material: usize, xf: Affine3A, shadow: bool) -> usize {
        let mesh = self.mesh(shape);
        let name = if self.group.is_empty() { name.to_string() } else { format!("{}/{name}", self.group) };
        self.dump.instances.push(InstanceDump {
            name,
            mesh,
            materials: vec![material as i64],
            transform: glam::Mat4::from(xf).to_cols_array(),
            shadow,
            shadow_only: false,
        });
        self.shapes.push(shape);
        self.dump.instances.len() - 1
    }

    fn put(&mut self, shape: Shape, material: &Mat, at: Affine3A, shadow: bool) -> usize {
        let id = self.add_material(material);
        self.node(shape.type_name(), shape, id, at, shadow)
    }

    fn at(parent: Affine3A, pos: Vec3) -> Affine3A {
        parent * Affine3A::from_translation(pos)
    }

    fn box_(&mut self, size: Vec3, pos: Vec3, material: &Mat) -> usize {
        self.put(Shape::Box(size), material, Affine3A::from_translation(pos), true)
    }

    fn box_flat(&mut self, size: Vec3, pos: Vec3, material: &Mat) -> usize {
        self.put(Shape::Box(size), material, Affine3A::from_translation(pos), false)
    }

    fn cylinder(&mut self, radius: f32, height: f32, pos: Vec3, material: &Mat) -> usize {
        self.put(cyl(radius, height), material, Affine3A::from_translation(pos), true)
    }

    fn sphere(&mut self, radius: f32, pos: Vec3, material: &Mat) -> usize {
        self.put(sph(radius), material, Affine3A::from_translation(pos), true)
    }

    fn quad(&mut self, size: Vec2, pos: Vec3, material: &Mat) -> usize {
        self.put(Shape::Quad(size), material, Affine3A::from_translation(pos), false)
    }

    fn rotate(&mut self, instance: usize, basis: Mat3A) {
        let t = &mut self.dump.instances[instance].transform;
        let x = glam::Mat4::from_cols_array(t);
        let pos = x.w_axis.truncate();
        *t = glam::Mat4::from(xf(basis, pos)).to_cols_array();
    }

    fn material_of(&self, instance: usize) -> usize {
        self.dump.instances[instance].materials[0] as usize
    }

    /// A glowing tube between two points on a plane facing the camera.
    fn neon_line(&mut self, a: Vec3, b: Vec3, color: Color, energy: f32, thickness: f32, flicker: f32) -> usize {
        let length = a.distance(b);
        let node = self.box_flat(Vec3::new(length + thickness, thickness, thickness), (a + b) / 2.0, &glow(color, 0.3, color, energy));
        self.rotate(node, euler(0.0, 0.0, (b.y - a.y).atan2(b.x - a.x)));
        let phase = self.rng.randf() * TAU;
        self.neon.push(Neon { material: self.material_of(node), base: energy as f64, phase, speed: flicker, flicker: false });
        node
    }

    pub fn label(&mut self, name: &str, text: &str, at: Affine3A, size: i32, color: Color, outline: i32) -> usize {
        self.dump.labels.push(json!({
            "name": name, "text": text, "transform": glam::Mat4::from(at).to_cols_array(),
            "weight": if size >= 32 { 800 } else { 600 }, "font_size": size, "pixel_size": 0.01,
            "modulate": color.array(), "outline_size": outline, "outline_modulate": [0.0, 0.0, 0.0, 0.8],
            "billboard": 0, "shaded": false,
        }));
        self.dump.labels.len() - 1
    }

    fn omni(&mut self, name: &str, color: Color, energy: f32, range: f32, pos: Vec3, fog: f32, shadow: bool) {
        self.dump.lights.push(json!({
            "name": name, "type": "OmniLight3D", "transform": glam::Mat4::from(Affine3A::from_translation(pos)).to_cols_array(),
            "color": color.array(), "energy": energy, "fog": fog, "size": 0.0, "shadow": shadow, "negative": false,
            "range": range, "attenuation": 1.0,
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn spot(&mut self, name: &str, color: Color, energy: f32, range: f32, angle: f32, at: Affine3A, fog: f32, shadow: bool, attenuation: f32, angle_attenuation: f32) {
        self.dump.lights.push(json!({
            "name": name, "type": "SpotLight3D", "transform": glam::Mat4::from(at).to_cols_array(),
            "color": color.array(), "energy": energy, "fog": fog, "size": 0.0, "shadow": shadow, "negative": false,
            "range": range, "attenuation": attenuation, "angle": angle, "angle_attenuation": angle_attenuation,
        }));
    }

    fn screen_material(texture: i64, energy: f32) -> Mat {
        Mat { albedo: rgb(energy, energy, energy), texture, unshaded: true, filter: 1, ..m(Color::WHITE, 1.0) }
    }

    fn platform_edge(&mut self, x0: f32, x1: f32, top: f32, z_front: f32) {
        let lip = glow(Color::hex("e2bd88"), 0.45, Color::hex("ffd9a0"), 0.25);
        self.box_flat(Vec3::new(x1 - x0, 0.045, 0.06), Vec3::new((x0 + x1) / 2.0, top - 0.0225, z_front), &lip);
    }

    fn prompt(&mut self, name: &str, items: &[(&str, &str)], pos: Vec3) {
        let items: Vec<(String, String)> = items.iter().map(|(k, t)| (k.to_string(), t.to_string())).collect();
        let px = prompt_size(&items);
        let texture = self.screen(&format!("prompt_{name}"), px, true);
        let mut material = Self::screen_material(texture, 1.0);
        material.transparent = true;
        material.disable_fog = true;
        let node = self.quad(Vec2::new(px[0] as f32, px[1] as f32) * 0.0042, pos, &material);
        self.dump.instances[node].name = format!("prompt_{name}");
        self.hidden.push(node);
        self.prompts.push(PromptDef { name: name.into(), items, pos, instance: node, material: self.material_of(node) });
    }
}

/// Width of a prompt texture: the pills of its buttons and labels.
pub fn prompt_size(items: &[(String, String)]) -> [u32; 2] {
    let mut width = 40.0;
    for (key, text) in items {
        width += if key == "LB" || key == "RB" { 150.0 } else { 96.0 } + crate::fonts::measure_bold(text, 44.0) + 36.0;
    }
    [(width as f32).ceil() as u32, 96]
}

/// Builds the room as room.gd does in `_ready`.
pub fn build() -> RoomBuild {
    let mut r = RoomBuild::new();
    r.dump.environment = environment();
    r.dump.camera = camera_value(CAMERA_BASE, CAMERA_TARGET);
    shell(&mut r);
    window(&mut r);
    logo(&mut r);
    tv(&mut r);
    game_shelf(&mut r);
    up_next(&mut r);
    shelves(&mut r);
    couch_corner(&mut r);
    jukebox(&mut r);
    exit_door(&mut r);
    r.prompt("music", &[("LB", "Previous"), ("Y", "Pause"), ("RB", "Next")], Vec3::new(l::JUKEBOX as f32, 3.35, BACK + 1.2));
    r.prompt("exit", &[("Y", "Leave")], Vec3::new(l::EXIT_DOOR as f32, 3.0, BACK + 0.4));
    for (i, x) in l::PROFILE_DOORS.iter().map(|&x| x as f32).enumerate() {
        r.prompt(&format!("door{i}"), &[("Y", "Pick up")], Vec3::new(x, 3.15, BACK + 0.4));
    }
    plaque(&mut r);
    ceiling_lights(&mut r);
    foreground(&mut r);
    string_lights(&mut r);
    r
}

pub fn camera_value(pos: Vec3, target: Vec3) -> Value {
    let basis = looking_at(target - pos, Vec3::Y);
    json!({"transform": glam::Mat4::from(xf(basis, pos)).to_cols_array(), "fov_y": 36.0, "near": 0.05, "far": 4000.0})
}

fn environment() -> Value {
    let c = |s: &str| Color::hex(s).array();
    json!({
        "background_color": c("06060b"),
        "ambient_light_color": c("3b3170"),
        "ambient_light_energy": 1.3,
        "sdfgi_energy": 1.6,
        "tonemap_exposure": 1.05,
        "glow_intensity": 0.7,
        "glow_strength": 1.05,
        "glow_hdr_threshold": 0.9,
        "volumetric_fog_enabled": true,
        "volumetric_fog_density": 0.04,
        "volumetric_fog_albedo": c("b8a8ff"),
        "volumetric_fog_emission": c("0a0718"),
        "volumetric_fog_emission_energy": 0.15,
        "volumetric_fog_length": 40.0,
        "volumetric_fog_anisotropy": 0.5,
        "volumetric_fog_gi_inject": 1.0,
        "adjustment_saturation": 1.12,
        "adjustment_contrast": 1.06,
    })
}

// ------------------------------------------------------------ textures

fn plank_texture() -> image::RgbaImage {
    let mut img = image::RgbaImage::new(512, 512);
    let mut rng = Pcg::new(3);
    let rows = 8;
    let (a, b, seam) = (Color::hex("5a3826"), Color::hex("7a4e32"), Color::hex("24150e"));
    for row in 0..rows {
        let y0 = row * 512 / rows;
        let offset = rng.randi_range(0, 400);
        let seams = [offset % 512, (offset + 256) % 512];
        for x in 0..512i32 {
            let plank = if (x - seams[0]).abs() < (x - seams[1]).abs() { 0 } else { 1 };
            let base = a.lerp(b, (rng.randf() as f64 * 0.15 + plank as f64 * 0.25) as f32);
            for y in y0..y0 + 512 / rows {
                let (xf, yf, rf) = (x as f64, y as f64, row as f64);
                let grain = (xf * 0.07 + yf * 0.9 + rf).sin() * 0.03 + (xf * 0.013 + rf * 3.0).sin() * 0.04;
                let mut c = if grain > 0.0 { base.lightened(grain as f32) } else { base.darkened(-grain as f32) };
                if y == y0 || seams.contains(&x) {
                    c = seam;
                }
                godot::set_pixel(&mut img, x as i64, y as i64, c);
            }
        }
    }
    img
}

fn wallpaper_texture() -> image::RgbaImage {
    let mut img = image::RgbaImage::new(256, 256);
    let (a, b, d) = (Color::hex("2b2552"), Color::hex("251f49"), Color::hex("372e66"));
    for y in 0..256i64 {
        for x in 0..256i64 {
            let stripe = (x / 32) % 2 == 0;
            let mut c = if stripe { a } else { b };
            // Small diamond pattern, like a retro arcade carpet on the wall.
            let dx = ((x % 32) - 16).abs();
            let dy = ((y % 32) - 16).abs();
            if dx + dy == 9 {
                c = d;
            }
            godot::set_pixel(&mut img, x, y, c);
        }
    }
    img
}

fn sky_texture() -> image::RgbaImage {
    let mut sky = image::RgbaImage::new(110, 150);
    let mut rng = Pcg::new(11);
    for y in 0..150 {
        let c = Color::hex("1a1240").lerp(Color::hex("4a2a6e"), (y as f64 / 150.0) as f32);
        for x in 0..110 {
            godot::set_pixel(&mut sky, x, y, c);
        }
    }
    for _ in 0..40 {
        let x = rng.randi_range(0, 109);
        let y = rng.randi_range(0, 70);
        godot::set_pixel(&mut sky, x as i64, y as i64, rgba(1.0, 1.0, 1.0, 1.0));
    }
    for y in 18..34 {
        for x in 68..84 {
            let (dx, dy) = ((x - 76) as f32, (y - 26) as f32);
            if ((dx * dx + dy * dy).sqrt() as f64) < 7.5 {
                godot::set_pixel(&mut sky, x, y, Color::hex("fff3c8"));
            }
        }
    }
    let mut x = 0;
    while x < 110 {
        let w = rng.randi_range(8, 18);
        let h = rng.randi_range(40, 95);
        let tone = Color::hex("0d0a1f").lerp(Color::hex("1d1638"), rng.randf());
        for bx in x..(110).min(x + w) {
            for by in 150 - h..150 {
                godot::set_pixel(&mut sky, bx as i64, by as i64, tone);
            }
        }
        let mut wy = 150 - h + 4;
        while wy < 148 {
            let mut wx = x + 2;
            while wx < (110).min(x + w) - 1 {
                if rng.randf() < 0.35 {
                    let c = if rng.randf() < 0.7 { Color::hex("ffd27a") } else { Color::hex("8cf2ff") };
                    godot::set_pixel(&mut sky, wx as i64, wy as i64, c);
                }
                wx += 4;
            }
            wy += 6;
        }
        x += w + rng.randi_range(0, 3);
    }
    sky
}

// ------------------------------------------------------------ room shell

fn shell(r: &mut RoomBuild) {
    let plank = plank_texture();
    let planks = r.image(plank);
    let floor = Mat { texture: planks, uv_scale: [5.0, 2.4], ..m(Color::WHITE, 0.32) };
    r.put(Shape::Plane(Vec2::new(26.0, 9.6)), &floor, Affine3A::from_translation(Vec3::new(0.0, 0.0, BACK + 4.8)), true);
    // Back wall with wallpaper and wainscot.
    let paper = wallpaper_texture();
    let paper = r.image(paper);
    let wall = Mat { texture: paper, uv_scale: [12.0, 5.4], ..m(Color::WHITE, 0.9) };
    r.quad(Vec2::new(26.0, 11.2), Vec3::new(0.0, 5.6, BACK), &wall);
    r.box_(Vec3::new(26.0, 1.2, 0.08), Vec3::new(0.0, 0.6, BACK + 0.04), &m(Color::hex("2c2140"), 0.6));
    r.box_(Vec3::new(26.0, 0.06, 0.12), Vec3::new(0.0, 1.22, BACK + 0.06), &m(Color::hex("392a52"), 0.5));
    // Side walls, slightly warmer, so the perspective reads.
    let side = Mat { texture: paper, uv_scale: [5.0, 5.4], ..m(Color::hex("1a1530"), 0.85) };
    for s in [-1.0f32, 1.0] {
        let wall = r.quad(Vec2::new(10.0, 11.2), Vec3::new(s * 12.0, 5.6, BACK + 5.0), &side);
        r.rotate(wall, euler(0.0, -s * PI / 2.0, 0.0));
    }
    // Ceiling with beams.
    let ceiling = r.quad(Vec2::new(26.0, 10.0), Vec3::new(0.0, 10.9, BACK + 5.0), &m(Color::hex("0f0c1e"), 0.9));
    r.rotate(ceiling, euler(PI / 2.0, 0.0, 0.0));
    for i in 0..6 {
        r.box_(Vec3::new(26.0, 0.35, 0.4), Vec3::new(0.0, 10.7, BACK + 0.6 + i as f32 * 1.8), &m(Color::hex("1c1530"), 0.8));
    }
}


// ------------------------------------------------------------ set pieces

fn window(r: &mut RoomBuild) {
    // A night city behind glass, with moonlight pouring in through the fog.
    let center = Vec3::new(-8.5, 7.0, BACK + 0.02);
    let size = Vec2::new(2.2, 3.0);
    let sky = sky_texture();
    let sky = r.image(sky);
    let glass = Mat { texture: sky, unshaded: true, filter: 0, ..m(rgb(1.25, 1.25, 1.35), 1.0) };
    r.quad(size, center, &glass);
    let frame = m(Color::hex("2d2140"), 0.6);
    r.box_(Vec3::new(size.x + 0.3, 0.16, 0.2), center + Vec3::new(0.0, size.y / 2.0 + 0.08, 0.05), &frame);
    r.box_(Vec3::new(size.x + 0.3, 0.2, 0.3), center + Vec3::new(0.0, -size.y / 2.0 - 0.1, 0.1), &frame);
    for s in [-1.0f32, 0.0, 1.0] {
        let w = if s == 0.0 { 0.1 } else { 0.16 };
        r.box_(Vec3::new(w, size.y, 0.14), center + Vec3::new(s * (size.x / 2.0 + 0.07) * s.abs(), 0.0, 0.05), &frame);
    }
    r.box_(Vec3::new(size.x, 0.08, 0.1), center + Vec3::new(0.0, 0.3, 0.04), &frame);
    let pos = center + Vec3::new(0.6, 0.6, -0.5);
    let at = xf(looking_at(Vec3::new(-4.0, 0.0, 2.5) - pos, Vec3::Y), pos);
    r.spot("moon", Color::hex("9fb4ff"), 6.0, 22.0, 22.0, at, 3.0, true, 1.0, 1.0);
}

fn logo(r: &mut RoomBuild) {
    // The GameNight wordmark as a real neon sign: the text is rendered into a
    // transparent texture and used as emission, so glow picks it up.
    let y = 8.0;
    let z = BACK + 0.12;
    let texture = r.screen("logo", [1280, 320], true);
    let sign = Mat {
        texture,
        transparent: true,
        emission: Some((Color::BLACK, 3.2)),
        emission_texture: texture,
        filter: 1,
        ..m(rgb(0.3, 0.3, 0.3), 1.0)
    };
    let node = r.quad(Vec2::new(10.4, 2.6), Vec3::new(0.0, y - 0.3, z), &sign);
    let material = r.material_of(node);
    r.neon.push(Neon { material, base: 3.2, phase: 0.0, speed: 0.0, flicker: true });
    r.omni("logo_lamp", AMBER, 2.2, 6.5, Vec3::new(0.0, y, z + 1.0), 1.0, false);
}

fn tv(r: &mut RoomBuild) {
    let c = v3(l::TV_CENTER);
    let s = Vec2::new(l::TV_SIZE[0] as f32, l::TV_SIZE[1] as f32);
    r.box_(Vec3::new(s.x + 0.36, s.y + 0.36, 0.22), c + Vec3::new(0.0, 0.0, -0.08), &mm(Color::hex("0e0d16"), 0.35, 0.4));
    let texture = r.screen("tv", [896, 504], false);
    let screen = r.quad(s, c + Vec3::new(0.0, 0.0, 0.05), &RoomBuild::screen_material(texture, 1.35));
    r.dump.instances[screen].name = "tv_screen".into();
    // TV console with game boxes; its top is a platform.
    r.box_(Vec3::new(6.6, 0.85, 0.9), Vec3::new(0.0, 0.43, BACK + 0.5), &m(Color::hex("231a2e"), 0.5));
    r.box_(Vec3::new(6.6, 0.05, 0.92), Vec3::new(0.0, 0.87, BACK + 0.5), &m(Color::hex("3b2c4a"), 0.3));
    r.platform_edge(-3.3, 3.3, 0.9, BACK + 0.96);
    let cases = [VIOLET, GREEN, AMBER, PINK, CYAN, Color::hex("f5e663")];
    for (i, case) in cases.iter().enumerate() {
        r.box_(Vec3::new(0.12, 0.62, 0.45), Vec3::new(-2.9 + i as f32 * 0.15, 1.2, BACK + 0.45), &m(case.darkened(0.3), 0.5));
    }
    r.box_(Vec3::new(0.9, 0.18, 0.5), Vec3::new(2.45, 0.98, BACK + 0.45), &m(Color::hex("e9e6f5"), 0.3));
    r.box_flat(Vec3::new(0.3, 0.04, 0.02), Vec3::new(2.45, 0.98, BACK + 0.71), &glow(GREEN, 0.3, GREEN, 4.0));
    // A floating button over the console says the TV is something to use.
    r.prompt("tv", &[("Y", "Start")], Vec3::new(0.0, 5.9, BACK + 0.3));
    r.omni("tv_light", VIOLET, 3.0, 10.0, c + Vec3::new(0.0, -0.4, 1.6), 0.6, false);
    // Speakers either side, sitting on floating shelves.
    for s2 in [-1.0f32, 1.0] {
        let x = s2 * 4.2;
        r.box_(Vec3::new(1.0, 0.1, 0.6), Vec3::new(x, 3.55, BACK + 0.3), &m(Color::hex("5b3b2a"), 0.55));
        r.platform_edge(x - 0.5, x + 0.5, 3.6, BACK + 0.61);
        let body = r.box_(Vec3::new(0.8, 1.3, 0.55), Vec3::new(x, 2.85, BACK + 0.32), &m(Color::hex("4a3a5e"), 0.45));
        let cone = r.cylinder(0.3, 0.04, Vec3::new(x, 2.65, BACK + 0.61), &mm(Color::hex("1a1622"), 0.3, 0.6));
        r.rotate(cone, euler(PI / 2.0, 0.0, 0.0));
        let tweeter = r.cylinder(0.12, 0.04, Vec3::new(x, 3.22, BACK + 0.61), &mm(Color::hex("1a1622"), 0.3, 0.6));
        r.rotate(tweeter, euler(PI / 2.0, 0.0, 0.0));
        r.dump.instances[body].name = "speaker".into();
    }
}

/// A game box like a DVD case: dark plastic shell with the cover in front.
/// Returns the instances (shell, spine, cover).
pub fn game_box(r: &mut RoomBuild, size: Vec2, root: Affine3A, cover: i64) -> [usize; 3] {
    let shell = r.add_material(&mm(Color::hex("12141f"), 0.35, 0.2));
    let a = r.node("BoxMesh", Shape::Box(Vec3::new(size.x + 0.05, size.y + 0.05, 0.1)), shell, root, true);
    let spine = r.add_material(&mm(Color::hex("23263a"), 0.3, 0.3));
    let b = r.node("BoxMesh", Shape::Box(Vec3::new(0.05, size.y + 0.05, 0.11)), spine, RoomBuild::at(root, Vec3::new(-size.x / 2.0, 0.0, 0.0)), false);
    let face = r.add_material(&Mat { texture: cover, filter: 1, ..m(Color::WHITE, 0.85) });
    // Printed artwork belongs to the room: receive its light and fog, unlike the TV.
    let c = r.node("QuadMesh", Shape::Quad(size), face, RoomBuild::at(root, Vec3::new(0.0, 0.0, 0.051)), false);
    [a, b, c]
}

fn game_shelf(r: &mut RoomBuild) {
    // Game spines in the cupboard, with a selected cover on top while browsing.
    let wood = m(Color::hex("5b3b2a"), 0.55);
    let dark = m(Color::hex("3a261c"), 0.6);
    let z = BACK + 0.4;
    let bx = ((l::BOOKCASE.0 as f32) + (l::BOOKCASE.1 as f32)) / 2.0;
    let bw = (l::BOOKCASE.1 as f32) - (l::BOOKCASE.0 as f32);
    r.box_(Vec3::new(bw, 2.65, 0.06), Vec3::new(bx, 1.325, z - 0.3), &dark);
    for side in [-1.0f32, 1.0] {
        r.box_(Vec3::new(0.08, 2.65, 0.66), Vec3::new(bx + side * (bw / 2.0 - 0.04), 1.325, z), &wood);
    }
    for y in l::SPINE_ROWS.iter().map(|&y| y as f32).chain([1.79, 2.65]) {
        r.box_(Vec3::new(bw, 0.07, 0.66), Vec3::new(bx, y - 0.035, z), &wood);
    }
    r.platform_edge(l::BOOKCASE.0 as f32, l::BOOKCASE.1 as f32, 2.65, z + 0.34);
    // Top compartment: a retro console and a plant, so the case never looks empty.
    r.box_(Vec3::new(0.6, 0.14, 0.4), Vec3::new(bx - 0.25, 1.86, z), &mm(Color::hex("c9c4d8"), 0.4, 0.2));
    r.box_flat(Vec3::new(0.12, 0.02, 0.02), Vec3::new(bx - 0.4, 1.9, z + 0.21), &glow(PINK, 0.3, PINK, 3.0));
    r.box_(Vec3::new(0.22, 0.06, 0.14), Vec3::new(bx - 0.25, 1.96, z + 0.1), &m(Color::hex("2a2838"), 0.5));
    r.cylinder(0.13, 0.22, Vec3::new(bx + 0.4, 1.9, z), &m(Color::hex("c46c4a"), 0.8));
    r.sphere(0.22, Vec3::new(bx + 0.4, 2.15, z), &m(Color::hex("2f8f5a"), 0.9));
    // Display stand: low cabinet, the featured box on an easel, a spot on it.
    let dx = ((l::DISPLAY.0 as f32) + (l::DISPLAY.1 as f32)) / 2.0;
    let dw = (l::DISPLAY.1 as f32) - (l::DISPLAY.0 as f32);
    r.box_(Vec3::new(dw, 0.78, 0.8), Vec3::new(dx, 0.39, z + 0.05), &wood);
    r.box_(Vec3::new(dw - 0.14, 0.5, 0.02), Vec3::new(dx, 0.39, z + 0.46), &m(Color::hex("24170f"), 0.5));
    r.box_(Vec3::new(dw + 0.05, 0.05, 0.85), Vec3::new(dx, 0.8, z + 0.05), &dark);
    r.platform_edge(l::DISPLAY.0 as f32, l::DISPLAY.1 as f32, 0.83, z + 0.48);
    let front = r.screen("box_front", [300, 420], false);
    r.group = "featured".into();
    let root = xf(euler(-0.06, 0.0, 0.0), Vec3::new(bx, 3.56, z + 0.1));
    let parts = game_box(r, Vec2::new(1.25, 1.75), root, front);
    r.group.clear();
    r.hidden.extend(parts);
    for side in [-1.0f32, 1.0] {
        // Xbox-style bumper buttons, drawn smooth like the Y badge in the hints.
        let key = if side < 0.0 { "LB" } else { "RB" };
        let texture = r.screen(&format!("bumper_{key}"), [400, 200], true);
        let mut material = RoomBuild::screen_material(texture, 1.6);
        material.transparent = true;
        material.disable_fog = true;
        let button = r.quad(Vec2::new(0.7, 0.35), Vec3::new(bx + side * 0.98, 3.56, z + 0.3), &material);
        r.dump.instances[button].name = if side < 0.0 { "shelf_lb" } else { "shelf_rb" }.into();
        r.hidden.push(button);
    }
    r.prompt("shelf", &[("Y", "Set up next"), ("X", "Queue")], Vec3::new(bx, 5.0, z + 0.3));
    let pos = Vec3::new(bx, 5.4, z + 1.6);
    r.spot("shelf_spot", Color::hex("ffd7a0"), 9.0, 5.0, 22.0, xf(looking_at(Vec3::new(bx, 3.56, z) - pos, Vec3::Y), pos), 0.6, false, 1.0, 1.0);
    r.omni("shelf_glow", Color::hex("b9a6ff"), 3.0, 3.2, Vec3::new(bx, 1.4, z + 1.0), 1.0, false);
}

fn up_next(r: &mut RoomBuild) {
    // One substantial preview for the next game; every later entry is text only.
    let z = BACK + 0.5;
    let frame = m(Color::hex("211d32"), 0.5);
    r.box_(Vec3::new(2.88, 1.38, 0.65), Vec3::new(-4.95, 0.72, z), &frame);
    r.box_(Vec3::new(2.88, 1.94, 0.16), NEXT_POSITION - Vec3::new(0.0, 0.0, 0.1), &frame);
    r.prompt("queue", &[("Y", "Play next"), ("X", "Skip next")], Vec3::new(-4.95, 3.5, z + 0.3));
    let texture = r.screen("queue0", [640, 420], false);
    let preview = r.quad(NEXT_SIZE, NEXT_POSITION, &RoomBuild::screen_material(texture, 1.05));
    r.dump.instances[preview].name = "queue_preview".into();
    let titles = r.screen("queue_titles", [640, 300], false);
    let node = r.quad(Vec2::new(2.7, 1.27), TITLES_POSITION, &RoomBuild::screen_material(titles, 1.05));
    r.dump.instances[node].name = "queue_titles".into();
    r.neon_line(NEXT_POSITION + Vec3::new(-1.35, 0.92, 0.02), NEXT_POSITION + Vec3::new(1.35, 0.92, 0.02), GREEN, 1.5, 0.025, 0.0);
}

fn shelves(r: &mut RoomBuild) {
    let wood = m(Color::hex("5b3b2a"), 0.55);
    let defs: [[f32; 3]; 9] = [
        [-11.6, -8.6, 4.7],
        [-7.4, -5.2, 5.9],
        [-2.6, 2.6, 6.55],
        [5.2, 7.7, 4.3],
        [-11.6, -9.6, 7.3],
        [-6.4, -4.4, 8.2],
        [-1.2, 1.2, 9.2],
        [4.4, 6.4, 8.2],
        [9.0, 11.6, 6.6],
    ];
    let mut rng = Pcg::new(5);
    for d in defs {
        // The TV shelf stays clear for the neon sign.
        let bare = d[2] == 6.55;
        let x0 = d[0].max(-11.95);
        let x1 = d[1].min(11.95);
        let w = x1 - x0;
        let cx = (x0 + x1) / 2.0;
        r.box_(Vec3::new(w, 0.12, 0.7), Vec3::new(cx, d[2] - 0.06, BACK + 0.35), &wood);
        r.platform_edge(x0, x1, d[2], BACK + 0.71);
        // Props: books, a plant or a trophy, kept to the back of the shelf.
        let mut x = x0 as f64 + 0.25;
        while x < x1 as f64 - 0.4 && !bare {
            let roll = rng.randf();
            if roll < 0.45 {
                for _ in 0..rng.randi_range(2, 5) {
                    let h = rng.randf_range_f32(0.35, 0.55);
                    let bc = Color::from_hsv(rng.randf(), 0.55, 0.55);
                    r.box_(Vec3::new(0.09, h, 0.38), Vec3::new(x as f32, d[2] + h / 2.0, BACK + 0.25), &m(bc, 0.7));
                    x += 0.1;
                }
                x += 0.25;
            } else if roll < 0.65 {
                r.cylinder(0.15, 0.25, Vec3::new((x + 0.1) as f32, d[2] + 0.125, BACK + 0.3), &m(Color::hex("c46c4a"), 0.8));
                r.sphere(0.26, Vec3::new((x + 0.1) as f32, d[2] + 0.42, BACK + 0.3), &m(Color::hex("2f8f5a"), 0.9));
                x += 0.6;
            } else if roll < 0.8 {
                let gold = mm(Color::hex("ffcf5a"), 0.25, 0.9);
                let at = (x + 0.1) as f32;
                r.cylinder(0.12, 0.08, Vec3::new(at, d[2] + 0.04, BACK + 0.3), &gold);
                r.cylinder(0.03, 0.2, Vec3::new(at, d[2] + 0.18, BACK + 0.3), &gold);
                r.sphere(0.13, Vec3::new(at, d[2] + 0.38, BACK + 0.3), &gold);
                x += 0.5;
            } else {
                x += 0.45;
            }
        }
    }
}

fn couch_corner(r: &mut RoomBuild) {
    let fabric = m(Color::hex("4a3290"), 0.95);
    let dark = m(Color::hex("35246b"), 0.95);
    let z = BACK + 0.75;
    r.box_(Vec3::new(3.3, 0.55, 1.2), Vec3::new(6.45, 0.45, z + 0.1), &fabric);
    r.box_(Vec3::new(3.3, 0.9, 0.4), Vec3::new(6.45, 1.1, z - 0.4), &dark);
    for s in [-1.0f32, 1.0] {
        r.box_(Vec3::new(0.35, 0.95, 1.2), Vec3::new(6.45 + s * 1.75, 0.5, z + 0.1), &dark);
    }
    for i in 0..3 {
        r.box_(Vec3::new(1.02, 0.18, 1.0), Vec3::new(5.4 + i as f32 * 1.05, 0.8, z + 0.15), &fabric);
    }
    let a = r.box_(Vec3::new(0.5, 0.42, 0.18), Vec3::new(5.2, 1.08, z - 0.12), &m(AMBER, 0.9));
    r.rotate(a, euler(0.0, 0.0, 0.25));
    let b = r.box_(Vec3::new(0.5, 0.42, 0.18), Vec3::new(7.7, 1.08, z - 0.12), &m(GREEN.darkened(0.2), 0.9));
    r.rotate(b, euler(0.0, 0.0, -0.2));
    // Floor lamp: warm light, a nice contrast to the neon.
    let lamp_x = 4.5;
    r.cylinder(0.025, 2.6, Vec3::new(lamp_x, 1.3, BACK + 0.5), &mm(Color::hex("c8a46a"), 0.3, 0.8));
    r.cylinder(0.2, 0.04, Vec3::new(lamp_x, 0.02, BACK + 0.5), &mm(Color::hex("c8a46a"), 0.3, 0.8));
    let shade = Shape::Cylinder { top: 0.22, bottom: 0.38, height: 0.45, radial: 64, rings: 4 };
    r.put(shade, &glow(Color::hex("ffd9a0"), 0.8, AMBER, 1.6), Affine3A::from_translation(Vec3::new(lamp_x, 2.65, BACK + 0.5)), true);
    r.omni("floor_lamp", Color::hex("ffb36b"), 2.4, 5.5, Vec3::new(lamp_x, 2.4, BACK + 0.9), 1.0, true);
}

fn jukebox(r: &mut RoomBuild) {
    // A proper jukebox: arched cabinet, now-playing screen, bubbling tubes
    // that dance while music plays, and three buttons on the front.
    let x = l::JUKEBOX as f32;
    let z = BACK + 0.55;
    let wood = mm(Color::hex("5a2438"), 0.35, 0.1);
    r.box_(Vec3::new(1.7, 2.1, 0.9), Vec3::new(x, 1.05, z), &wood);
    let up = euler(PI / 2.0, 0.0, 0.0);
    let arch = r.cylinder(0.85, 0.9, Vec3::new(x, 2.1, z), &wood);
    r.rotate(arch, up);
    let chrome = mm(Color::hex("d8d4e8"), 0.15, 0.9);
    let rim = r.cylinder(0.88, 0.06, Vec3::new(x, 2.1, z + 0.44), &chrome);
    r.rotate(rim, up);
    let dome = r.cylinder(0.74, 0.04, Vec3::new(x, 2.1, z + 0.47), &glow(Color::hex("ffcf8a"), 0.2, AMBER, 1.3));
    r.rotate(dome, up);
    let material = r.material_of(dome);
    r.neon.push(Neon { material, base: 1.3, phase: 0.0, speed: 1.3, flicker: false });
    // The title strip has a transparent background and stays inside the dome.
    let texture = r.screen("jukebox", [320, 160], true);
    let mut strip_mat = RoomBuild::screen_material(texture, 1.0);
    strip_mat.transparent = true;
    r.quad(Vec2::new(1.1, 0.55), Vec3::new(x, 2.15, z + 0.5), &strip_mat);
    for i in 0..7 {
        let c = Color::from_hsv(i as f32 / 7.0, 0.75, 1.0);
        let tube = r.box_flat(Vec3::new(0.1, 0.7, 0.05), Vec3::new(x - 0.6 + i as f32 * 0.2, 0.9, z + 0.47), &glow(c, 0.3, c, 3.0));
        r.dump.instances[tube].name = format!("eq{i}");
        let material = r.material_of(tube);
        r.neon.push(Neon { material, base: 3.0, phase: i as f32 * 1.2, speed: 3.0, flicker: false });
    }
    r.box_(Vec3::new(1.75, 0.08, 0.95), Vec3::new(x, 0.45, z), &chrome);
    r.box_(Vec3::new(1.75, 0.08, 0.95), Vec3::new(x, 1.3, z), &chrome);
    for (i, px) in l::MUSIC_PADS.iter().map(|&x| x as f32).enumerate() {
        let button = r.cylinder(0.13, 0.06, Vec3::new(px, 0.28, z + 0.48), &glow(PINK, 0.3, PINK, 2.0));
        r.rotate(button, up);
        let icon = r.screen(&format!("music_icon{i}"), [64, 64], true);
        let mut icon_mat = RoomBuild::screen_material(icon, 1.0);
        icon_mat.transparent = true;
        r.quad(Vec2::new(0.18, 0.18), Vec3::new(px, 0.28, z + 0.52), &icon_mat);
    }
    r.omni("jukebox_light", Color::hex("ff8fb8"), 1.8, 3.5, Vec3::new(x, 0.9, z + 1.3), 1.0, false);
}

fn exit_door(r: &mut RoomBuild) {
    let x = l::EXIT_DOOR as f32;
    let z = BACK + 0.06;
    r.box_(Vec3::new(1.2, 2.3, 0.12), Vec3::new(x, 1.15, z), &m(Color::hex("6a4532"), 0.6));
    r.box_(Vec3::new(1.0, 2.1, 0.06), Vec3::new(x, 1.05, z + 0.08), &m(Color::hex("9a6544"), 0.55));
    let base = Vec3::new(x, 0.0, z);
    for (a, b) in [
        (Vec3::new(-0.62, 0.0, 0.14), Vec3::new(-0.62, 2.32, 0.14)),
        (Vec3::new(0.62, 0.0, 0.14), Vec3::new(0.62, 2.32, 0.14)),
        (Vec3::new(-0.62, 2.32, 0.14), Vec3::new(0.62, 2.32, 0.14)),
    ] {
        r.neon_line(base + a, base + b, GREEN, 2.5, 0.04, 0.0);
    }
    r.sphere(0.05, Vec3::new(x + 0.35, 1.05, z + 0.14), &mm(Color::hex("ffcf5a"), 0.2, 0.9));
    let sign = r.box_flat(Vec3::new(0.8, 0.3, 0.06), Vec3::new(x, 2.55, z + 0.06), &glow(Color::hex("0b2a1c"), 0.3, GREEN, 1.4));
    r.dump.instances[sign].name = "exit_sign".into();
    r.label("exit_label", "EXIT", Affine3A::from_translation(Vec3::new(x, 2.55, z + 0.1)), 40, rgb(0.6, 2.6, 1.4), 0);
    r.omni("exit_light", GREEN, 2.0, 3.5, Vec3::new(x, 2.4, z + 0.5), 1.0, false);
}

fn plaque(r: &mut RoomBuild) {
    let c = v3(l::PLAQUE_CENTER);
    r.box_(Vec3::new(2.3, 2.75, 0.1), c + Vec3::new(0.0, 0.0, -0.02), &m(Color::hex("0e0c18"), 0.4));
    let texture = r.screen("plaque", [320, 380], false);
    // White enough to read after tone mapping, low enough not to bloom.
    let mut material = RoomBuild::screen_material(texture, 1.15);
    material.filter = 0;
    r.quad(Vec2::new(2.2, 2.63), c + Vec3::new(0.0, 0.0, 0.04), &material);
}

/// Where a moving head points at `time` (room.gd's `_process`).
pub fn mover_target(side: f32, time: f32) -> Vec3 {
    let t = time * 0.35 + if side < 0.0 { 0.0 } else { PI };
    Vec3::new(t.sin() * 7.0 - side * 1.5, 0.0, 1.0 + (t * 0.7).cos() * 1.5)
}

pub fn mover_origin(side: f32) -> Vec3 {
    Vec3::new(side * 9.5, 10.2, 1.5)
}

pub const DISCO_ORIGIN: Vec3 = Vec3::new(2.8, 9.8, 0.4);

fn ceiling_lights(r: &mut RoomBuild) {
    // Fixed spots that paint coloured cones through the haze. They hang in
    // front of the play line and off to the side, so the light hits players
    // at an angle and throws a real shadow back onto the floor and wall.
    let spots = [(-6.8, VIOLET, Vec3::new(-4.4, 0.0, -0.6)), (-1.6, PINK, Vec3::new(0.6, 0.0, -0.6)), (7.6, AMBER, Vec3::new(5.6, 0.0, -0.6))];
    for (i, (x, color, target)) in spots.into_iter().enumerate() {
        let housing = r.cylinder(0.22, 0.4, Vec3::new(x, 10.4, 6.0), &mm(Color::hex("221d30"), 0.4, 0.6));
        r.dump.instances[housing].name = "spot".into();
        let pos = Vec3::new(x, 9.95, 6.0);
        r.spot(&format!("ceiling{i}"), color, 18.0, 16.0, 15.0, xf(looking_at(target - pos, Vec3::Y), pos), 5.0, true, 0.5, 0.4);
    }
    // Two moving heads that sweep slowly across the room.
    for (i, s) in [-1.0f32, 1.0].into_iter().enumerate() {
        let origin = mover_origin(s);
        let head = xf(looking_at(mover_target(s, 0.0) - origin, Vec3::Y), origin);
        r.group = format!("mover{i}");
        let id = r.add_material(&mm(Color::hex("2a2438"), 0.4, 0.6));
        r.node("CylinderMesh", cyl(0.18, 0.35), id, head, true);
        r.group.clear();
        r.spot(&format!("mover{i}"), if s < 0.0 { CYAN } else { GREEN }, 14.0, 20.0, 6.0, head, 8.0, false, 1.0, 0.3);
    }
    // Disco ball: tiny mirrors are faked by a faceted metallic sphere.
    let disco = Affine3A::from_translation(DISCO_ORIGIN);
    r.group = "disco".into();
    let wire = r.add_material(&mm(Color::hex("888888"), 0.3, 1.0));
    r.node("CylinderMesh", cyl(0.01, 0.8), wire, RoomBuild::at(disco, Vec3::new(0.0, 0.45, 0.0)), true);
    let mirror = r.add_material(&mat(Color::hex("dcdcf0"), 0.05, 1.0, Color::hex("6a5cff"), 0.4));
    r.node("SphereMesh", Shape::Sphere { radius: 0.38, height: 0.76, radial: 14, rings: 7 }, mirror, disco, true);
    r.group.clear();
    r.omni("disco", Color::hex("c8c0ff"), 1.2, 4.0, DISCO_ORIGIN + Vec3::new(0.0, -0.8, 0.5), 1.0, false);
}

fn foreground(r: &mut RoomBuild) {
    // A rug, a coffee table with snacks and plants close to the camera give
    // the room depth without ever covering the players' plane.
    r.box_flat(Vec3::new(9.0, 0.02, 3.4), Vec3::new(0.0, 0.01, 1.4), &m(Color::hex("2a1f4a"), 1.0));
    r.box_flat(Vec3::new(8.6, 0.022, 3.0), Vec3::new(0.0, 0.012, 1.4), &m(Color::hex("3a2a66"), 1.0));
    for (i, c) in [AMBER, VIOLET, PINK].into_iter().enumerate() {
        r.box_flat(Vec3::new(8.2 - i as f32 * 0.6, 0.024, 0.06), Vec3::new(0.0, 0.013, i as f32 * 0.12), &m(c.darkened(0.2), 1.0));
    }
    let table_z = 3.6;
    r.box_(Vec3::new(3.0, 0.1, 1.1), Vec3::new(-0.5, 0.5, table_z), &m(Color::hex("6b4430"), 0.35));
    for sx in [-1.0f32, 1.0] {
        for sz in [-1.0f32, 1.0] {
            r.box_(Vec3::new(0.08, 0.45, 0.08), Vec3::new(-0.5 + sx * 1.35, 0.23, table_z + sz * 0.45), &m(Color::hex("3b2618"), 0.5));
        }
    }
    r.cylinder(0.32, 0.12, Vec3::new(-1.2, 0.61, table_z), &m(Color::hex("e9e6f5"), 0.3));
    for i in 0..7 {
        r.sphere(0.07, Vec3::new(-1.35 + (i % 4) as f32 * 0.1, 0.72, table_z - 0.1 + (i / 4) as f32 * 0.15), &m(Color::hex("ffcf5a"), 0.9));
    }
    for (i, c) in [PINK, GREEN, CYAN].into_iter().enumerate() {
        r.cylinder(0.06, 0.2, Vec3::new(-0.75 + i as f32 * 0.25, 0.65, table_z + 0.1 * (i as f32 - 1.0)), &mm(c, 0.25, 0.7));
    }
    // The two controllers from the GameNight logo, put down mid-evening.
    logo_pad(r, AMBER, Vec3::new(0.3, 0.6, table_z - 0.1), 0.24);
    logo_pad(r, VIOLET, Vec3::new(0.75, 0.62, table_z + 0.2), -0.23);
    // A controller nobody has joined with yet makes these jump, with A Join
    // above them; with nobody in the room the A Join stays up.
    r.prompt("join", &[("A", "Join")], Vec3::new(0.52, 1.45, table_z + 0.1));
    for s in [-1.0f32, 1.0] {
        let x = s * 11.0;
        r.cylinder(0.35, 0.6, Vec3::new(x, 0.3, 2.5), &m(Color::hex("c46c4a"), 0.8));
        for leaf in 0..7 {
            let a = leaf as f32 * TAU / 7.0;
            let node = r.box_(Vec3::new(0.12, 1.4, 0.02), Vec3::new(x + a.cos() * 0.2, 1.2, 2.5 + a.sin() * 0.2), &m(Color::hex("2f8f5a").darkened(leaf as f32 * 0.05), 0.9));
            r.rotate(node, euler(a.sin() * 0.5, a, a.cos() * 0.5));
        }
    }
}

/// The pose of a logo pad: hopping by `k` (0..1) on `side`.
pub fn pad_xf(pos: Vec3, yaw: f32, k: f32, side: f32) -> Affine3A {
    Affine3A::from_mat3_translation((euler(0.1 * k, yaw, 0.16 * k * side) * Mat3A::from_diagonal(Vec3::splat(1.6))).into(), pos + Vec3::new(0.0, 0.12 * k, 0.0))
}

/// A controller shaped like the ones in the logo: a rounded body with two
/// round grips, a d-pad and two buttons knocked out in dark.
fn logo_pad(r: &mut RoomBuild, color: Color, pos: Vec3, yaw: f32) {
    let pad = pad_xf(pos, yaw, 0.0, 1.0);
    let name = format!("pad{}", r.pads.len());
    r.group = name.clone();
    // A faint glow of their own keeps them readable in the dim foreground.
    let body = r.add_material(&glow(color, 0.45, color, 0.5));
    let ink = r.add_material(&m(Color::hex("0b0b12"), 0.6));
    r.node("BoxMesh", Shape::Box(Vec3::new(0.36, 0.07, 0.17)), body, RoomBuild::at(pad, Vec3::new(0.0, 0.0, -0.02)), true);
    for side in [-1.0f32, 1.0] {
        r.node("CylinderMesh", cyl(0.085, 0.07), body, RoomBuild::at(pad, Vec3::new(side * 0.13, 0.0, 0.04)), true);
    }
    r.node("BoxMesh", Shape::Box(Vec3::new(0.09, 0.012, 0.028)), ink, RoomBuild::at(pad, Vec3::new(-0.1, 0.04, -0.02)), false);
    r.node("BoxMesh", Shape::Box(Vec3::new(0.028, 0.012, 0.09)), ink, RoomBuild::at(pad, Vec3::new(-0.1, 0.04, -0.02)), false);
    for b in [Vec3::new(0.09, 0.04, -0.05), Vec3::new(0.13, 0.04, 0.0)] {
        r.node("CylinderMesh", cyl(0.022, 0.012), ink, RoomBuild::at(pad, b), true);
    }
    r.group.clear();
    r.pads.push((name, pos, yaw, body));
    r.pad_pose.push(pad);
}

fn string_lights(r: &mut RoomBuild) {
    // Party bulbs sagging along the top of the back wall.
    let colors = [AMBER, PINK, CYAN, GREEN, VIOLET];
    let spans = [(-11.8f32, -4.0f32), (-4.0, 4.0), (4.0, 11.8)];
    let mut k = 0;
    for (a, b) in spans {
        let count = 14;
        let mut prev = Vec3::ZERO;
        for i in 0..=count {
            let t = i as f32 / count as f32;
            let x = a + (b - a) * t;
            let y = 10.15 - (t * PI).sin() * 0.7;
            let pos = Vec3::new(x, y, BACK + 0.3);
            if i > 0 {
                let wire = r.box_flat(Vec3::new(pos.distance(prev), 0.015, 0.015), (pos + prev) / 2.0, &m(Color::hex("15121f"), 0.8));
                r.rotate(wire, euler(0.0, 0.0, (pos.y - prev.y).atan2(pos.x - prev.x)));
            }
            prev = pos;
            if i == 0 || i == count {
                continue;
            }
            let c = colors[k % colors.len()];
            k += 1;
            let bulb = r.put(sph(0.07), &glow(c, 0.3, c, 6.0), Affine3A::from_translation(pos + Vec3::new(0.0, -0.09, 0.0)), false);
            let material = r.material_of(bulb);
            r.neon.push(Neon { material, base: 6.0, phase: k as f32 * 0.9, speed: 1.7, flicker: false });
        }
    }
}

// ------------------------------------------------------------ live updates

/// What the room's animation depends on besides the clock.
#[derive(Clone, Copy, Debug)]
pub struct Mood {
    pub music_playing: bool,
    /// The TV light eases towards this (the next game's colour).
    pub color: Color,
    /// Seconds since the last nudge of the table pads, and how long the
    /// "A Join" hint still shows.
    pub pad_hop: f32,
    pub pad_activity: f32,
}

impl Default for Mood {
    fn default() -> Self {
        Mood { music_playing: false, color: VIOLET, pad_hop: 99.0, pad_activity: 0.0 }
    }
}

pub const PAD_HOP: f32 = 0.35;

fn set_transform(t: &mut [f32; 16], at: Affine3A) {
    *t = glam::Mat4::from(at).to_cols_array();
}

impl RoomBuild {
    fn find(&self, name: &str) -> Vec<usize> {
        self.dump.instances.iter().enumerate().filter(|(_, i)| i.name == name || i.name.starts_with(&format!("{name}/"))).map(|(k, _)| k).collect()
    }

    fn light(&self, name: &str) -> usize {
        self.dump.lights.iter().position(|l| l["name"] == name).unwrap_or_else(|| panic!("no light {name}"))
    }

    /// room.gd `_process` at `time` with `delta` since the last pose: neon
    /// pulses, the logo flicker, moving heads, the disco ball, the jukebox
    /// EQ, the TV light and the table pads. Changes the description in
    /// place; the caller uploads what changed.
    pub fn pose(&mut self, time: f32, delta: f32, mood: &Mood) {
        for n in &self.neon {
            // GDScript works in doubles here.
            let mut f = 1.0f64;
            if n.speed > 0.0 {
                f = 0.75 + 0.25 * (time as f64 * n.speed as f64 + n.phase as f64).sin();
            }
            if n.flicker && time % 9.0 > 8.6 {
                f = if (time * 20.0) as i64 % 3 == 0 { 0.35 } else { 1.0 };
            }
            self.dump.materials[n.material]["emission_energy"] = json!((n.base * f) as f32);
        }
        for (i, s) in [-1.0f32, 1.0].into_iter().enumerate() {
            let origin = mover_origin(s);
            let head = xf(looking_at(mover_target(s, time) - origin, Vec3::Y), origin);
            for k in self.find(&format!("mover{i}")) {
                set_transform(&mut self.dump.instances[k].transform, head);
            }
            let light = self.light(&format!("mover{i}"));
            self.dump.lights[light]["transform"] = json!(glam::Mat4::from(head).to_cols_array());
        }
        let disco = xf(euler(0.0, time * 0.6, 0.0), DISCO_ORIGIN);
        for k in self.find("disco") {
            // The wire hangs above the ball.
            let wire = matches!(self.shapes[k], Shape::Cylinder { top, .. } if top == 0.01);
            let offset = if wire { Vec3::new(0.0, 0.45, 0.0) } else { Vec3::ZERO };
            set_transform(&mut self.dump.instances[k].transform, disco * Affine3A::from_translation(offset));
        }
        let light = self.light("disco");
        self.dump.lights[light]["transform"] = json!(glam::Mat4::from(disco * Affine3A::from_translation(Vec3::new(0.0, -0.8, 0.5))).to_cols_array());
        for i in 0..7 {
            let mut level = 0.35;
            if mood.music_playing {
                let f = i as f32;
                level = 0.45 + 0.55 * ((time * (3.1 + f * 0.7) + f * 1.9).sin() * (time * 1.7 + f).sin()).abs();
            }
            // Grows up from the base, never into the dome.
            let pos = Vec3::new(l::JUKEBOX as f32 - 0.6 + i as f32 * 0.2, 0.55 + level * 0.35, BACK + 0.55 + 0.47);
            for k in self.find(&format!("eq{i}")) {
                set_transform(&mut self.dump.instances[k].transform, Affine3A::from_scale_rotation_translation(Vec3::new(1.0, level, 1.0), glam::Quat::IDENTITY, pos));
            }
        }
        let tv = self.light("tv_light");
        let c = &self.dump.lights[tv]["color"];
        let current = rgba(c[0].as_f64().unwrap_or(0.0) as f32, c[1].as_f64().unwrap_or(0.0) as f32, c[2].as_f64().unwrap_or(0.0) as f32, 1.0);
        self.dump.lights[tv]["color"] = json!(current.lerp(mood.color, delta * 2.0).array());
        self.dump.lights[tv]["energy"] = json!(3.0 + (time * 7.0).sin() * 0.12 + (time * 2.3).sin() * 0.15);
        let t = (mood.pad_hop / PAD_HOP).clamp(0.0, 1.0);
        let k = (PI * t).sin();
        for (i, (name, pos, yaw, body)) in self.pads.clone().into_iter().enumerate() {
            let side = if i % 2 == 0 { 1.0 } else { -1.0 };
            let pad = pad_xf(pos, yaw, k, side);
            let back = self.pad_pose[i].inverse();
            for n in self.find(&name) {
                let at = Affine3A::from_mat4(glam::Mat4::from_cols_array(&self.dump.instances[n].transform));
                set_transform(&mut self.dump.instances[n].transform, pad * back * at);
            }
            self.pad_pose[i] = pad;
            let glow = 0.5 + 1.6 * k.max((mood.pad_activity / 0.6).clamp(0.0, 1.0) * 0.4);
            self.dump.materials[body]["emission_energy"] = json!(glow);
        }
    }
}
