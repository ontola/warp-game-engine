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
use render::scene::{Material, ALPHA_TEST, EMISSIVE, PIXEL_ART, TWO_SIDED};
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
    #[serde(default)]
    pub particles: Vec<Value>,
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
pub struct InstanceDump { pub name: String, pub mesh: usize, pub materials: Vec<i64>, pub transform: [f32; 16], pub shadow: bool, #[serde(default)] pub shadow_only: bool }

/// Godot sRGB colour (may exceed 1 for HDR) to linear, as its shaders do.
pub fn linear(c: &Value) -> Vec3 {
    let f = |i: usize| c[i].as_f64().unwrap_or(0.0) as f32;
    let l = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    Vec3::new(l(f(0)), l(f(1)), l(f(2)))
}
fn alpha(c: &Value) -> f32 { c[3].as_f64().unwrap_or(1.0) as f32 }
fn num(v: &Value, k: &str) -> f32 { v[k].as_f64().unwrap_or(0.0) as f32 }
pub fn mat4(v: &Value) -> Mat4 { let a: Vec<f32> = v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default(); Mat4::from_cols_slice(&a) }

/// `xf` turned to face the camera, keeping its position and scale.
pub fn billboard(xf: Mat4, camera: &render::Camera) -> Mat4 {
    let (scale, _, trans) = xf.to_scale_rotation_translation();
    let back = (camera.pos - camera.target).normalize_or(Vec3::Z);
    let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
    let up = back.cross(right);
    Mat4::from_cols((right * scale.x).extend(0.0), (up * scale.y).extend(0.0), (back * scale.z).extend(0.0), trans.extend(1.0))
}

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
            // The players' capsule bodies: shadow casters no camera sees.
            if inst.shadow_only { scene.instances.last_mut().unwrap().hide_in_views(0xff); continue; }
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

