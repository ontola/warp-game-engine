//! Room parts that show party data: the profile doors on the right wall
//! (main.gd `_make_door`: a gold frame, the waiting person's face, their
//! name and a picture light) and the game spines in the bookcase (room.gd
//! `set_shelf`). Faces, names and titles are painted into one screen
//! layer; the boxes come from the kit and are recoloured per person or
//! game.

use crate::character;
use crate::dump::{linear, Room};
use crate::fonts::faces;
use crate::godot::Color;
use crate::kit::{door_size, Kit, DOORS, SPINES};
use crate::paint::Canvas;
use game_room_sim::layout as l;
use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::headlights::LocalLight;
use render::hud::{FontFace, HudDrawList, HudImage, ImageFilter, TextAlign};
use render::scene::{Material, Scene, ALPHA_TEST, EMISSIVE, PIXEL_ART, TWO_SIDED};
use render::screen::Screen;
use render::slots::Slots;
use serde_json::{json, Value};

const SIDE: f32 = render::screen::SIZE as f32;
/// Portraits sit in the layer eight times enlarged (32 px art).
const PORTRAIT_UP: u32 = 8;
const PORTRAIT_PX: u32 = character::PORTRAIT * PORTRAIT_UP;
/// Name tags: 30 px Outfit at twice the size, in cells this big.
const TAG: [u32; 2] = [640, 128];
const TAG_SS: f32 = 2.0;
/// Spine titles: 16 px at four times the size.
const TITLE: [u32; 2] = [512, 96];
const TITLE_SS: f32 = 4.0;

fn portrait_cell(i: usize) -> [u32; 2] { [i as u32 * (PORTRAIT_PX + 8), 0] }
fn tag_cell(i: usize) -> [u32; 2] { [i as u32 * (TAG[0] + 8), PORTRAIT_PX + 8] }
fn title_cell(i: usize) -> [u32; 2] { [(i as u32 % 4) * (TITLE[0] + 8), PORTRAIT_PX + TAG[1] + 16 + (i as u32 / 4) * (TITLE[1] + 8)] }

/// A quad of `size` metres showing `cell` (`px` pixels) of the layer.
fn quad(scene: &mut Scene, material: u32, size: [f32; 2], cell: [u32; 2], px: [u32; 2]) -> usize {
    let (u0, v0) = (cell[0] as f32 / SIDE, cell[1] as f32 / SIDE);
    let (u1, v1) = ((cell[0] + px[0]) as f32 / SIDE, (cell[1] + px[1]) as f32 / SIDE);
    let corner = |x: f32, y: f32| Vertex { pos: [(x - 0.5) * size[0], (0.5 - y) * size[1], 0.0], normal: [0.0, 0.0, 1.0], uv: [u0 + (u1 - u0) * x, v0 + (v1 - v0) * y] };
    let verts = [corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0), corner(0.0, 0.0)];
    scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &verts, indices: &[0, 1, 2, 0, 2, 3], material }]))
}

fn gone() -> Affine3A { Affine3A::from_scale(Vec3::splat(1e-4)) }

/// A label instance, parked shrunk; returns (instance, mesh).
fn hidden(scene: &mut Scene, mesh: usize) -> (usize, usize) {
    scene.add_instance(mesh, gone());
    (scene.instances.len() - 1, mesh)
}

/// An unshaded text material as the Stage makes Label3D's: white text
/// times the (HDR) modulate, blended by the painted alpha.
fn label_material(scene: &mut Scene, layer: i32, modulate: [f64; 3]) -> u32 {
    let c = linear(&json!(modulate));
    let peak = c.max_element().max(1e-6);
    let m = scene.add_material(Material { tint: (c / peak).to_array(), layer, roughness: 1.0, flags: EMISSIVE | TWO_SIDED, scale: peak, metal: 0.0 });
    scene.set_material_blend(m, 1.0, true);
    scene.set_material_object_sort(m);
    m
}

#[derive(Clone, PartialEq)]
struct Door { id: String, name: String, skin: String, avatar: String }

pub struct Board {
    layer: i32,
    slots: Slots,
    portraits: Vec<usize>,
    /// Label instances and their meshes.
    tags: Vec<(usize, usize)>,
    titles: Vec<(usize, usize)>,
    /// Label instances moved since the last flush.
    moved: Vec<usize>,
    frame: usize,
    bar: usize,
    canvas: Vec<(usize, usize)>,
    spines: Vec<(usize, usize)>,
    band: usize,
    doors: Vec<Door>,
    shelf: Vec<(String, String)>,
    /// Painted contents changed; repaint the layer.
    dirty: bool,
    /// Materials recoloured this frame.
    recolored: Vec<usize>,
    lights: Vec<(LocalLight, f32)>,
}

