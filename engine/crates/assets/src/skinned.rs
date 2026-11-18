//! Portable weighted characters: JSON metadata, original indexed mesh and
//! baked final skin palettes. This importer has no DCC/runtime dependency.
//! Mesh records retain all eight influences; fixed clusters preserve exact
//! topology. Linear matrix interpolation and clip blending stay inside the
//! union of authored frame bounds. Each actor receives an independent mesh.
use glam::{Affine3A, Mat4, Vec3, Vec4};
use render::{
    cluster::{ClusterMesh, Part, Vertex},
    scene::{ALPHA_TEST, HAIR, Material, SKIN, SKINNED, Scene, TWO_SIDED},
    skinning::{self, Influence},
};
use serde::Deserialize;
mod warp_human;
mod warp_skin;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
struct Manifest {
    #[serde(flatten)]
    rig: RigFile,
    mesh_file: String,
    vertex_count: usize,
    index_count: usize,
    vertex_stride_bytes: usize,
    index_offset_bytes: usize,
    surfaces: Vec<SurfaceFile>,
}

#[derive(Deserialize)]
struct RigFile {
    version: u32,
    name: String,
    animation_file: String,
    joint_count: usize,
    matrix_stride_bytes: usize,
    clips: Vec<ClipFile>,
    joints: Vec<JointFile>,
}
#[derive(Deserialize)]
struct JointFile {
    name: String,
    inverse_bind_rows: [f32; 12],
}
#[derive(Deserialize)]
struct SurfaceFile {
    first_vertex: usize,
    vertex_count: usize,
    first_index: usize,
    index_count: usize,
    material: MaterialFile,
}
#[derive(Deserialize)]
struct MaterialFile {
    name: String,
    albedo_texture: Option<String>,
    albedo: [f32; 4],
    roughness: f32,
    metallic: f32,
    #[serde(default)]
    albedo_color_space: String,
    #[serde(default)]
    normal_texture: Option<String>,
    #[serde(default = "one")]
    normal_scale: f32,
    #[serde(default)]
    two_sided: bool,
    #[serde(default)]
    alpha_mode: String,
}
#[derive(Deserialize)]
struct ClipFile {
    name: String,
    duration: f32,
    frames: usize,
    looping: bool,
    matrix_offset_bytes: usize,
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
    #[serde(default)]
    sockets: Vec<HashMap<String, [f32; 12]>>,
}

#[derive(Clone)]
pub struct Clip {
    pub name: String,
    pub duration: f32,
    pub frames: usize,
    pub looping: bool,
    pub min: Vec3,
    pub max: Vec3,
    matrices: std::sync::Arc<[Mat4]>,
    sockets: std::sync::Arc<[HashMap<String, Mat4>]>,
}
struct Surface {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    influences: Vec<Influence>,
    material: MaterialFile,
    texture: Option<(String, std::sync::Arc<image::RgbaImage>)>,
    normal: Option<image::RgbaImage>,
    skin_hair: Option<SkinHair>,
}

/// Which parts of a surface are skin or hair (render::scene::SKIN, HAIR).
#[derive(Clone, Debug)]
pub enum SkinHair {
    /// The whole surface is skin.
    Skin,
    /// The whole surface is hair.
    Hair,
    /// Per texel of the albedo texture: 0 hair, 128 skin, 255 neither
    /// (keeps the surface's own material), linear between. Becomes the
    /// albedo alpha; resized to the texture if its size differs.
    Mask(image::GrayImage),
    /// The opaque texture already carries the shading mask in its alpha channel.
    PackedMask,
}
fn one() -> f32 {
    1.0
}
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub struct Character {
    pub name: String,
    pub joint_count: usize,
    pub clips: Vec<Clip>,
    /// Conservative object-space union of all baked frames and bind vertices.
    pub min: Vec3,
    pub max: Vec3,
    pub triangles: usize,
    face: Option<warp_human::FaceRig>,
    surfaces: Vec<Surface>,
    joints: Vec<(String, Mat4)>,
}

