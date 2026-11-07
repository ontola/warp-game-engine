//! Car bodies and wheels from the glTF models (src/car_model.gd): the body
//! in the player's colour, four wheels placed by the suspension. Their
//! materials are procedural car surfaces (car_look.rs, car_material.wgsl).

use crate::car_look::{Kind, LampRole, Looks, Slot};
use glam::{Affine3A, Quat, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, Scene};
use std::collections::HashMap;

pub struct CarMeshes {
    pub body: usize,
    /// FL, FR, RL, RR (the physics' wheel order), each a mesh id.
    pub wheels: [Option<usize>; 4],
}

pub struct Wheel {
    pub anchor: Vec3,
    pub compression: f32,
    pub steer: f32,
    pub spin: f32,
}

/// A glTF material as authored, before it becomes scene materials.
#[derive(Clone)]
struct Authored {
    material: Material,
    /// glTF alpha blend: (alpha, double sided).
    blend: Option<(f32, bool)>,
    part: String,
    kind: Kind,
    /// Colour, roughness and metalness factors.
    factors: [f32; 5],
}

struct Node {
    name: String,
    parts: Vec<(Vec<Vertex>, Vec<u32>, usize)>,
}

/// Imported body-space bounds, measured from the bundled GLBs. A shared
/// lights/glass material also covers windows and front lamps: never tag it
/// wholesale. The selected indices are moved, not copied or offset.
fn rear_lens_role(model:&str, material:&str, vertices:&[Vertex], tri:&[u32]) -> Option<LampRole> {
    if !tri.iter().all(|&i|vertices.get(i as usize).is_some_and(|v|
        v.pos.iter().all(|x|x.is_finite()) && v.pos[2]>=1.65 && (0.24..=0.65).contains(&v.pos[1]))) {return None;}
    match (model,material) {
        ("asti_stradale","UCB_Lights_and_Glass.003")|("mv6","UCB_Lights_and_Glass.002")=>Some(LampRole::RearAtlas),
        ("toyama_xr","Body_Glass_Red.001")=>Some(LampRole::Red),
        ("toyama_xr","Body_Glass.001")=>Some(LampRole::Reverse),
        _=>None,
    }
}

/// min y, max y, min z, max z of some vertices.
fn bounds<'a>(verts: impl Iterator<Item = &'a Vertex>) -> [f32; 4] {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for v in verts {
        lo = lo.min(Vec3::from(v.pos));
        hi = hi.max(Vec3::from(v.pos));
    }
    [lo.y, hi.y, lo.z, hi.z]
}

