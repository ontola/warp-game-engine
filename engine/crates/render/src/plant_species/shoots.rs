//! Species-level arrangements of the existing solid leaf blade and tapered shoot.
//! Coordinates are metres in the growth tip's +Y frame. Each recipe returns
//! a single shared part, regardless of how many leaflets it carries.
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{self, leaf, tube, unit, Shoot},
};
use glam::Vec3;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

#[derive(Clone, Copy, Debug)]
pub(super) enum ShootRecipe {
    Broadleaf([f32; 3]),
    WoollyWillow,
    Broom,
    Dracaena,
    Kapok,
    Raffia,
}

impl ShootRecipe {
    pub(super) fn build(self, variant: u32) -> PartGeometry {
        let mut part = PartGeometry {
            id: 0,
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let v = variant % 3;
        match self {
            Self::Broadleaf(scale) => part = leafy_shoot(v, Vec3::from(scale)),
            Self::WoollyWillow => willow(&mut part, v),
            Self::Broom => broom(&mut part, v),
            Self::Dracaena => dracaena(&mut part, v),
            Self::Kapok => kapok(&mut part, v),
            Self::Raffia => raffia(&mut part, v),
        }
        identify(&mut part);
        part
    }
}

// Assembly puts each foliage part on a growing shoot tip. Keep the spray's
// young green stalk and petioles with its closed leaf blades in one part so
// the leaves remain connected, including in the voxel representation.
fn leafy_shoot(variant: u32, scale: Vec3) -> PartGeometry {
    let (mut shoot, blades) = plant_spray::broadleaf_spray(variant);
    let offset = shoot.vertices.len() as u32;
    shoot.vertices.extend(blades.vertices);
    shoot
        .indices
        .extend(blades.indices.into_iter().map(|i| offset + i));
    for (i, v) in shoot.vertices.iter_mut().enumerate() {
        let mut pos = Vec3::from(v.pos) * scale;
        let normal = (Vec3::from(v.normal) / scale).normalize();
        if i < offset as usize {
            // Retain a fine, solid green stalk when the metre spray is
            // shortened for dwarf shrubs (particularly its tiny petioles).
            pos += normal * 0.0004;
        }
        v.pos = pos.to_array();
        v.normal = normal.to_array();
    }
    shoot
}

/// The shared cook keys on part IDs. Hash the final source positions, normals, UVs
/// and topology so two equally tessellated but different leaves cannot alias.
fn identify(shoot: &mut PartGeometry) {
    let vertex_bits = shoot
        .vertices
        .iter()
        .flat_map(|v| v.pos.into_iter().chain(v.normal).chain(v.uv))
        .map(f32::to_bits);
    shoot.id = vertex_bits
        .chain(shoot.indices.iter().copied())
        .fold(0x4752_4f57_4c45_4146u64, |h, bits| {
            (h ^ u64::from(bits)).wrapping_mul(0x100_0000_01b3)
        });
}

fn stalk(
    part: &mut PartGeometry,
    root: Vec3,
    delta: Vec3,
    bend: Vec3,
    radius: f32,
    rings: usize,
) -> Shoot {
    let shoot = Shoot {
        root,
        delta,
        bend,
        radius,
    };
    tube(part, shoot, rings, 5);
    shoot
}

// Blade points along delta; facing is its preferred face normal. A rolled
// leaflet uses a different face orientation while staying on the same axis.
fn blade(part: &mut PartGeometry, root: Vec3, delta: Vec3, facing: Vec3, width: f32, curl: f32) {
    let dir = delta.normalize();
    let across = facing.cross(dir).normalize();
    leaf(part, root, dir, across, delta.length(), width, curl);
}

fn radial(angle: f32) -> Vec3 {
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

fn willow(part: &mut PartGeometry, v: u32) {
    // Alternate, short-petioled, broadly ovate silver leaves. Upper leaves
    // turn up around the bud; the mature lower ones unfold away from the twig.
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.014, 0.121, 0.0),
        Vec3::Z * 0.008,
        0.002,
        5,
    );
    let rotation = 0.54 * v as f32;
    for i in 0..8 {
        let t = 0.12 + i as f32 * 0.118;
        let out = radial(2.40 * i as f32 + rotation);
        let dir = (out + Vec3::Y * (0.32 + 0.70 * t)).normalize();
        let petiole = stalk(part, axis.point(t), dir * 0.006, Vec3::ZERO, 0.00085, 1);
        let length = (0.040 + 0.016 * unit(v, 1, i)) * (1.0 - 0.35 * t.powi(3));
        blade(
            part,
            petiole.point(1.0),
            dir * length,
            Vec3::Y,
            length * 0.65,
            0.007,
        );
    }
}

