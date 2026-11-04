//! Offline: a skinned glTF/GLB into the portable character format that
//! `assets::skinned::Character` loads (JSON, mesh.bin, anim.bin, textures).
//! Meshes parented to a bone (weapons, hats) ride that bone. glTF is already
//! right-handed, +Y up, counter-clockwise, top-left UVs, as the format wants.
//!
//! List what a file holds:
//!   cargo run -p assets --example gltf_character -- in.glb
//! Convert it, baking clips at 30 fps (`!` marks a clip that plays once):
//!   cargo run -p assets --example gltf_character -- in.glb --out DIR --name skeleton
//!     --clip idle=Idle --clip die=Death_A! --hide Helmet --scale 1.0
//!     --attach Blade.gltf=handslot.r
use glam::{Mat4, Quat, Vec3};
use gltf::animation::util::ReadOutputs;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

const FPS: f32 = 30.0;

#[derive(Clone, Copy)]
struct Trs {
    t: Vec3,
    r: Quat,
    s: Vec3,
}

impl Trs {
    fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.s, self.r, self.t)
    }
}

enum Keys {
    T(Vec<Vec3>),
    R(Vec<Quat>),
    S(Vec<Vec3>),
}

struct Channel {
    node: usize,
    times: Vec<f32>,
    keys: Keys,
    step: bool,
    cubic: bool,
}

impl Channel {
    /// The keys around `t` and how far between them.
    fn span(&self, t: f32) -> (usize, usize, f32) {
        let n = self.times.len();
        if t <= self.times[0] || n == 1 {
            return (0, 0, 0.0);
        }
        if t >= self.times[n - 1] {
            return (n - 1, n - 1, 0.0);
        }
        let b = self.times.partition_point(|&x| x <= t);
        let a = b - 1;
        let k = (t - self.times[a]) / (self.times[b] - self.times[a]).max(1e-6);
        (a, b, if self.step { 0.0 } else { k })
    }

    fn apply(&self, t: f32, pose: &mut [Trs]) {
        let (a, b, k) = self.span(t);
        // Cubic splines store (in tangent, value, out tangent) per key; the
        // values alone are close enough at 30 fps.
        let at = |i: usize| if self.cubic { i * 3 + 1 } else { i };
        let p = &mut pose[self.node];
        match &self.keys {
            Keys::T(v) => p.t = v[at(a)].lerp(v[at(b)], k),
            Keys::R(v) => p.r = v[at(a)].slerp(v[at(b)], k).normalize(),
            Keys::S(v) => p.s = v[at(a)].lerp(v[at(b)], k),
        }
    }
}

fn rows(m: Mat4) -> [f32; 12] {
    let r = m.transpose().to_cols_array();
    std::array::from_fn(|i| r[i])
}

fn globals(pose: &[Trs], order: &[usize], parent: &[Option<usize>], root: Mat4) -> Vec<Mat4> {
    let mut g = vec![Mat4::IDENTITY; pose.len()];
    for &i in order {
        g[i] = parent[i].map_or(root, |p| g[p]) * pose[i].matrix();
    }
    g
}

fn put(out: &mut Vec<u8>, floats: &[f32]) {
    for f in floats {
        out.extend_from_slice(&f.to_le_bytes());
    }
}

struct Vert {
    pos: Vec3,
    normal: Vec3,
    uv: [f32; 2],
    joints: [u32; 8],
    weights: [f32; 8],
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let input = PathBuf::from(args.first().ok_or("usage: gltf_character in.glb [--out DIR --name NAME --clip out=Source[!] --hide NODE --scale S]")?);
    let (mut out_dir, mut name, mut scale) = (None, String::new(), 1.0f32);
    let (mut clips, mut hide): (Vec<(String, String, bool)>, Vec<String>) = (vec![], vec![]);
    let mut attach: Vec<(PathBuf, String)> = vec![];
    let mut i = 1;
    while i < args.len() {
        let value = args.get(i + 1).cloned().ok_or("flag without value")?;
        match args[i].as_str() {
            "--out" => out_dir = Some(PathBuf::from(value)),
            "--name" => name = value,
            "--scale" => scale = value.parse().map_err(|_| "bad --scale")?,
            "--hide" => hide.push(value),
            "--attach" => {
                let (file, bone) = value.rsplit_once('=').ok_or("--attach file.gltf=bone")?;
                attach.push((file.into(), bone.into()));
            }
            "--clip" => {
                let (to, from) = value.split_once('=').ok_or("--clip out=Source")?;
                let once = from.ends_with('!');
                clips.push((to.into(), from.trim_end_matches('!').into(), !once));
            }
            f => return Err(format!("unknown flag {f}")),
        }
        i += 2;
    }

