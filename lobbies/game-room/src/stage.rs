//! The room on Warp's renderer: scene, lights, Godot-matched tone, glow,
//! fog and baked indirect light, the Label3D tags, and the painted screens.
//! Shared by `game-room shot` and the live lobby.

use crate::dump::{self, linear, mat4, Room, Tuning};
use crate::paint::{Canvas, Prompt, Show};
use glam::Vec3;
use render::headlights::LocalLight;
use render::hud::{HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::{BoxRoom, Grid, Probe};
use render::screen::Screen;
use render::{Gpu, Lighting, Renderer, Scene};
use std::collections::HashMap;
use std::f32::consts::PI;

/// Look knobs, defaults matched to the Godot lobby (see `game-room shot`).
#[derive(Clone, Debug)]
pub struct Options {
    pub tuning: Tuning,
    /// Indirect light, scaled like Godot's sdfgi_energy.
    pub gi: f32,
    pub probes: bool,
    pub rays: u32,
    /// Exposure; None follows the room's tonemap exposure.
    pub ev: Option<f32>,
    pub taa: f32,
    pub sharpen: f32,
    pub bloom_add: bool,
    pub bloom: Option<f32>,
    pub bloom_rounds: u32,
    pub bloom_threshold: Option<f32>,
    pub fog: bool,
    pub fog_gi: f32,
    pub fog_density: f32,
    pub fog_intensity: f32,
    /// Degrees the fog beams' cones widen (Godot's coarse froxels).
    pub fog_blur: f32,
    /// A light bake saved earlier ([`Stage::light_cache`]): used instead of
    /// baking when it was made from the same room, grid, rays and gi. The
    /// browser build ships one, since a page bakes on one thread.
    pub light_cache: Option<Vec<u8>>,
}

impl Default for Options {
    fn default() -> Self {
        Self { tuning: Tuning { light: PI, ambient: PI }, gi: 1.6, probes: true, rays: 64, ev: None, taa: 0.0, sharpen: 0.5,
            bloom_add: false, bloom: None, bloom_rounds: 6, bloom_threshold: None, fog: true, fog_gi: 0.4, fog_density: 1.0, fog_intensity: 0.6, fog_blur: 12.0, light_cache: None }
    }
}

impl Options {
    /// From command-line style getters (`--gi 1.2`, `--no-fog`, ...).
    pub fn from_args(numf: impl Fn(&str, f32) -> f32, flag: impl Fn(&str) -> bool, has: impl Fn(&str) -> bool) -> Self {
        let d = Self::default();
        let opt = |k: &str| has(k).then(|| numf(k, 0.0));
        Self {
            tuning: Tuning { light: numf("--light", PI), ambient: numf("--ambient", PI) },
            gi: numf("--gi", d.gi), probes: !flag("--no-probes"), rays: numf("--rays", d.rays as f32) as u32,
            ev: opt("--ev"), taa: numf("--taa", d.taa), sharpen: numf("--sharpen", d.sharpen),
            bloom_add: flag("--bloom-add"), bloom: opt("--bloom"), bloom_rounds: numf("--bloom-rounds", d.bloom_rounds as f32) as u32,
            bloom_threshold: opt("--bloom-threshold"), fog: !flag("--no-fog"), fog_gi: numf("--fog-gi", d.fog_gi),
            fog_density: numf("--fog-density", d.fog_density), fog_intensity: numf("--fog", d.fog_intensity), fog_blur: numf("--fog-blur", d.fog_blur), light_cache: None,
        }
    }
}

pub struct Stage {
    pub room: Room,
    pub scene: Scene,
    pub light: Lighting,
    pub probes: Vec<Probe>,
    pub options: Options,
    pub bake_seconds: f32,
    labels: (i32, HudDrawList),
    /// What the light bake was made from ([`Stage::light_cache`]).
    light_key: u64,
}

impl Stage {
    /// Build the scene and bake its indirect light. `extend` adds the
    /// caller's own instances (players, props) before the bake.
    pub fn new(mut room: Room, options: Options, extend: impl FnOnce(&mut Scene, &Room)) -> Result<Self, String> {
        let mut scene = Scene::default();
        let bg = linear(&room.dump.environment["background_color"]);
        scene.sky = Some((4, 2, vec![[bg.x, bg.y, bg.z, 1.0]; 8]));
        room.build(&mut scene, &options.tuning)?;
        // Probes over the room interior, half a metre apart.
        let lo = room.min.max(Vec3::new(-12.0, 0.0, -3.0)) + Vec3::splat(0.25);
        let hi = room.max.min(Vec3::new(12.0, 11.0, 8.0)) - Vec3::splat(0.25);
        let cell = 0.5;
        let dims = ((hi - lo) / cell).ceil().max(Vec3::ONE);
        let grid = Grid { origin: lo, cell, dims: [dims.x as u32 + 1, dims.y as u32 + 1, dims.z as u32 + 1] };
        if options.probes { scene.reserve_irradiance_volume(grid)?; }
        let env = room.dump.environment.clone();
        let ev = options.ev.unwrap_or_else(|| godot_ev(num(&env, "tonemap_exposure", 1.0)));
        let mut light = Lighting { sun_lux: 0.0, sky_lux: 0.0, fog_density: 0.0, fog_start: 1000.0, fog_end: 2000.0, ev100: ev,
            contrast: num(&env, "adjustment_contrast", 1.0), saturation: num(&env, "adjustment_saturation", 1.0),
            // The lobby uses 2x MSAA, no TAA: crisp pixel art.
            taa: options.taa, sharpen: options.sharpen, ..Default::default() };
        room.add_sprites(&mut scene, &room.camera);
        room.add_particles(&mut scene, &room.camera)?;
        let labels = add_labels(&mut scene, &room);
        extend(&mut scene, &room);

        // Indirect light: the room's own bounce, scaled like Godot's sdfgi_energy.
        let bake_start = web_time::Instant::now();
        // Like SDFGI: rays that leave the room see Godot's ambient colour.
        let bake = BoxRoom { blocks: room.blocks.clone(), lights: room.all_lights(), sky: room.ambient / PI };
        // Counts and sizes only: a browser's trig differs from native in the
        // last bits, so the exact numbers would never match. The cache comes
        // from the same source tree (build-web.py bakes it with each build).
        let light_key = fnv(format!("{} {} {:?} {} {}", bake.blocks.len(), bake.lights.len(), grid.dims, options.rays, options.gi).as_bytes());
        let probes = if !options.probes { vec![] }
        else if let Some(probes) = options.light_cache.as_deref().and_then(|bytes| cached_probes(bytes, light_key)) { probes }
        else {
            let mut probes = bake.bake(&grid, options.rays, 2)?;
            for p in &mut probes { for a in &mut p.axes { *a *= options.gi; } }
            probes
        };
        let bake_seconds = bake_start.elapsed().as_secs_f32();

        // Godot's volumetric fog: lamp beams, ambient light in the haze, and
        // the scene behind dimmed by it.
        if env["volumetric_fog_enabled"].as_bool().unwrap_or(false) && options.fog {
            let albedo = linear(&env["volumetric_fog_albedo"]);
            // Mean probe irradiance stands in for SDFGI's injection into the froxels.
            let mean = if probes.is_empty() { room.ambient } else { probes.iter().map(|p| p.axes.iter().sum::<Vec3>() / 6.0).sum::<Vec3>() / probes.len() as f32 };
            let inject = num(&env, "volumetric_fog_gi_inject", 1.0) * options.fog_gi;
            let emission = linear(&env["volumetric_fog_emission"]) * num(&env, "volumetric_fog_emission_energy", 0.0);
            light.volumetric = Some(render::volumetric::Volumetric {
                density: num(&env, "volumetric_fog_density", 0.0) * options.fog_density,
                color: albedo,
                anisotropy: num(&env, "volumetric_fog_anisotropy", 0.2),
                intensity: options.fog_intensity,
                extinction: 1.0,
                ambient: albedo * mean * inject / (4.0 * PI) + emission,
                length: num(&env, "volumetric_fog_length", 64.0),
            });
        }
        Ok(Self { room, scene, light, probes, options, bake_seconds, labels, light_key })
    }

    /// The light bake as bytes for [`Options::light_cache`]: a tag, what
    /// it was made from (a hash) and the probes.
    pub fn light_cache(&self) -> Vec<u8> {
        let mut out = LIGHT_TAG.to_vec();
        out.extend_from_slice(&self.light_key.to_le_bytes());
        for p in &self.probes { for a in &p.axes { for c in a.to_array() { out.extend_from_slice(&c.to_le_bytes()); } } }
        out
    }

    /// Set up a renderer made from `scene`: lights, glow, probes, fog beams
    /// and the label tags.
    pub fn upload(&self, gpu: &Gpu, renderer: &Renderer, screen: &mut Screen) -> Result<(), String> {
        self.relight(renderer)?;
        let env = &self.room.dump.environment;
        let mut options = renderer.debug_options();
        // Godot's glow: levels 3-6 summed, screen-blended. The 0.36 matches its
        // capture: Godot's levels are smaller per level than our wide rounds.
        options.bloom_screen = !self.options.bloom_add;
        options.bloom_strength = self.options.bloom.unwrap_or(num(env, "glow_intensity", 0.8) * 0.36);
        options.bloom_rounds = self.options.bloom_rounds;
        options.bloom_threshold = self.options.bloom_threshold.unwrap_or(num(env, "glow_hdr_threshold", 2.0));
        renderer.set_debug_options(options)?;
        if !self.probes.is_empty() { renderer.set_irradiance_volume(gpu, &self.probes)?; }
        screen.draw(gpu, renderer, self.labels.0, [0.0; 4], &self.labels.1)
    }

    /// Hand the room's current lights to the renderer (after they moved).
    pub fn relight(&self, renderer: &Renderer) -> Result<(), String> {
        self.relight_with(renderer, &[])
    }

    /// The room's lights plus `extra` unshadowed ones (players' glows).
    pub fn relight_with(&self, renderer: &Renderer, extra: &[(LocalLight, f32)]) -> Result<(), String> {
        let room = &self.room;
        let mut locals = room.locals.clone();
        let mut fog = room.local_fog.clone();
        for (l, f) in extra { if locals.len() < render::headlights::MAX_LOCAL_LIGHTS { locals.push(*l); fog.push(*f); } }
        renderer.set_local_lights(&locals)?;
        renderer.set_headlights(&room.spots)?;
        renderer.set_spot_falloff(&room.spot_falloff);
        if self.light.volumetric.is_some() {
            // Every lamp scatters by its volumetric fog energy, its cone widened
            // like Godot's coarse froxels (power kept).
            let blur = self.options.fog_blur.to_radians();
            let mut all = locals.clone();
            all.extend(room.all_lights().into_iter().skip(room.locals.len()));
            let beams: Vec<_> = all.iter().zip(fog.iter().chain(&room.spot_fog)).filter(|(_, e)| **e > 0.0).map(|(l, e)| {
                let mut b = *l;
                if l.outer_cos > -0.99 {
                    let (outer, inner) = (l.outer_cos.acos(), l.inner_cos.acos());
                    let wide = (outer + blur).min(PI);
                    b.outer_cos = wide.cos();
                    b.inner_cos = (inner + blur).min(wide - 0.01).cos();
                    b.intensity_cd *= (1.0 - l.outer_cos) / (1.0 - b.outer_cos).max(1e-4);
                }
                b.intensity_cd *= *e;
                b
            }).collect();
            renderer.set_volumetric_beams(&beams)?;
        }
        Ok(())
    }

    /// Paint the screens of every atlas layer (a layer is redrawn whole).
    pub fn paint(&self, gpu: &Gpu, renderer: &Renderer, screen: &mut Screen, show: &Show, prompts: &HashMap<String, Prompt>, only: Option<&[&str]>) -> Result<(), String> {
        let mut layers: Vec<i32> = self.room.screens.iter().filter(|(n, _)| only.is_none_or(|o| o.contains(&n.as_str()))).map(|(_, s)| s.layer).collect();
        layers.sort();
        layers.dedup();
        for layer in layers {
            let mut list = HudDrawList::new([render::screen::SIZE; 2]);
            let mut names: Vec<_> = self.room.screens.iter().filter(|s| s.1.layer == layer).collect();
            names.sort_by(|a, b| a.0.cmp(b.0));
            for (name, rect) in names {
                let mut canvas = Canvas::scaled(&mut list, rect.rect, rect.k);
                if !crate::paint::paint(&mut canvas, name, show, prompts) { eprintln!("game-room: no painter for screen {name}"); }
            }
            screen.draw(gpu, renderer, layer, [0.0; 4], &list)?;
        }
        Ok(())
    }
}

/// Label3D tags (Outfit, unshaded) as glowing blended quads in the scene, so
/// fog, depth and anti-aliasing treat them like Godot does. All tags share
/// one screen layer; returns it and the draw list that fills it.
fn add_labels(scene: &mut Scene, room: &Room) -> (i32, HudDrawList) {
    let semibold = crate::fonts::faces().regular;
    let bold = crate::fonts::faces().bold;
    let side = render::screen::SIZE;
    let layer = scene.add_screen_texture("labels");
    let mut hud = HudDrawList::new([side, side]);
    let (mut x, mut y, mut row) = (0.0f32, 0.0f32, 0.0f32);
    let srgb = |v: f32| if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for l in &room.dump.labels {
        let text = l["text"].as_str().unwrap_or("").to_string();
        let face = if l["weight"].as_i64().unwrap_or(600) >= 700 { bold } else { semibold };
        let ss = 2.0;
        let size = num(l, "font_size", 32.0) * ss;
        let outline = num(l, "outline_size", 0.0) * ss;
        let [tw, th] = render::hud::measure(&text, face, size);
        let pad = outline + 4.0;
        let (cw, ch) = ((tw + pad * 2.0).ceil().min(side as f32), (th + pad * 2.0).ceil().min(side as f32));
        if x + cw > side as f32 { x = 0.0; y += row; row = 0.0; }
        if y + ch > side as f32 { eprintln!("game-room: label atlas full, skipping {text:?}"); continue; }
        let modulate = linear(&l["modulate"]);
        let peak = modulate.max_element().max(1e-6);
        let oc = linear(&l["outline_modulate"]) / peak;
        let color = [255, 255, 255, 255];
        // The material tints by the text colour; the outline keeps its own ratio to it.
        let c = modulate / peak;
        let ocolor = [to8(srgb(oc.x / c.x.max(1e-3))), to8(srgb(oc.y / c.y.max(1e-3))), to8(srgb(oc.z / c.z.max(1e-3))), to8(dump_alpha(&l["outline_modulate"]))];
        let run = |id: u64, at: [f32; 2], color: [u8; 4]| TextRun { id, text: text.clone(), position: at, bounds: [x, y, cw, ch],
            font_size: size, color, font: face, align: TextAlign::Left, width: cw };
        let base = hud.text.len() as u64 * 32;
        if outline > 0.0 {
            for k in 0..16 {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                hud.text.push(run(base + k + 1, [x + pad + a.cos() * outline * 0.5, y + pad + a.sin() * outline * 0.5], ocolor));
            }
        }
        hud.text.push(run(base + 31, [x + pad, y + pad], color));
        let px = num(l, "pixel_size", 0.01) / ss;
        let (w, h) = (cw * px, ch * px);
        let uv = |u: f32, v: f32| [(x + u * cw) / side as f32, (y + v * ch) / side as f32];
        let corner = |u: f32, v: f32| render::cluster::Vertex { pos: [(u - 0.5) * w, (0.5 - v) * h, 0.0], normal: [0.0, 0.0, 1.0], uv: uv(u, v) };
        let verts = [corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0), corner(0.0, 0.0)];
        let material = scene.add_material(render::scene::Material { tint: c.to_array(), layer, roughness: 1.0,
            flags: render::scene::EMISSIVE | render::scene::TWO_SIDED, scale: peak, metal: 0.0 });
        scene.set_material_blend(material, dump_alpha(&l["modulate"]), true);
        scene.set_material_object_sort(material);
        let mesh = scene.add_mesh(&render::cluster::ClusterMesh::build(&[render::cluster::Part { vertices: &verts, indices: &[0, 1, 2, 0, 2, 3], material }]));
        let mut xf = mat4(&l["transform"]);
        if l["billboard"].as_i64().unwrap_or(0) != 0 { xf = dump::billboard(xf, &room.camera); }
        let (scale, rot, trans) = xf.to_scale_rotation_translation();
        scene.add_instance(mesh, glam::Affine3A::from_scale_rotation_translation(scale, rot, trans));
        x += cw + 2.0;
        row = row.max(ch + 2.0);
    }
    (layer, hud)
}

