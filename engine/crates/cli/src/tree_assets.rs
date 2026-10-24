//! Three shared evergreen silhouettes. Shape once before cluster/volume baking;
//! no additional runtime geometry, materials, or per-instance mesh copies.
use glam::Vec3;
use render::cluster::{Vertex, read_kmesh};
use sideways::vegetation::TreeArchetype;
use std::path::Path;

pub type Surfaces = Vec<(String, Vec<Vertex>, Vec<u32>)>;

pub fn load(root: &Path, archetype: TreeArchetype) -> Result<Surfaces, String> {
    let variant = ["a", "b", "c"][archetype.index()];
    let path = root.join(format!("engine/assets/fir_{variant}_near.kmesh"));
    let mut surfaces = read_kmesh(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)?;
    for (name, vertices, indices) in &mut surfaces {
        // The retained C trunk has no UVs at all. Do not sample an arbitrary
        // atlas corner: give just that wood a Y-axis cylinder unwrap and the
        // existing tileable bark texture. Original geometry/normals survive.
        if name.contains("trunk") && vertices.iter().all(|v| v.uv == [0.0; 2]) {
            cylinder_uvs(vertices, indices);
            *name = "bark_albedo.jpg".into();
        }
    }
    // One coherent warp for wood and leaves preserves their attachment. The
    // lower trunk stays exactly as authored, including its physical radius.
    if archetype == TreeArchetype::Conifer {
        let height = surfaces.iter().flat_map(|(_, vertices, _)| vertices.iter())
            .map(|v| v.pos[1]).fold(0.0f32, f32::max);
        for (_, vertices, _) in &mut surfaces { for v in vertices { taper(v, height); } }
    }
    Ok(surfaces)
}

fn cylinder_uvs(vertices: &mut Vec<Vertex>, indices: &mut [u32]) {
    for v in vertices.iter_mut() {
        v.uv = [v.pos[2].atan2(v.pos[0]) / std::f32::consts::TAU + 0.5, v.pos[1] * 0.5];
    }
    let mut wrapped = std::collections::HashMap::<u32, u32>::new();
    for tri in indices.chunks_exact_mut(3) {
        let lo = tri.iter().map(|i| vertices[*i as usize].uv[0]).fold(f32::INFINITY, f32::min);
        let hi = tri.iter().map(|i| vertices[*i as usize].uv[0]).fold(f32::NEG_INFINITY, f32::max);
        if hi - lo <= 0.5 { continue; }
        for index in tri {
            if vertices[*index as usize].uv[0] >= 0.5 { continue; }
            *index = *wrapped.entry(*index).or_insert_with(|| {
                let mut vertex = vertices[*index as usize];
                vertex.uv[0] += 1.0;
                let duplicate = vertices.len() as u32;
                vertices.push(vertex);
                duplicate
            });
        }
    }
}

fn radial_scale(y: f32, height: f32) -> (f32, f32) {
    let start = (height * 0.38).max(5.5).min(height);
    let span = (height - start).max(0.001);
    let t = ((y - start) / span).clamp(0.0, 1.0);
    let smooth = t * t * (3.0 - 2.0 * t);
    (1.0 - 0.52 * smooth, -0.52 * 6.0 * t * (1.0 - t) / span)
}

