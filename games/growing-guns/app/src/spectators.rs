//! Small shared crowd silhouettes. Feet at zero, facing +Z; no rig or textures.
use glam::Vec3;
use render::{cluster::ClusterMesh, primitives::{self, Primitive}};

fn append(dst: &mut Primitive, mut p: Primitive, size: Vec3, at: Vec3) {
    let first = dst.vertices.len() as u32;
    for v in &mut p.vertices {
        v.pos = (Vec3::from(v.pos) * size + at).to_array();
        v.normal = (Vec3::from(v.normal) / size).normalize().to_array();
    }
    dst.vertices.extend(p.vertices);
    dst.indices.extend(p.indices.into_iter().map(|i| first + i));
}

pub fn meshes(shirt: u32, skin: u32, dark: u32, variant: usize) -> (ClusterMesh, ClusterMesh) {
    let mut clothes = Primitive { material: shirt, ..Default::default() };
    let mut face = Primitive { material: skin, ..Default::default() };
    let mut trousers = Primitive { material: dark, ..Default::default() };
    let mut sleeves = Primitive { material: shirt, ..Default::default() };
    let width = [0.94, 1.06, 1.16][variant % 3];
    // A tapered, six-sided torso: shoulders wider than the waist.
    append(&mut clothes, primitives::cylinder(shirt, 0.19, 0.135, 6), Vec3::new(width, 0.35, 0.64), Vec3::new(0., 0.54, 0.));
    append(&mut face, primitives::sphere(skin, 4, 6), Vec3::new(0.105, 0.13, 0.095), Vec3::new(0., 0.81, 0.));
    // Nose establishes facing without eyes or high-frequency face textures.
    append(&mut face, primitives::cube(skin), Vec3::new(0.035, 0.035, 0.045), Vec3::new(0., 0.80, 0.09));
    append(&mut trousers, primitives::sphere(dark, 2, 6), Vec3::new(0.108, 0.055, 0.098), Vec3::new(0., 0.90, -0.008));
    for side in [-1., 1.] {
        append(&mut trousers, primitives::cylinder(dark, 0.065, 0.05, 4), Vec3::new(1., 0.36, 1.), Vec3::new(side * 0.075, 0.20, 0.));
        append(&mut trousers, primitives::cube(dark), Vec3::new(0.105, 0.045, 0.16), Vec3::new(side * 0.075, 0.025, 0.025));
        append(&mut sleeves, primitives::cylinder(shirt, 0.05, 0.035, 6), Vec3::new(1., 0.31, 1.), Vec3::new(side * 0.21, 0.50, 0.015));
    }
    (ClusterMesh::build(&[clothes.part(), face.part(), trousers.part()]), sleeves.mesh())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn figures_fit_the_existing_gpu_animation_bounds_and_geometry_budget() {
        for variant in 0..3 {
            let (body, arms) = meshes(0, 1, 2, variant);
            let count = body.leaf_triangles() + arms.leaf_triangles();
            eprintln!("spectator {variant}: {count} triangles, {} clusters", body.clusters.len() + arms.clusters.len());
            assert!(count <= 260);
            for v in body.vertices.iter().chain(&arms.vertices) {
                let p = Vec3::from(v.pos);
                assert!(p.is_finite() && p.y >= -0.001 && p.y <= 1.0);
                assert!((p - Vec3::Y * 0.5).length() < 0.8);
                assert!((Vec3::from(v.normal).length() - 1.0).abs() < 0.001);
            }
        }
    }
}
