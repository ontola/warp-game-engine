//! Species-level arrangements of the existing solid leaf blade and tapered shoot.
//! Coordinates are metres in the growth tip's +Y frame. Each recipe returns
//! a single shared part, regardless of how many leaflets it carries.
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{self, Shoot, leaf, tube, unit},
};
use glam::Vec3;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

#[derive(Clone, Copy, Debug)]
pub(super) enum ShootRecipe {
    Broadleaf([f32; 3]),
    WoollyWillow,
    Broom,
    Dracaena,
    Coffee,
    Kapok,
    Mahogany,
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
            Self::Coffee => coffee(&mut part, v),
            Self::Kapok => kapok(&mut part, v),
            Self::Mahogany => mahogany(&mut part, v),
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
    // The green assimilating shoots carry little persistent leaf. Some remain
    // upright; on longer sprays the soft distal growth bends past the arch.
    let length = 0.35 + 0.065 * v as f32;
    let reach = [0.05, 0.22, 0.32][v as usize];
    let axis = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(reach, length * (1.0 - 0.26 * v as f32), 0.023),
        Vec3::new(-0.018, 0.065 * v as f32, 0.02),
        0.0026,
        8,
    );
    for i in 0..8 {
        let t = 0.09 + 0.10 * i as f32;
        let out = radial(2.40 * i as f32 + 0.63 * v as f32);
        let twig_length = (0.19 + 0.15 * unit(v, 1, i)) * (1.0 - 0.52 * t);
        let sag = 0.10 + 0.13 * v as f32 + 0.12 * unit(v, 2, i);
        let delta = (out * 0.55 + Vec3::X * (0.23 + reach) + Vec3::Y * (0.8 - sag * 1.7))
            .normalize()
            * twig_length;
        let bend = (Vec3::Y * sag - out * 0.06) * twig_length;
        let twig = stalk(part, axis.point(t), delta, bend, 0.00155, 6);
        if i % 3 == 0 {
            let split = delta * 0.54 + out * twig_length * 0.18;
            stalk(part, twig.point(0.36), split, bend * 0.64, 0.00105, 4);
        }
    }
}

// Coffea canephora: the lateral shoot carries two ranks of opposite, simple
// leaves. Each pair attaches at one node; the newest pair is still unfolding.
fn coffee(part: &mut PartGeometry, v: u32) {
    let pairs = 3 + v;
    let length = 0.075 * pairs as f32 + 0.03;
    let stem = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.02, length, -0.025),
        Vec3::new(-0.012, 0.0, 0.018),
        0.0026,
        6,
    );
    for node in 0..pairs {
        let t = (node as f32 + 0.65) / pairs as f32;
        let origin = stem.point(t);
        let azimuth = -0.35 * v as f32 + 0.32 * (unit(v, node, 11) - 0.5);
        let across = radial(azimuth);
        let face = radial(azimuth + FRAC_PI_2);
        let apical = node + 1 == pairs;
        for (side, sign) in [(0, -1.0), (1, 1.0)] {
            let out = across * sign;
            let vigor = unit(v, node, 12 + side);
            let opening = if apical { 0.90 } else { 0.19 };
            let petiole_dir = (out + Vec3::Y * (opening + 0.34 * vigor)).normalize();
            let petiole = stalk(
                part,
                origin,
                petiole_dir * (0.010 + 0.008 * vigor),
                Vec3::ZERO,
                0.0012,
                2,
            );
            let blade_m = if apical {
                0.080 + 0.045 * vigor
            } else {
                0.17 + 0.075 * vigor
            };
            let width = blade_m * (0.36 + 0.08 * unit(v, node, 14 + side));
            // Mature blades hang out of the shoot plane. The short young
            // pair still points forwards and cups the growing tip.
            let hang = if apical { -0.10 } else { 0.17 + 0.42 * vigor };
            let axis = (petiole_dir - face * hang).normalize();
            let roll = 0.88 * (unit(v, node, 16 + side) - 0.5);
            let facing = face * roll.cos() + Vec3::Y * roll.sin();
            let base = petiole.point(1.0);
            let curl = -blade_m * (0.18 + 0.17 * vigor);
            blade(part, base, axis * blade_m, facing, width, curl);
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

// Khaya anthotheca. Compound leaves spiral around the end of a shoot.
// Each leaf finishes in an opposite pair, with a bare petiole below its lowest pair.
fn mahogany(part: &mut PartGeometry, v: u32) {
    let shoot = stalk(
        part,
        Vec3::ZERO,
        Vec3::new(0.014, 0.23, 0.008),
        Vec3::Z * 0.006,
        0.0038,
        5,
    );
    for leaf in 0..4 {
        let out = radial(2.40 * leaf as f32 + 0.60 * v as f32);
        let across = out.cross(Vec3::Y);
        let length = 0.19 + 0.052 * unit(v, 1, leaf);
        // Upper leaves are newly expanded; the lower petioles arch out and down.
        let axis = (out + Vec3::Y * (-0.44 + 0.27 * leaf as f32)).normalize();
        let midrib = stalk(
            part,
            shoot.point(0.32 + 0.22 * leaf as f32),
            axis * length,
            Vec3::Y * 0.031,
            0.0018,
            5,
        );
        let pairs = 3 + (v + leaf) % 3;
        let petiole = 0.048 + 0.012 * unit(v, 2, leaf);
        for pair in 0..pairs {
            let f = pair as f32 / (pairs - 1) as f32;
            let t = f + (1.0 - f) * petiole / length;
            let node = midrib.point(t);
            for (index, side) in [-1.0, 1.0].into_iter().enumerate() {
                let key = leaf * 10 + pair * 2 + index as u32;
                let dir = (across * side + out * (0.38 + 0.43 * f)
                    - Vec3::Y * (0.24 + 0.27 * unit(v, 3, key)))
                .normalize();
                let stalklet = stalk(
                    part,
                    node,
                    dir * (0.007 + 0.003 * unit(v, 4, key)),
                    Vec3::ZERO,
                    0.0009,
                    1,
                );
                let blade_len = (0.10 + 0.040 * f) * (0.95 + 0.10 * unit(v, 5, key));
                blade(
                    part,
                    stalklet.point(1.0),
                    dir * blade_len,
                    Vec3::Y + out * (0.20 + 0.50 * unit(v, 6, key)),
                    blade_len * 0.41,
                    -0.018,
                );
            }
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
