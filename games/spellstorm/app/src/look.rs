//! The Godot materials and meshes of the kits as Warp materials and
//! meshes: StandardMaterial3D settings map onto Warp's material, the
//! game's own shaders (Shape.material's noise, stone, cloth, the floor) onto
//! the SURFACE hook (surface.wgsl), with their numbers in `Scene::aux`.
use crate::kits::{Kit, Kits};
use glam::{Affine3A, Mat4, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, EMISSIVE, SURFACE, TWO_SIDED};
use render::Scene;
use serde_json::Value;
use std::collections::HashMap;

pub const PAINT: u32 = 0;
pub const NOISE: u32 = 1;
pub const STONE: u32 = 2;
pub const CLOTH: u32 = 3;
pub const FLOOR: u32 = 4;

/// Material::layer for one of surface.wgsl's surfaces, its numbers at `aux`.
pub fn surface_layer(kind: u32, aux: usize) -> i32 {
    -2 - (kind | (aux as u32) << 3) as i32
}

pub fn srgb(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Godot sRGB colour (may exceed 1) to linear.
pub fn linear(v: &Value) -> Vec3 {
    let f = |i: usize| v[i].as_f64().unwrap_or(0.0) as f32;
    Vec3::new(srgb(f(0)), srgb(f(1)), srgb(f(2)))
}

pub fn linear3(c: [f32; 3]) -> Vec3 {
    Vec3::new(srgb(c[0]), srgb(c[1]), srgb(c[2]))
}

fn num(v: &Value, k: &str, d: f32) -> f32 {
    v[k].as_f64().map(|x| x as f32).unwrap_or(d)
}

pub fn push_f32(aux: &mut Vec<u32>, v: f32) {
    aux.push(v.to_bits());
}

fn push_v3(aux: &mut Vec<u32>, v: Vec3) {
    for x in v.to_array() {
        push_f32(aux, x);
    }
}

/// How a Godot material draws in Warp.
#[derive(Clone, Copy, Debug)]
pub struct Mat {
    pub id: u32,
    /// The vertex colours go in the uvs (the material multiplies by them).
    pub vcol: bool,
    /// Drawn but never casts a shadow (glows, additive cards).
    pub no_shadow: bool,
    /// A camera-facing card (halo.gdshader, billboard materials).
    pub billboard: bool,
    /// Its picture is this cell of the card atlas: the uvs are moved there.
    pub cell: Option<u32>,
}

/// One drawn piece of a kit: a Warp mesh at a node.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub mesh: usize,
    pub node: usize,
    pub shadow: bool,
    pub billboard: bool,
}

pub struct Look {
    mats: HashMap<usize, Mat>,
    meshes: HashMap<(usize, usize, usize), usize>,
    /// Kit meshes cut for breaking, by (mesh, material).
    chunks: HashMap<(usize, usize), Vec<(usize, Vec3)>>,
    /// Kit texture -> atlas tile (layer * 64 + cell).
    tiles: Vec<u32>,
    /// Built kits: their pieces.
    built: HashMap<String, Vec<Piece>>,
    /// The card atlas (halos), 8 x 8 cells, and its next free cell.
    halo_layer: i32,
    halo_next: u32,
}

impl Look {
    pub fn empty() -> Look {
        Look { mats: HashMap::new(), meshes: HashMap::new(), chunks: HashMap::new(), tiles: vec![], built: HashMap::new(), halo_layer: -1, halo_next: 0 }
    }