/// Load a car glTF as car number `car`. Its opaque materials become
/// procedural car surfaces: the paint in `game_colour` unless the data
/// sets another. Each surface's slot is appended to `slots`, in the order
/// of `Scene::car_materials`, so edited data can be applied later.
pub fn load(scene: &mut Scene, path: &std::path::Path, car: usize, game_colour: [f32; 3], looks: &Looks, slots: &mut Vec<Slot>, textures: &mut HashMap<String, i32>) -> Result<CarMeshes, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Unvalidated: the cars use EXT_texture_webp, which gltf 1.4 does not know.
    let g = gltf::Gltf::from_slice_without_validation(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let blob = g.blob.as_deref().ok_or("glb without binary chunk")?;
    let key = path.display().to_string();
    let model = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let mut authored = Vec::new();
    for m in g.materials() {
        let name = m.name().unwrap_or("").to_string();
        let pbr = m.pbr_metallic_roughness();
        let f = pbr.base_color_factor();
        let mut layer = -1;
        if let Some(info) = pbr.base_color_texture() {
            let tex = info.texture();
            // The image is in EXT_texture_webp's source, or the core source.
            let index = tex
                .extension_value("EXT_texture_webp")
                .and_then(|v| v.get("source"))
                .and_then(|v| v.as_u64())
                .map(|i| i as usize)
                .or_else(|| tex.source().map(|s| s.index()))
                .ok_or("texture without image")?;
            let img = g.images().nth(index).ok_or("missing image")?;
            let tkey = format!("{key}#{index}");
            layer = match textures.get(&tkey) {
                Some(l) => *l,
                None => {
                    let data = match img.source() {
                        gltf::image::Source::View { view, .. } => &blob[view.offset()..view.offset() + view.length()],
                        gltf::image::Source::Uri { .. } => return Err("external image in car glb".into()),
                    };
                    let rgba = image::load_from_memory(data).map_err(|e| format!("{tkey}: {e}"))?.to_rgba8();
                    let l = scene.add_image(&tkey, rgba);
                    textures.insert(tkey, l);
                    l
                }
            };
        }
        let (part, kind) = looks.part_for(&name);
        let mut flags = if kind == Kind::Paint { render::scene::CLEARCOAT } else { 0 };
        // The renderer's local ambient proxies represent opaque bodywork.
        // Glass/alpha coverage cannot safely be filled as a solid box.
        if m.alpha_mode() != gltf::material::AlphaMode::Opaque || f[3] < 0.999 {
            flags |= render::scene::NO_AMBIENT_OCCLUSION;
        }
        if m.double_sided() { flags |= render::scene::TWO_SIDED; }
        if m.alpha_mode() == gltf::material::AlphaMode::Mask { flags |= render::scene::ALPHA_TEST; }
        let blend = (m.alpha_mode() == gltf::material::AlphaMode::Blend).then_some((f[3], m.double_sided()));
        // Paint is generated, never textured. Untextured parts take their
        // colour from the data, which car_material.wgsl multiplies in.
        let layer = if kind == Kind::Paint { -1 } else { layer };
        let tint = if kind != Kind::Textured && layer < 0 { [1.0; 3] } else { [f[0], f[1], f[2]] };
        let material = Material { tint, layer, roughness: pbr.roughness_factor(), flags, scale: 1.0, metal: pbr.metallic_factor() };
        authored.push(Authored { material, blend, part, kind, factors: [f[0], f[1], f[2], pbr.roughness_factor(), pbr.metallic_factor()] });
    }
    // Geometry first: the body's bounds and the wheel positions tell the
    // shader where grime collects.
    let mut nodes = Vec::new();
    let mut axles = Vec::new();
    for node in g.nodes() {
        let Some(mesh) = node.mesh() else { continue };
        let name = node.name().unwrap_or("").to_string();
        // Wheels are built around their own centre; the body in car space.
        let (t, r, s) = node.transform().decomposed();
        if name.starts_with("wheel") { axles.push(Vec3::from(t)); }
        let local = if name.starts_with("wheel") { Affine3A::from_scale_rotation_translation(Vec3::from(s), Quat::from_array(r), Vec3::ZERO) } else { Affine3A::from_scale_rotation_translation(Vec3::from(s), Quat::from_array(r), Vec3::from(t)) };
        let mut parts = Vec::new();
        for prim in mesh.primitives() {
            let reader = prim.reader(|_| Some(blob));
            let Some(pos) = reader.read_positions() else { continue };
            let pos: Vec<[f32; 3]> = pos.collect();
            let nrm: Vec<[f32; 3]> = reader.read_normals().map(|n| n.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; pos.len()]);
            let uv: Vec<[f32; 2]> = reader.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; pos.len()]);
            let idx: Vec<u32> = reader.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..pos.len() as u32).collect());
            let verts: Vec<Vertex> = pos
                .iter()
                .zip(&nrm)
                .zip(&uv)
                .map(|((p, n), t)| Vertex { pos: local.transform_point3(Vec3::from(*p)).to_array(), normal: local.transform_vector3(Vec3::from(*n)).normalize_or(Vec3::Y).to_array(), uv: *t })
                .collect();
            let gi=prim.material().index().unwrap_or(0);
            let mut ordinary=Vec::new();let mut lens=Vec::new();let mut role=None;
            for tri in idx.chunks_exact(3) {
                let selected=if name.starts_with("body") {rear_lens_role(&model,prim.material().name().unwrap_or(""),&verts,tri)} else {None};
                if let Some(selected)=selected {role=Some(selected);lens.extend_from_slice(tri);}else{ordinary.extend_from_slice(tri);}
            }
            if let Some(role)=role {
                let mut a=authored.get(gi).ok_or("lamp primitive without material")?.clone();
                a.part=role.part().into();a.kind=Kind::Lamp;
                let lamp_id=authored.len();authored.push(a);
                // Both sets retain exact source positions, normals and UVs;
                // their triangle counts sum to the original primitive count.
                parts.push((verts.clone(),lens,lamp_id));
            }
            if !ordinary.is_empty(){parts.push((verts,ordinary,gi));}
        }
        nodes.push(Node { name, parts });
    }
    let body_bounds = nodes.iter().find(|n| n.name.starts_with("body")).map(|n| bounds(n.parts.iter().flat_map(|p| &p.0))).unwrap_or([-0.4, 1.0, -2.0, 2.0]);
    let radius = nodes.iter().filter(|n| n.name.starts_with("wheel")).flat_map(|n| n.parts.iter().flat_map(|p| &p.0)).map(|v| v.pos[1].hypot(v.pos[2])).fold(0.0f32, f32::max);
    let radius = if radius > 0.0 { radius } else { 0.33 };
    let body_axles = if axles.is_empty() {
        [-1.3, 1.3, radius, 0.8]
    } else {
        let front = axles.iter().map(|a| a.z).fold(f32::INFINITY, f32::min);
        let rear = axles.iter().map(|a| a.z).fold(f32::NEG_INFINITY, f32::max);
        [front, rear, radius, axles.iter().map(|a| a.x.abs()).sum::<f32>() / axles.len() as f32]
    };
    // One scene material per glTF material and use (body or wheel).
    let mut made: HashMap<(usize, bool), u32> = HashMap::new();
    let mut body = None;
    let mut wheels = [None; 4];
    for mut node in nodes {
        let wheel = node.name.starts_with("wheel");
        let paint: Vec<bool> = node.parts.iter().map(|p| authored.get(p.2).is_some_and(|a| a.kind == Kind::Paint)).collect();
        if paint.contains(&true) {
            // Paint has no texture: its uv carries the baked curvature.
            let curv = curvature(&node.parts.iter().map(|p| (&p.0[..], &p.1[..])).collect::<Vec<_>>());
            for ((part, is_paint), c) in node.parts.iter_mut().zip(&paint).zip(curv) {
                if *is_paint {
                    for (v, k) in part.0.iter_mut().zip(c) {
                        v.uv = k;
                    }
                }
            }
        }
        let wheel_bounds = bounds(node.parts.iter().flat_map(|p| &p.0));
        let mut parts = Vec::new();
        for (verts, idx, gi) in &node.parts {
            let Some(a) = authored.get(*gi) else { continue };
            let id = match made.get(&(*gi, wheel)) {
                Some(id) => *id,
                None => {
                    let mut material = a.material;
                    // Glass keeps the forward pass's own shading.
                    if a.blend.is_none() || a.kind==Kind::Lamp {
                        let slot = Slot {
                            car,
                            model: model.clone(),
                            game_colour,
                            part: a.part.clone(),
                            gltf: a.factors,
                            body: if wheel { wheel_bounds } else { body_bounds },
                            axles: if wheel { [0.0, 0.0, radius, -1.0] } else { body_axles },
                        };
                        material.flags |= scene.add_car_material(looks.material(&slot)?);
                        slots.push(slot);
                    }
                    let id = scene.add_material(material);
                    if let Some((alpha, two_sided)) = a.blend {
                        scene.set_material_blend(id, alpha, two_sided);
                    }
                    made.insert((*gi, wheel), id);
                    id
                }
            };
            parts.push(Part { vertices: verts, indices: idx, material: id });
        }
        let id = scene.add_mesh(&ClusterMesh::build(&parts));
        let slot = ["wheel_FL", "wheel_FR", "wheel_RL", "wheel_RR"].iter().position(|w| node.name.starts_with(w));
        match slot {
            Some(k) => wheels[k] = Some(id),
            None if node.name.starts_with("body") => body = Some(id),
            None => {} // calipers: later, with the knuckles
        }
    }
    Ok(CarMeshes { body: body.ok_or("car glb has no body")?, wheels })
}

