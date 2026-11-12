//! Gibs, after Growing Guns' `violence.gd`: a body killed by an explosion
//! or by overkill bursts into Voronoi chunks of its own meshes (flesh caps
//! on every cut, `render::primitives::shatter`) plus flying blood and meat
//! blobs; a headshot bursts the head and the torso tumbles on. Pieces are
//! rigid bodies that bounce through the arena, then go after their time or
//! when newer gore needs the room (Godot caps live chunks the same way).
use assets::skinned::Character;
use glam::{Affine3A, Quat, Vec3};
use physics::{Body, BodyDesc, Physics, Shape};
use render::cluster::{ClusterMesh, Part as MeshPart, Vertex};
use render::primitives::{self, Primitive};
use render::scene::{Material, NO_SHADOW};
use render::slots::{SlotId, Slots};
use render::Scene;

/// Live gore pieces at once (Godot: 32 rigid chunks; Rapier takes more).
pub const MAX_PIECES: usize = 160;
const VARIANTS: usize = 3;
/// Godot GIB_CHUNK_COUNT for the blob's primitive meshes.
const CELLS: usize = 3;
const LIFETIME: f32 = 12.0;

/// A body part that can burst: where it sits on the player (scale, offset
/// from the player position in player yaw space) and its cached pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part { Body, Head, Eyes }

#[derive(Clone, Debug)]
struct ShardMesh { mesh: usize, centroid: Vec3, half: Vec3 }

#[derive(Clone, Debug, Default)]
struct CharacterGore {
    body: Vec<ShardMesh>,
    head: Vec<ShardMesh>,
}

struct Piece { body: Body, slot: SlotId, mesh: usize, scale: Vec3, born: f32, ink:Option<Vec3> }

pub struct Gore {
    /// Per roster character model, anatomical body and head gib pieces.
    character_shards: Vec<CharacterGore>,
    /// Fallback per player, per part, a few shatter layouts for primitive bodies.
    shards: Vec<[Vec<Vec<ShardMesh>>; 3]>,
    cut: [usize; 3],
    blobs: [usize; 4],
    slots: Slots,
    pieces: Vec<Piece>,
    rng: u64,
}

/// The source part's transform on a player at `pos` facing `yaw`.
pub fn part_xf(part: Part, pos: Vec3, yaw: f32) -> Affine3A {
    let rot = Quat::from_rotation_y(yaw);
    match part {
        Part::Body => Affine3A::from_scale_rotation_translation(Vec3::new(0.42, 0.5, 0.42), rot, pos + Vec3::Y * -0.25),
        Part::Head => Affine3A::from_scale_rotation_translation(Vec3::splat(0.3), rot, pos + Vec3::Y * 0.55),
        Part::Eyes => Affine3A::from_scale_rotation_translation(Vec3::new(0.2, 0.07, 0.05), rot, pos + Vec3::Y * 0.6 + rot * Vec3::new(0.0, 0.0, -0.27)),
    }
}