/// A piece of a character that follows one joint rigidly, for renderers
/// without skinning: bind-pose mesh (the joint's palette matrix places it),
/// simplified, with the piece's average texture colour as its `tint`.
#[derive(Clone, Debug)]
pub struct RigidPart {
    pub joint: usize,
    pub tint: [f32; 3],
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
pub struct ActorHandle {
    pub actor: usize,
    pub instance: usize,
    pub mesh: usize,
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 256 * 1024 * 1024 {
        return Err(format!(
            "{} exceeds character file capacity",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}
fn source_path(base: &Path, name: &str) -> Result<PathBuf, String> {
    let path = Path::new(name);
    if name.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("character source paths must be relative files".into());
    }
    Ok(base.join(path))
}
fn word(data: &[u8], index: usize) -> u32 {
    u32::from_le_bytes(data[index..index + 4].try_into().unwrap())
}
fn float(data: &[u8], index: usize) -> f32 {
    f32::from_bits(word(data, index))
}
fn from_rows(r: [f32; 12]) -> Mat4 {
    Mat4::from_cols(
        Vec4::new(r[0], r[4], r[8], 0.0),
        Vec4::new(r[1], r[5], r[9], 0.0),
        Vec4::new(r[2], r[6], r[10], 0.0),
        Vec4::new(r[3], r[7], r[11], 1.0),
    )
}
fn frame_pair(frames: usize, duration: f32, seconds: f32, looping: bool) -> (usize, usize, f32) {
    let position = if looping {
        (seconds / duration).rem_euclid(1.0) * frames as f32
    } else {
        (seconds / duration).clamp(0.0, 1.0) * (frames - 1) as f32
    };
    let first = (position.floor() as usize).min(frames - 1);
    let second = if looping {
        (first + 1) % frames
    } else {
        (first + 1).min(frames - 1)
    };
    (first, second, position - first as f32)
}

impl Character {
    /// Skinned vertices one actor of this character adds to a scene.
    pub fn vertex_count(&self) -> usize { self.surfaces.iter().map(|s| s.influences.len()).sum() }

    pub fn load(path: &Path) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_slice(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
        if manifest.vertex_stride_bytes != 96
            || !(1..=200_000).contains(&manifest.vertex_count)
            || !(3..=1_200_000).contains(&manifest.index_count)
            || manifest.index_count % 3 != 0
            || manifest.surfaces.is_empty()
            || manifest.surfaces.len() > 128
        {
            return Err("unsupported character version, layout or count".into());
        }
        let mut rig = Self::from_rig(path, manifest.rig)?;
        let base = path.parent().unwrap_or(Path::new("."));
        let mesh = read(&source_path(base, &manifest.mesh_file)?)?;
        if manifest.index_offset_bytes != manifest.vertex_count * 96
            || mesh.len() != manifest.index_offset_bytes + manifest.index_count * 4
        {
            return Err("character mesh length does not match declared records".into());
        }
        let mut vertices = Vec::with_capacity(manifest.vertex_count);
        let mut influences = Vec::with_capacity(manifest.vertex_count);
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for data in mesh[..manifest.index_offset_bytes].chunks_exact(96) {
            let vertex = Vertex {
                pos: std::array::from_fn(|i| float(data, i * 4)),
                normal: std::array::from_fn(|i| float(data, 12 + i * 4)),
                uv: std::array::from_fn(|i| float(data, 24 + i * 4)),
            };
            let influence = Influence {
                joints: std::array::from_fn(|i| word(data, 32 + i * 4)),
                weights: std::array::from_fn(|i| float(data, 64 + i * 4)),
            };
            let sum: f32 = influence.weights.iter().sum();
            if !vertex
                .pos
                .iter()
                .chain(&vertex.normal)
                .chain(&vertex.uv)
                .all(|f| f.is_finite())
                || Vec3::from(vertex.normal).length_squared() < 1e-12
                || influence
                    .joints
                    .iter()
                    .any(|j| *j as usize >= rig.joint_count)
                || influence.weights.iter().any(|w| !w.is_finite() || *w < 0.0)
                || !sum.is_finite()
                || (sum - 1.0).abs() > 0.001
            {
                return Err("character has invalid weighted vertex data".into());
            }
            // Preserve exact authored weights: exported pose bounds use these
            // same coefficients, including their harmless f32 sum drift.
            min = min.min(Vec3::from(vertex.pos));
            max = max.max(Vec3::from(vertex.pos));
            vertices.push(vertex);
            influences.push(influence);
        }
        let indices: Vec<_> = mesh[manifest.index_offset_bytes..]
            .chunks_exact(4)
            .map(|d| word(d, 0))
            .collect();
        let mut surfaces = Vec::new();
        let mut next_vertex = 0;
        let mut next_index = 0;
        for source in manifest.surfaces {
            let end_vertex = source
                .first_vertex
                .checked_add(source.vertex_count)
                .ok_or("surface vertex overflow")?;
            let end_index = source
                .first_index
                .checked_add(source.index_count)
                .ok_or("surface index overflow")?;
            if source.first_vertex != next_vertex
                || source.first_index != next_index
                || source.vertex_count == 0
                || source.index_count % 3 != 0
                || end_vertex > vertices.len()
                || end_index > indices.len()
                || indices[source.first_index..end_index]
                    .iter()
                    .any(|i| (*i as usize) < source.first_vertex || (*i as usize) >= end_vertex)
            {
                return Err("character surfaces must partition vertices and reference their own vertex range".into());
            }
            let material = &source.material;
            if !material
                .albedo
                .iter()
                .all(|f| f.is_finite() && *f >= 0.0 && *f <= 1.0)
                || !material.roughness.is_finite()
                || !(0.0..=1.0).contains(&material.roughness)
                || !material.metallic.is_finite()
                || !(0.0..=1.0).contains(&material.metallic)
                || !material.normal_scale.is_finite()
                || !(0.0..=16.0).contains(&material.normal_scale)
                || !matches!(material.albedo_color_space.as_str(), "" | "linear" | "sRGB")
                || !matches!(material.alpha_mode.as_str(), "" | "OPAQUE" | "MASK")
            {
                return Err(format!("unsupported skinned material {}", material.name));
            }
            let texture = material
                .albedo_texture
                .as_ref()
                .filter(|name| !name.is_empty())
                .map(|name| {
                    let path = source_path(base, name)?;
                    let image = image::load_from_memory(&read(&path)?)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .to_rgba8();
                    Ok::<_, String>((path.to_string_lossy().into_owned(), image.into()))
                })
                .transpose()?;
            let normal = material
                .normal_texture
                .as_ref()
                .filter(|name| !name.is_empty())
                .map(|name| {
                    let path = source_path(base, name)?;
                    let mut image = image::load_from_memory(&read(&path)?)
                        .map_err(|e| format!("{}: {e}", path.display()))?
                        .to_rgba8();
                    if material.normal_scale != 1.0 {
                        for pixel in image.pixels_mut() {
                            let normal = Vec3::new(
                                (pixel[0] as f32 / 127.5 - 1.0) * material.normal_scale,
                                (pixel[1] as f32 / 127.5 - 1.0) * material.normal_scale,
                                pixel[2] as f32 / 127.5 - 1.0,
                            )
                            .normalize_or_zero();
                            for axis in 0..3 {
                                pixel[axis] = ((normal[axis] * 0.5 + 0.5) * 255.0)
                                    .round()
                                    .clamp(0.0, 255.0)
                                    as u8;
                            }
                        }
                    }
                    Ok::<_, String>(image)
                })
                .transpose()?;
            surfaces.push(Surface {
                vertices: vertices[source.first_vertex..end_vertex].to_vec(),
                indices: indices[source.first_index..end_index]
                    .iter()
                    .map(|i| *i - source.first_vertex as u32)
                    .collect(),
                influences: influences[source.first_vertex..end_vertex].to_vec(),
                material: source.material,
                texture,
                normal,
                skin_hair: None,
            });
            next_vertex = end_vertex;
            next_index = end_index;
        }
        if next_vertex != vertices.len() || next_index != indices.len() {
            return Err("incomplete character surface coverage".into());
        }
        rig.min = rig.min.min(min - Vec3::splat(0.001));
        rig.max = rig.max.max(max + Vec3::splat(0.001));
        rig.triangles = indices.len() / 3;
        rig.surfaces = surfaces;
        Ok(rig)
    }

    /// Load just the skeleton and animation clips for a procedurally fitted body.
    pub fn load_animation_rig(path: &Path) -> Result<Self, String> {
        let manifest = serde_json::from_slice(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_rig(path, manifest)
    }

    /// [`Character::load_animation_rig`] from memory: the rig manifest and
    /// its animation file (for a rig built into the executable).
    pub fn animation_rig_from_bytes(manifest: &[u8], animation: &[u8]) -> Result<Self, String> {
        let manifest: RigFile = serde_json::from_slice(manifest).map_err(|e| format!("animation rig: {e}"))?;
        Self::check_rig(&manifest)?;
        Self::rig_from(manifest, animation)
    }

    fn check_rig(manifest: &RigFile) -> Result<(), String> {
        if manifest.version != 1 || manifest.matrix_stride_bytes != 48
            || !(1..=skinning::MAX_JOINTS).contains(&manifest.joint_count)
            || !(1..=32).contains(&manifest.clips.len()) {
            return Err("unsupported animation rig version, layout or count".into());
        }
        Ok(())
    }

    fn from_rig(path: &Path, manifest: RigFile) -> Result<Self, String> {
        Self::check_rig(&manifest)?;
        let base = path.parent().unwrap_or(Path::new("."));
        let animation = read(&source_path(base, &manifest.animation_file)?)?;
        Self::rig_from(manifest, &animation)
    }

    fn rig_from(manifest: RigFile, animation: &[u8]) -> Result<Self, String> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        if manifest.joints.len() != manifest.joint_count {
            return Err("character joint metadata count mismatch".into());
        }
        let joints: Vec<_> = manifest
            .joints
            .into_iter()
            .map(|j| (j.name, from_rows(j.inverse_bind_rows).inverse()))
            .collect();
        if joints.iter().any(|(_, m)| !skinning::valid_matrix(*m)) {
            return Err("invalid inverse bind matrix".into());
        }
        let mut clips = Vec::new();
        let mut names = std::collections::HashSet::new();
        for clip in manifest.clips {
            if !names.insert(clip.name.clone())
                || !(2..=20_000).contains(&clip.frames)
                || !clip.duration.is_finite()
                || !(0.01..=600.0).contains(&clip.duration)
                || clip.matrix_offset_bytes % 48 != 0
            {
                return Err("invalid animation clip metadata".into());
            }
            let matrix_count = clip.frames * manifest.joint_count;
            let end = clip
                .matrix_offset_bytes
                .checked_add(matrix_count * 48)
                .ok_or("animation offset overflow")?;
            let bytes = animation
                .get(clip.matrix_offset_bytes..end)
                .ok_or("truncated character animation")?;
            let matrices: Vec<_> = bytes
                .chunks_exact(48)
                .map(|d| from_rows(std::array::from_fn(|i| float(d, i * 4))))
                .collect();
            if matrices.iter().any(|m| !skinning::valid_matrix(*m)) {
                return Err("invalid character animation matrix".into());
            }
            let clip_min = Vec3::from(clip.bounds_min);
            let clip_max = Vec3::from(clip.bounds_max);
            if !clip_min.is_finite() || !clip_max.is_finite() || !clip_min.cmple(clip_max).all() {
                return Err("invalid animation bounds".into());
            }
            min = min.min(clip_min);
            max = max.max(clip_max);
            let sockets: Vec<_> = clip
                .sockets
                .into_iter()
                .map(|frame| {
                    frame
                        .into_iter()
                        .map(|(name, rows)| (name, from_rows(rows)))
                        .collect::<HashMap<_, _>>()
                })
                .collect();
            if !sockets.is_empty()
                && (sockets.len() != clip.frames
                    || sockets
                        .iter()
                        .flat_map(|s| s.values())
                        .any(|m| !skinning::valid_matrix(*m)))
            {
                return Err("invalid animation socket track".into());
            }
            clips.push(Clip {
                name: clip.name,
                duration: clip.duration,
                frames: clip.frames,
                looping: clip.looping,
                min: clip_min,
                max: clip_max,
                matrices: matrices.into(),
                sockets: sockets.into(),
            });
        }
        // A millimetre covers exporter and GPU f32 rounding.
        min -= Vec3::splat(0.001);
        max += Vec3::splat(0.001);
        Ok(Self {
            face: None,
            name: manifest.name,
            joint_count: manifest.joint_count,
            clips,
            min,
            max,
            triangles: 0,
            surfaces: Vec::new(),
            joints,
        })
    }

    pub fn sample(&self, name: &str, seconds: f32, looping: bool) -> Result<Vec<Mat4>, String> {
        let mut palette = vec![Mat4::IDENTITY; self.joint_count];
        self.sample_into(name, seconds, looping, &mut palette)?;
        Ok(palette)
    }

    /// Reuse the caller's joint palette during per-frame animation sampling.
    pub fn sample_into(&self, name: &str, seconds: f32, looping: bool, palette: &mut [Mat4]) -> Result<(), String> {
        if !seconds.is_finite() {
            return Err("animation sample time must be finite".into());
        }
        let clip = self
            .clips
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| format!("missing character clip {name}"))?;
        let (first, second, t) = frame_pair(clip.frames, clip.duration, seconds, looping);
        if palette.len() != self.joint_count { return Err("incorrect character palette length".into()); }
        let source_count = clip.matrices.len() / clip.frames;
        skinning::blend_palette_into(
            &clip.matrices[first * source_count..(first + 1) * source_count],
            &clip.matrices[second * source_count..(second + 1) * source_count],
            t,
            &mut palette[..source_count],
        )?;
        self.pose_face(palette, seconds);
        Ok(())
    }

    pub fn sample_socket(
        &self,
        clip: &str,
        socket: &str,
        seconds: f32,
        looping: bool,
    ) -> Result<Mat4, String> {
        if !seconds.is_finite() {
            return Err("animation sample time must be finite".into());
        }
        let clip = self
            .clips
            .iter()
            .find(|c| c.name == clip)
            .ok_or("missing character socket clip")?;
        let (first, second, t) = frame_pair(clip.frames, clip.duration, seconds, looping);
        let a = clip
            .sockets
            .get(first)
            .and_then(|f| f.get(socket))
            .ok_or("missing character socket")?;
        let b = clip
            .sockets
            .get(second)
            .and_then(|f| f.get(socket))
            .ok_or("missing character socket")?;
        Ok(*a * (1.0 - t) + *b * t)
    }

    /// Joint names and model-space bind/rest transforms in skin palette order.
    /// Names may repeat for joints with distinct inverse binds; every palette
    /// entry is retained. A skin matrix multiplied by its rest transform yields
    /// that joint's animated model-space transform.
    pub fn joints(&self) -> impl ExactSizeIterator<Item = (&str, Mat4)> + Clone + '_ {
        self.joints.iter().map(|(name, rest)| (name.as_str(), *rest))
    }