pub fn num(v: &serde_json::Value, k: &str, d: f32) -> f32 { v[k].as_f64().map(|x| x as f32).unwrap_or(d) }
fn dump_alpha(c: &serde_json::Value) -> f32 { c[3].as_f64().unwrap_or(1.0) as f32 }

/// Warp's ev100 that matches Godot's AgX at `exposure` (measured with
/// `game-room tone` against Godot 4.5: equal mid greys at +0.43 stop).
pub fn godot_ev(exposure: f32) -> f32 { (1.0 / (1.2 * exposure)).log2() + 0.43 }

const LIGHT_TAG: &[u8; 8] = b"WARPGI01";

/// FNV-1a: the same on every platform and Rust version (std's hashers are not promised to be).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// The probes in a light cache made with `key`; None for another room or a broken file.
fn cached_probes(bytes: &[u8], key: u64) -> Option<Vec<Probe>> {
    let rest = bytes.strip_prefix(LIGHT_TAG)?;
    let (k, floats) = rest.split_at_checked(8)?;
    if u64::from_le_bytes(k.try_into().ok()?) != key || floats.len() % 72 != 0 { return None; }
    Some(floats.chunks_exact(72).map(|p| {
        let f = |i: usize| f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap());
        Probe { axes: std::array::from_fn(|a| Vec3::new(f(a * 3), f(a * 3 + 1), f(a * 3 + 2))) }
    }).collect())
}
