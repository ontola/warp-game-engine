//! Shared masonry panels: one instance per wall section, not per stone.
//! Cook once, then use the normal cluster hierarchy for projected-error LOD.
use crate::primitives::{self, Primitive};
use glam::Vec3;

/// Unit box with staggered quarried courses and a recessed solid core.
/// Counts are bounded to keep accidental authoring costs under control.
pub fn panel(material: u32, columns: u32, rows: u32, seed: u32) -> Primitive {
    let (columns, rows) = (columns.clamp(1, 16), rows.clamp(1, 16));
    let mut out = Primitive { material, ..Default::default() };
    append(&mut out, primitives::cube(material), Vec3::splat(0.90), Vec3::ZERO);
    for row in 0..rows {
        let shift = if row % 2 == 1 { 0.5 } else { 0.0 };
        for col in 0..=columns {
            let left = ((col as f32 - shift) / columns as f32).max(0.0);
            let right = ((col as f32 + 1.0 - shift) / columns as f32).min(1.0);
            if right <= left { continue; }
            let key = seed.wrapping_add(row * 97 + col * 13);
            let shape = crate::quarried_block::quarried_block(material, 0.0, key);
            let size = Vec3::new((right - left) * 0.985, 0.975 / rows as f32,
                0.96 + (key % 7) as f32 * 0.006);
            let center = Vec3::new((left + right) * 0.5 - 0.5,
                (row as f32 + 0.5) / rows as f32 - 0.5, 0.0);
            append(&mut out, shape, size, center);
        }
    }
    out
}

fn append(out: &mut Primitive, mut part: Primitive, scale: Vec3, at: Vec3) {
    let base = out.vertices.len() as u32;
    for v in &mut part.vertices {
        v.pos = (Vec3::from(v.pos) * scale + at).to_array();
        v.normal = (Vec3::from(v.normal) / scale).normalize().to_array();
    }
    out.vertices.extend(part.vertices);
    out.indices.extend(part.indices.into_iter().map(|i| i + base));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn masonry_has_finite_outward_faces_and_a_cheaper_distant_hierarchy() {
        for (columns, rows) in [(4, 8), (6, 1)] {
            let p = panel(0, columns, rows, 11);
            for v in &p.vertices {
                assert!(Vec3::from(v.pos).abs().max_element() <= 0.50001);
                assert!((Vec3::from(v.normal).length() - 1.0).abs() < 0.001);
            }
            for t in p.indices.chunks_exact(3) {
                let a = p.vertices[t[0] as usize];
                let b = p.vertices[t[1] as usize];
                let c = p.vertices[t[2] as usize];
                assert!((Vec3::from(b.pos) - Vec3::from(a.pos)).cross(Vec3::from(c.pos) - Vec3::from(a.pos)).dot(Vec3::from(a.normal)) > 0.0);
            }
            let mesh = p.mesh();
            eprintln!("masonry {columns}x{rows}: {} source, {} root triangles, {} levels", mesh.leaf_triangles(), mesh.root_triangles(), mesh.levels);
            assert!(mesh.root_triangles() < mesh.leaf_triangles());
        }
    }
}