fn broom(part: &mut PartGeometry, v: u32) {
    // Dry-season Retama photosynthesizes through its rush-like twigs. Long,
    // slender branchlets follow an arch; there are no persistent leaf blades.
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.04, 0.36, 0.025),
        Vec3::X * -0.038,
        0.0023,
        7,
    );
    for i in 0..9 {
        let t = 0.09 + 0.081 * i as f32;
        let out = radial(2.40 * i as f32 + 0.7 * v as f32);
        let length = (0.19 + 0.085 * unit(v, 1, i)) * (1.0 - 0.48 * t);
        let delta = (Vec3::Y + out * 0.36 + Vec3::X * 0.19).normalize() * length;
        let arc = (out * 0.13 - Vec3::X * 0.11) * length;
        let twig = stalk(part, axis.point(t), delta, arc, 0.0012, 5);
        if i % 2 == 0 {
            let split = (delta.normalize() + out * 0.32).normalize() * (length * 0.58);
            stalk(part, twig.point(0.37), split, out * 0.008, 0.00078, 4);
        }
    }
}

fn dracaena(part: &mut PartGeometry, v: u32) {
    // Thin canes end in loose clusters of three or four long elliptic blades.
    // The leaf bases spiral within one short section of the cane.
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.004, 0.085, 0.002),
        Vec3::Z * 0.003,
        0.0021,
        4,
    );
    let count = if v == 1 { 4 } else { 3 };
    for i in 0..count {
        let t = 0.67 + 0.14 * i as f32 / (count - 1) as f32 + 0.04 * unit(v, 1, i);
        let angle = i as f32 * TAU / count as f32 + 0.6 * v as f32 + 0.25 * unit(v, 2, i);
        let out = radial(angle);
        let dir = (out + Vec3::Y * (0.19 + 0.29 * unit(v, 3, i))).normalize();
        let base = stalk(part, axis.point(t), dir * 0.012, Vec3::ZERO, 0.0015, 1);
        let length = 0.115 + 0.033 * unit(v, 4, i);
        blade(
            part,
            base.point(1.0),
            dir * length,
            Vec3::Y,
            length * 0.32,
            -0.025,
        );
    }
}

fn kapok(part: &mut PartGeometry, v: u32) {
    // A compound leaf shares a single point among five to seven leaflets.
    // Several such long petioles alternate along the short end of the twig.
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.03, 0.39, 0.01),
        Vec3::Z * 0.021,
        0.0045,
        6,
    );
    for i in 0..6 {
        let t = 0.10 + 0.17 * i as f32;
        let out = radial(2.4 * i as f32 + 0.54 * v as f32);
        let across = out.cross(Vec3::Y);
        let petiole = stalk(
            part,
            axis.point(t),
            (out + Vec3::Y * 0.48) * (0.11 + 0.036 * unit(v, 1, i)),
            Vec3::Y * 0.012,
            0.0019,
            3,
        );
        let count = if (i + v) % 3 == 0 { 7 } else { 5 };
        for j in 0..count {
            let f = 2.0 * j as f32 / (count - 1) as f32 - 1.0;
            let angle = f * (1.40 + 0.07 * unit(v, 2, i));
            let dir = (out * angle.cos() + across * angle.sin() - Vec3::Y * 0.12).normalize();
            let length = (0.162 + 0.020 * unit(v, 3, i * 8 + j)) * (1.0 - 0.41 * f.abs().powi(2));
            blade(
                part,
                petiole.point(1.0),
                dir * length,
                Vec3::Y,
                length * 0.31,
                -0.023,
            );
        }
    }
}

fn raffia(part: &mut PartGeometry, v: u32) {
    // Juvenile frond. One uninterrupted rachis carries two staggered ranks of
    // linear leaflets. The unbladed petiole and fine tapered tip stay legible.
    let length = 1.57 + 0.14 * v as f32;
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.025, length, -0.20 * length),
        Vec3::new(0.012, 0.025, 0.115 * length),
        0.013,
        14,
    );
    let pairs = 24 + v;
    for i in 0..pairs * 2 {
        let t = 0.15 + 0.84 * i as f32 / (pairs * 2) as f32;
        let side = if i % 2 == 0 { -1.0 } else { 1.0 };
        let f = (t - 0.15) / 0.84;
        let envelope = (PI * (0.19 + f * 0.79)).sin();
        let len = (0.10 + 0.38 * envelope) * (0.89 + 0.15 * unit(v, 1, i));
        let dir = Vec3::new(
            side * (0.90 - 0.39 * f),
            0.26 + 0.56 * f,
            0.12 * (FRAC_PI_2 * f).cos() + 0.13 * (unit(v, 2, i) - 0.5),
        )
        .normalize();
        let faced = Vec3::new(0.0, 0.30 * (unit(v, 3, i) - 0.5), 1.0);
        blade(
            part,
            axis.point(t),
            dir * len,
            faced,
            0.020 + 0.012 * envelope,
            -0.20 * len,
        );
    }
}