/// Signed mean curvature per vertex (1/m, convex positive) of a mesh made
/// of several parts: each edge's dihedral angle times its length, over the
/// vertex's area (Meyer et al. 2003), on positions welded across parts and
/// hard edges. An open border counts as a convex right angle: panel edges
/// are where paint chips. Returns (lightly, widely smoothed) per vertex.
pub fn curvature(parts: &[(&[Vertex], &[u32])]) -> Vec<Vec<[f32; 2]>> {
    let mut weld: HashMap<[i64; 3], u32> = HashMap::new();
    let mut points: Vec<Vec3> = Vec::new();
    let ids: Vec<Vec<u32>> = parts
        .iter()
        .map(|(v, _)| {
            v.iter()
                .map(|v| {
                    let k = v.pos.map(|x| (x * 1e4).round() as i64);
                    *weld.entry(k).or_insert_with(|| {
                        points.push(Vec3::from(v.pos));
                        points.len() as u32 - 1
                    })
                })
                .collect()
        })
        .collect();
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for ((_, idx), id) in parts.iter().zip(&ids) {
        for t in idx.chunks_exact(3) {
            let t = [id[t[0] as usize], id[t[1] as usize], id[t[2] as usize]];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                tris.push(t);
            }
        }
    }
    let n = points.len();
    let mut area = vec![0.0f32; n];
    let mut normals = Vec::with_capacity(tris.len());
    let mut edges: HashMap<(u32, u32), Vec<(usize, u32)>> = HashMap::new();
    for (f, t) in tris.iter().enumerate() {
        let c = (points[t[1] as usize] - points[t[0] as usize]).cross(points[t[2] as usize] - points[t[0] as usize]);
        normals.push(c.normalize_or_zero());
        for k in 0..3 {
            area[t[k] as usize] += c.length() / 6.0;
            let (a, b) = (t[k], t[(k + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push((f, t[(k + 2) % 3]));
        }
    }
    let mut sum = vec![0.0f32; n];
    let mut next: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (&(a, b), faces) in &edges {
        next[a as usize].push(b);
        next[b as usize].push(a);
        let len = points[a as usize].distance(points[b as usize]);
        let angle = match faces[..] {
            [(f, _), (g, opposite)] => {
                let theta = normals[f].dot(normals[g]).clamp(-1.0, 1.0).acos();
                // Convex when the other face folds away below this one.
                if normals[f].dot(points[opposite as usize] - points[a as usize]) < 0.0 { theta } else { -theta }
            }
            [_] => std::f32::consts::FRAC_PI_2,
            _ => 0.0,
        };
        sum[a as usize] += angle * len;
        sum[b as usize] += angle * len;
    }
    let raw: Vec<f32> = (0..n).map(|i| if area[i] > 0.0 { sum[i] / (4.0 * area[i]) } else { 0.0 }).collect();
    let smooth = |field: &[f32]| -> Vec<f32> {
        (0..n).map(|i| if next[i].is_empty() { field[i] } else { 0.5 * field[i] + 0.5 * next[i].iter().map(|j| field[*j as usize]).sum::<f32>() / next[i].len() as f32 }).collect()
    };
    let sharp = smooth(&raw);
    let mut broad = sharp.clone();
    for _ in 0..4 {
        broad = smooth(&broad);
    }
    ids.iter().map(|id| id.iter().map(|&w| [sharp[w as usize], broad[w as usize]]).collect()).collect()
}

/// Body first, followed by the wheels that the model contains, in suspension order.
fn parts<'a>(m: &'a CarMeshes, ride: f32, wheels: &'a [Wheel]) -> impl Iterator<Item = (usize, Affine3A)> + 'a {
    let body = (m.body, Affine3A::from_translation(Vec3::Y * ride));
    std::iter::once(body).chain(m.wheels.iter().zip(wheels).filter_map(|(&mesh, w)| {
        let rotation = Quat::from_rotation_y(w.steer) * Quat::from_rotation_x(-w.spin);
        Some((mesh?, Affine3A::from_rotation_translation(rotation, w.anchor + Vec3::Y * w.compression)))
    }))
}

pub fn place(scene: &mut Scene, m: &CarMeshes, pose: Affine3A, ride: f32, wheels: &[Wheel]) {
    for (i, (mesh, local)) in parts(m, ride, wheels).enumerate() {
        scene.add_instance(mesh, pose * local);
        if i == 0 { scene.mark_solid(); }
    }
}

pub fn update(scene: &mut Scene, first: usize, m: &CarMeshes, pose: Affine3A, ride: f32, wheels: &[Wheel]) {
    scene.update_children(first, pose, parts(m, ride, wheels));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspension_pose_matches_the_original_slots_with_a_missing_wheel() {
        let mut scene = Scene::default();
        let mesh = scene.add_mesh(&render::primitives::cube(0).mesh());
        let car = CarMeshes { body: mesh, wheels: [Some(mesh), None, Some(mesh), Some(mesh)] };
        let wheels = [0.2, 0.6, 1.3, -0.1].map(|f| Wheel {
            anchor: Vec3::new(f, 0.5, -f), compression: f * 0.1, steer: f, spin: f * 2.0,
        });
        let root = Affine3A::from_rotation_translation(Quat::from_rotation_z(0.4), Vec3::new(11.0, 2.0, -5.0));
        place(&mut scene, &car, root, 0.12, &wheels);
        for pose in [root, root * root] {
            if pose != root { update(&mut scene, 0, &car, pose, 0.12, &wheels); }
            let mut expected = vec![pose * Affine3A::from_translation(Vec3::Y * 0.12)];
            for i in [0, 2, 3] {
                let w = &wheels[i];
                expected.push(pose * Affine3A::from_rotation_translation(
                    Quat::from_rotation_y(w.steer) * Quat::from_rotation_x(-w.spin), w.anchor + Vec3::Y * w.compression));
            }
            assert_eq!(scene.instances.len(), expected.len());
            for (i, xf) in expected.into_iter().enumerate() {
                let rows = glam::Mat4::from(xf).transpose();
                assert_eq!(scene.instances[i].rows, [rows.x_axis.to_array(), rows.y_axis.to_array(), rows.z_axis.to_array()]);
                assert_eq!(scene.instances[i].flags & render::scene::SOLID != 0, i == 0);
            }
        }
    }

    #[test]
    fn rear_lenses_are_original_triangles_and_do_not_include_windows_or_fronts() {
        for (model,expected) in [("asti_stradale",[252,0,0]),("mv6",[76,0,0]),("toyama_xr",[0,276,144])] {
            let path=sideways::game_assets::asset("cars").join(format!("{model}.glb"));
            let bytes=std::fs::read(path).unwrap();let glb=gltf::Gltf::from_slice_without_validation(&bytes).unwrap();let blob=glb.blob.as_deref().unwrap();
            let node=glb.nodes().find(|n|n.name().is_some_and(|s|s.starts_with("body"))).unwrap();
            let (t,r,s)=node.transform().decomposed();let xf=Affine3A::from_scale_rotation_translation(Vec3::from(s),Quat::from_array(r),Vec3::from(t));
            let mut counts=[0;3];let mut sides=[[0;2];3];
            for prim in node.mesh().unwrap().primitives() {
                let reader=prim.reader(|_|Some(blob));
                let vertices:Vec<_>=reader.read_positions().unwrap().map(|p|Vertex{pos:xf.transform_point3(Vec3::from(p)).to_array(),normal:[0.0;3],uv:[0.0;2]}).collect();
                let indices:Vec<_>=reader.read_indices().unwrap().into_u32().collect();
                let mut selected=0;let mut ordinary=0;
                for tri in indices.chunks_exact(3) {
                    if let Some(role)=rear_lens_role(model,prim.material().name().unwrap_or(""),&vertices,tri) {
                        counts[role as usize]+=1;selected+=1;
                        let x=tri.iter().map(|i|vertices[*i as usize].pos[0]).sum::<f32>();sides[role as usize][usize::from(x>0.0)]+=1;
                    }else{ordinary+=1;}
                }
                assert_eq!((selected+ordinary)*3,indices.len(),"source triangles lost or duplicated");
            }
            assert_eq!(counts,expected,"{model}: source lens mapping changed");
            for (n,side) in counts.into_iter().zip(sides) {if n>0{assert!(side.iter().all(|n|*n>0));}}
        }
        let vertex=|z,y|Vertex{pos:[0.5,y,z],normal:[0.0;3],uv:[0.0;2]};
        for (model,material) in [("asti_stradale","UCB_Lights_and_Glass.003"),("toyama_xr","Body_Glass.001")] {
            assert!(rear_lens_role(model,material,&[vertex(-1.9,0.4);3],&[0,1,2]).is_none());
            assert!(rear_lens_role(model,material,&[vertex(1.4,0.9);3],&[0,1,2]).is_none());
        }
        assert!(rear_lens_role("custom","UCB_Lights_and_Glass.003",&[vertex(1.8,0.4);3],&[0,1,2]).is_none());
    }

    /// A cube's edges are convex; turned inside out they are concave.
    #[test]
    fn curvature_sign_follows_convexity() {
        let corners: Vec<Vec3> = (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
        // Outward-facing, counter-clockwise quads.
        let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let verts: Vec<Vertex> = corners.iter().map(|p| Vertex { pos: p.to_array(), normal: [0.0; 3], uv: [0.0; 2] }).collect();
        let mut idx = Vec::new();
        for f in faces {
            idx.extend([f[0], f[1], f[2], f[0], f[2], f[3]]);
        }
        let k = curvature(&[(&verts, &idx)]);
        assert!(k[0].iter().all(|c| c[0] > 0.5), "{k:?}");
        let flipped: Vec<u32> = idx.chunks(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
        let k = curvature(&[(&verts, &flipped)]);
        assert!(k[0].iter().all(|c| c[0] < -0.5), "{k:?}");
    }
}