impl Board {
    pub fn reserve(kit: &Kit, scene: &mut Scene, room: &Room) -> Self {
        let layer = scene.add_screen_texture("board");
        // Sprite3D: shaded, alpha cut, nearest, double sided.
        let face = scene.add_material(Material { tint: [1.0; 3], layer, roughness: 1.0, flags: ALPHA_TEST | TWO_SIDED | PIXEL_ART, scale: 1.0, metal: 0.0 });
        let size = door_size() - 0.1;
        let portraits = (0..DOORS).map(|i| quad(scene, face, [size, size], portrait_cell(i), [PORTRAIT_PX; 2])).collect();
        let tag = label_material(scene, layer, [2.2, 1.6, 0.8]);
        let tag_size = [TAG[0] as f32 / TAG_SS * 0.01, TAG[1] as f32 / TAG_SS * 0.01];
        // Blended geometry must exist when the renderer is built (its
        // triangles are gathered then), so the labels are fixed instances
        // shrunk away while unused, like the kit's fireballs.
        let tags = (0..DOORS).map(|i| { let q = quad(scene, tag, tag_size, tag_cell(i), TAG); hidden(scene, q) }).collect();
        let title = label_material(scene, layer, [1.4, 1.4, 1.5]);
        let title_size = [TITLE[0] as f32 / TITLE_SS * 0.01, TITLE[1] as f32 / TITLE_SS * 0.01];
        let titles = (0..SPINES).map(|i| { let q = quad(scene, title, title_size, title_cell(i), TITLE); hidden(scene, q) }).collect();
        let slots = Slots::reserve(scene, DOORS * 5 + SPINES * 3);
        let _ = room;
        Self { layer, slots, portraits, tags, titles, frame: kit.door_frame, bar: kit.door_bar, canvas: kit.door_canvas.clone(), spines: kit.spines.clone(),
            band: kit.spine_band, moved: vec![], doors: vec![], shelf: vec![], dirty: true, recolored: vec![], lights: vec![] }
    }

