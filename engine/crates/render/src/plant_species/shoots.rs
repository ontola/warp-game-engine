//! Species-level arrangements of the existing solid leaf blade and tapered shoot.
//! Coordinates are metres in the growth tip's +Y frame. Each recipe returns
//! a single shared part, regardless of how many leaflets it carries.
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, leaf, tube, unit},
};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

#[path = "leaf_arrangement.rs"]
pub(super) mod arrangement;
#[path = "grass.rs"]
mod grass;
#[path = "leafy_twigs.rs"]
pub(super) mod leafy_twigs;
#[path = "juniper.rs"]
pub(super) mod juniper;

pub(super) use arrangement::paired_sites;
use arrangement::{OppositeShoot, PalmateShoot, PinnateShoot};

#[derive(Clone, Copy, Debug)]
pub(super) enum ShootRecipe {
    LeafyTwig(&'static leafy_twigs::Twig),
    WoollyWillow,
    Crowberry,
    Juniper,
    ScotsPine,
    Broom,
    Dracaena,
    Opposite(&'static OppositeShoot),
    Palmate(&'static PalmateShoot),
    Pinnate(&'static PinnateShoot),
    Fern,
    Drinn,
    Raffia,
}

impl ShootRecipe {
    pub(super) const COFFEE: Self = Self::Opposite(&arrangement::COFFEE);
    pub(super) const KAPOK: Self = Self::Palmate(&arrangement::KAPOK);
    pub(super) const MAHOGANY: Self = Self::Pinnate(&arrangement::MAHOGANY);

    pub(super) fn build(self, variant: u32) -> PartGeometry {
        let mut part = PartGeometry {
            id: 0,
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let v = variant % 3;
        match self {
            Self::LeafyTwig(recipe) => recipe.build(&mut part, v),
            Self::WoollyWillow => willow(&mut part, v),
            Self::Juniper => return juniper::shoot(v % 2).needles,
            Self::ScotsPine => unreachable!("paired pine needles and their wood are built together"),
            Self::Crowberry => crowberry(&mut part, v),
            Self::Broom => broom(&mut part, v),
            Self::Dracaena => dracaena(&mut part, v),
            Self::Opposite(recipe) => recipe.build(&mut part, v),
            Self::Palmate(recipe) => recipe.build(&mut part, v),
            Self::Pinnate(recipe) => recipe.build(&mut part, v),
            Self::Fern => super::fern::frond(&mut part, v),
            Self::Drinn => grass::drinn(&mut part, v),
            Self::Raffia => raffia(&mut part, v),
        }
        identify(&mut part);
        part
    }
}

/// The shared cook keys on part IDs. Hash the final source positions, normals, UVs
/// and topology so two equally tessellated but different leaves cannot alias.
pub(super) fn identify(shoot: &mut PartGeometry) {
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

pub(super) fn stalk(
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
pub(super) fn blade(part: &mut PartGeometry, root: Vec3, delta: Vec3, facing: Vec3, width: f32, curl: f32) {
    let dir = delta.normalize();
    let across = facing.cross(dir).normalize();
    leaf(part, root, dir, across, delta.length(), width, curl);
}

pub(super) fn radial(angle: f32) -> Vec3 {
    Vec3::new(angle.cos(), 0.0, angle.sin())
}

/// An evergreen Empetrum extension shoot. Alternate, blunt, revolute leaves
/// crowd the apex. All dimensions are metres (Blefjell reference, Burke flora).
fn crowberry(part: &mut PartGeometry, v: u32) {
    let axis = stalk(part, Vec3::ZERO, Vec3::new(0.002, 0.031 + 0.004 * v as f32, 0.001),
        Vec3::X * 0.0015, 0.00085, 5);
    for i in 0..25 {
        let f = (i as f32 + 0.6) / 25.0;
        let t = 1.0 - (1.0 - f).powf(1.22);
        let out = radial(2.40 * i as f32 + v as f32 + 0.22 * unit(v, 1, i));
        let dir = (out + Vec3::Y * (0.48 + 1.45 * t)).normalize();
        let length = (0.0052 + 0.0025 * unit(v, 2, i)) * (1.0 - 0.19 * t);
        let side = Vec3::Y.cross(dir).normalize();
        let upper = dir.cross(side);
        let leaf = Shoot { root: axis.point(t), delta: dir * length,
            bend: upper * length * 0.055, radius: 0.00062 };
        let start = part.vertices.len();
        let indices = part.indices.len();
        tube(part, leaf, 4, 8);
        for vertex in &mut part.vertices[start..] {
            let y = vertex.uv[1];
            let width = [0.60, 0.91, 1.0, 0.94, 0.60][(4.0 * y).round() as usize];
            let (sin, cos) = (TAU * vertex.uv[0]).sin_cos();
            // Inset the lower centre between the two rolled margins.
            let groove = if sin < -0.9 && y > 0.0 && y < 1.0 { 0.00031 } else { 0.0 };
            vertex.pos = (leaf.point(y) + width *
                (side * cos * 0.00068 + upper * (sin * 0.00044 + groove))).to_array();
        }
        update_normals(part, start, indices);
    }
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
    // Salix lanata: a full, round shoulder, smooth sides and a short point.
    let leaf = leafy_twigs::BladeShape {
        margin: &[0.15, 0.77, 0.97, 1.0, 0.90, 0.64, 0.12],
        width_ratio: 0.74,
        thickness: 0.00044,
    };
    for i in 0..8 {
        let t = 0.12 + i as f32 * 0.118;
        let out = radial(2.40 * i as f32 + rotation);
        let dir = (out + Vec3::Y * (0.32 + 0.70 * t)).normalize();
        let petiole = stalk(part, axis.point(t), dir * 0.006, Vec3::ZERO, 0.00085, 1);
        let length = (0.040 + 0.016 * unit(v, 1, i)) * (1.0 - 0.35 * t.powi(3));
        let roll = 0.56 * (unit(v, 2, i) - 0.5);
        leaf.build(part, petiole.point(1.0), dir, length, roll);
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
        let position = i as f32 / (count - 1) as f32;
        let dir = (out + Vec3::Y * (0.10 + 0.55 * position + 0.11 * unit(v, 3, i))).normalize();
        let base = stalk(part, axis.point(t), dir * 0.012, Vec3::ZERO, 0.0015, 1);
        let length = 0.115 + 0.033 * unit(v, 4, i);
        // Mature blades bow past the horizontal. Younger leaves still cup
        // towards the apex. Their midribs roll independently at each petiole.
        let roll = 0.90 * (unit(v, 5, i) - 0.5);
        let facing = glam::Quat::from_axis_angle(dir, roll) * Vec3::Y;
        let hang = 0.37 - 0.24 * position;
        blade(part, base.point(1.0), dir * length, facing, length * 0.32, -hang * length);
    }
}

fn raffia(part: &mut PartGeometry, v: u32) {
    // A young Raphia laurentii frond: the bare petiole stands up from the
    // sheath; the older rachis arches out and its long pinnae hang at the ends.
    let length = [2.65, 3.24, 3.57][v as usize];
    let reach = [0.27, 0.50, 0.62][v as usize];
    let rise = [1.0, 0.92, 0.80][v as usize];
    let delta = Vec3::new(0.027, rise, reach) * length;
    let bend = Vec3::new(0.029, 0.15, -0.17) * length;
    let axis = stalk(part, Vec3::Y * 0.024, delta, bend, 0.029, 14);
    let pairs = 34 + 3 * v;
    for site in paired_sites(pairs, 0.27, 0.985, 0.50) {
        let f = site.fraction;
        let t = site.t;
        let tangent = (delta + bend * (4.0 - 8.0 * t)).normalize();
        let across = tangent.cross(Vec3::Z).normalize();
        let upper = tangent.cross(across).normalize();
        let roll = 0.16 * (TAU * f + 0.73 * v as f32).sin();
        let flank = across * roll.cos() + upper * roll.sin();
        let envelope = (PI * (0.10 + 0.88 * f)).sin();
        let len = (0.14 + 0.63 * envelope) * (0.91 + 0.15 * unit(v, 1, site.index));
        let axis_dir = (flank * site.side + tangent * (0.17 + 0.63 * f)
            - Vec3::Y * (0.05 + 0.20 * unit(v, 2, site.index))).normalize();
        let face = upper + across * site.side * (0.19 + 0.32 * unit(v, 3, site.index));
        blade(part, axis.point(t), axis_dir * len, face,
            0.013 + 0.024 * envelope, -len * (0.10 + 0.12 * envelope));
    }
}

// The lamina and petal change the silhouette of their source sections. Compute
// normals from the finished solid, angle-weighted as in the shared plant parts.
pub(super) fn update_normals(part: &mut PartGeometry, first: usize, first_index: usize) {
    let vertices = &mut part.vertices[first..];
    let mut sum = vec![glam::DVec3::ZERO; vertices.len()];
    for tri in part.indices[first_index..].chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|v| v as usize - first);
        let pos = |i: usize| Vec3::from(vertices[i].pos).as_dvec3();
        let n = (pos(b) - pos(a)).cross(pos(c) - pos(a)).normalize();
        for (i, j, k) in [(a, b, c), (b, c, a), (c, a, b)] {
            let (u, v) = (pos(j) - pos(i), pos(k) - pos(i));
            sum[i] += n * u.cross(v).length().atan2(u.dot(v));
        }
    }
    for (vertex, n) in vertices.iter_mut().zip(sum) {
        vertex.normal = n.normalize().as_vec3().to_array();
    }
}