impl Gore {
    /// `parts[p]`: player p's fallback primitives; `characters`: loaded roster models.
    pub fn reserve(scene: &mut Scene, parts: &[[Primitive; 3]], characters: &[Character], seed: u64, cut: Option<[usize; 3]>) -> Self {
        // Flesh: Godot's cap colours run from deep to raw red.
        let flesh: Vec<u32> = [[0.45, 0.06, 0.05], [0.62, 0.11, 0.08], [0.82, 0.16, 0.11]]
            .map(|c| scene.add_material(Material { tint: c.map(|v: f32| v.powf(2.2)), layer: -1, roughness: 0.9, flags: 0, scale: 1.0, metal: 0.0 })).to_vec();
        let character_shards: Vec<CharacterGore> = characters
            .iter()
            .enumerate()
            .map(|(idx, model)| build_character_gore(scene, model, &flesh, idx))
            .collect();
        let mut shards = Vec::new();
        if character_shards.is_empty() {
            for (p, prims) in parts.iter().enumerate() {
                let per: [Vec<Vec<ShardMesh>>; 3] = std::array::from_fn(|k| (0..VARIANTS).map(|v| {
                    primitives::shatter(&prims[k], CELLS, flesh[(p + k + v) % flesh.len()], seed ^ ((p * 31 + k * 7 + v) as u64 + 1))
                        .into_iter().map(|s| ShardMesh { mesh: scene.add_mesh(&s.mesh()), centroid: s.centroid, half: s.half }).collect()
                }).collect());
                shards.push(per);
            }
        }
        // Blood and meat blobs (Godot spawn_blood_gib_blobs): dark blood and flesh.
        let blobs = [[0.30, 0.018, 0.012], [0.20, 0.016, 0.012], [0.92, 0.18, 0.12], [0.55, 0.09, 0.07]].map(|c| {
            let m = scene.add_material(Material { tint: c.map(|v: f32| v.powf(2.2)), layer: -1, roughness: 0.4, flags: 0, scale: 1.0, metal: 0.0 });
            scene.add_mesh(&crate::blood::drop_mesh(m).mesh())
        });
        let slots = Slots::reserve(scene, MAX_PIECES);
        let cut = cut.unwrap_or_else(|| std::array::from_fn(|i| scene.add_mesh(&crate::stadium::gibs::unit_shard(flesh[i], i).mesh())));
        Gore { cut, character_shards, shards, blobs, slots, pieces: Vec::new(), rng: seed | 1 }
    }

    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.f() }

    pub fn len(&self) -> usize { self.pieces.len() }

    fn spawn(&mut self, scene: &mut Scene, phys: &mut Physics, mesh: usize, scale: Vec3, shape: Shape, pos: Vec3, rot: Quat, vel: Vec3, now: f32) {
        if self.pieces.len() >= MAX_PIECES {
            let old = self.pieces.remove(0);
            phys.remove(old.body);
            self.slots.remove(scene, old.slot);
        }
        let spin = Vec3::new(self.range(-10.0, 10.0), self.range(-10.0, 10.0), self.range(-10.0, 10.0));
        let body = phys.add(&BodyDesc { rot, vel, spin, density: 900.0, restitution: 0.15, friction: 0.8, damping: 0.1, ccd: true, ..BodyDesc::new(shape, pos) });
        let Some(slot) = self.slots.add(scene, mesh, Affine3A::from_scale_rotation_translation(scale, rot, pos)) else { phys.remove(body); return };
        let i = self.slots.instance(slot);
        scene.instances[i].flags |= NO_SHADOW;
        self.pieces.push(Piece { body, slot, mesh, scale, born: now, ink:(self.rng%3==0).then_some(vel) });
    }

    /// A small, directional burst from the head, using the audience's irregular cut meshes.
    pub fn headshot(&mut self, scene: &mut Scene, phys: &mut Physics, head: Vec3, velocity: Vec3, direction: Vec3, now: f32) {
        for i in 0..6 {
            let radial = Vec3::new(self.range(-1., 1.), self.range(-0.4, 1.), self.range(-1., 1.)).normalize_or(Vec3::Y);
            let size = Vec3::new(self.range(0.032, 0.063), self.range(0.04, 0.085), self.range(0.025, 0.05));
            let pos = head + radial * self.range(0.025, 0.09);
            let vel = velocity * 0.35 + direction * self.range(1.8, 3.5) + radial * self.range(0.8, 2.5) + Vec3::Y * 1.5;
            let rotation = Quat::from_rotation_y(self.range(0., std::f32::consts::TAU));
            self.spawn(scene, phys, self.cut[i % self.cut.len()], size, Shape::Box(size * 0.5), pos, rotation, vel, now);
        }
    }

    /// Burst `parts` of player `p` (at `pos`, facing `yaw`). `dir` is the
    /// killing push, `chaos` 0..1.1 how violent (Godot `spawn_ragdoll`).
    #[allow(clippy::too_many_arguments)]
    pub fn burst(&mut self, scene: &mut Scene, phys: &mut Physics, p: usize, parts: &[Part], pos: Vec3, yaw: f32, vel: Vec3, dir: Vec3, chaos: f32, now: f32) {
        let kb = 1.6;
        let dir = dir.try_normalize().unwrap_or(Vec3::Y);
        let base = vel * 0.5 + dir * self.range(2.2, 3.9) * kb * (1.0 + chaos * 0.18) + Vec3::Y * self.range(1.2, 2.8) * kb * (1.2 + chaos * 0.06);
        let strength = 1.45 * kb * (1.0 + chaos * 0.2);
        let rot = Quat::from_rotation_y(yaw);
        if !self.character_shards.is_empty() {
            let cg = &self.character_shards[p % self.character_shards.len()];
            let mut active = Vec::new();
            if parts.contains(&Part::Body) {
                active.extend(cg.body.iter().cloned());
            }
            if parts.contains(&Part::Head) || parts.contains(&Part::Eyes) {
                active.extend(cg.head.iter().cloned());
            }
            let xf = Affine3A::from_rotation_translation(rot, pos);
            for s in active {
                let at = xf.transform_point3(s.centroid);
                let out = (at - pos - Vec3::Y * 0.1).try_normalize().unwrap_or(Vec3::Y);
                let jitter = Vec3::new(self.range(-1.5, 1.5), 0.0, self.range(-1.5, 1.5));
                let vel = base + jitter + out * strength + Vec3::Y * strength * 0.2;
                let half = s.half.max(Vec3::splat(0.025));
                self.spawn(scene, phys, s.mesh, Vec3::ONE, Shape::Box(half), at, rot, vel, now);
            }
        } else if !self.shards.is_empty() {
            for &part in parts {
                let k = part as usize;
                let v = (self.f() * VARIANTS as f32) as usize % VARIANTS;
                let src = part_xf(part, pos, yaw);
                let scale = Vec3::new(src.matrix3.x_axis.length(), src.matrix3.y_axis.length(), src.matrix3.z_axis.length());
                let shards = self.shards[p.min(self.shards.len() - 1)][k][v].clone();
                for s in shards {
                    let at = src.transform_point3(s.centroid);
                    let out = (at - pos - Vec3::Y * 0.1).try_normalize().unwrap_or(Vec3::Y);
                    let jitter = Vec3::new(self.range(-1.5, 1.5), 0.0, self.range(-1.5, 1.5));
                    let vel = base + jitter + out * strength + Vec3::Y * strength * 0.2;
                    let half = (s.half * scale).max(Vec3::splat(0.025));
                    self.spawn(scene, phys, s.mesh, scale, Shape::Box(half), at, rot, vel, now);
                }
            }
        }
        // Blood and meat blobs, more for worse deaths.
        let sev = (0.9 + chaos * 1.6).clamp(0.35, 2.8);
        let n = ((5.0 + 15.0 * sev / 2.5 + chaos * 6.0).round() as usize).clamp(4, 24);
        for _ in 0..n {
            let r = self.range(0.035, 0.13) * (0.85 + 0.7 * sev / 2.5);
            let at = pos + Vec3::new(self.range(-0.35, 0.35), self.range(-0.15, 0.45), self.range(-0.35, 0.35));
            let out = (at - pos).try_normalize().unwrap_or(dir);
            let b = strength * self.range(0.85, 1.35) * (1.0 + chaos * 0.22);
            let vel = base + out * b + Vec3::Y * b * self.range(0.15, 0.35);
            let mesh = self.blobs[if self.f() < 0.38 { 2 + (self.f() * 2.0) as usize % 2 } else { (self.f() * 2.0) as usize % 2 }];
            let scale = Vec3::new(self.range(0.75, 1.25), self.range(0.65, 1.05), self.range(0.75, 1.25)) * r;
            self.spawn(scene, phys, mesh, scale, Shape::Sphere(r), at, rot, vel, now);
        }
    }

    /// Follow the physics; retire pieces past their time.
    pub fn update(&mut self, scene: &mut Scene, phys: &mut Physics, now: f32, mut splat:impl FnMut(&mut Scene,Vec3,Vec3,Vec3)) {
        let mut k = 0;
        while k < self.pieces.len() {
            let pc = &mut self.pieces[k];
            if now - pc.born > LIFETIME {
                let pc = self.pieces.remove(k);
                phys.remove(pc.body);
                self.slots.remove(scene, pc.slot);
                continue;
            }
            if let Some((pos, rot)) = phys.pose(pc.body) {
                if let Some(velocity) = pc.ink {
                    if let Some((at, normal)) = phys.static_contact(pc.body) {
                        splat(scene, at, normal, velocity);
                        pc.ink = None;
                    } else {
                        pc.ink = phys.velocity(pc.body);
                    }
                }
                // Shrink away over the last second.
                let fade = ((LIFETIME - (now - pc.born)) / 1.0).clamp(0.0, 1.0);
                self.slots.set(scene, pc.slot, pc.mesh, Affine3A::from_scale_rotation_translation(pc.scale * fade.max(0.01), rot, pos));
            }
            k += 1;
        }
    }

    pub fn clear(&mut self, scene: &mut Scene, phys: &mut Physics) {
        for pc in self.pieces.drain(..) { phys.remove(pc.body); self.slots.remove(scene, pc.slot); }
    }

}