    let bytes = std::fs::read(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let g = gltf::Gltf::from_slice_without_validation(&bytes).map_err(|e| e.to_string())?;
    let base = input.parent().map(PathBuf::from).unwrap_or_default();
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for b in g.buffers() {
        buffers.push(match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone().ok_or("glb without binary chunk")?,
            gltf::buffer::Source::Uri(u) => std::fs::read(base.join(u)).map_err(|e| format!("{u}: {e}"))?,
        });
    }
    let data = |b: gltf::Buffer| buffers.get(b.index()).map(|v| v.as_slice());

    let count = g.nodes().count();
    let mut parent = vec![None; count];
    for n in g.nodes() {
        for c in n.children() {
            parent[c.index()] = Some(n.index());
        }
    }
    let mut order: Vec<usize> = Vec::new();
    let mut seen = vec![false; count];
    while order.len() < count {
        for n in 0..count {
            if !seen[n] && parent[n].is_none_or(|p| seen[p]) {
                seen[n] = true;
                order.push(n);
            }
        }
    }
    let rest: Vec<Trs> = g
        .nodes()
        .map(|n| {
            let (t, r, s) = n.transform().decomposed();
            Trs { t: t.into(), r: Quat::from_array(r), s: s.into() }
        })
        .collect();

    if out_dir.is_none() {
        for n in g.nodes() {
            if n.mesh().is_some() {
                println!("mesh node {:?} parent {:?} skin {}", n.name().unwrap_or(""), parent[n.index()].and_then(|p| g.nodes().nth(p)).and_then(|p| p.name().map(String::from)), n.skin().is_some());
            }
        }
        for s in g.skins() {
            println!("skin: {} joints", s.joints().count());
        }
        for a in g.animations() {
            let end = a.channels().filter_map(|c| c.reader(data).read_inputs()).flatten().fold(0.0f32, f32::max);
            println!("clip {:?} {end:.2}s", a.name().unwrap_or(""));
        }
        return Ok(());
    }
    let out_dir = out_dir.unwrap();
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    if name.is_empty() {
        return Err("--name".into());
    }

