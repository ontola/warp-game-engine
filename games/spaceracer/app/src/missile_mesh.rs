//! Modern faceted cruise missile, translated from the retained weapon_vfx/Ship.loft recipe.
use glam::{Affine3A, Quat, Vec3};
use render::{
    Scene,
    cluster::{ClusterMesh, Vertex},
    primitives::{self, Primitive},
    scene::Material,
};
fn loft(material: u32, rings: &[[f32; 3]]) -> Primitive {
    let cross = [
        [-1., -0.05],
        [-0.96, 0.10],
        [-0.71, 0.67],
        [-0.60, 0.72],
        [0.60, 0.72],
        [0.71, 0.67],
        [0.96, 0.10],
        [1., -0.05],
        [0.65, -0.5],
        [-0.65, -0.5],
    ];
    let point = |i: usize, j: usize| {
        Vec3::new(
            cross[j][0] * rings[i][1],
            cross[j][1] * rings[i][2],
            rings[i][0],
        )
    };
    let mut p = Primitive {
        material,
        ..Default::default()
    };
    let mut tri = |a: Vec3, b: Vec3, c: Vec3| {
        let normal = (b - a).cross(c - a).normalize_or_zero();
        let start = p.vertices.len() as u32;
        for pos in [a, b, c] {
            p.vertices.push(Vertex {
                pos: pos.to_array(),
                normal: normal.to_array(),
                uv: [pos.x, pos.z * 0.2],
            });
        }
        p.indices.extend([start, start + 1, start + 2]);
    };
    for i in 0..rings.len() - 1 {
        for j in 0..10 {
            let k = (j + 1) % 10;
            tri(point(i, j), point(i + 1, j), point(i, k));
            tri(point(i, k), point(i + 1, j), point(i + 1, k));
        }
    }
    for i in [0, rings.len() - 1] {
        for j in 0..10 {
            let a = point(i, j);
            let b = point(i, (j + 1) % 10);
            let c = Vec3::new(0., 0., rings[i][0]);
            if i == 0 { tri(c, a, b) } else { tri(c, b, a) }
        }
    }
    p
}
pub fn mesh(scene: &mut Scene) -> usize {
    let hull = scene.add_material(Material {
        tint: [0.485, 0.533, 0.462],
        layer: -1,
        roughness: 0.47,
        metal: 0.24,
        flags: 0,
        scale: 1.,
    });
    let radome = scene.add_material(Material {
        tint: [0.138, 0.171, 0.144],
        layer: -1,
        roughness: 0.5,
        metal: 0.18,
        flags: 0,
        scale: 1.,
    });
    let steel = scene.add_material(Material {
        tint: [0.22, 0.25, 0.28],
        layer: -1,
        roughness: 0.28,
        metal: 0.75,
        flags: 0,
        scale: 1.,
    });
    let black = scene.add_material(Material {
        tint: [0.003, 0.005, 0.009],
        layer: -1,
        roughness: 0.6,
        metal: 0.2,
        flags: 0,
        scale: 1.,
    });
    let mut pieces = vec![];
    let mut add = |mut p: Primitive, scale: Vec3, rot: Quat, pos: Vec3| {
        let xf = Affine3A::from_scale_rotation_translation(scale * 0.42, rot, pos * 0.42);
        for v in &mut p.vertices {
            v.pos = xf.transform_point3(Vec3::from(v.pos)).to_array();
            v.normal = (rot * (Vec3::from(v.normal) / scale))
                .normalize_or_zero()
                .to_array();
        }
        pieces.push(p);
    };
    add(
        loft(
            hull,
            &[
                [-4., 0.63, 1.1],
                [-3.3, 0.95, 1.55],
                [2.6, 0.95, 1.55],
                [3.1, 0.85, 1.4],
            ],
        ),
        Vec3::ONE,
        Quat::IDENTITY,
        Vec3::ZERO,
    );
    add(
        loft(
            radome,
            &[[3.08, 0.85, 1.4], [4.35, 0.67, 1.06], [5.9, 0.05, 0.10]],
        ),
        Vec3::ONE,
        Quat::IDENTITY,
        Vec3::ZERO,
    );
    for side in [-1., 1.] {
        add(
            loft(
                radome,
                &[[-1.7, 1.35, 0.08], [-1.2, 1.45, 0.10], [0.45, 0.1, 0.06]],
            ),
            Vec3::ONE,
            Quat::from_rotation_z(side * -0.07),
            Vec3::new(side * 1.85, -0.28, -0.1),
        );
        add(
            loft(
                radome,
                &[[-0.65, 0.9, 0.09], [0.1, 0.85, 0.1], [1.05, 0.03, 0.04]],
            ),
            Vec3::ONE,
            Quat::from_rotation_z(side * 0.65),
            Vec3::new(side * 0.9, 0.4, -3.15),
        );
        add(
            primitives::cube(steel),
            Vec3::new(0.025, 0.1, 2.7),
            Quat::IDENTITY,
            Vec3::new(side * 0.94, 0.13, 0.65),
        );
    }
    add(
        primitives::cylinder(black, 0.5, 0.5, 12),
        Vec3::new(1., 0.12, 1.),
        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        Vec3::new(0., 0., -4.08),
    );
    add(
        primitives::cylinder(steel, 0.66, 0.55, 12),
        Vec3::new(1., 0.10, 1.),
        Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
        Vec3::new(0., 0., -3.96),
    );
    scene.add_mesh(&ClusterMesh::build(
        &pieces.iter().map(Primitive::part).collect::<Vec<_>>(),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loft_winding_faces_outward() {
        let p = loft(0, &[[-2., 1., 1.], [2., 1., 1.]]);
        for t in p.indices.chunks_exact(3) {
            let a = Vec3::from(p.vertices[t[0] as usize].pos);
            let b = Vec3::from(p.vertices[t[1] as usize].pos);
            let c = Vec3::from(p.vertices[t[2] as usize].pos);
            assert!((b - a).cross(c - a).dot((a + b + c) / 3.) > 0.);
        }
    }
}