/// Partition a character model's posed geometry into textured anatomical body
/// chunks (pelvis, left/right chest, arms, forearms, thighs, boots) and 3
/// head/helmet chunks, each sealed with a raw flesh cap at open cut edges.
fn build_character_gore(scene: &mut Scene, model: &Character, flesh: &[u32], model_idx: usize) -> CharacterGore {
    let Some(palette) = model.sample("pistol_idle", 0.0, true).ok() else {
        return CharacterGore::default();
    };
    let surfaces = model.posed_surfaces(scene, &palette, super::characters::model_transform(Vec3::ZERO, 0.0));
    let joint_parts: Vec<usize> = model.joints().map(|(name, _)| super::character_ragdoll::part(name)).collect();

    // Compute head center so head triangles split into 3 balanced chunks.
    let mut head_sum = Vec3::ZERO;
    let mut head_count = 0usize;
    for (_, vertices, indices, dom) in &surfaces {
        for tri in indices.chunks_exact(3) {
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            if joint_parts.get(dom[a]).copied().unwrap_or(1) == 2
                || joint_parts.get(dom[b]).copied().unwrap_or(1) == 2
                || joint_parts.get(dom[c]).copied().unwrap_or(1) == 2
            {
                let tc = (Vec3::from(vertices[a].pos) + Vec3::from(vertices[b].pos) + Vec3::from(vertices[c].pos)) / 3.0;
                head_sum += tc;
                head_count += 1;
            }
        }
    }
    let head_center = if head_count > 0 { head_sum / head_count as f32 } else { Vec3::new(0.0, 0.55, 0.0) };

    // 11 body buckets (0: pelvis, 1: left chest, 2: right chest, 3..11: limbs 3..=10)
    // and 3 head buckets (left face, right face, crown/back).
    let mut body_tris: Vec<Vec<(usize, [u32; 3])>> = vec![Vec::new(); 11];
    let mut head_tris: Vec<Vec<(usize, [u32; 3])>> = vec![Vec::new(); 3];

    for (surf_idx, (_, vertices, indices, dom)) in surfaces.iter().enumerate() {
        for tri in indices.chunks_exact(3) {
            let [a, b, c] = [tri[0], tri[1], tri[2]];
            let pa = joint_parts.get(dom[a as usize]).copied().unwrap_or(1);
            let pb = joint_parts.get(dom[b as usize]).copied().unwrap_or(1);
            let pc = joint_parts.get(dom[c as usize]).copied().unwrap_or(1);
            let tc = (Vec3::from(vertices[a as usize].pos) + Vec3::from(vertices[b as usize].pos) + Vec3::from(vertices[c as usize].pos)) / 3.0;
            let part = if pa == 2 || pb == 2 || pc == 2 { 2 } else { pa };
            if part == 2 {
                let rel = tc - head_center;
                let angle = rel.z.atan2(rel.x);
                let bucket = (((angle + std::f32::consts::PI) / std::f32::consts::TAU * 3.0).floor() as usize).min(2);
                head_tris[bucket].push((surf_idx, [a, b, c]));
            } else {
                let bucket = match part {
                    0 => 0,
                    1 => if tc.x < 0.0 { 1 } else { 2 },
                    p => p.clamp(3, 10),
                };
                body_tris[bucket].push((surf_idx, [a, b, c]));
            }
        }
    }

    let body = body_tris
        .into_iter()
        .enumerate()
        .filter_map(|(k, tris)| {
            let cap = flesh[(model_idx + k) % flesh.len()];
            build_character_shard(scene, &surfaces, &tris, cap)
        })
        .collect();
    let head = head_tris
        .into_iter()
        .enumerate()
        .filter_map(|(k, tris)| {
            let cap = flesh[(model_idx + k + 1) % flesh.len()];
            build_character_shard(scene, &surfaces, &tris, cap)
        })
        .collect();
    CharacterGore { body, head }
}

