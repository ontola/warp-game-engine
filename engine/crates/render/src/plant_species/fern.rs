//! A sterile frond of Nephrolepis biserrata. Local +Z points out of the crown.
use super::shoots::{blade, paired_sites, stalk};
use crate::{foliage_assembly::PartGeometry, plant_spray::unit};
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
        blade(part, rachis.point(site.t), dir * length, upper, width, -droop * length);
    }
}
