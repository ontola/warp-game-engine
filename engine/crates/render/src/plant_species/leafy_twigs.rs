//! Alternate leaves and short shoots on birches and two North African shrubs.
use super::{radial, stalk, update_normals};
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, tube, unit},
};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

#[derive(Debug)]
pub(in crate::plant_species) struct Twig {
    length: f32,
    nodes: u32,
    leaves_per_node: u32,
    leaf_length: f32,
    width_ratio: f32,
    petiole: f32,
    /// Half-width samples from leaf base to apex, including marginal teeth.
    margin: &'static [f32],
    thickness: f32,
    /// Alternating lateral spur shoots carried by the main stem.
    spurs: u32,
}

pub(in crate::plant_species) const MOUNTAIN_BIRCH: Twig = Twig {
    length: 0.22,
    nodes: 7,
    leaves_per_node: 1,
    leaf_length: 0.037,
    width_ratio: 0.76,
    petiole: 0.009,
    thickness: 0.00040,
    spurs: 3,
    margin: &[
        0.18, 0.57, 0.76, 0.67, 0.93, 0.77, 1.0, 0.88, 0.93, 0.74, 0.80, 0.57,
        0.62, 0.42, 0.44, 0.26, 0.15, 0.04,
    ],
};
pub(in crate::plant_species) const DWARF_BIRCH: Twig = Twig {
    length: 0.099,
    nodes: 7,
    leaves_per_node: 1,
    leaf_length: 0.0125,
    width_ratio: 1.05,
    petiole: 0.0028,
    thickness: 0.00032,
    spurs: 2,
    margin: &[
        0.24, 0.60, 0.69, 0.65, 0.88, 0.92, 0.86, 1.0, 1.0, 0.90, 0.94, 0.90, 0.76, 0.78, 0.68,
        0.47, 0.36, 0.13,
    ],
};
pub(in crate::plant_species) const SALTBUSH: Twig = Twig {
    length: 0.138,
    nodes: 9,
    leaves_per_node: 1,
    leaf_length: 0.031,
    width_ratio: 0.59,
    petiole: 0.0055,
    thickness: 0.00058,
    spurs: 2,
    margin: &[0.12, 0.29, 0.59, 0.88, 1.0, 0.82, 0.59, 0.38, 0.15, 0.065],
};
pub(in crate::plant_species) const NITRARIA: Twig = Twig {
    length: 0.144,
    nodes: 6,
    leaves_per_node: 3,
    leaf_length: 0.0175,
    width_ratio: 0.44,
    petiole: 0.0026,
    thickness: 0.00090,
    spurs: 2,
    margin: &[0.15, 0.28, 0.43, 0.63, 0.80, 0.96, 1.0, 0.94, 0.72, 0.28],
};

impl Twig {
    pub(super) fn build(&self, part: &mut PartGeometry, variant: u32) {
        let dir = Vec3::new(0.11 * (variant as f32 - 1.0), 1.0, 0.08).normalize();
        let stem = stalk(
            part,
            Vec3::ZERO,
            dir * self.length,
            Vec3::X * self.length * 0.034,
            self.length * 0.012,
            5,
        );
        self.foliate(part, stem, variant, self.nodes, 0);
        for s in 0..self.spurs {
            let f = 0.21 + 0.60 * (s as f32 + 0.4) / self.spurs as f32;
            let aim = (radial(variant as f32 + s as f32 * 2.41) + Vec3::Y * 0.56).normalize();
            let radius = self.length * 0.005 + 0.00035;
            let spur = stalk(
                part,
                stem.point(f),
                aim * (0.24 * self.length),
                Vec3::Y * self.length * 0.026,
                radius,
                2,
            );
            self.foliate(part, spur, variant, 3, 1 + s);
        }
    }

    fn foliate(&self, part: &mut PartGeometry, stem: Shoot, v: u32, nodes: u32, shoot: u32) {
        for i in 0..nodes {
            let t = (i as f32 + 0.62) / nodes as f32;
            let node = stem.point(t);
            let angle = v as f32 + i as f32 * 2.41 + shoot as f32 * 1.3;
            for j in 0..self.leaves_per_node {
                let id = shoot * 32 + i * 3 + j;
                let out = radial(angle + j as f32 * 1.98 + 0.33 * unit(v, 1, id));
                let dir = (out
                    + Vec3::Y * (0.06 + 0.68 * unit(v, 2, id))
                    + stem.delta.normalize() * 0.24)
                    .normalize();
                let leaf_size = (0.80 + 0.30 * unit(v, 3, id)) * (1.0 - 0.28 * t.powi(3));
                let stalk_length = self.petiole * leaf_size;
                let petiole = Shoot {
                    root: node,
                    delta: dir * stalk_length,
                    bend: Vec3::ZERO,
                    radius: 0.00052,
                };
                tube(part, petiole, 2, 3);
                let roll = 0.94 * (unit(v, 4, id) - 0.5);
                let shape = BladeShape { margin: self.margin, width_ratio: self.width_ratio, thickness: self.thickness };
                shape.build(part, petiole.point(1.0), dir, self.leaf_length * leaf_size, roll);
            }
        }
    }

}

/// A smooth or toothed blade with a folded midrib and closed cross sections.
pub(super) struct BladeShape {
    pub(super) margin: &'static [f32],
    pub(super) width_ratio: f32,
    pub(super) thickness: f32,
}

impl BladeShape {
    pub(super) fn build(&self, part: &mut PartGeometry, root: Vec3, dir: Vec3, length: f32, roll: f32) {
        let side = glam::Quat::from_axis_angle(dir, roll) * Vec3::Y.cross(dir).normalize();
        let upper = dir.cross(side);
        let start = part.vertices.len();
        let first_index = part.indices.len();
        let rings = self.margin.len() - 1;
        let axis = Shoot {
            root,
            delta: dir * length,
            bend: Vec3::ZERO,
            radius: self.thickness,
        };
        tube(part, axis, rings, 6);
        for vertex in &mut part.vertices[start..] {
            let t = vertex.uv[1];
            let (sin, cos) = (vertex.uv[0] * TAU).sin_cos();
            let breadth = self.margin[(t * rings as f32).round() as usize];
            let fold = (0.035 * cos.abs() * breadth - 0.070 * t * t) * length;
            let along = t + 0.026 * (PI * t).sin() * cos.abs();
            vertex.pos = (root
                + dir * (along * length)
                + side * (cos * breadth * 0.5 * self.width_ratio * length)
                + upper * (fold + sin * self.thickness))
                .to_array();
        }
        update_normals(part, start, first_index);
    }
}
