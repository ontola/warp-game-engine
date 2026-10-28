//! The Godot Game Room as exported by gamenight/lobbies/game-room/tools/
//! warp_export.gd (scene.json, meshes.bin, tex*.png), turned into a Warp
//! scene: meshes, materials, textures, lights, sprites and text tags.
//!
//! Godot's non-physical units map onto Warp's photometric ones with one
//! factor: a Godot light of energy 1 lights white albedo like `LIGHT` lux,
//! and display values are nits at the exposure `exposure_ev`.
use glam::{Affine3A, Mat4, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::headlights::{LocalLight, SpotLight};
use render::irradiance_volume::Block;
use render::scene::{Material, EMISSIVE, TWO_SIDED};
use render::Scene;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub struct Dump {
    pub meshes: Vec<MeshDump>,
    pub materials: Vec<Value>,
    pub textures: Vec<TextureDump>,
    pub instances: Vec<InstanceDump>,
    pub lights: Vec<Value>,
    pub labels: Vec<Value>,
    pub sprites: Vec<Value>,
    pub camera: Value,
    pub environment: Value,
}

#[derive(Deserialize)]
pub struct MeshDump { pub surfaces: Vec<SurfaceDump> }
#[derive(Deserialize)]
pub struct SurfaceDump { positions: [usize; 2], normals: [usize; 2], uvs: [usize; 2], indices: [usize; 2], material: i64 }
#[derive(Deserialize)]
pub struct TextureDump { pub file: String }
#[derive(Deserialize)]
pub struct InstanceDump { pub name: String, pub mesh: usize, pub materials: Vec<i64>, pub transform: [f32; 16], pub shadow: bool }

/// Godot sRGB colour (may exceed 1 for HDR) to linear, as its shaders do.
pub fn linear(c: &Value) -> Vec3 {
    let f = |i: usize| c[i].as_f64().unwrap_or(0.0) as f32;
    let l = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    Vec3::new(l(f(0)), l(f(1)), l(f(2)))
}
fn alpha(c: &Value) -> f32 { c[3].as_f64().unwrap_or(1.0) as f32 }
fn num(v: &Value, k: &str) -> f32 { v[k].as_f64().unwrap_or(0.0) as f32 }
pub fn mat4(v: &Value) -> Mat4 { let a: Vec<f32> = v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default(); Mat4::from_cols_slice(&a) }

pub struct Tuning {
    /// Lux on a surface facing a Godot light of energy 1 at 1 m.
    pub light: f32,
    /// Lux of Godot's flat ambient colour at energy 1.
    pub ambient: f32,
}

pub struct Room {
    pub dir: PathBuf,
    pub dump: Dump,
    pub images: Vec<image::RgbaImage>,
    pub locals: Vec<LocalLight>,
    pub spots: Vec<SpotLight>,
    /// Per spot: Godot's distance exponent and volumetric fog energy.
    pub spot_falloff: Vec<f32>,
    pub spot_fog: Vec<f32>,
    pub local_fog: Vec<f32>,
    /// Bake geometry: every opaque instance as its world box.
    pub blocks: Vec<Block>,
    pub ambient: Vec3,
    pub camera: render::Camera,
    pub min: Vec3,
    pub max: Vec3,
}

impl Room {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(dir.join("scene.json")).map_err(|e| format!("{}: {e}", dir.display()))?;
        let dump: Dump = serde_json::from_str(&text).map_err(|e| format!("scene.json: {e}"))?;
        let images = dump.textures.iter().map(|t| image::open(dir.join(&t.file)).map(|i| i.to_rgba8()).map_err(|e| format!("{}: {e}", t.file)))
            .collect::<Result<Vec<_>, _>>()?;
        let cam = mat4(&dump.camera["transform"]);
        let pos = cam.w_axis.truncate();
        let camera = render::Camera { pos, target: pos - cam.z_axis.truncate(), fov_y: num(&dump.camera, "fov_y").to_radians(), near: num(&dump.camera, "near").max(0.05) };
        Ok(Self { dir: dir.to_path_buf(), dump, images, locals: vec![], spots: vec![], spot_falloff: vec![], spot_fog: vec![], local_fog: vec![], blocks: vec![], ambient: Vec3::ZERO, camera, min: Vec3::ZERO, max: Vec3::ZERO })
    }

    /// Fill `scene` with the room's meshes; collect lights and bake blocks.
    pub fn build(&mut self, scene: &mut Scene, tuning: &Tuning) -> Result<(), String> {
        let bin = std::fs::read(self.dir.join("meshes.bin")).map_err(|e| format!("meshes.bin: {e}"))?;
        let floats = |r: [usize; 2]| -> Vec<f32> { bin[r[0]..r[0] + r[1]].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect() };
        let ints = |r: [usize; 2]| -> Vec<u32> { bin[r[0]..r[0] + r[1]].chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u32).collect() };
        let mut layers: HashMap<usize, (i32, Vec2)> = HashMap::new();
        let mut materials: HashMap<usize, (u32, Vec3, Vec3, Vec2)> = HashMap::new();
        let mut meshes: HashMap<(usize, Vec<i64>), usize> = HashMap::new();
        let fallback = scene.add_material(Material { tint: [0.8; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for inst in &self.dump.instances {
            let mesh = &self.dump.meshes[inst.mesh];
            let key = (inst.mesh, inst.materials.clone());
            let xf = Mat4::from_cols_array(&inst.transform);
            let mut local_min = Vec3::splat(f32::MAX);
            let mut local_max = Vec3::splat(f32::MIN);
            let mut albedo = Vec3::ZERO;
            let mut emission = Vec3::ZERO;
            let mut blended = false;
            let mut parts_data = vec![];
            for (s, surface) in mesh.surfaces.iter().enumerate() {
                let m = inst.materials.get(s).copied().filter(|&m| m >= 0).unwrap_or(surface.material);
                let (id, a, e, uv_factor) = if m >= 0 { self.material(scene, m as usize, &mut layers, &mut materials)? } else { (fallback, Vec3::splat(0.8), Vec3::ZERO, Vec2::ONE) };
                if m >= 0 && self.dump.materials[m as usize]["transparency"].as_i64().unwrap_or(0) != 0 { blended = true; }
                albedo = a;
                emission = emission.max(e);
                let d = if m >= 0 { Some(&self.dump.materials[m as usize]) } else { None };
                let get = |k: &str, i: usize, def: f64| d.map(|d| d[k][i].as_f64().unwrap_or(def)).unwrap_or(def) as f32;
                let uv_scale = Vec2::new(get("uv1_scale", 0, 1.0), get("uv1_scale", 1, 1.0));
                let uv_off = Vec2::new(get("uv1_offset", 0, 0.0), get("uv1_offset", 1, 0.0));
                let p = floats(surface.positions);
                let n = floats(surface.normals);
                let t = floats(surface.uvs);
                let verts: Vec<Vertex> = (0..p.len() / 3).map(|i| {
                    let pos = Vec3::new(p[i * 3], p[i * 3 + 1], p[i * 3 + 2]);
                    let w = xf.transform_point3(pos);
                    local_min = local_min.min(w);
                    local_max = local_max.max(w);
                    let uv = (Vec2::new(t[i * 2], t[i * 2 + 1]) * uv_scale + uv_off) * uv_factor;
                    Vertex { pos: pos.to_array(), normal: [n[i * 3], n[i * 3 + 1], n[i * 3 + 2]], uv: uv.to_array() }
                }).collect();
                parts_data.push((verts, ints(surface.indices), id));
            }
            if parts_data.is_empty() { continue; }
            let id = match meshes.get(&key) {
                Some(&id) => id,
                None => {
                    let parts: Vec<Part> = parts_data.iter().map(|(v, i, m)| Part { vertices: v, indices: i, material: *m }).collect();
                    let id = scene.add_mesh(&ClusterMesh::build(&parts));
                    meshes.insert(key, id);
                    id
                }
            };
            let (scale, rot, trans) = xf.to_scale_rotation_translation();
            scene.add_instance(id, Affine3A::from_scale_rotation_translation(scale, rot, trans));
            if !blended && local_min.x < local_max.x {
                // Thin quads still block light in the bake.
                let pad = (Vec3::splat(0.02) - (local_max - local_min)).max(Vec3::ZERO) * 0.5;
                self.blocks.push(Block { min: local_min - pad, max: local_max + pad, albedo, emission });
                min = min.min(local_min);
                max = max.max(local_max);
            }
        }
        self.min = min;
        self.max = max;
        self.lights(tuning);
        self.ambient = linear(&self.dump.environment["ambient_light_color"]) * num(&self.dump.environment, "ambient_light_energy") * tuning.ambient;
        Ok(())
    }

    /// Texture layer and the uv factor that maps the image inside it. Layers
    /// are square: a wide or tall picture sits in the top-left corner of a
    /// square at its own aspect, so it keeps sharp (isotropic) mips.
    fn layer(&self, scene: &mut Scene, tex: i64, layers: &mut HashMap<usize, (i32, Vec2)>) -> (i32, Vec2) {
        if tex < 0 { return (-1, Vec2::ONE); }
        let tex = tex as usize;
        *layers.entry(tex).or_insert_with(|| {
            let img = &self.images[tex];
            let (w, h) = img.dimensions();
            if w == h { return (scene.add_image(&format!("godot#{tex}"), img.clone()), Vec2::ONE); }
            let side = w.max(h);
            let mut square = image::RgbaImage::new(side, side);
            image::imageops::overlay(&mut square, img, 0, 0);
            (scene.add_image(&format!("godot#{tex}"), square), Vec2::new(w as f32 / side as f32, h as f32 / side as f32))
        })
    }

    /// Average linear colour of a texture (for the bake).
    fn average(&self, tex: i64) -> Vec3 {
        if tex < 0 { return Vec3::ONE; }
        let img = &self.images[tex as usize];
        let l = |v: u8| { let v = v as f32 / 255.0; if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } };
        let mut sum = Vec3::ZERO;
        let mut weight = 0.0;
        for p in img.pixels().step_by(7) { let a = p[3] as f32 / 255.0; sum += Vec3::new(l(p[0]), l(p[1]), l(p[2])) * a; weight += a; }
        if weight > 0.0 { sum / weight } else { Vec3::ZERO }
    }

    /// Warp material id, bake albedo and bake emission for Godot material `m`.
    fn material(&self, scene: &mut Scene, m: usize, layers: &mut HashMap<usize, (i32, Vec2)>, cache: &mut HashMap<usize, (u32, Vec3, Vec3, Vec2)>) -> Result<(u32, Vec3, Vec3, Vec2), String> {
        if let Some(&v) = cache.get(&m) { return Ok(v); }
        let d = &self.dump.materials[m];
        let tex = d["albedo_texture"].as_i64().unwrap_or(-1);
        let (layer, mut uv_factor) = self.layer(scene, tex, layers);
        let tint = linear(&d["albedo"]);
        let mut flags = 0;
        if d["cull"].as_i64().unwrap_or(0) == 2 { flags |= TWO_SIDED; }
        let mut out = Material { tint: tint.to_array(), layer, roughness: num(d, "roughness").clamp(0.02, 1.0), flags, scale: 1.0, metal: num(d, "metallic") };
        let albedo = tint * self.average(tex);
        let mut emission = Vec3::ZERO;
        if d["unshaded"].as_bool().unwrap_or(false) {
            // Unshaded: the albedo is the output radiance.
            out.flags |= EMISSIVE;
            let peak = tint.max_element().max(1e-6);
            out.tint = (tint / peak).to_array();
            out.scale = peak;
            emission = albedo;
        } else if d["emission_enabled"].as_bool().unwrap_or(false) {
            // Godot adds the emission texture to the emission colour.
            let etex = d["emission_texture"].as_i64().unwrap_or(-1);
            let color = linear(&d["emission"]) + if etex >= 0 { Vec3::ONE } else { Vec3::ZERO };
            let e = color * num(d, "emission_energy");
            let peak = e.max_element().max(1e-6);
            out.flags |= EMISSIVE;
            out.tint = (e / peak).to_array();
            out.scale = peak;
            if etex >= 0 { (out.layer, uv_factor) = self.layer(scene, etex, layers); }
            else if layer >= 0 { out.layer = -1; }
            emission = e * self.average(etex);
        }
        let id = scene.add_material(out);
        let transparency = d["transparency"].as_i64().unwrap_or(0);
        if transparency != 0 {
            scene.set_material_blend(id, alpha(&d["albedo"]), out.flags & TWO_SIDED != 0);
        }
        cache.insert(m, (id, albedo, emission, uv_factor));
        Ok((id, albedo, emission, uv_factor))
    }

    fn lights(&mut self, tuning: &Tuning) {
        for l in &self.dump.lights {
            let energy = num(l, "energy");
            if energy <= 0.0 || l["negative"].as_bool().unwrap_or(false) { continue; }
            let xf = mat4(&l["transform"]);
            let position = xf.w_axis.truncate();
            let direction = -xf.z_axis.truncate().normalize_or(Vec3::NEG_Y);
            let intensity = linear(&l["color"]) * energy * tuning.light;
            let range = num(l, "range");
            let falloff = num(l, "attenuation");
            let size = num(l, "size").max(0.05);
            let fog = num(l, "fog");
            match l["type"].as_str().unwrap_or("") {
                "OmniLight3D" => { self.local_fog.push(fog); self.locals.push(LocalLight { position, direction: Vec3::NEG_Y, intensity_cd: intensity, range_m: range,
                    inner_cos: -0.999, outer_cos: -1.0, source_radius_m: size, falloff }) }
                "SpotLight3D" => {
                    let angle = num(l, "angle").to_radians();
                    // Godot's cone: 1 - rim^k with rim linear in (1 - cos); a
                    // smoothstep from the edge to well inside comes close.
                    let k = num(l, "angle_attenuation").max(0.05);
                    let inner = angle * (1.0 - 0.75 * k.min(1.0)).max(0.05);
                    if l["shadow"].as_bool().unwrap_or(false) {
                        let up = if direction.y.abs() > 0.9 { Vec3::Z } else { Vec3::Y };
                        self.spot_falloff.push(falloff);
                        self.spot_fog.push(fog);
                        self.spots.push(SpotLight { position, direction, up, intensity_cd: intensity, range_m: range,
                            inner_angle_radians: inner, outer_angle_radians: angle });
                    } else {
                        self.local_fog.push(fog);
                        self.locals.push(LocalLight { position, direction, intensity_cd: intensity, range_m: range,
                            inner_cos: inner.cos(), outer_cos: angle.cos(), source_radius_m: size, falloff });
                    }
                }
                _ => {}
            }
        }
        self.spots.truncate(render::headlights::MAX_LIGHTS);
        self.locals.truncate(render::headlights::MAX_LOCAL_LIGHTS);
    }

    /// Every light as an unshadowed local light, for the bake and sprites.
    pub fn all_lights(&self) -> Vec<LocalLight> {
        let mut all = self.locals.clone();
        all.extend(self.spots.iter().map(|s| LocalLight { position: s.position, direction: s.direction, intensity_cd: s.intensity_cd, range_m: s.range_m,
            inner_cos: s.inner_angle_radians.cos(), outer_cos: s.outer_angle_radians.cos(), source_radius_m: 0.1, falloff: 2.0 })
            .zip(&self.spot_falloff).map(|(l, &f)| LocalLight { falloff: f, ..l }));
        all
    }
}