fn build_character_shard(
    scene: &mut Scene,
    surfaces: &[(u32, Vec<Vertex>, Vec<u32>, Vec<usize>)],
    tris: &[(usize, [u32; 3])],
    cap_mat: u32,
) -> Option<ShardMesh> {
    if tris.is_empty() {
        return None;
    }
    let mut sub_surfaces: Vec<(u32, Vec<Vertex>, Vec<u32>)> = Vec::new();
    let mut remap: std::collections::HashMap<(usize, u32), (usize, u32)> = std::collections::HashMap::new();
    let mut surf_slot: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let mut pos_sum = Vec3::ZERO;
    let mut pos_count = 0usize;

    for &(surf_idx, [a, b, c]) in tris {
        let slot = *surf_slot.entry(surf_idx).or_insert_with(|| {
            let s = sub_surfaces.len();
            sub_surfaces.push((surfaces[surf_idx].0, Vec::new(), Vec::new()));
            s
        });
        for v_idx in [a, b, c] {
            let (_, local_idx) = *remap.entry((surf_idx, v_idx)).or_insert_with(|| {
                let v = surfaces[surf_idx].1[v_idx as usize];
                pos_sum += Vec3::from(v.pos);
                pos_count += 1;
                let sub = &mut sub_surfaces[slot].1;
                sub.push(v);
                (slot, sub.len() as u32 - 1)
            });
            sub_surfaces[slot].2.push(local_idx);
        }
    }
    if pos_count < 3 {
        return None;
    }
    let centroid = pos_sum / pos_count as f32;
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for (_, verts, _) in &mut sub_surfaces {
        for v in verts {
            let p = Vec3::from(v.pos) - centroid;
            lo = lo.min(p);
            hi = hi.max(p);
            v.pos = p.to_array();
        }
    }

    // Find open boundary edges across all triangles in this piece and close them
    // with a double-sided flesh cap fan toward the piece centroid (Vec3::ZERO).
    let key = |v: Vec3| (v * 1e4).round().as_ivec3();
    let mut edges: std::collections::HashMap<(glam::IVec3, glam::IVec3), (Vec3, Vec3, u32)> = std::collections::HashMap::new();
    for (_, verts, idx) in &sub_surfaces {
        for t in idx.chunks_exact(3) {
            for e in 0..3 {
                let pa = Vec3::from(verts[t[e] as usize].pos);
                let pb = Vec3::from(verts[t[(e + 1) % 3] as usize].pos);
                let (ka, kb) = (key(pa), key(pb));
                if ka == kb {
                    continue;
                }
                let k = if (ka.x, ka.y, ka.z) <= (kb.x, kb.y, kb.z) { (ka, kb) } else { (kb, ka) };
                edges.entry(k).and_modify(|c| c.2 += 1).or_insert((pa, pb, 1));
            }
        }
    }
    let mut cap_verts = Vec::new();
    let mut cap_indices = Vec::new();
    // Stable triangle order keeps meshlet packing identical across replay rewinds.
    let mut edges: Vec<_> = edges.into_iter().collect();
    edges.sort_unstable_by_key(|((a, b), _)| (a.to_array(), b.to_array()));
    for (_, (pa, pb, count)) in edges {
        if count != 1 {
            continue;
        }
        let n = pa.cross(pb).normalize_or(Vec3::Y);
        for (tri, nrm) in [([Vec3::ZERO, pa, pb], n), ([Vec3::ZERO, pb, pa], -n)] {
            let base = cap_verts.len() as u32;
            for (i, v) in tri.iter().enumerate() {
                cap_verts.push(Vertex { pos: v.to_array(), normal: nrm.to_array(), uv: [i as f32 * 0.5, 0.0] });
            }
            cap_indices.extend([base, base + 1, base + 2]);
        }
    }

    // Add inner backfaces so open armor/clothing shells never look hollow when tumbling.
    for (_, verts, idx) in &mut sub_surfaces {
        let back: Vec<u32> = idx.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
        let flipped = verts.len() as u32;
        let inner: Vec<Vertex> = verts.iter().map(|v| Vertex { normal: (-Vec3::from(v.normal)).to_array(), ..*v }).collect();
        verts.extend(inner);
        idx.extend(back.into_iter().map(|k| k + flipped));
    }

    let coarse_mat = sub_surfaces[0].0;
    let mut parts: Vec<MeshPart> = sub_surfaces
        .iter()
        .filter(|(_, _, idx)| !idx.is_empty())
        .map(|(mat, verts, idx)| MeshPart { vertices: verts, indices: idx, material: *mat })
        .collect();
    if !cap_indices.is_empty() {
        parts.push(MeshPart { vertices: &cap_verts, indices: &cap_indices, material: cap_mat });
    }
    if parts.is_empty() {
        return None;
    }
    let cluster_mesh = ClusterMesh::build(&parts).with_coarse(primitives::cube(coarse_mat).mesh(), 0.04);
    let mesh = scene.add_mesh(&cluster_mesh);
    let half = ((hi - lo) * 0.5).clamp(Vec3::splat(0.03), Vec3::splat(0.45));
    Some(ShardMesh { mesh, centroid, half })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::characters::Characters;

    #[test]
    fn gore_builds_character_model_shards_and_spawns_head_and_body_pieces() {
        let characters = Characters::load_roster(3, 3, &[]).expect("roster characters load");
        let mut scene = Scene::default();
        let mut phys = Physics::new(Vec3::new(0.0, -9.81, 0.0), 60.0);
        let mut gore = Gore::reserve(&mut scene, &[], characters.models(), 42, None);
        assert_eq!(gore.character_shards.len(), characters.models().len());
        for cg in &gore.character_shards {
            assert!(cg.body.len() >= 8, "expected anatomical body chunks, got {}", cg.body.len());
            assert_eq!(cg.head.len(), 3, "expected 3 head chunks, got {}", cg.head.len());
        }

        let head = Vec3::new(1., 2., 3.);
        gore.headshot(&mut scene, &mut phys, head, Vec3::ZERO, Vec3::X, 0.);
        assert_eq!(gore.pieces.len(), 6);
        for piece in &gore.pieces {
            assert!(gore.cut.contains(&piece.mesh) && piece.scale.max_element() <= 0.085);
            assert!(phys.pose(piece.body).unwrap().0.distance(head) <= 0.091);
        }

        // Full body overkill burst spawns the character's anatomical body + head chunks.
        gore.clear(&mut scene, &mut phys);
        gore.burst(&mut scene, &mut phys, 1, &[Part::Body, Part::Head, Part::Eyes], Vec3::new(0.0, 1.0, 0.0), 0.0, Vec3::ZERO, Vec3::Z, 0.9, 1.0);
        let burst_pieces: Vec<usize> = gore.pieces.iter().map(|p| p.mesh).collect();
        for bs in &gore.character_shards[1].body {
            assert!(burst_pieces.contains(&bs.mesh), "body burst must spawn actual character body shard mesh");
        }
    }
}
