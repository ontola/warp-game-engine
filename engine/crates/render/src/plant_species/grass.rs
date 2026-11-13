//! Drinn (*Stipagrostis pungens*), in metres. A rhizome sends up leafy and
//! fertile tillers. The catalogue places these axes independently around the tussock.
use super::{blade, radial, stalk};
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, tube, unit},
};
use glam::Vec3;
use std::f32::consts::PI;

pub(super) fn drinn(part: &mut PartGeometry, v: u32) {
    let fertile = v == 2;
    let length = [0.36, 0.47, 0.62][v as usize];
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.014 * (v as f32 - 1.0), length, 0.075),
        Vec3::new(0.0, 0.012, -0.023),
        0.0024,
        6,
    );
    // Blades emerge in two ranks at different nodes; the upper flag blade is
    // much shorter than a basal leaf.
    for node in 0..6 {
        let t = [0.08, 0.15, 0.25, 0.40, 0.58, 0.78][node as usize];
        let sign = if node % 2 == 0 { 1.0 } else { -1.0 };
        let spread = 0.26 + 0.30 * unit(v, 7, node);
        let side = radial(0.48 * (v as f32 - 1.0) + 0.32 * (unit(v, 3, node) - 0.5)) * sign;
        let outward = (side * spread + Vec3::Z * (0.17 + 0.16 * t)).normalize();
        let dir = (outward * (0.47 - 0.21 * t) + Vec3::Y).normalize();
        let blade_length = (0.235 + 0.060 * unit(v, 4, node)) * (1.0 - 0.50 * t * t);
        blade(
            part,
            axis.point(t),
            dir * blade_length,
            outward,
            0.0024 + 0.0005 * unit(v, 5, node),
            0.012 + 0.022 * (1.0 - t),
        );
    }
    // A developed tiller branches from its low nodes. A short branch carries
    // its own fan of sharp leaves; the flowering culm extends above them.
    for branch in 0..=v {
        let side = radial(2.34 * branch as f32 + 0.71 * v as f32);
        let out = (side * 0.65 + Vec3::Z * 0.48).normalize();
        let reach = 0.10 + 0.08 * unit(v, 8, branch);
        let lateral = stalk(
            part,
            axis.point(0.045 + branch as f32 * 0.073),
            out * reach + Vec3::Y * (0.12 + 0.12 * unit(v, 9, branch)),
            -out * 0.018,
            0.00165,
            4,
        );
        let across = Vec3::Y.cross(out);
        for node in 0..4 {
            let flank = if node % 2 == 0 { 1.0 } else { -1.0 };
            let along = 0.18 + node as f32 * 0.24;
            let face = (out + flank * 0.54 * across).normalize();
            let dir = (face * (0.75 + 0.42 * unit(branch, 10, node)) + Vec3::Y).normalize();
            let length = 0.158 + 0.074 * unit(v + branch, 11, node);
            blade(
                part,
                lateral.point(along),
                dir * length,
                face,
                0.0026,
                0.022,
            );
        }
    }
    if fertile {
        panicle(part, axis.point(1.0));
    }
}

fn panicle(part: &mut PartGeometry, base: Vec3) {
    // A contracted, lance-shaped head with short pedicels. Three 12–15 mm
    // awns emerge together from the tip of each enclosed floret.
    let axis = stalk(
        part,
        base,
        Vec3::new(0.009, 0.14, 0.012),
        Vec3::ZERO,
        0.0010,
        4,
    );
    for node in 0..11 {
        let t = (node as f32 + 0.35) / 11.0;
        let spread = 0.002 + 0.004 * (PI * t).sin();
        for flank in 0..2 {
            let out = radial(2.4 * node as f32 + 2.1 * flank as f32);
            let origin = axis.point(t + flank as f32 * 0.014);
            let pedicel = stalk(
                part,
                origin,
                out * spread + Vec3::Y * 0.008,
                Vec3::ZERO,
                0.00085,
                1,
            );
            let floret = pedicel.point(1.0);
            let dir = (out * 0.28 + Vec3::Y).normalize();
            blade(part, floret, dir * 0.014, out, 0.0018, 0.0);
            for awn in 0..3 {
                let splay = radial(2.094 * awn as f32 + 1.4 * node as f32);
                let delta =
                    (splay * 0.33 + dir).normalize() * (0.012 + 0.003 * unit(node, flank, awn));
                let awn = Shoot {
                    root: floret + dir * 0.014,
                    delta,
                    bend: -splay * 0.0006,
                    radius: 0.00056,
                };
                tube(part, awn, 2, 3);
            }
        }
    }
}