    /// Recover a named model-space joint from a sampled or blended palette.
    /// Palettes may repeat bones for distinct inverse binds; the first matching
    /// entry yields the same animated joint position.
    pub fn joint_position(&self, name: &str, palette: &[Mat4]) -> Option<Vec3> {
        if palette.len() != self.joint_count {
            return None;
        }
        let (index, (_, rest)) = self.joints.iter().enumerate().find(|(_, j)| j.0 == name)?;
        Some((palette[index] * *rest).transform_point3(Vec3::ZERO))
    }

    /// Add exactly one independently deformed scene mesh and one instance.
    /// Reuses identical materials/textures on repeated calls for the same file.
    pub fn add_actor(&self, scene: &mut Scene, transform: Affine3A) -> Result<ActorHandle, String> {
        self.add_actor_with_bounds(scene, transform, self.min, self.max)
    }

    /// An actor whose every surface is drawn with `material` instead of the
    /// character's own (a body turned to lava, stone or a ghost), with the
    /// same skeleton, clips and skinning; `None` keeps the authored ones.
    pub fn add_actor_with_material(&self, scene: &mut Scene, transform: Affine3A, material: Option<u32>) -> Result<ActorHandle, String> {
        self.add_actor_full(scene, transform, self.min, self.max, material)
    }