    /// Put every noise picture in 2048 px atlas layers (8 x 8 cells).
    pub fn new(kits: &Kits, scene: &mut Scene) -> Result<Look, String> {
        let n = kits.file.textures.len();
        let mut tiles = vec![0; n];
        let mut layer_of = Vec::new();
        for chunk in 0..n.div_ceil(64) {
            let mut atlas = image::RgbaImage::from_pixel(2048, 2048, image::Rgba([128, 128, 128, 255]));
            for cell in 0..64.min(n - chunk * 64) {
                let t = chunk * 64 + cell;
                let d = &kits.file.textures[t];
                let img = vfs::read(kits.dir.join(&d.file)).map_err(|e| e.to_string()).and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string())).map_err(|e| format!("{}: {e}", d.file))?.to_luma8();
                let img = if (img.width(), img.height()) != (256, 256) { image::imageops::resize(&img, 256, 256, image::imageops::FilterType::Triangle) } else { img };
                let (ox, oy) = ((cell % 8) as u32 * 256, (cell / 8) as u32 * 256);
                for (x, y, p) in img.enumerate_pixels() {
                    atlas.put_pixel(ox + x, oy + y, image::Rgba([p[0], p[0], p[0], 255]));
                }
                tiles[t] = cell as u32;
            }
            let layer = scene.add_image(&format!("spellstorm-noise-{chunk}"), atlas);
            layer_of.push(layer as u32);
            for cell in 0..64.min(n - chunk * 64) {
                tiles[chunk * 64 + cell] |= layer as u32 * 64;
            }
        }
        let _ = layer_of;
        let halo_layer = scene.add_image("spellstorm-cards", image::RgbaImage::from_pixel(2048, 2048, image::Rgba([0, 0, 0, 0])));
        Ok(Look { mats: HashMap::new(), meshes: HashMap::new(), chunks: HashMap::new(), tiles, built: HashMap::new(), halo_layer, halo_next: 0 })
    }

    /// Godot material `m` (a kit material index) in the scene.
    pub fn material(&mut self, kits: &Kits, scene: &mut Scene, m: usize) -> Mat {
        if let Some(&v) = self.mats.get(&m) {
            return v;
        }
        let d = kits.material(m).clone();
        let v = self.make(kits, scene, &d);
        self.mats.insert(m, v);
        v
    }

    fn make(&mut self, kits: &Kits, scene: &mut Scene, d: &Value) -> Mat {
        let plain = |scene: &mut Scene, m: Material| Mat { id: scene.add_material(m), vcol: false, no_shadow: false, billboard: false, cell: None };
        match d["type"].as_str().unwrap_or("") {
            "StandardMaterial3D" => {
                let albedo = linear(&d["albedo"]);
                let alpha = d["albedo"][3].as_f64().unwrap_or(1.0) as f32;
                let mut flags = 0;
                if d["two_sided"].as_bool().unwrap_or(false) {
                    flags |= TWO_SIDED;
                }
                let rough = num(d, "roughness", 1.0).clamp(0.03, 1.0);
                let metal = num(d, "metallic", 0.0);
                let vcol = d["vertex_color"].as_bool().unwrap_or(false);
                let billboard = d["billboard"].as_i64().unwrap_or(0) != 0;
                if d["unshaded"].as_bool().unwrap_or(false) {
                    // Unshaded: the albedo is what shows.
                    let peak = albedo.max_element().max(1e-6);
                    let id = scene.add_material(Material { tint: (albedo / peak).to_array(), layer: -1, roughness: 1.0, flags: flags | EMISSIVE, scale: peak, metal: 0.0 });
                    if d["additive"].as_bool().unwrap_or(false) {
                        scene.set_material_additive(id, alpha, true);
                    } else if d["transparent"].as_bool().unwrap_or(false) && alpha < 0.999 {
                        scene.set_material_blend(id, alpha, flags & TWO_SIDED != 0);
                        scene.set_material_object_sort(id);
                    }
                    return Mat { id, vcol: false, no_shadow: true, billboard, cell: None };
                }
                let mut m = Material { tint: albedo.to_array(), layer: -1, roughness: rough, flags, scale: 1.0, metal };
                if d["emission"].is_array() {
                    // Godot adds emission on top of the lit surface.
                    let e = linear(&d["emission"]) * num(d, "emission_energy", 1.0);
                    let peak = e.max_element();
                    if peak > 0.0 {
                        // EMISSIVE glows `tint * scale` for a surface without a
                        // texture; keep the tint the albedo when they agree.
                        m.flags |= EMISSIVE;
                        m.scale = peak / albedo.max_element().max(1e-6);
                    }
                }
                let tex = d["albedo_texture"].as_i64().unwrap_or(-1);
                let ntex = d["normal_texture"].as_i64().unwrap_or(-1);
                let mut packs = false;
                if d["triplanar"].as_bool().unwrap_or(false) && tex >= 0 {
                    let o = aux_start(scene);
                    let strength = if ntex >= 0 { kits.file.textures[ntex as usize].bump_strength } else { 0.0 };
                    let aux = &mut scene.aux;
                    aux.push(self.tiles[tex as usize]);
                    aux.push(if ntex >= 0 { self.tiles[ntex as usize] } else { u32::MAX });
                    push_f32(aux, strength);
                    push_f32(aux, d["uv1_scale"][0].as_f64().unwrap_or(1.0) as f32);
                    push_f32(aux, num(d, "normal_scale", 1.0));
                    let ramp = &d["albedo_ramp"];
                    let offsets = ramp["offsets"].as_array().cloned().unwrap_or_default();
                    let colors = ramp["colors"].as_array().cloned().unwrap_or_default();
                    // Godot's Gradient keeps its points sorted by offset.
                    let mut pts: Vec<(f32, [f32; 3])> = offsets.iter().zip(&colors).map(|(o, c)| (o.as_f64().unwrap_or(0.0) as f32, [0, 1, 2].map(|i| c[i].as_f64().unwrap_or(0.0) as f32))).collect();
                    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                    if pts.is_empty() {
                        pts = vec![(0.0, [0.0; 3]), (1.0, [1.0; 3])];
                    }
                    aux.push(pts.len() as u32);
                    for (t, c) in pts {
                        push_f32(aux, t);
                        for x in c {
                            push_f32(aux, x);
                        }
                    }
                    m.layer = surface_layer(NOISE, o);
                    m.flags |= SURFACE;
                    // Without vertex colours the uvs carry white.
                    packs = true;
                    let _ = vcol;
                } else if vcol {
                    m.layer = surface_layer(PAINT, 0);
                    m.flags |= SURFACE;
                    packs = true;
                }
                let id = scene.add_material(m);
                if d["transparent"].as_bool().unwrap_or(false) && alpha < 0.999 {
                    if d["additive"].as_bool().unwrap_or(false) {
                        scene.set_material_additive(id, alpha, flags & TWO_SIDED != 0);
                    } else {
                        scene.set_material_blend(id, alpha, flags & TWO_SIDED != 0);
                        scene.set_material_object_sort(id);
                    }
                }
                Mat { id, vcol: packs, no_shadow: false, billboard, cell: None }
            }
            "ShaderMaterial" => {
                let p = &d["params"];
                let shader = d["shader"].as_str().unwrap_or("");
                match shader {
                    "stone" => {
                        let o = aux_start(scene);
                        let aux = &mut scene.aux;
                        push_v3(aux, linear(&p["color"]));
                        push_f32(aux, num(p, "scale", 1.6));
                        push_f32(aux, num(p, "bump", 0.06));
                        push_v3(aux, linear(&p["moss"]));
                        push_f32(aux, num(p, "moss_amount", 0.0));
                        push_f32(aux, num(p, "rough", 0.9));
                        plain(scene, Material { tint: [1.0; 3], layer: surface_layer(STONE, o), roughness: 0.9, flags: SURFACE, scale: 1.0, metal: 0.0 })
                    }
                    "cloth" => {
                        let o = aux_start(scene);
                        let aux = &mut scene.aux;
                        push_v3(aux, linear(&p["color"]));
                        push_f32(aux, num(p, "weave", 90.0));
                        push_f32(aux, num(p, "bump", 0.012));
                        let id = scene.add_material(Material { tint: [1.0; 3], layer: surface_layer(CLOTH, o), roughness: 0.92, flags: SURFACE | TWO_SIDED, scale: 1.0, metal: 0.0 });
                        Mat { id, vcol: true, no_shadow: false, billboard: false, cell: None }
                    }
                    "halo" => {
                        // halo.gdshader: a soft glow card added over the scene,
                        // baked into a cell of the card atlas (it only depends on
                        // the uv), turned to the camera every frame.
                        let c = linear(&p["color"]);
                        let s = num(p, "strength", 1.0);
                        let cell = self.halo_next;
                        self.halo_next += 1;
                        let peak = bake_halo(&mut scene.textures[self.halo_layer as usize].rgba, cell, c, s);
                        let id = scene.add_material(Material { tint: [1.0; 3], layer: self.halo_layer, roughness: 1.0, flags: EMISSIVE | TWO_SIDED, scale: peak, metal: 0.0 });
                        scene.set_material_additive(id, 1.0, true);
                        Mat { id, vcol: false, no_shadow: true, billboard: true, cell: Some(cell) }
                    }
                    // Godot's cobweb (dressing.gd `_cobweb`) was a shader made in
                    // code: the kit has it empty. Faint strands, spokes and rings
                    // from the corner, baked into a card and added over the scene.
                    "" if p.as_object().is_none_or(|o| o.is_empty()) => {
                        let cell = self.halo_next;
                        self.halo_next += 1;
                        let peak = bake_web(&mut scene.textures[self.halo_layer as usize].rgba, cell);
                        let id = scene.add_material(Material { tint: [1.0; 3], layer: self.halo_layer, roughness: 1.0, flags: EMISSIVE | TWO_SIDED, scale: peak * 0.05, metal: 0.0 });
                        scene.set_material_additive(id, 1.0, true);
                        Mat { id, vcol: false, no_shadow: true, billboard: false, cell: Some(cell) }
                    }
                    _ => {
                        // element.gdshader and friends: the element's colour, glowing.
                        let base = if p["base"].is_array() { linear(&p["base"]) } else if p["color"].is_array() { linear(&p["color"]) } else { Vec3::ONE };
                        let e = base * num(p, "energy", 1.0);
                        let peak = e.max_element().max(1e-6);
                        let id = scene.add_material(Material { tint: (e / peak).to_array(), layer: -1, roughness: 1.0, flags: EMISSIVE, scale: peak, metal: 0.0 });
                        Mat { id, vcol: false, no_shadow: true, billboard: false, cell: None }
                    }
                }
            }
            _ => plain(scene, Material { tint: [0.5; 3], layer: -1, roughness: 0.9, flags: 0, scale: 1.0, metal: 0.0 }),
        }
    }

    /// Kit mesh `mesh` drawn with kit material `mat` (or each surface's own).
    /// Returns the scene mesh and the first surface's material.
    pub fn mesh(&mut self, kits: &Kits, scene: &mut Scene, mesh: usize, mat: Option<usize>) -> Option<(usize, Mat)> {
        let mats: Vec<Mat> = kits.file.meshes[mesh]
            .surfaces
            .iter()
            .map(|s| mat.or((s.material >= 0).then_some(s.material as usize)).map(|m| self.material(kits, scene, m)))
            .collect::<Option<Vec<_>>>()?;
        self.mesh_with(kits, scene, mesh, &mats, (mat.unwrap_or(usize::MAX), 0))
    }

    /// Kit mesh `mesh` with these materials (one per surface), cached by `key`.
    pub fn mesh_with(&mut self, kits: &Kits, scene: &mut Scene, mesh: usize, mats: &[Mat], key: (usize, usize)) -> Option<(usize, Mat)> {
        let key = (mesh, key.0, key.1);
        if let Some(&id) = self.meshes.get(&key) {
            return Some((id, mats[0]));
        }
        let (verts, idx) = surfaces(kits, mesh, mats);
        let parts: Vec<Part> = verts.iter().zip(&idx).zip(mats).filter(|((_, i), _)| !i.is_empty()).map(|((v, i), m)| Part { vertices: v, indices: i, material: m.id }).collect();
        if parts.is_empty() {
            return None;
        }
        let id = scene.add_mesh(&ClusterMesh::build(&parts));
        self.meshes.insert(key, id);
        Some((id, mats[0]))
    }

    /// Kit mesh `mesh` cut into chunks for when it breaks (prop.gd's
    /// pieces): its triangles grouped by a grid over its bounds, a cell to
    /// about every 35 cm. Each chunk is its own scene mesh, centred on
    /// itself, with where that middle sits in the kit mesh's space.
    pub fn chunks(&mut self, kits: &Kits, scene: &mut Scene, mesh: usize, mat: Option<usize>) -> Vec<(usize, Vec3)> {
        let key = (mesh, mat.unwrap_or(usize::MAX));
        if let Some(c) = self.chunks.get(&key) {
            return c.clone();
        }
        let mats: Option<Vec<Mat>> = kits.file.meshes[mesh]
            .surfaces
            .iter()
            .map(|s| mat.or((s.material >= 0).then_some(s.material as usize)).map(|m| self.material(kits, scene, m)))
            .collect();
        let Some(mats) = mats else { return vec![] };
        let (verts, idx) = surfaces(kits, mesh, &mats);
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for v in verts.iter().flatten() {
            lo = lo.min(Vec3::from(v.pos));
            hi = hi.max(Vec3::from(v.pos));
        }
        let mut out = Vec::new();
        if lo.x.is_finite() {
            let n = ((hi - lo) / 0.35).round().clamp(Vec3::ONE, Vec3::splat(3.0));
            let cell = |p: Vec3| -> usize {
                let c = ((p - lo) / (hi - lo).max(Vec3::splat(1e-4)) * n).floor().clamp(Vec3::ZERO, n - 1.0);
                (c.x + c.y * n.x + c.z * n.x * n.y) as usize
            };
            let cells = (n.x * n.y * n.z) as usize;
            // Per cell, per surface: its triangles' corners.
            let mut tri: Vec<Vec<Vec<u32>>> = vec![vec![Vec::new(); verts.len()]; cells];
            for (s, (v, ix)) in verts.iter().zip(&idx).enumerate() {
                // A candle's glow card is no solid: cut up and left flat
                // (not turned to the camera) its pieces drew as hard little
                // squares of light (Joep: round the watcher's death). It
                // just goes out.
                if mats[s].cell.is_some() || mats[s].billboard {
                    continue;
                }
                for t in ix.chunks_exact(3) {
                    let mid = (Vec3::from(v[t[0] as usize].pos) + Vec3::from(v[t[1] as usize].pos) + Vec3::from(v[t[2] as usize].pos)) / 3.0;
                    tri[cell(mid)][s].extend_from_slice(t);
                }
            }
            for per in tri {
                let corners: Vec<Vec3> = per.iter().zip(&verts).flat_map(|(ix, v)| ix.iter().map(|&i| Vec3::from(v[i as usize].pos))).collect();
                if corners.is_empty() {
                    continue;
                }
                let mid = corners.iter().sum::<Vec3>() / corners.len() as f32;
                let mut cv: Vec<Vec<Vertex>> = Vec::new();
                let mut ci: Vec<Vec<u32>> = Vec::new();
                for (ix, v) in per.iter().zip(&verts) {
                    cv.push(ix.iter().map(|&i| Vertex { pos: (Vec3::from(v[i as usize].pos) - mid).to_array(), ..v[i as usize] }).collect());
                    ci.push((0..ix.len() as u32).collect());
                }
                let parts: Vec<Part> = cv.iter().zip(&ci).zip(&mats).filter(|((_, i), _)| !i.is_empty()).map(|((v, i), m)| Part { vertices: v, indices: i, material: m.id }).collect();
                out.push((scene.add_mesh(&ClusterMesh::build(&parts)), mid));
            }
        }
        self.chunks.insert(key, out.clone());
        out
    }

    /// The wizard's pieces for seat `seat`, its cloth in the seat's colours
    /// (by weave, see view::wizard_cloth); the white skirt cloth stays.
    pub fn wizard(&mut self, kits: &Kits, scene: &mut Scene, seat: usize, cloth: &HashMap<u32, Vec3>) -> Vec<Piece> {
        let Some(kit) = kits.kit("wizard") else { return vec![] };
        let kit = kit.clone();
        let mut recoloured: HashMap<usize, Mat> = HashMap::new();
        let mut out = Vec::new();
        for (i, n) in kit.nodes.iter().enumerate() {
            let (Some(mesh), Some(m)) = (n.mesh, n.material) else { continue };
            if !kit.shown(i) {
                continue;
            }
            let d = kits.material(m);
            let weave = d["params"]["weave"].as_f64().unwrap_or(0.0).round() as u32;
            let white = d["params"]["color"][0].as_f64().unwrap_or(0.0) >= 0.999;
            let mat = match (d["shader"].as_str(), cloth.get(&weave)) {
                (Some("cloth"), Some(&c)) if !white => *recoloured.entry(m).or_insert_with(|| {
                    let o = aux_start(scene);
                    let aux = &mut scene.aux;
                    push_v3(aux, c);
                    push_f32(aux, weave as f32);
                    push_f32(aux, num(&d["params"], "bump", 0.012));
                    let id = scene.add_material(Material { tint: [1.0; 3], layer: surface_layer(CLOTH, o), roughness: 0.92, flags: SURFACE | TWO_SIDED, scale: 1.0, metal: 0.0 });
                    Mat { id, vcol: true, no_shadow: false, billboard: false, cell: None }
                }),
                _ => self.material(kits, scene, m),
            };
            let key = if recoloured.contains_key(&m) { (m, seat + 1) } else { (m, 0) };
            if let Some((id, mt)) = self.mesh_with(kits, scene, mesh, &[mat], key) {
                out.push(Piece { mesh: id, node: i, shadow: n.shadow && !mt.no_shadow, billboard: mt.billboard });
            }
        }
        out
    }

    /// The drawn pieces of kit `name`, built once.
    pub fn pieces(&mut self, kits: &Kits, scene: &mut Scene, name: &str) -> Vec<Piece> {
        if let Some(p) = self.built.get(name) {
            return p.clone();
        }
        let Some(kit) = kits.kit(name) else { return vec![] };
        let kit: Kit = kit.clone();
        let mut out = Vec::new();
        for (i, n) in kit.nodes.iter().enumerate() {
            let Some(mesh) = n.mesh else { continue };
            if !kit.shown(i) {
                continue;
            }
            if let Some((id, m)) = self.mesh(kits, scene, mesh, n.material) {
                out.push(Piece { mesh: id, node: i, shadow: n.shadow && !m.no_shadow, billboard: m.billboard });
            }
        }
        self.built.insert(name.to_string(), out.clone());
        out
    }
}