    /// Sprite3D players and door faces as alpha-tested quads in the scene, so
    /// they are lit per pixel, shadowed and fogged like Godot's shaded sprites.
    pub fn add_sprites(&self, scene: &mut Scene, camera: &render::Camera) {
        let mut layers: HashMap<usize, (i32, Vec2)> = HashMap::new();
        for s in &self.dump.sprites {
            let tex = s["texture"].as_i64().unwrap_or(-1);
            if tex < 0 { continue; }
            let img = &self.images[tex as usize];
            let (layer, uv_factor) = *layers.entry(tex as usize).or_insert_with(|| {
                // Pixel art: a nearest upscale keeps hard pixels under linear filtering.
                let k = (2048 / img.width().max(img.height())).clamp(1, 8);
                let big = image::imageops::resize(img, img.width() * k, img.height() * k, image::imageops::FilterType::Nearest);
                // A full-size layer: no resampling on upload.
                let side = big.width().max(big.height()).max(2048);
                let mut square = image::RgbaImage::new(side, side);
                image::imageops::overlay(&mut square, &big, 0, 0);
                (scene.add_image(&format!("sprite#{tex}"), square), Vec2::new(big.width() as f32 / side as f32, big.height() as f32 / side as f32))
            });
            let (hf, vf) = (s["hframes"].as_u64().unwrap_or(1).max(1) as u32, s["vframes"].as_u64().unwrap_or(1).max(1) as u32);
            let frame = s["frame"].as_u64().unwrap_or(0) as u32;
            let px = num(s, "pixel_size");
            let (fw, fh) = ((img.width() / hf) as f32 * px, (img.height() / vf) as f32 * px);
            let offset = Vec2::new(s["offset"][0].as_f64().unwrap_or(0.0) as f32, s["offset"][1].as_f64().unwrap_or(0.0) as f32) * px;
            let (mut u0, v0, mut du, dv) = ((frame % hf) as f32 / hf as f32, (frame / hf) as f32 / vf as f32, 1.0 / hf as f32, 1.0 / vf as f32);
            if s["flip_h"].as_bool().unwrap_or(false) { u0 += du; du = -du; }
            let corner = |x: f32, y: f32| Vertex {
                pos: [offset.x + (x - 0.5) * fw, offset.y + (0.5 - y) * fh, 0.0], normal: [0.0, 0.0, 1.0],
                uv: (Vec2::new(u0 + x * du, v0 + y * dv) * uv_factor).to_array() };
            let verts = [corner(0.0, 1.0), corner(1.0, 1.0), corner(1.0, 0.0), corner(0.0, 0.0)];
            let shaded = s["shaded"].as_bool().unwrap_or(false);
            let modulate = linear(&s["modulate"]);
            let peak = modulate.max_element().max(1e-6);
            let material = scene.add_material(Material { tint: (modulate / if shaded { 1.0 } else { peak }).to_array(), layer, roughness: 1.0,
                flags: ALPHA_TEST | TWO_SIDED | if shaded { 0 } else { EMISSIVE } | if s["filter"].as_i64().unwrap_or(0) == 0 { PIXEL_ART } else { 0 }, scale: if shaded { 1.0 } else { peak }, metal: 0.0 });
            let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &verts, indices: &[0, 1, 2, 0, 2, 3], material }]));
            let mut xf = mat4(&s["transform"]);
            if s["billboard"].as_i64().unwrap_or(0) != 0 { xf = billboard(xf, camera); }
            let (scale, rot, trans) = xf.to_scale_rotation_translation();
            scene.add_instance(mesh, Affine3A::from_scale_rotation_translation(scale, rot, trans));
        }
    }

    /// CPUParticles3D as they were at the capture: one camera-facing quad per
    /// live particle (Godot's particle billboards), unshaded in its colour.
    pub fn add_particles(&self, scene: &mut Scene, camera: &render::Camera) -> Result<(), String> {
        let bin = std::fs::read(self.dir.join("meshes.bin")).map_err(|e| format!("meshes.bin: {e}"))?;
        let back = (camera.pos - camera.target).normalize_or(Vec3::Z);
        let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
        let up = back.cross(right);
        for p in &self.particles() {
            let Some(range) = p["buffer"].as_array().map(|r| [r[0].as_u64().unwrap_or(0) as usize, r[1].as_u64().unwrap_or(0) as usize]) else { continue };
            let stride = p["stride"].as_u64().unwrap_or(0) as usize;
            if stride < 16 || p["material"].as_i64().unwrap_or(-1) < 0 { continue; }
            let f: Vec<f32> = bin[range[0]..range[0] + range[1]].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let m = &self.dump.materials[p["material"].as_i64().unwrap() as usize];
            let vertex_color = m["vertex_color"].as_bool().unwrap_or(false);
            let albedo = Vec3::new(m["albedo"][0].as_f64().unwrap_or(1.0) as f32, m["albedo"][1].as_f64().unwrap_or(1.0) as f32, m["albedo"][2].as_f64().unwrap_or(1.0) as f32);
            let transparent = m["transparency"].as_i64().unwrap_or(0) != 0;
            let emitter = mat4(&p["transform"]);
            let quad = Vec2::new(p["quad_size"][0].as_f64().unwrap_or(0.1) as f32, p["quad_size"][1].as_f64().unwrap_or(0.1) as f32);
            let mut verts = vec![];
            let mut parts = vec![];
            for k in f.chunks_exact(stride) {
                let scale = Vec3::new(k[0], k[4], k[8]).length();
                if scale <= 1e-5 { continue; }
                // The multimesh is in the emitter's space.
                let at = emitter.transform_point3(Vec3::new(k[3], k[7], k[11]));
                // Godot multiplies albedo by the (linear) vertex colour.
                let color = if vertex_color { albedo * Vec3::new(k[12], k[13], k[14]) } else { linear(&m["albedo"]) };
                let peak = color.max_element().max(1e-6);
                let material = scene.add_material(Material { tint: (color / peak).to_array(), layer: -1, roughness: 1.0, flags: EMISSIVE | TWO_SIDED, scale: peak, metal: 0.0 });
                if transparent { scene.set_material_blend(material, alpha(&m["albedo"]), true); scene.set_material_object_sort(material); }
                let (r, u) = (right * quad.x * scale * 0.5, up * quad.y * scale * 0.5);
                let corner = |c: Vec3, uv: [f32; 2]| Vertex { pos: c.to_array(), normal: back.to_array(), uv };
                verts.push([corner(at - r - u, [0.0, 1.0]), corner(at + r - u, [1.0, 1.0]), corner(at + r + u, [1.0, 0.0]), corner(at - r + u, [0.0, 0.0])]);
                parts.push(material);
            }
            if parts.is_empty() { continue; }
            let parts: Vec<Part> = verts.iter().zip(&parts).map(|(v, &material)| Part { vertices: v, indices: &[0, 1, 2, 0, 2, 3], material }).collect();
            let mesh = scene.add_mesh(&ClusterMesh::build(&parts));
            scene.add_instance(mesh, Affine3A::IDENTITY);
        }
        Ok(())
    }

    fn particles(&self) -> Vec<Value> { self.dump.particles.iter().filter(|p| p["emitting"].as_bool().unwrap_or(false)).cloned().collect() }

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
            scene.set_material_object_sort(id);
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