    /// Add an actor with conservative object-space bounds for additional poses,
    /// such as physics-driven joints. These bounds affect only the new actor's
    /// mesh and instance; the shared character and other actors are unchanged.
    ///
    /// Bounds must be finite, ordered, enclose `self.min..=self.max`, and produce
    /// a finite culling sphere. The caller must enclose every additional pose.
    pub fn add_actor_with_bounds(
        &self,
        scene: &mut Scene,
        transform: Affine3A,
        min: Vec3,
        max: Vec3,
    ) -> Result<ActorHandle, String> {
        self.add_actor_full(scene, transform, min, max, None)
    }

    /// Register or reuse the scene materials for every surface of this character.
    /// The model cut into one piece per joint: each triangle goes to the joint
    /// that moves it most, and vertices within `cell` metres merge, which is
    /// what makes it cheap. Pieces in the same place do not share vertices,
    /// so joints show small seams when they bend: stiff, but far from a blob.
    pub fn rigid_parts(&self, cell: f32) -> Vec<RigidPart> {
        #[derive(Default)]
        struct Build { key: HashMap<[i32; 3], u32>, sum: Vec<(Vec3, Vec3, f32)>, indices: Vec<u32>, colour: Vec3, area: f32 }
        let mut parts: HashMap<usize, Build> = HashMap::new();
        for surface in &self.surfaces {
            let albedo = |uv: [f32; 2]| -> Vec3 {
                let base = Vec3::from_slice(&surface.material.albedo[..3]);
                let Some((_, image)) = &surface.texture else { return base };
                let (w, h) = image.dimensions();
                let (x, y) = ((uv[0].rem_euclid(1.0) * w as f32) as u32, (uv[1].rem_euclid(1.0) * h as f32) as u32);
                let p = image.get_pixel(x.min(w - 1), y.min(h - 1));
                Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0 * base
            };
            for tri in surface.indices.chunks_exact(3) {
                let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
                let mut weight: HashMap<u32, f32> = HashMap::new();
                for &v in &[a, b, c] {
                    let influence = &surface.influences[v];
                    for (j, w) in influence.joints.iter().zip(&influence.weights) { *weight.entry(*j).or_default() += *w; }
                }
                let Some((&joint, _)) = weight.iter().max_by(|x, y| x.1.total_cmp(y.1).then(y.0.cmp(x.0))) else { continue };
                let part = parts.entry(joint as usize).or_default();
                let at = |v: usize| Vec3::from(surface.vertices[v].pos);
                let area = (at(b) - at(a)).cross(at(c) - at(a)).length() * 0.5;
                let centre = [0, 1].map(|k| (surface.vertices[a].uv[k] + surface.vertices[b].uv[k] + surface.vertices[c].uv[k]) / 3.0);
                part.colour += albedo(centre) * area;
                part.area += area;
                let mut corner = [0u32; 3];
                for (k, &v) in [a, b, c].iter().enumerate() {
                    let pos = at(v);
                    let key = (pos / cell).floor().as_ivec3().to_array();
                    let next = part.sum.len() as u32;
                    let id = *part.key.entry(key).or_insert(next);
                    if id == next { part.sum.push((Vec3::ZERO, Vec3::ZERO, 0.0)); }
                    let s = &mut part.sum[id as usize];
                    s.0 += pos; s.1 += Vec3::from(surface.vertices[v].normal); s.2 += 1.0;
                    corner[k] = id;
                }
                if corner[0] != corner[1] && corner[1] != corner[2] && corner[0] != corner[2] { part.indices.extend(corner); }
            }
        }
        let mut out: Vec<RigidPart> = parts.into_iter().filter(|(_, p)| !p.indices.is_empty()).map(|(joint, p)| {
            let colour = p.colour / p.area.max(1e-6);
            RigidPart { joint, tint: colour.to_array().map(srgb_to_linear),
                positions: p.sum.iter().map(|s| s.0 / s.2).collect(), normals: p.sum.iter().map(|s| s.1.normalize_or(Vec3::Y)).collect(), indices: p.indices }
        }).collect();
        out.sort_by_key(|p| p.joint);
        out
    }