/// halo.gdshader's picture for `color` at `strength` into cell `cell` of
/// `atlas` (sRGB, the added light already multiplied by its alpha).
/// Returns the peak radiance it was scaled by.
fn bake_halo(atlas: &mut image::RgbaImage, cell: u32, color: Vec3, strength: f32) -> f32 {
    let size = 256u32;
    let mut px = vec![Vec3::ZERO; (size * size) as usize];
    for y in 0..size {
        for x in 0..size {
            let p = glam::Vec2::new((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32) * 2.0 - glam::Vec2::ONE;
            let r = p.length();
            let k = (1.0 - r).clamp(0.0, 1.0).powf(2.4);
            let inner = (1.0 - r * 2.2).clamp(0.0, 1.0).powi(2);
            let albedo = (color * k + color.lerp(Vec3::ONE, 0.6) * inner * 0.8) * strength;
            let alpha = (k * strength).clamp(0.0, 1.0);
            px[(y * size + x) as usize] = albedo * alpha;
        }
    }
    let peak = px.iter().map(|c| c.max_element()).fold(1e-6, f32::max);
    let enc = |v: f32| {
        let v = (v / peak).clamp(0.0, 1.0);
        let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0).round() as u8
    };
    let (ox, oy) = ((cell % 8) * size, (cell / 8) * size);
    for y in 0..size {
        for x in 0..size {
            let c = px[(y * size + x) as usize];
            atlas.put_pixel(ox + x, oy + y, image::Rgba([enc(c.x), enc(c.y), enc(c.z), 255]));
        }
    }
    peak
}