    let root = Mat4::from_scale(Vec3::splat(scale));
    let rest_globals = globals(&rest, &order, &parent, root);
    // (joint node, inverse bind) -> palette slot.
    let mut palette: Vec<(usize, Mat4)> = Vec::new();
    let mut slot = |node: usize, inverse: Mat4| -> u32 {
        if let Some(i) = palette.iter().position(|p| p.0 == node && p.1.abs_diff_eq(inverse, 1e-5)) {
            return i as u32;
        }
        palette.push((node, inverse));
        palette.len() as u32 - 1
    };
    let mut verts: Vec<Vert> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut surfaces = Vec::new();
    let mut textures: HashMap<usize, String> = HashMap::new();
    for n in g.nodes() {
        let Some(mesh) = n.mesh() else { continue };
        let node_name = n.name().unwrap_or("");
        if hide.iter().any(|h| h == node_name) {
            continue;
        }
        let bind: Option<Vec<(u32, Mat4)>> = n.skin().map(|skin| {
            let reader = skin.reader(data);
            let ibm: Vec<Mat4> = reader
                .read_inverse_bind_matrices()
                .map(|m| m.map(|c| Mat4::from_cols_array_2d(&c)).collect())
                .unwrap_or_default();
            skin.joints()
                .enumerate()
                .map(|(i, j)| {
                    // The root scale goes round the skin: scaled joints, and
                    // bind space scaled to match.
                    let inv = ibm.get(i).copied().unwrap_or(Mat4::IDENTITY) * root.inverse();
                    (slot(j.index(), inv), inv)
                })
                .collect()
        });
        let rigid = rest_globals[n.index()];
        let rigid_slot = if bind.is_none() { slot(n.index(), rigid.inverse()) } else { 0 };
        for prim in mesh.primitives() {
            let reader = prim.reader(data);
            let positions: Vec<[f32; 3]> = reader.read_positions().ok_or("no positions")?.collect();
            let normals: Vec<[f32; 3]> = reader.read_normals().map(|r| r.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
            let uvs: Vec<[f32; 2]> = reader.read_tex_coords(0).map(|r| r.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; positions.len()]);
            let joints: Vec<[u16; 4]> = reader.read_joints(0).map(|r| r.into_u16().collect()).unwrap_or_default();
            let weights: Vec<[f32; 4]> = reader.read_weights(0).map(|r| r.into_f32().collect()).unwrap_or_default();
            let first_vertex = verts.len();
            let first_index = indices.len();
            for v in 0..positions.len() {
                let (mut js, mut ws) = ([0u32; 8], [0f32; 8]);
                let (pos, normal);
                match &bind {
                    Some(b) if v < joints.len() => {
                        // Bind space: where the inverse binds take it.
                        pos = root.transform_point3(positions[v].into());
                        normal = Vec3::from(normals[v]).normalize_or(Vec3::Y);
                        let sum: f32 = weights[v].iter().sum::<f32>().max(1e-6);
                        for k in 0..4 {
                            js[k] = b[joints[v][k] as usize].0;
                            ws[k] = weights[v][k] / sum;
                        }
                    }
                    _ => {
                        pos = rigid.transform_point3(positions[v].into());
                        normal = rigid.transform_vector3(normals[v].into()).normalize_or(Vec3::Y);
                        js[0] = rigid_slot;
                        ws[0] = 1.0;
                    }
                }
                verts.push(Vert { pos, normal, uv: uvs[v], joints: js, weights: ws });
            }
            let local: Vec<u32> = match reader.read_indices() {
                Some(r) => r.into_u32().collect(),
                None => (0..positions.len() as u32).collect(),
            };
            indices.extend(local.iter().map(|i| i + first_vertex as u32));
            let m = prim.material();
            let pbr = m.pbr_metallic_roughness();
            let texture = match pbr.base_color_texture() {
                Some(info) => {
                    let image = info.texture().source().ok_or("texture without image")?;
                    let id = image.index();
                    if !textures.contains_key(&id) {
                        let file = format!("{name}_{id}.png");
                        let raw = match image.source() {
                            gltf::image::Source::View { view, .. } => {
                                let b = &buffers[view.buffer().index()];
                                b[view.offset()..view.offset() + view.length()].to_vec()
                            }
                            gltf::image::Source::Uri { uri, .. } => std::fs::read(base.join(uri)).map_err(|e| format!("{uri}: {e}"))?,
                        };
                        let decoded = image::load_from_memory(&raw).map_err(|e| e.to_string())?;
                        decoded.save(out_dir.join(&file)).map_err(|e| e.to_string())?;
                        textures.insert(id, file);
                    }
                    textures[&id].clone()
                }
                None => String::new(),
            };
            surfaces.push(json!({
                "name": format!("{node_name}/{}", prim.index()),
                "first_vertex": first_vertex, "vertex_count": positions.len(),
                "first_index": first_index, "index_count": local.len(),
                "material": {
                    "name": m.name().unwrap_or(""), "albedo_texture": texture,
                    "albedo": pbr.base_color_factor(), "albedo_color_space": "linear",
                    "roughness": pbr.roughness_factor(), "metallic": pbr.metallic_factor(),
                    "two_sided": m.double_sided(), "alpha_mode": "OPAQUE",
                },
            }));
        }
    }
    // Props held by a bone: rigid on it, from their own files.
    for (file, bone) in &attach {
        let at = g.nodes().find(|n| n.name() == Some(bone)).ok_or_else(|| format!("no bone {bone}"))?.index();
        let joint = slot(at, rest_globals[at].inverse());
        let raw = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let p = gltf::Gltf::from_slice_without_validation(&raw).map_err(|e| e.to_string())?;
        let dir = file.parent().map(PathBuf::from).unwrap_or_default();
        let mut own: Vec<Vec<u8>> = Vec::new();
        for b in p.buffers() {
            own.push(match b.source() {
                gltf::buffer::Source::Bin => p.blob.clone().ok_or("glb without binary chunk")?,
                gltf::buffer::Source::Uri(u) => std::fs::read(dir.join(u)).map_err(|e| format!("{u}: {e}"))?,
            });
        }
        let stem = file.file_stem().and_then(|f| f.to_str()).unwrap_or("prop");
        for n in p.nodes() {
            let Some(mesh) = n.mesh() else { continue };
            let xf = rest_globals[at] * Mat4::from_cols_array_2d(&n.transform().matrix());
            for prim in mesh.primitives() {
                let reader = prim.reader(|b| own.get(b.index()).map(|v| v.as_slice()));
                let positions: Vec<[f32; 3]> = reader.read_positions().ok_or("no positions")?.collect();
                let normals: Vec<[f32; 3]> = reader.read_normals().map(|r| r.collect()).unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
                let uvs: Vec<[f32; 2]> = reader.read_tex_coords(0).map(|r| r.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; positions.len()]);
                let (first_vertex, first_index) = (verts.len(), indices.len());
                for v in 0..positions.len() {
                    let (mut js, mut ws) = ([0u32; 8], [0f32; 8]);
                    js[0] = joint;
                    ws[0] = 1.0;
                    verts.push(Vert { pos: xf.transform_point3(positions[v].into()), normal: xf.transform_vector3(normals[v].into()).normalize_or(Vec3::Y), uv: uvs[v], joints: js, weights: ws });
                }
                let local: Vec<u32> = match reader.read_indices() {
                    Some(r) => r.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                indices.extend(local.iter().map(|i| i + first_vertex as u32));
                let m = prim.material();
                let pbr = m.pbr_metallic_roughness();
                let mut texture = String::new();
                if let Some(image) = pbr.base_color_texture().and_then(|t| t.texture().source()) {
                    if let gltf::image::Source::Uri { uri, .. } = image.source() {
                        texture = format!("{name}_{}", uri.rsplit('/').next().unwrap_or(uri));
                        std::fs::copy(dir.join(uri), out_dir.join(&texture)).map_err(|e| format!("{uri}: {e}"))?;
                    }
                }
                surfaces.push(json!({
                    "name": format!("{stem}/{}", prim.index()),
                    "first_vertex": first_vertex, "vertex_count": positions.len(),
                    "first_index": first_index, "index_count": local.len(),
                    "material": {
                        "name": m.name().unwrap_or(""), "albedo_texture": texture,
                        "albedo": pbr.base_color_factor(), "albedo_color_space": "linear",
                        "roughness": pbr.roughness_factor(), "metallic": pbr.metallic_factor(),
                        "two_sided": m.double_sided(), "alpha_mode": "OPAQUE",
                    },
                }));
            }
        }
    }
    if palette.len() > 256 {
        return Err(format!("{} joints, the format holds 256", palette.len()));
    }

