//! Static glTF 2.0 models (.glb or .gltf with external .bin and images)
//! into a scene: every mesh node, with the node hierarchy's transforms,
//! becomes one clustered mesh in model space; materials map to Warp
//! materials (base colour, roughness, metalness, double sided, alpha mask)
//! with base-colour and normal textures. Skinning and animation are not
//! read. FBX: convert to glTF first (Blender or FBX2glTF).
//!
//! ```ignore
//! let model = assets::gltf::load(&mut scene, Path::new("stump.gltf"))?;
//! scene.add_instance(model.mesh, Affine3A::from_translation(at));
//! ```
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, Scene, ALPHA_TEST, TWO_SIDED};
use glam::{Mat4, Vec3};
use std::path::Path;

pub struct Model {
    /// The whole model as one scene mesh, in model space.
    pub mesh: usize,
    /// Axis-aligned bounds in model space.
    pub min: Vec3,
    pub max: Vec3,
    pub triangles: usize,
    /// Warp material per glTF material, in glTF order.
    pub materials: Vec<u32>,
}

fn read_uri(base: &Path, uri: &str) -> Result<Vec<u8>, String> {
    if uri.starts_with("data:") {
        let b64 = uri.split_once(";base64,").ok_or("unsupported data URI")?.1;
        return base64_decode(b64);
    }
    let path = base.join(uri.replace("%20", " "));
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let val = |c: u8| -> Option<u32> { Some(match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' | b'-' => 62, b'/' | b'_' => 63, _ => return None } as u32) };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() { continue; }
        acc = (acc << 6) | val(c).ok_or("bad base64")?;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); }
    }
    Ok(out)
}

/// Load a glTF file into `scene` (materials, textures and one mesh).
pub fn load(scene: &mut Scene, path: &Path) -> Result<Model, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let g = ::gltf::Gltf::from_slice_without_validation(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut buffers = Vec::new();
    for b in g.buffers() {
        buffers.push(match b.source() {
            ::gltf::buffer::Source::Bin => g.blob.clone().ok_or("glb without binary chunk")?,
            ::gltf::buffer::Source::Uri(uri) => read_uri(base, uri)?,
        });
    }
    let image = |index: usize| -> Result<image::RgbaImage, String> {
        let img = g.images().nth(index).ok_or("missing image")?;
        let data = match img.source() {
            ::gltf::image::Source::View { view, .. } => {
                let b = &buffers[view.buffer().index()];
                b[view.offset()..view.offset() + view.length()].to_vec()
            }
            ::gltf::image::Source::Uri { uri, .. } => read_uri(base, uri)?,
        };
        Ok(image::load_from_memory(&data).map_err(|e| format!("image {index}: {e}"))?.to_rgba8())
    };
    let key = path.display().to_string();
    let mut materials = Vec::new();
    for m in g.materials() {
        let pbr = m.pbr_metallic_roughness();
        let f = pbr.base_color_factor();
        let mut layer = -1;
        if let Some(info) = pbr.base_color_texture() {
            if let Some(src) = info.texture().source().map(|s| s.index()) {
                let name = format!("{key}#{src}");
                layer = match scene.textures.iter().position(|t| t.name == name) {
                    Some(l) => l as i32,
                    None => {
                        let l = scene.add_image(&name, image(src)?);
                        // A normal map rides along with its albedo layer.
                        if let Some(n) = m.normal_texture().and_then(|n| n.texture().source()).map(|s| s.index()) {
                            if let Ok(nrm) = image(n) { scene.textures[l as usize].normal = Some(nrm); }
                        }
                        l
                    }
                };
            }
        }
        let mut flags = 0;
        if m.double_sided() { flags |= TWO_SIDED; }
        if m.alpha_mode() == ::gltf::material::AlphaMode::Mask { flags |= ALPHA_TEST; }
        materials.push(scene.add_material(Material { tint: [f[0], f[1], f[2]], layer, roughness: pbr.roughness_factor(), flags, scale: 1.0, metal: pbr.metallic_factor() }));
    }
    let fallback = scene.add_material(Material { tint: [0.8; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
    // Walk every scene's node tree with accumulated transforms.
    let mut stack: Vec<(::gltf::Node, Mat4)> = g.default_scene().or_else(|| g.scenes().next())
        .map(|s| s.nodes().map(|n| (n, Mat4::IDENTITY)).collect()).unwrap_or_default();
    let mut owned: Vec<(Vec<Vertex>, Vec<u32>, u32)> = Vec::new();
    let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    while let Some((node, parent)) = stack.pop() {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        let normal_m = world.inverse().transpose();
        if let Some(mesh) = node.mesh() {
            for prim in mesh.primitives() {
                if prim.mode() != ::gltf::mesh::Mode::Triangles { continue; }
                let reader = prim.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
                let Some(pos) = reader.read_positions() else { continue };
                let pos: Vec<[f32; 3]> = pos.collect();
                let nrm: Vec<[f32; 3]> = reader.read_normals().map(|n| n.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; pos.len()]);
                let uv: Vec<[f32; 2]> = reader.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; pos.len()]);
                let mut idx: Vec<u32> = reader.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..pos.len() as u32).collect());
                // A mirrored node flips winding.
                if world.determinant() < 0.0 { for t in idx.chunks_exact_mut(3) { t.swap(1, 2); } }
                let verts: Vec<Vertex> = pos.iter().zip(&nrm).zip(&uv).map(|((p, n), t)| {
                    let p = world.transform_point3(Vec3::from(*p));
                    min = min.min(p);
                    max = max.max(p);
                    Vertex { pos: p.to_array(), normal: normal_m.transform_vector3(Vec3::from(*n)).normalize_or(Vec3::Y).to_array(), uv: *t }
                }).collect();
                let material = prim.material().index().and_then(|i| materials.get(i).copied()).unwrap_or(fallback);
                owned.push((verts, idx, material));
            }
        }
        for child in node.children() { stack.push((child, world)); }
    }
    if owned.is_empty() { return Err(format!("{}: no triangle meshes", path.display())); }
    let triangles = owned.iter().map(|p| p.1.len() / 3).sum();
    let parts: Vec<Part> = owned.iter().map(|(v, i, m)| Part { vertices: v, indices: i, material: *m }).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&parts));
    Ok(Model { mesh, min, max, triangles, materials })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("AAEC").unwrap(), [0, 1, 2]);
    }

    #[test]
    fn loads_a_gltf_with_external_buffer_and_textures() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets/props/tree_stump_01/tree_stump_01.gltf");
        if !path.exists() { return; }
        let mut scene = Scene::default();
        let m = load(&mut scene, &path).unwrap();
        assert!(m.triangles > 1000, "{}", m.triangles);
        assert!(m.max.y > m.min.y && m.max.y < 5.0, "{:?} {:?}", m.min, m.max);
        assert_eq!(scene.textures.len(), 1);
        assert!(scene.textures[0].normal.is_some(), "normal map attached");
        assert!(scene.materials[m.materials[0] as usize].layer >= 0);
    }
}
