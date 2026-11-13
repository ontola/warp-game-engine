//! A sterile frond of Nephrolepis biserrata. Local +Z points out of the crown.
use super::shoots::{paired_sites, stalk, update_normals};
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, tube, unit},
};
use glam::Vec3;

pub(super) fn frond(part: &mut PartGeometry, variant: u32) {
    let size = 0.87 + 0.11 * variant as f32;
    let reach = [0.45, 0.71, 0.86][variant as usize];
    let rise = [1.14, 1.04, 0.83][variant as usize];
    let delta = Vec3::new(0.045 * (variant as f32 - 1.0), rise, reach) * size;
    let bend = Vec3::new(0.030, 0.25 + 0.045 * variant as f32, -0.08) * size;
    let rachis = stalk(part, Vec3::ZERO, delta, bend, 0.0025 * size, 24);

    // The bare stipe leads into two staggered ranks, with slightly reduced
    // basal pinnae and a long, finely divided point to the frond.
    let pairs = 32 + 3 * variant;
    for site in paired_sites(pairs, 0.16, 0.995, 0.47) {
        let f = site.fraction;
        let tangent = (delta + bend * (4.0 - 8.0 * site.t)).normalize();
        let face = tangent.cross(Vec3::X).normalize();
        let roll = 0.33 * (4.4 * site.t + variant as f32).sin()
            + 0.12 * (unit(variant, 3, site.index) - 0.5);
        let across = (Vec3::X * roll.cos() + face * roll.sin()).normalize();
        let upper = tangent.cross(across).normalize();
        let forward = 0.12 + 0.67 * f.powi(3) + 0.14 * unit(variant, 1, site.index);
        let dir = (across * site.side + tangent * forward - upper * 0.10).normalize();
        let basal = 0.69 + 0.31 * (f * 13.0).min(1.0);
        let taper = 1.0 - 0.95 * f.powi(4);
        let length = size * 0.124 * basal * taper * (0.94 + 0.09 * unit(variant, 2, site.index));
        let width = size * 0.023 * basal * taper.sqrt();
        let droop = 0.10 + 0.16 * unit(variant, 4, site.index);
        pinna(
            part,
            rachis.point(site.t),
            dir,
            upper,
            length,
            width,
            droop,
            site.side,
        );
    }
}

/// The asymmetric blade attaches directly to the rachis. Its nearly truncate
/// base and long parallel margins are distinct from the ovate broadleaf part.
fn pinna(
    part: &mut PartGeometry,
    root: Vec3,
    dir: Vec3,
    up: Vec3,
    length: f32,
    width: f32,
    droop: f32,
    side: f32,
) {
    let across = up.cross(dir).normalize();
    let upper = dir.cross(across);
    let first = part.vertices.len();
    let first_index = part.indices.len();
    let axis = Shoot {
        root,
        delta: dir * length,
        bend: Vec3::ZERO,
        radius: width * 0.50,
    };
    // Preserve the long parallel-sided lamina and narrowing, slightly
    // undulating tip with seven closed sections per leaflet.
    const SECTIONS: [(f32, f32); 7] = [
        (0.00, 0.20),
        (0.035, 0.91),
        (0.25, 1.00),
        (0.61, 0.91),
        (0.77, 0.78),
        (0.83, 0.57),
        (1.00, 0.085),
    ];
    tube(part, axis, SECTIONS.len() - 1, 4);
    for (ring, vertices) in part.vertices[first..].chunks_exact_mut(4).enumerate() {
        let (t, breadth) = SECTIONS[ring];
        let turn = 0.16 * side * t.powi(3);
        let tangent = (dir - upper * (2.0 * droop * t) - across * (4.0 * turn)).normalize();
        let face = tangent.cross(across);
        let centre = root + (dir * t - upper * (droop * t * t) - across * (turn * t)) * length;
        for (v, (x, z)) in
            vertices
                .iter_mut()
                .zip([(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)])
        {
            // A short asymmetrical attachment, with the acroscopic corner
            // swept towards the frond apex. Both sides meet the same solid axis.
            let skew = width * side * (0.18 * (1.0 - t).powi(6) + 0.37 * t.powi(4)) * breadth;
            let half_depth = (0.00029 + 0.00011 * (1.0 - t)) * breadth.sqrt();
            let vpos = centre
                + across * (width * breadth * 0.5 * x)
                + tangent * (x * skew)
                + face * (half_depth * z);
            v.pos = vpos.to_array();
        }
    }
    update_normals(part, first, first_index);
}