/// A cobweb into card `cell` (dressing.gd's web shader): spokes out of the
/// corner (uv 0) and torn rings round it, fading at the rim. Returns its peak.
fn bake_web(atlas: &mut image::RgbaImage, cell: u32) -> f32 {
    let size = 256u32;
    let line = |v: f32, w: f32| 1.0 - ((v - 0.5).fract().abs() - 0.5).abs().min(1.0) / w;
    let (ox, oy) = ((cell % 8) * size, (cell / 8) * size);
    for y in 0..size {
        for x in 0..size {
            let p = glam::Vec2::new((x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32);
            let r = p.length();
            let a = p.y.atan2(p.x) / std::f32::consts::FRAC_PI_2;
            let spokes = line(a * 9.0, 0.06).clamp(0.0, 1.0);
            let rings = line(r * 11.0 + (a * 56.5).sin() * 0.12, 0.07).clamp(0.0, 1.0);
            let edge = 1.0 - ((r - 0.82) / 0.18).clamp(0.0, 1.0);
            let h = ((a * 9.0).floor() * 12.9 + (r * 11.0).floor() * 78.2).sin() * 43758.5;
            let torn = if h.fract().abs() >= 0.25 { 1.0 } else { 0.0 };
            let w = spokes.max(rings * torn) * edge;
            let v = (w * 0.9).clamp(0.0, 1.0);
            let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            let b = (s * 255.0).round() as u8;
            atlas.put_pixel(ox + x, oy + y, image::Rgba([b, b, (b as f32 * 0.97) as u8, 255]));
        }
    }
    0.9
}

fn aux_start(scene: &mut Scene) -> usize {
    if scene.aux.is_empty() {
        scene.aux.push(0);
    }
    scene.aux.len()
}

/// Kit mesh `mesh`'s surfaces as vertices and indices, with these materials.
fn surfaces(kits: &Kits, mesh: usize, mats: &[Mat]) -> (Vec<Vec<Vertex>>, Vec<Vec<u32>>) {
    let mut verts: Vec<Vec<Vertex>> = Vec::new();
    let mut idx: Vec<Vec<u32>> = Vec::new();
    for (s, m) in kits.file.meshes[mesh].surfaces.iter().zip(mats) {
        let a = kits.arrays(s);
        let v = (0..a.positions.len())
            .map(|i| {
                let n = a.normals.get(i).copied().unwrap_or(Vec3::Y);
                let uv = if m.vcol {
                    let c = a.colors.get(i).copied().unwrap_or([1.0; 4]);
                    let b = c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round());
                    [b[0] + 256.0 * b[1], b[2] + 256.0 * b[3]]
                } else if let Some(cell) = m.cell {
                    let uv = a.uvs.get(i).copied().unwrap_or([0.0; 2]);
                    let (cx, cy) = ((cell % 8) as f32, (cell / 8) as f32);
                    [(cx + 0.02 + uv[0].clamp(0.0, 1.0) * 0.96) / 8.0, (cy + 0.02 + uv[1].clamp(0.0, 1.0) * 0.96) / 8.0]
                } else {
                    a.uvs.get(i).copied().unwrap_or([0.0; 2])
                };
                Vertex { pos: a.positions[i].to_array(), normal: n.to_array(), uv }
            })
            .collect();
        verts.push(v);
        idx.push(a.indices);
    }
    (verts, idx)
}

/// Place a node's model transform at `at` (an Affine for the scene).
pub fn affine(m: Mat4) -> Affine3A {
    Affine3A::from_mat4(m)
}