    /// Light `surface` (in manifest order) as skin and/or hair. Call before
    /// adding actors; a mask needs a textured surface.
    pub fn set_skin_hair(&mut self, surface: usize, kind: SkinHair) -> Result<(), String> {
        let count = self.surfaces.len();
        let target = self.surfaces.get_mut(surface)
            .ok_or_else(|| format!("{}: no surface {surface} (of {count})", self.name))?;
        if matches!(kind, SkinHair::PackedMask) && (target.texture.is_none() || target.material.alpha_mode == "MASK") {
            return Err(format!("{}: packed shading mask requires an opaque texture", self.name));
        }
        let kind = match kind {
            SkinHair::Mask(mask) => {
                let Some((_, image)) = &target.texture else {
                    return Err(format!("{}: skin mask on untextured surface {surface}", self.name));
                };
                if target.material.alpha_mode == "MASK" {
                    return Err(format!("{}: surface {surface} uses its alpha for cutouts", self.name));
                }
                let (w, h) = image.dimensions();
                SkinHair::Mask(if mask.dimensions() == (w, h) { mask } else {
                    image::imageops::resize(&mask, w, h, image::imageops::FilterType::Triangle)
                })
            }
            other => other,
        };
        target.skin_hair = Some(kind);
        Ok(())
    }

    pub fn surface_materials(&self, scene: &mut Scene) -> Vec<u32> {
        let mut material_ids = Vec::with_capacity(self.surfaces.len());
        for surface in &self.surfaces {
            let mask = match &surface.skin_hair { Some(SkinHair::Mask(mask)) => Some(mask), _ => None };
            let layer = if let Some((name, image)) = &surface.texture {
                let key = format!(
                    "{name}#normal={:?}:{}{}",
                    surface.material.normal_texture, surface.material.normal_scale,
                    if mask.is_some() { "#skin_hair" } else { "" }
                );
                scene
                    .textures
                    .iter()
                    .position(|t| t.name == key)
                    .map(|i| i as i32)
                    .unwrap_or_else(|| {
                        let mut image = (**image).clone();
                        if let Some(mask) = mask {
                            for (pixel, m) in image.pixels_mut().zip(mask.pixels()) { pixel[3] = m[0]; }
                        }
                        let layer = scene.add_image(&key, image);
                        scene.textures[layer as usize].normal = surface.normal.clone();
                        layer
                    })
            } else {
                -1
            };
            let source = &surface.material;
            let tint = std::array::from_fn(|i| {
                if source.albedo_color_space == "sRGB" {
                    srgb_to_linear(source.albedo[i])
                } else {
                    source.albedo[i]
                }
            });
            let material = Material {
                tint,
                layer,
                roughness: source.roughness,
                flags: if source.two_sided { TWO_SIDED } else { 0 }
                    | if source.alpha_mode == "MASK" {
                        ALPHA_TEST
                    } else {
                        0
                    }
                    | match surface.skin_hair {
                        None => 0,
                        Some(SkinHair::Skin) => SKIN,
                        Some(SkinHair::Hair) => HAIR,
                        Some(SkinHair::Mask(_) | SkinHair::PackedMask) => SKIN | HAIR,
                    },
                scale: 1.0,
                metal: source.metallic,
            };
            let id = scene
                .materials
                .iter()
                .position(|m| {
                    m.tint == material.tint
                        && m.layer == material.layer
                        && m.roughness == material.roughness
                        && m.flags == material.flags
                        && m.scale == material.scale
                        && m.metal == material.metal
                })
                .map(|i| i as u32)
                .unwrap_or_else(|| scene.add_material(material));
            material_ids.push(id);
        }
        material_ids
    }