    /// The waiting profiles (newest at the doors) and the shelf's games
    /// around the selection; lays everything out when they change.
    pub fn update(&mut self, room: &mut Room, scene: &mut Scene, pending: &[Value], shelf: &[(String, String)], selected: usize, light: f32) {
        let doors: Vec<Door> = pending.iter().take(DOORS).map(|p| {
            let profile = &p["profile"];
            Door { id: crate::host::get(p, "id", ""), name: crate::host::get(profile, "display_name", "Player"), skin: crate::host::get(profile, "skin_color", ""),
                avatar: crate::host::get(profile, "avatar", "") }
        }).collect();
        // room.gd `set_shelf`: with more games than slots, the window around the selection.
        let per_row = ((l::BOOKCASE.1 - l::BOOKCASE.0 - 0.24) / 0.22) as usize;
        let slots = per_row * l::SPINE_ROWS.len();
        let first = if shelf.len() > slots { (selected as i64 - slots as i64 / 2).clamp(0, (shelf.len() - slots) as i64) as usize } else { 0 };
        let shown: Vec<(String, String)> = shelf.iter().skip(first).take(slots).cloned().collect();
        if doors == self.doors && shown == self.shelf { return; }
        self.doors = doors;
        self.shelf = shown;
        self.dirty = true;
        self.slots.clear(scene);
        self.lights.clear();
        for &(i, mesh) in self.tags.iter().chain(&self.titles) {
            scene.update_instance(i, mesh, gone());
            self.moved.push(i);
        }
        let mut place = |scene: &mut Scene, moved: &mut Vec<usize>, (i, mesh): (usize, usize), xf: Affine3A| {
            scene.update_instance(i, mesh, xf);
            moved.push(i);
        };
        let mesh = |k: usize| room.scene_mesh[k].expect("board mesh");
        let size = door_size();
        let z = l::BACK as f32 + 0.06;
        let y = 2.05;
        for (i, door) in self.doors.iter().enumerate() {
            let x = l::PROFILE_DOORS[i] as f32;
            let skin = Color::from_string(&door.skin, crate::room::AMBER);
            let at = |p: Vec3| Affine3A::from_translation(p);
            self.slots.add(scene, mesh(self.frame), at(Vec3::new(x, y, z)));
            let (canvas, material) = self.canvas[i];
            // Dark canvas with a soft wash of the person's skin tone.
            let base = Color::hex("2a2140");
            room.dump.materials[material]["albedo"] = json!(base.lerp(skin, 0.25).array());
            room.dump.materials[material]["emission"] = json!(base.lerp(skin, 0.3).array());
            self.recolored.push(material);
            self.slots.add(scene, mesh(canvas), at(Vec3::new(x, y, z + 0.05)));
            self.slots.add(scene, self.portraits[i], at(Vec3::new(x, y, z + 0.07)));
            place(scene, &mut self.moved, self.tags[i], at(Vec3::new(x, y + size / 2.0 + 0.32, z + 0.2)));
            self.slots.add(scene, mesh(self.bar), at(Vec3::new(x, y + size / 2.0 + 0.1, z + 0.12)));
            // A small picture light above the frame, tilted down at the face.
            let pitch = (-62f32).to_radians();
            let direction = Vec3::new(0.0, pitch.sin(), -pitch.cos());
            let angle = 38f32.to_radians();
            let color = linear(&json!(Color::hex("ffd9a0").lerp(skin, 0.2).array()));
            self.lights.push((LocalLight { position: Vec3::new(x, y + size / 2.0 + 0.12, z + 0.35), direction, intensity_cd: color * 7.0 * light, range_m: 2.6,
                inner_cos: (angle * 0.25).cos(), outer_cos: angle.cos(), source_radius_m: 0.05, falloff: 1.0 }, 1.0));
        }
        for (n, (_, color)) in self.shelf.iter().enumerate() {
            let row = l::SPINE_ROWS[n / per_row] as f32;
            let x = l::BOOKCASE.0 as f32 + 0.23 + (n % per_row) as f32 * 0.22;
            let height = 0.7;
            let back = l::BACK as f32;
            let (spine, material) = self.spines[n];
            let tint = Color::from_string(color, crate::room::VIOLET).darkened(0.25);
            room.dump.materials[material]["albedo"] = json!(tint.array());
            self.recolored.push(material);
            self.slots.add(scene, mesh(spine), Affine3A::from_translation(Vec3::new(x, row + height / 2.0, back + 0.4)));
            self.slots.add(scene, mesh(self.band), Affine3A::from_translation(Vec3::new(x, row + height - 0.06, back + 0.4)));
            place(scene, &mut self.moved, self.titles[n], Affine3A::from_rotation_translation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(x, row + height / 2.0 - 0.04, back + 0.66)));
        }
    }

    pub fn lights(&self) -> &[(LocalLight, f32)] { &self.lights }

    pub fn flush(&mut self, gpu: &render::Gpu, renderer: &render::Renderer, room: &Room, scene: &Scene, screen: &mut Screen) -> Result<(), String> {
        self.slots.flush(gpu, renderer, scene)?;
        for i in std::mem::take(&mut self.moved) { renderer.update_instances(gpu, i, &scene.instances[i..i + 1])?; }
        for m in std::mem::take(&mut self.recolored) {
            if let Some((id, material)) = room.material_now(m) { renderer.update_materials(gpu, id as usize, &[material])?; }
        }
        if std::mem::take(&mut self.dirty) { self.paint(gpu, renderer, screen)?; }
        Ok(())
    }

    fn paint(&self, gpu: &render::Gpu, renderer: &render::Renderer, screen: &mut Screen) -> Result<(), String> {
        let mut list = HudDrawList::new([render::screen::SIZE; 2]);
        for (i, door) in self.doors.iter().enumerate() {
            let face = crate::face::decode(&door.avatar);
            let mut art = character::portrait(&door.skin, face.as_ref().map(|f| character::Face { image: &f.image, center: f.center }).as_ref());
            for px in art.pixels_mut() { if px[3] == 0 { *px = image::Rgba([11, 11, 18, 0]); } }
            let art = image::imageops::resize(&art, PORTRAIT_PX, PORTRAIT_PX, image::imageops::FilterType::Nearest);
            let picture = std::sync::Arc::new(render::picture::Picture::new(art.width(), art.height(), art.into_raw())?);
            let image = HudImage(2000 + i as u64);
            screen.set_image(gpu, image, &picture, ImageFilter::Nearest);
            let [x, y] = portrait_cell(i);
            list.image([x as f32, y as f32, PORTRAIT_PX as f32, PORTRAIT_PX as f32], image, [0.0, 0.0, 1.0, 1.0], [1.0; 4], [0.0, 0.0, SIDE, SIDE]);
        }
        let mut c = Canvas::new(&mut list, [0, 0, render::screen::SIZE, render::screen::SIZE]);
        for (i, door) in self.doors.iter().enumerate() {
            let [x, y] = tag_cell(i);
            let name: String = door.name.chars().take(20).collect();
            label(&mut c, &name, [x, y], TAG, 30.0 * TAG_SS, 10.0 * TAG_SS, faces().regular, f32::MAX);
        }
        for (n, (title, _)) in self.shelf.iter().enumerate() {
            let [x, y] = title_cell(n);
            // Narrow spines: the title shrinks to 0.58 m.
            label(&mut c, &title.to_uppercase(), [x, y], TITLE, 16.0 * TITLE_SS, 4.0 * TITLE_SS, faces().regular, 0.58 / 0.01 * TITLE_SS);
        }
        screen.draw(gpu, renderer, self.layer, [0.0; 4], &list)
    }
}

/// Centred text with a dark outline (Label3D's outline_modulate 0,0,0,0.8),
/// shrunk to `max_width` pixels.
#[allow(clippy::too_many_arguments)]
fn label(c: &mut Canvas, text: &str, cell: [u32; 2], px: [u32; 2], size: f32, outline: f32, face: FontFace, max_width: f32) {
    let w = render::hud::measure(text, face, size)[0];
    let size = if w > max_width { size * max_width / w } else { size };
    let outline = outline * size / size.max(1e-3);
    let baseline = cell[1] as f32 + px[1] as f32 / 2.0 + size * 0.35;
    let at = [cell[0] as f32, baseline];
    c.text_outline(face, at, text, TextAlign::Center, px[0] as f32, size, outline, Color::BLACK.with_alpha(0.8));
    c.text(face, at, text, TextAlign::Center, px[0] as f32, size, Color::WHITE);
}
