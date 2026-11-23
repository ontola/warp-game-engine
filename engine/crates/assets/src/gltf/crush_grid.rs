//! Split an opted-in model at four planes in model space. The game can move
//! these nine meshes independently. All instances share their cooked geometry.
use super::{Document, ModelPart, policy};
use glam::{Vec2, Vec3};
use kern::crush::{Grid, Vertex as ClipVertex};
use render::{cluster::{ClusterMesh, Part, Vertex}, scene::Scene};
use std::collections::HashMap;

#[derive(Default)]
struct Surface {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    by_vertex: HashMap<[u32; 8], u32>,
}

impl Surface {
    fn index(&mut self, v: Vertex) -> u32 {
        let key = [v.pos[0], v.pos[1], v.pos[2], v.normal[0], v.normal[1], v.normal[2], v.uv[0], v.uv[1]].map(f32::to_bits);
        *self.by_vertex.entry(key).or_insert_with(|| {
            let id = self.vertices.len() as u32;
            self.vertices.push(v);
            id
        })
    }

    fn triangle(&mut self, a: Vertex, b: Vertex, c: Vertex) {
        let ab = Vec3::from(b.pos) - Vec3::from(a.pos);
        let ac = Vec3::from(c.pos) - Vec3::from(a.pos);
        if ab.cross(ac).length_squared() < 1e-18 { return; }
        let ids = [self.index(a), self.index(b), self.index(c)];
        self.indices.extend_from_slice(&ids);
    }
}


#[derive(Clone, Copy)]
struct Point(Vertex);

impl ClipVertex for Point {
    fn position(self) -> Vec3 { self.0.pos.into() }
    fn on_plane(self, other: Self, axis: usize, plane: f32) -> Self {
        let (a,b)=(self.0,other.0);
        let t=(plane-a.pos[axis])/(b.pos[axis]-a.pos[axis]);
        Point(Vertex {
            pos: self.position().on_plane(other.position(),axis,plane).to_array(),
            normal: Vec3::from(a.normal).lerp(Vec3::from(b.normal),t).normalize_or(Vec3::Y).to_array(),
            uv: Vec2::from(a.uv).lerp(Vec2::from(b.uv),t).to_array(),
        })
    }
}
/// Grid boundaries are left X, right X, front Z and rear Z.
pub(super) fn build(scene: &mut Scene, doc: &Document, cuts: [f32; 4]) -> Result<Vec<(u8, ModelPart)>, String> {
    if !Grid(cuts).valid() {
        return Err("warpCrushGrid needs four finite ordered X and Z boundaries".into());
    }
    let grid = Grid(cuts);
    let mut cells: [Vec<(u32, Surface)>; 9] = Default::default();
    let mut poly = Vec::with_capacity(8);
    let mut work = Vec::with_capacity(8);
    for prim in doc.nodes.iter().flat_map(|n| &n.primitives) {
        let mut parts: [Surface; 9] = Default::default();
        for tri in prim.indices.chunks_exact(3) {
            let vertices = [tri[0],tri[1],tri[2]].map(|i| Point(prim.vertices[i as usize]));
            grid.split(vertices, &mut poly, &mut work, |cell,[a,b,c]| parts[cell].triangle(a.0,b.0,c.0));
        }
        for (cell, surface) in cells.iter_mut().zip(parts) {
            if !surface.indices.is_empty() { cell.push((prim.material, surface)); }
        }
    }
    let mut out = Vec::new();
    for (i, cell) in cells.into_iter().enumerate() {
        if cell.is_empty() {
            out.push((i as u8, ModelPart { mesh: scene.add_mesh(&ClusterMesh::default()), min: Vec3::ZERO, max: Vec3::ZERO, hinge: grid.hinge(i) }));
            continue;
        }
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = -min;
        let parts: Vec<_> = cell.iter().map(|(material, s)| {
            for v in &s.vertices { min = min.min(Vec3::from(v.pos)); max = max.max(Vec3::from(v.pos)); }
            Part { material: *material, vertices: &s.vertices, indices: &s.indices }
        }).collect();
        let mesh = ClusterMesh::build_with_constraints(&parts, |part| policy(scene, doc, part), |part, i| {
            let p = part.vertices[i].pos;
            p[0] == cuts[0] || p[0] == cuts[1] || p[2] == cuts[2] || p[2] == cuts[3]
        });
        let hinge = grid.hinge(i);
        out.push((i as u8, ModelPart { mesh: scene.add_mesh(&mesh), min, max, hinge }));
    }
    Ok(out)
}