    /// Extract posed surfaces in `transform` space along with each vertex's
    /// dominant joint index `(material_id, vertices, indices, dominant_joints)`.
    pub fn posed_surfaces(
        &self,
        scene: &mut Scene,
        palette: &[Mat4],
        transform: Affine3A,
    ) -> Vec<(u32, Vec<Vertex>, Vec<u32>, Vec<usize>)> {
        let materials = self.surface_materials(scene);
        self.surfaces
            .iter()
            .zip(materials)
            .map(|(surface, material)| {
                let mut vertices = Vec::with_capacity(surface.vertices.len());
                let mut dominant = Vec::with_capacity(surface.vertices.len());
                for (v, inf) in surface.vertices.iter().zip(&surface.influences) {
                    let mut pos = Vec3::ZERO;
                    let mut nrm = Vec3::ZERO;
                    let mut best_joint = 0usize;
                    let mut best_weight = -1.0f32;
                    for k in 0..8 {
                        let w = inf.weights[k];
                        let j = (inf.joints[k] as usize).min(palette.len().saturating_sub(1));
                        if w > best_weight {
                            best_weight = w;
                            best_joint = j;
                        }
                        if w > 0.0 && j < palette.len() {
                            let m = palette[j];
                            pos += m.transform_point3(Vec3::from(v.pos)) * w;
                            nrm += m.transform_vector3(Vec3::from(v.normal)) * w;
                        }
                    }
                    let world_pos = transform.transform_point3(pos);
                    let world_nrm = transform.transform_vector3(nrm).normalize_or(Vec3::Y);
                    vertices.push(Vertex {
                        pos: world_pos.to_array(),
                        normal: world_nrm.to_array(),
                        uv: v.uv,
                    });
                    dominant.push(best_joint);
                }
                (material, vertices, surface.indices.clone(), dominant)
            })
            .collect()
    }

