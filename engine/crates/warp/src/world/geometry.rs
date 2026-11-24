//! Script-authored indexed triangles. Their storage can be recycled independently
//! of imported meshes, humans and the dynamic instance slots in the same scene.
use super::{Look, Shape, Spawn, World};
use glam::Vec3;
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{MeshRef, Scene};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

#[derive(Debug)]
pub(crate) struct MeshPart {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub look: Look,
}
pub(crate) const MAX_VERTICES: usize = 65_536;
pub(crate) const MAX_INDICES: usize = 196_608;

#[derive(Default)]
struct Spans(Vec<Range<usize>>);
impl Spans {
    fn put<T: Copy>(&mut self, dst: &mut Vec<T>, data: &[T]) -> Range<usize> {
        if let Some((i, _)) = self
            .0
            .iter()
            .enumerate()
            .filter(|(_, r)| r.len() >= data.len())
            .min_by_key(|(_, r)| r.len())
        {
            let start = self.0[i].start;
            self.0[i].start += data.len();
            if self.0[i].is_empty() {
                self.0.remove(i);
            }
            let at = start..start + data.len();
            dst[at.clone()].copy_from_slice(data);
            return at;
        }
        let start = dst.len();
        dst.extend_from_slice(data);
        start..dst.len()
    }
    fn release<T>(&mut self, dst: &mut Vec<T>, span: Range<usize>) {
        let mut i = self.0.partition_point(|r| r.start < span.start);
        self.0.insert(i, span);
        if i > 0 && self.0[i - 1].end == self.0[i].start {
            self.0[i - 1].end = self.0.remove(i).end;
            i -= 1;
        }
        if i + 1 < self.0.len() && self.0[i].end == self.0[i + 1].start {
            self.0[i].end = self.0.remove(i + 1).end;
        }
        if self.0[i].end == dst.len() {
            dst.truncate(self.0.remove(i).start);
        }
    }
}

struct Geometry {
    source: Arc<[MeshPart]>,
    bounds: (Vec3, Vec3),
    mesh: usize,
    materials: Range<usize>,
    vertices: Range<usize>,
    cluster_verts: Range<usize>,
    tris: Range<usize>,
    clusters: Range<usize>,
}
#[derive(Default)]
pub(super) struct Pool {
    entries: BTreeMap<u32, Geometry>,
    mesh_slots: Vec<usize>,
    materials: Spans,
    vertices: Spans,
    cluster_verts: Spans,
    tris: Spans,
    clusters: Spans,
    live_vertices: usize,
    live_indices: usize,
}

impl Pool {
    pub(super) fn mesh(&self, key: u32) -> Result<(usize, (Vec3, Vec3)), String> {
        self.entries
            .get(&key)
            .map(|g| (g.mesh, g.bounds))
            .ok_or_else(|| "unknown generated mesh".into())
    }

    pub(super) fn release(&mut self, scene: &mut Scene, key: u32) {
        let Some(g) = self.entries.remove(&key) else {
            return;
        };
        for p in g.source.iter() {
            self.live_vertices -= p.vertices.len();
            self.live_indices -= p.indices.len();
        }
        // Unreferenced descriptors must remain harmless until another mesh takes
        // these ranges; render diagnostics inspect the entire cluster array.
        for c in &mut scene.clusters[g.clusters.clone()] {
            c.first_vert = 0;
            c.first_tri = 0;
            c.tri_count = 0;
            c.material = 0;
        }
        scene.meshes[g.mesh] = MeshRef {
            first_cluster: 0,
            cluster_count: 0,
            sphere: [0.; 4],
        };
        self.mesh_slots.push(g.mesh);
        self.materials.release(&mut scene.materials, g.materials);
        self.vertices.release(&mut scene.vertices, g.vertices);
        self.cluster_verts
            .release(&mut scene.cluster_verts, g.cluster_verts);
        self.tris.release(&mut scene.tris, g.tris);
        self.clusters.release(&mut scene.clusters, g.clusters);
    }
}

impl World {
    pub(crate) fn add_geometry(
        &mut self,
        source: Vec<MeshPart>,
        mut spawn: Spawn,
    ) -> Result<u32, String> {
        if self.entities.len() >= self.capacity() {
            return Err("world.mesh: entity budget exceeded".into());
        }
        let vertices: usize = source.iter().map(|p| p.vertices.len()).sum();
        let indices: usize = source.iter().map(|p| p.indices.len()).sum();
        if self.geometry.live_vertices + vertices > 4 * MAX_VERTICES
            || self.geometry.live_indices + indices > 4 * MAX_INDICES
        {
            return Err("world.mesh: live generated geometry budget exceeded".into());
        }
        let key = self.next;
        self.install_geometry(key, source.into());
        spawn.shape = Shape::Generated(key);
        self.spawn(spawn)
    }