    let mut anim_bin: Vec<u8> = Vec::new();
    let mut clip_docs = Vec::new();
    let (mut all_min, mut all_max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for (to, from, looping) in &clips {
        let anim = g.animations().find(|a| a.name() == Some(from)).ok_or_else(|| format!("no clip {from}"))?;
        let mut channels = Vec::new();
        let mut duration = 0.0f32;
        for c in anim.channels() {
            let r = c.reader(data);
            let times: Vec<f32> = r.read_inputs().ok_or("no times")?.collect();
            duration = duration.max(*times.last().unwrap_or(&0.0));
            let keys = match r.read_outputs().ok_or("no outputs")? {
                ReadOutputs::Translations(v) => Keys::T(v.map(Vec3::from).collect()),
                ReadOutputs::Rotations(v) => Keys::R(v.into_f32().map(Quat::from_array).collect()),
                ReadOutputs::Scales(v) => Keys::S(v.map(Vec3::from).collect()),
                ReadOutputs::MorphTargetWeights(_) => continue,
            };
            let mode = c.sampler().interpolation();
            channels.push(Channel {
                node: c.target().node().index(),
                times,
                keys,
                step: mode == gltf::animation::Interpolation::Step,
                cubic: mode == gltf::animation::Interpolation::CubicSpline,
            });
        }
        let intervals = ((duration * FPS).round() as usize).max(1);
        let frames = if *looping { intervals.max(2) } else { intervals + 1 };
        let offset = anim_bin.len();
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for f in 0..frames {
            let t = duration * f as f32 / intervals as f32;
            let mut pose = rest.clone();
            for c in &channels {
                c.apply(t, &mut pose);
            }
            let world = globals(&pose, &order, &parent, root);
            let mats: Vec<Mat4> = palette.iter().map(|(node, inv)| world[*node] * *inv).collect();
            for m in &mats {
                put(&mut anim_bin, &rows(*m));
            }
            for v in &verts {
                let mut p = Vec3::ZERO;
                for k in 0..8 {
                    if v.weights[k] > 0.0 {
                        p += mats[v.joints[k] as usize].transform_point3(v.pos) * v.weights[k];
                    }
                }
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
        all_min = all_min.min(lo);
        all_max = all_max.max(hi);
        clip_docs.push(json!({
            "name": to, "duration": duration, "frames": frames, "looping": looping,
            "matrix_offset_bytes": offset, "bounds_min": lo.to_array(), "bounds_max": hi.to_array(),
        }));
        println!("  {to} <- {from}: {frames} frames, {duration:.2}s, {lo:.2} .. {hi:.2}");
    }

    let mut mesh_bin: Vec<u8> = Vec::with_capacity(verts.len() * 96 + indices.len() * 4);
    for v in &verts {
        put(&mut mesh_bin, &v.pos.to_array());
        put(&mut mesh_bin, &v.normal.to_array());
        put(&mut mesh_bin, &v.uv);
        for j in v.joints {
            mesh_bin.extend_from_slice(&j.to_le_bytes());
        }
        put(&mut mesh_bin, &v.weights);
    }
    for i in &indices {
        mesh_bin.extend_from_slice(&i.to_le_bytes());
    }
    let mesh_file = format!("{name}.mesh.bin");
    let anim_file = format!("{name}.anim.bin");
    std::fs::write(out_dir.join(&mesh_file), mesh_bin).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join(&anim_file), anim_bin).map_err(|e| e.to_string())?;
    let joints: Vec<_> = palette
        .iter()
        .map(|(node, inv)| {
            let n = g.nodes().nth(*node).unwrap();
            json!({ "name": n.name().unwrap_or(""), "inverse_bind_rows": rows(*inv) })
        })
        .collect();
    let doc = json!({
        "version": 1, "name": name, "mesh_file": mesh_file, "animation_file": anim_file,
        "vertex_count": verts.len(), "index_count": indices.len(), "joint_count": palette.len(),
        "vertex_stride_bytes": 96, "index_offset_bytes": verts.len() * 96, "matrix_stride_bytes": 48,
        "coordinate_system": "right-handed, +Y up, +Z model forward, metres",
        "winding": "counter-clockwise", "uv_origin": "top-left",
        "bounds_min": all_min.to_array(), "bounds_max": all_max.to_array(),
        "surfaces": surfaces, "joints": joints, "clips": clip_docs,
        "source": { "file": input.file_name().and_then(|f| f.to_str()), "importer": "gltf_character", "animation_fps": FPS },
    });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(out_dir.join(format!("{name}.json")), text + "\n").map_err(|e| e.to_string())?;
    println!("{name}: {} vertices, {} triangles, {} joints", verts.len(), indices.len() / 3, palette.len());
    Ok(())
}