    fn add_actor_full(&self, scene: &mut Scene, transform: Affine3A, min: Vec3, max: Vec3, material: Option<u32>) -> Result<ActorHandle, String> {
        if !min.is_finite()
            || !max.is_finite()
            || !min.cmple(max).all()
            || !min.cmple(self.min).all()
            || !max.cmpge(self.max).all()
        {
            return Err("actor bounds must be finite, ordered and enclose character bounds".into());
        }
        let center = (min + max) * 0.5;
        let radius = (max - min).length() * 0.5;
        if !center.is_finite() || !radius.is_finite() {
            return Err("actor bounds exceed finite culling sphere capacity".into());
        }
        if scene.skinning.actors.len() >= skinning::MAX_ACTORS || !transform.is_finite() {
            return Err("invalid actor transform or skinning actor capacity exceeded".into());
        }
        let initial = self.sample(&self.clips[0].name, 0.0, self.clips[0].looping)?;
        let material_ids = match material {
            Some(m) => vec![m; self.surfaces.len()],
            None => self.surface_materials(scene),
        };
        let parts: Vec<_> = self
            .surfaces
            .iter()
            .zip(material_ids)
            .map(|(s, material)| Part {
                vertices: &s.vertices,
                indices: &s.indices,
                material,
            })
            .collect();
        let mesh = ClusterMesh::build_fixed(&parts, center.extend(radius).to_array());
        let first_vertex = scene.vertices.len() as u32;
        let vertex_count = mesh.vertices.len() as u32;
        let mesh = scene.add_mesh(&mesh);
        let instance = scene.instances.len();
        scene.add_instance(mesh, transform);
        scene.instances[instance].flags |= SKINNED;
        let actor = scene.skinning.actors.len();
        if let Some(face) = &self.face && material.is_none() {
            scene.hair_grooms.push((actor, face.head, face.groom));
        }
        scene.skinning.actors.push(skinning::Actor {
            first_vertex,
            vertex_count,
            owner: instance as u32,
            palette: initial,
            influences: self
                .surfaces
                .iter()
                .flat_map(|s| s.influences.iter().copied())
                .collect(),
        });
        Ok(ActorHandle {
            actor,
            instance,
            mesh,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose_bounds_character() -> Character {
        Character {
            face: None,
            name: "pose bounds fixture".into(),
            joint_count: 3,
            clips: vec![Clip {
                name: "idle".into(),
                duration: 1.0,
                frames: 2,
                looping: false,
                min: Vec3::ZERO,
                max: Vec3::ONE,
                matrices: vec![Mat4::IDENTITY; 6].into(),
                sockets: Default::default(),
            }],
            min: Vec3::ZERO,
            max: Vec3::ONE,
            triangles: 1,
            surfaces: vec![Surface {
                vertices: [Vec3::ZERO, Vec3::X, Vec3::Y]
                    .map(|pos| Vertex {
                        pos: pos.to_array(),
                        normal: Vec3::Z.to_array(),
                        uv: [0.0; 2],
                    })
                    .to_vec(),
                indices: vec![0, 1, 2],
                influences: vec![
                    Influence {
                        joints: [0; 8],
                        weights: [0.125; 8],
                    };
                    3
                ],
                material: MaterialFile {
                    name: "plain".into(),
                    albedo_texture: None,
                    albedo: [1.0; 4],
                    roughness: 0.8,
                    metallic: 0.0,
                    albedo_color_space: "linear".into(),
                    normal_texture: None,
                    normal_scale: 1.0,
                    two_sided: false,
                    alpha_mode: "OPAQUE".into(),
                },
                texture: None,
                normal: None,
                skin_hair: None,
            }],
            joints: vec![
                ("Root".into(), Mat4::from_translation(Vec3::Y)),
                ("Hand".into(), Mat4::from_translation(Vec3::X)),
                ("Root".into(), Mat4::from_translation(Vec3::Z)),
            ],
        }
    }

    #[test]
    fn skin_and_hair_set_material_flags() {
        let mut character = pose_bounds_character();
        assert!(character.set_skin_hair(0, SkinHair::Mask(image::GrayImage::new(4, 4))).is_err(), "mask needs a texture");
        assert!(character.set_skin_hair(1, SkinHair::Skin).is_err());
        let mut scene = Scene::default();
        let plain = character.surface_materials(&mut scene)[0];
        assert_eq!(scene.materials[plain as usize].flags & (SKIN | HAIR), 0);
        character.set_skin_hair(0, SkinHair::Skin).unwrap();
        let skin = character.surface_materials(&mut scene)[0];
        assert_eq!(scene.materials[skin as usize].flags & (SKIN | HAIR), SKIN);
        character.set_skin_hair(0, SkinHair::Hair).unwrap();
        let hair = character.surface_materials(&mut scene)[0];
        assert_eq!(scene.materials[hair as usize].flags & (SKIN | HAIR), HAIR);
        assert_eq!(render::scene::car_tag(scene.materials[hair as usize].flags), 0);
    }

    #[test]
    fn joints_preserve_palette_order_and_duplicate_names() {
        let character = pose_bounds_character();
        let mut joints = character.joints();
        assert_eq!(joints.len(), character.joint_count);
        assert_eq!(joints.next(), Some(("Root", Mat4::from_translation(Vec3::Y))));
        assert_eq!(joints.next(), Some(("Hand", Mat4::from_translation(Vec3::X))));
        assert_eq!(joints.next(), Some(("Root", Mat4::from_translation(Vec3::Z))));
        assert_eq!(joints.next(), None);
    }

    #[test]
    fn extra_pose_bounds_apply_to_one_actor_without_changing_character() {
        let character = pose_bounds_character();
        let mut scene = Scene::default();
        let original = character.add_actor(&mut scene, Affine3A::IDENTITY).unwrap();
        let original_sphere = scene.meshes[original.mesh].sphere;
        let min = Vec3::new(-4.0, -3.0, -2.0);
        let max = Vec3::new(4.0, 5.0, 6.0);
        let transform = Affine3A::from_scale_rotation_translation(
            Vec3::splat(2.0),
            glam::Quat::IDENTITY,
            Vec3::new(5.0, 2.0, -3.0),
        );
        let extra = character
            .add_actor_with_bounds(&mut scene, transform, min, max)
            .unwrap();
        let after = character.add_actor(&mut scene, Affine3A::IDENTITY).unwrap();
        let sphere = [0.0, 1.0, 2.0, Vec3::splat(4.0).length()];
        assert_eq!(scene.meshes[extra.mesh].sphere, sphere);
        let mesh = &scene.meshes[extra.mesh];
        assert!(mesh.cluster_count > 0);
        for cluster in &scene.clusters
            [mesh.first_cluster as usize..(mesh.first_cluster + mesh.cluster_count) as usize]
        {
            assert_eq!(cluster.cull, sphere);
            assert_eq!(cluster.parent, sphere);
            assert_eq!(cluster.own, sphere);
        }
        assert_eq!(
            scene.instances[extra.instance].sphere,
            [5.0, 4.0, 1.0, sphere[3] * 2.0]
        );
        assert_eq!(scene.skinning.actors[extra.actor].owner, extra.instance as u32);
        assert_eq!(scene.meshes[original.mesh].sphere, original_sphere);
        assert_eq!(scene.meshes[after.mesh].sphere, original_sphere);
        assert_eq!(scene.instances[original.instance].sphere, original_sphere);
        assert_eq!((character.min, character.max), (Vec3::ZERO, Vec3::ONE));
        assert_eq!(
            (character.clips[0].min, character.clips[0].max),
            (Vec3::ZERO, Vec3::ONE)
        );
        assert_eq!(scene.materials.len(), 1);
        scene
            .skinning
            .validate(&scene.vertices, &scene.instances)
            .unwrap();
    }

    #[test]
    fn invalid_extra_pose_bounds_leave_scene_unchanged() {
        let character = pose_bounds_character();
        let mut scene = Scene::default();
        for (min, max) in [
            (Vec3::splat(f32::NAN), Vec3::ONE),
            (Vec3::ZERO, Vec3::splat(f32::INFINITY)),
            (Vec3::ONE, Vec3::ZERO),
            (Vec3::new(0.1, 0.0, 0.0), Vec3::ONE),
            (Vec3::ZERO, Vec3::new(1.0, 1.0, 0.9)),
            (Vec3::splat(-f32::MAX), Vec3::splat(f32::MAX)),
        ] {
            assert!(
                character
                    .add_actor_with_bounds(&mut scene, Affine3A::IDENTITY, min, max)
                    .is_err()
            );
            assert!(scene.vertices.is_empty());
            assert!(scene.clusters.is_empty());
            assert!(scene.meshes.is_empty());
            assert!(scene.instances.is_empty());
            assert!(scene.materials.is_empty());
            assert!(scene.textures.is_empty());
            assert!(scene.skinning.actors.is_empty());
        }
    }

    #[test]
    fn portable_mesh_loads_with_empty_optional_maps_and_independent_actors() {
        let folder = std::env::temp_dir().join(format!(
            "warp-skinning-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(folder.clone());
        let mut mesh = Vec::new();
        for position in [Vec3::ZERO, Vec3::X, Vec3::Y] {
            for value in position
                .to_array()
                .into_iter()
                .chain([0.0, 0.0, 1.0, 0.0, 0.0])
            {
                mesh.extend(value.to_le_bytes());
            }
            for _ in 0..8 {
                mesh.extend(0u32.to_le_bytes());
            }
            for _ in 0..8 {
                mesh.extend(0.125f32.to_le_bytes());
            }
        }
        for index in 0u32..3 {
            mesh.extend(index.to_le_bytes());
        }
        let identity = [
            1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ];
        let mut translated = identity;
        translated[3] = 0.1;
        let animation: Vec<_> = identity
            .into_iter()
            .chain(translated)
            .flat_map(f32::to_le_bytes)
            .collect();
        let manifest = serde_json::json!({"version":1,"name":"fixture","mesh_file":"mesh.bin","animation_file":"animation.bin",
            "vertex_count":3,"index_count":3,"joint_count":1,"vertex_stride_bytes":96,"index_offset_bytes":288,"matrix_stride_bytes":48,
            "surfaces":[{"first_vertex":0,"vertex_count":3,"first_index":0,"index_count":3,"material":{"name":"plain","albedo_texture":"",
                "normal_texture":"","albedo":[1.0,1.0,1.0,1.0],"roughness":0.8,"metallic":0.0,"alpha_mode":"OPAQUE"}}],
            "joints":[{"name":"Root","inverse_bind_rows":identity}],
            "clips":[{"name":"idle","duration":1.0,"frames":2,"looping":false,"matrix_offset_bytes":0,
                "bounds_min":[0.0,0.0,0.0],"bounds_max":[1.1,1.0,0.0]}]});
        std::fs::write(folder.join("mesh.bin"), mesh).unwrap();
        std::fs::write(folder.join("animation.bin"), animation).unwrap();
        std::fs::write(
            folder.join("model.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let character = Character::load(&folder.join("model.json")).unwrap();
        let palette = character.sample("idle", 0.5, false).unwrap();
        assert!(
            character
                .joint_position("Root", &palette)
                .unwrap()
                .abs_diff_eq(Vec3::new(0.05, 0.0, 0.0), 1e-6)
        );
        let mut scene = Scene::default();
        let a = character.add_actor(&mut scene, Affine3A::IDENTITY).unwrap();
        let b = character
            .add_actor(&mut scene, Affine3A::from_translation(Vec3::X))
            .unwrap();
        assert_eq!((a.instance, b.instance), (0, 1));
        assert_ne!(a.mesh, b.mesh);
        assert_ne!(a.actor, b.actor);
        assert_eq!(scene.materials.len(), 1);
        assert!(scene.textures.is_empty());
        assert_eq!(scene.vertices.len(), 6);
        assert_eq!(scene.skinning.actors[0].influences[0].weights, [0.125; 8]);
        scene
            .skinning
            .validate(&scene.vertices, &scene.instances)
            .unwrap();
        let sphere = scene.meshes[a.mesh].sphere;
        let mut reused = vec![Mat4::IDENTITY; character.joint_count];
        let pointer = reused.as_ptr();
        for step in 0..128 {
            let time = step as f32 * 0.017;
            let clip = &character.clips[0];
            let (first, second, t) = frame_pair(clip.frames, clip.duration, time, true);
            character.sample_into(&clip.name, time, true, &mut reused).unwrap();
            for (joint, m) in reused.iter().enumerate() {
                assert_eq!(*m, clip.matrices[first * character.joint_count + joint] * (1.0 - t)
                    + clip.matrices[second * character.joint_count + joint] * t);
            }
            assert_eq!(pointer, reused.as_ptr());
        }
        assert!(character.sample_into(&character.clips[0].name, 0.0, true, &mut []).is_err());
        for source in &character.surfaces[0].vertices {
            let p = skinning::skin_position(
                Vec3::from(source.pos),
                &character.surfaces[0].influences[0],
                &palette,
            );
            assert!(p.distance(Vec3::from_slice(&sphere)) <= sphere[3]);
        }
    }
    #[test]
    fn loop_wrap_and_non_loop_endpoint_are_continuous() {
        assert_eq!(frame_pair(4, 2.0, 2.0, true), (0, 1, 0.0));
        assert_eq!(frame_pair(4, 2.0, 1.75, true), (3, 0, 0.5));
        assert_eq!(frame_pair(5, 2.0, 3.0, false), (4, 4, 0.0));
        assert_eq!(frame_pair(5, 2.0, -1.0, false), (0, 1, 0.0));
    }
    #[test]
    fn row_major_palette_preserves_translation_and_rotation() {
        let matrix = from_rows([0.0, -1.0, 0.0, 3.0, 1.0, 0.0, 0.0, 4.0, 0.0, 0.0, 1.0, 5.0]);
        assert!(
            matrix
                .transform_point3(Vec3::X)
                .abs_diff_eq(Vec3::new(3.0, 5.0, 5.0), 1e-6)
        );
    }
}