    pub(super) fn copy_geometry(&self, key: u32, target: &mut World) {
        target.install_geometry(key, self.geometry.entries[&key].source.clone());
    }

    fn install_geometry(&mut self, key: u32, source: Arc<[MeshPart]>) {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let palette: Vec<_> = source.iter().map(|p| super::material(&p.look)).collect();
        let materials = self
            .geometry
            .materials
            .put(&mut self.scene.materials, &palette);
        let mut parts = Vec::with_capacity(source.len());
        for (slot, part) in source.iter().enumerate() {
            for v in &part.vertices {
                let pos = Vec3::from(v.pos);
                lo = lo.min(pos);
                hi = hi.max(pos);
            }
            parts.push(Part {
                vertices: &part.vertices,
                indices: &part.indices,
                material: (materials.start + slot) as u32,
            });
        }
        let mut cooked = ClusterMesh::build(&parts);
        let pool = &mut self.geometry;
        let sc = &mut self.scene;
        let vertices = pool.vertices.put(&mut sc.vertices, &cooked.vertices);
        for index in &mut cooked.cluster_verts {
            *index += vertices.start as u32;
        }
        let cluster_verts = pool
            .cluster_verts
            .put(&mut sc.cluster_verts, &cooked.cluster_verts);
        let tris = pool.tris.put(&mut sc.tris, &cooked.tris);
        for cluster in &mut cooked.clusters {
            cluster.first_tri += tris.start as u32;
            cluster.first_vert += cluster_verts.start as u32;
        }
        let clusters = pool.clusters.put(&mut sc.clusters, &cooked.clusters);
        let descriptor = MeshRef {
            first_cluster: clusters.start as u32,
            cluster_count: clusters.len() as u32,
            sphere: cooked.sphere,
        };
        let mesh = if let Some(i) = pool.mesh_slots.pop() {
            sc.meshes[i] = descriptor;
            i
        } else {
            sc.meshes.push(descriptor);
            sc.meshes.len() - 1
        };
        let bounds = ((lo + hi) * 0.5, ((hi - lo) * 0.5).max(Vec3::splat(0.001)));
        for p in source.iter() {
            pool.live_vertices += p.vertices.len();
            pool.live_indices += p.indices.len();
        }
        pool.entries.insert(
            key,
            Geometry {
                source,
                bounds,
                mesh,
                materials,
                vertices,
                cluster_verts,
                tris,
                clusters,
            },
        );
        self.rebuild = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn triangle(x: f32) -> Vec<MeshPart> {
        let vertices = [[x, 0., 0.], [x, 0., 1.], [x + 1., 0., 0.]]
            .map(|pos| Vertex {
                pos,
                normal: [0., 1., 0.],
                uv: [0., 0.],
            })
            .to_vec();
        vec![MeshPart {
            vertices,
            indices: vec![0, 1, 2],
            look: Look {
                surface: Some(41),
                ..Default::default()
            },
        }]
    }

    #[test]
    fn space_is_reused_without_touching_neighbors_and_survives_scene_refresh() {
        let mut w = World::new(Path::new("."), 16);
        let first = w.add_geometry(triangle(3.), Spawn::default()).unwrap();
        let fixed = w
            .spawn(Spawn {
                shape: Shape::Sphere,
                ..Default::default()
            })
            .unwrap();
        let second = w.add_geometry(triangle(9.), Spawn::default()).unwrap();
        let lengths = |w: &World| {
            [
                w.scene.vertices.len(),
                w.scene.tris.len(),
                w.scene.cluster_verts.len(),
                w.scene.clusters.len(),
                w.scene.materials.len(),
                w.scene.meshes.len(),
            ]
        };
        let before = lengths(&w);
        assert!(w.remove(first));
        for i in 0..20 {
            let id = w
                .add_geometry(triangle(i as f32), Spawn::default())
                .unwrap();
            assert_eq!(lengths(&w), before);
            assert_eq!(w.room_occluders(&[id]).unwrap().triangles(), 1);
            assert!(w.remove(id));
        }
        w.fresh_scene().unwrap();
        assert_eq!(w.room_occluders(&[second]).unwrap().triangles(), 1);
        assert!(w.room_occluders(&[fixed]).unwrap().triangles() > 12);
        assert_eq!(w.get(second).unwrap().bounds().center.x, 9.5);
        assert!(w.remove(second));
        assert_eq!(w.geometry.live_vertices, 0);
        assert_eq!(w.geometry.live_indices, 0);
    }
}