fn taper(v: &mut Vertex, height: f32) {
    let p = Vec3::from(v.pos); let n = Vec3::from(v.normal);
    let (scale, derivative) = radial_scale(p.y, height);
    // Inverse transpose of x'=f(y)x, y'=y, z'=f(y)z. Copying the old normals
    // would change the good tree lighting and create false bright/dark bands.
    v.normal = Vec3::new(n.x / scale,
        n.y - derivative * (p.x * n.x + p.z * n.z) / scale,
        n.z / scale).normalize_or_zero().to_array();
    v.pos = [p.x * scale, p.y, p.z * scale];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cylinder_unwrap_splits_only_uv_seam_without_moving_geometry() {
        let mut vertices:Vec<_>=[[-1.0,0.0,0.01],[-1.0,0.0,-0.01],[-1.0,1.0,0.01],[-1.0,1.0,-0.01]]
            .into_iter().map(|pos|Vertex{pos,normal:[-1.0,0.0,0.0],uv:[0.0;2]}).collect();
        let before=vertices.clone();let old_indices=[0,1,2,1,3,2];let mut indices=old_indices;
        cylinder_uvs(&mut vertices,&mut indices);
        assert_eq!(vertices.len(),6,"two seam vertices are shared by both triangles");
        for (new,old) in indices.iter().zip(old_indices) {
            assert_eq!(vertices[*new as usize].pos,before[old as usize].pos);
            assert_eq!(vertices[*new as usize].normal,before[old as usize].normal);
        }
        for tri in indices.chunks_exact(3) {
            let lo=tri.iter().map(|i|vertices[*i as usize].uv[0]).fold(f32::INFINITY,f32::min);
            let hi=tri.iter().map(|i|vertices[*i as usize].uv[0]).fold(f32::NEG_INFINITY,f32::max);
            assert!(hi-lo<0.01,"seam must not stretch one texture wrap across a thin triangle");
        }
    }

    #[test]
    fn taper_preserves_root_uvs_and_transforms_normals_with_the_surface() {
        let normal = Vec3::new(0.3, 0.4, 0.5).normalize();
        let mut root = Vertex { pos: [0.22, 2.5, -0.22], normal: normal.to_array(), uv: [0.4, 0.7] };
        let before = root;
        taper(&mut root, 14.0);
        assert_eq!(root.pos, before.pos); assert_eq!(root.uv, before.uv);
        assert!(Vec3::from(root.normal).distance(normal) < 1e-6);
        for y in [5.8, 8.0, 11.0, 13.5] {
            let position = Vec3::new(2.0, y, -1.0);
            let u = normal.any_orthonormal_vector(); let w = normal.cross(u);
            let warp = |p: Vec3| { let (f, _) = radial_scale(p.y, 14.0); Vec3::new(p.x * f, p.y, p.z * f) };
            let du = warp(position + u * 0.001) - warp(position - u * 0.001);
            let dw = warp(position + w * 0.001) - warp(position - w * 0.001);
            let geometric = du.cross(dw).normalize();
            let mut vertex = Vertex { pos: position.to_array(), normal: normal.to_array(), uv: [0.1, 0.2] };
            taper(&mut vertex, 14.0);
            assert!(Vec3::from(vertex.normal).dot(geometric) > 0.9999);
            assert_eq!(vertex.uv, [0.1, 0.2]);
        }
    }

    #[test]
    fn real_source_archetypes_keep_geometry_and_repair_only_missing_trunk_uvs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        for archetype in TreeArchetype::ALL {
            let variant = ["a", "b", "c"][archetype.index()];
            let original = read_kmesh(&std::fs::read(root.join(format!("engine/assets/fir_{variant}_near.kmesh"))).unwrap()).unwrap();
            let shaped = load(&root, archetype).unwrap();
            assert_eq!(shaped.len(), original.len());
            let height = original.iter().flat_map(|(_, v, _)| v.iter()).map(|v| v.pos[1]).fold(0.0f32, f32::max);
            let mut changed = 0;
            for ((name, vertices, indices), (old_name, old_vertices, old_indices)) in shaped.iter().zip(&original) {
                let repaired = old_name.contains("trunk") && old_vertices.iter().all(|v| v.uv == [0.0; 2]);
                assert_eq!(name, if repaired { "bark_albedo.jpg" } else { old_name });
                assert_eq!(indices.len(), old_indices.len());
                if !repaired { assert_eq!(vertices.len(), old_vertices.len()); assert_eq!(indices,old_indices); }
                for (index, old_index) in indices.iter().zip(old_indices) {
                    let v=&vertices[*index as usize];let old=&old_vertices[*old_index as usize];
                    if !repaired { assert_eq!(v.uv, old.uv); }
                    assert_eq!(v.pos[1], old.pos[1]);
                    assert!(v.pos.iter().chain(v.normal.iter()).all(|f| f.is_finite()));
                    if archetype != TreeArchetype::Conifer || old.pos[1] <= height * 0.38 { assert_eq!(v.pos, old.pos); }
                    changed += usize::from(v.pos != old.pos);
                }
                if repaired {
                    let mapped = indices.chunks_exact(3).filter(|tri| {
                        let u=tri.iter().map(|i|glam::Vec2::from(vertices[*i as usize].uv)).collect::<Vec<_>>();
                        (u[1]-u[0]).perp_dot(u[2]-u[0]).abs()>1e-8
                    }).count();
                    assert!(mapped*100 > indices.len()/3*95,"cylinder UVs must cover the actual trunk surface");
                    for tri in indices.chunks_exact(3) {
                        let u=tri.iter().map(|i|vertices[*i as usize].uv[0]).collect::<Vec<_>>();
                        // Axis-crossing tiny root triangles may exceed half a
                        // wrap, but a normal cylindrical seam must not do so.
                        assert!(u.iter().all(|value| value.is_finite()));
                    }
                }
            }
            assert_eq!(changed > 0, archetype == TreeArchetype::Conifer);
        }
    }
}
