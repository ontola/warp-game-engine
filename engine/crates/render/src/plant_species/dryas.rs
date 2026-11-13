//! Flowering mountain avens, Dryas octopetala. Each woody runner can root
//! at its tip. Short leafy side shoots turn up from the horizontal woody axis.
use super::shoots::{blade, identify, radial, update_normals};
use crate::{
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, tube, unit},
};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

pub(super) struct Mesh {
    pub(super) wood: PartGeometry,
    pub(super) leaves: PartGeometry,
    pub(super) petals: PartGeometry,
    pub(super) stamens: PartGeometry,
}

/// Soil attachment point of the next runner in the current runner's local frame.
pub(super) fn tip(id: u32) -> Vec3 {
    match id {
        0 => Vec3::new(-0.009, 0.0, 0.053),
        3 => Vec3::new(-0.004, 0.0, 0.027),
        _ => Vec3::new(0.012, 0.0, 0.065),
    }
}

fn runner(id: u32) -> Shoot {
    Shoot {
        root: Vec3::Y * 0.00075,
        delta: tip(id),
        bend: Vec3::new(if id == 0 { 0.004 } else { -0.004 }, 0.0019, 0.0),
        radius: 0.00075,
    }
}

fn node_count(id: u32) -> u32 {
    if id == 1 || id == 2 { 4 } else { 3 }
}

/// Base of an actual short shoot on the parent runner, in its local frame.
pub(super) fn node(id: u32, index: usize) -> Vec3 {
    runner(id).point(0.10 + 0.79 * index as f32 / (node_count(id) - 1) as f32)
}

fn empty() -> PartGeometry {
    PartGeometry {
        id: 0,
        vertices: Vec::new(),
        indices: Vec::new(),
    }
}

pub(super) fn shoot_variant(id: u32) -> Mesh {
    let v = if id == 2 { 1 } else { id };
    let mut mesh = Mesh {
        wood: empty(),
        leaves: empty(),
        petals: empty(),
        stamens: empty(),
    };
    tube(&mut mesh.wood, runner(id), 6, 5);
    for i in 0..node_count(id) {
        let point = node(id, i as usize);
        let out = radial(0.4 + 2.67 * i as f32 + 0.4 * v as f32);
        let length = 0.010 + 0.008 * unit(v, 1, i);
        let twig = Shoot {
            root: point,
            delta: out * length + Vec3::Y * (0.004 + 0.003 * unit(v, 2, i)),
            bend: out * 0.001 - Vec3::Y * 0.0013,
            radius: 0.00056,
        };
        tube(&mut mesh.wood, twig, 3, 3);
        // Evergreen leaves insert alternately near the end of the short shoot.
        // The smaller terminal leaf stands nearer upright than the older blades.
        let blades = 3 + (i + v) % 2;
        for j in 0..blades {
            let f = j as f32 / (blades - 1) as f32;
            let key = i * 4 + j;
            let heading = radial(2.40 * j as f32 + 2.67 * i as f32 + 0.56 * v as f32);
            let dir =
                (heading * (1.0 - 0.28 * f) + out * 0.23 + Vec3::Y * (0.14 + 0.58 * f)).normalize();
            let petiole = twig.point(0.50 + 0.46 * f);
            let reach = 0.0021 + 0.0015 * unit(v, 3, key);
            narrow_axis(&mut mesh.leaves, petiole, dir, Vec3::Y, reach, 0.00055);
            let length = (0.014 + 0.003 * unit(v, 4, key)) * (1.0 - 0.29 * f * f);
            crenate_leaf(&mut mesh.leaves, petiole + dir * reach, dir, length);
        }
        if id == 2 && i == 2 {
            bloom(&mut mesh, twig.point(0.92));
        }
    }
    // Parts with identical geometry share identity across the vegetative and
    // flowering stages. Blooming adds the peduncle and sepals to the green part.
    for part in [
        &mut mesh.wood,
        &mut mesh.leaves,
        &mut mesh.petals,
        &mut mesh.stamens,
    ] {
        if !part.vertices.is_empty() {
            identify(part);
        }
    }
    mesh
}

/// Petioles and filaments end inside the organ they support. Stretch the
/// existing closed blade along the axis and give it a nearly round cross-section.
fn narrow_axis(
    part: &mut PartGeometry,
    root: Vec3,
    dir: Vec3,
    facing: Vec3,
    length: f32,
    width: f32,
) {
    let start = part.vertices.len();
    let upper = dir.cross(facing.cross(dir)).normalize();
    blade(part, root, dir * length, facing, width, 0.0);
    let depth_scale = width / 0.0013;
    for vertex in &mut part.vertices[start..] {
        let pos = Vec3::from(vertex.pos);
        vertex.pos = (pos + upper * ((pos - root).dot(upper) * (depth_scale - 1.0))).to_array();
        let n = Vec3::from(vertex.normal);
        vertex.normal = (n + upper * (n.dot(upper) * (depth_scale.recip() - 1.0)))
            .normalize()
            .to_array();
    }
}

fn crenate_leaf(part: &mut PartGeometry, base: Vec3, dir: Vec3, length: f32) {
    let across = Vec3::Y.cross(dir).normalize();
    let upper = dir.cross(across);
    let width = length * 0.51;
    let first = part.vertices.len();
    let first_index = part.indices.len();
    // A single closed blade. Cross-sections alternate between the four pairs
    // of rounded teeth and their sinuses; the broad, truncated base meets the petiole.
    const BREADTH: [f32; 11] = [
        0.32, 0.82, 0.69, 1.0, 0.78, 0.98, 0.72, 0.87, 0.60, 0.69, 0.13,
    ];
    let axis = Shoot {
        root: base,
        delta: dir * length,
        bend: Vec3::ZERO,
        radius: width * 0.5,
    };
    tube(part, axis, BREADTH.len() - 1, 8);
    for p in &mut part.vertices[first..] {
        let t = p.uv[1];
        let ring = (t * (BREADTH.len() - 1) as f32).round() as usize;
        let angle = TAU * p.uv[0];
        let (sin, cos) = angle.sin_cos();
        let along = t + (1.0 - cos.abs()) * (PI * t).sin() * 0.044;
        let flank = 1.0 - (2.0 * cos.abs() - 1.0).powi(2);
        let vein = 0.00026 + 0.00022 * (1.0 - (TAU * 5.0 * t).cos());
        let depth = flank * vein * (PI * t).sin() - 0.0004 * t * t;
        p.pos = (base
            + dir * (length * along)
            + across * (0.5 * width * BREADTH[ring] * cos)
            + upper * (depth + 0.00019 * sin))
            .to_array();
    }
    update_normals(part, first, first_index);
}

fn bloom(mesh: &mut Mesh, base: Vec3) {
    let scape = Shoot {
        root: base,
        delta: Vec3::new(-0.004, 0.061, 0.005),
        bend: Vec3::new(0.0018, 0.0, 0.002),
        radius: 0.00052,
    };
    tube(&mut mesh.leaves, scape, 7, 3);
    let centre = scape.point(1.0);
    let upper = Vec3::new(-0.17, 1.0, 0.23).normalize();
    let u = Vec3::Z.cross(upper).normalize();
    let w = upper.cross(u);
    let ray = |angle: f32| u * angle.cos() + w * angle.sin();
    for i in 0..8 {
        let angle = TAU * i as f32 / 8.0;
        let out = ray(angle);
        let sepal = ray(angle + PI / 8.0);
        blade(
            &mut mesh.leaves,
            centre - upper * 0.0007,
            sepal * 0.0061 - upper * 0.0016,
            upper,
            0.0022,
            -0.0004,
        );
        // The petal's broad, rounded end faces outwards. Its narrow pointed
        // end tucks underneath the stamens, shaping the shallow white cup.
        let len = 0.0114 + 0.0014 * unit(2, 7, i);
        let angle = angle + 0.045 * (unit(2, 8, i) - 0.5);
        let dir = (ray(angle) + upper * (0.28 + 0.14 * unit(2, 9, i))).normalize();
        let root = centre + out * 0.0011 - upper * 0.0004;
        petal(&mut mesh.petals, root, dir, upper, len);
    }
    // Numerous free stamens form a low golden ring around the shorter centre.
    for i in 0..24 {
        let out = ray(i as f32 * 2.40);
        let f = ((i as f32 + 0.5) / 24.0).sqrt();
        let root = centre + out * (0.0004 + 0.0021 * f);
        let delta = out * (0.0004 + 0.0009 * f) + upper * (0.0017 + 0.0014 * f);
        narrow_axis(
            &mut mesh.stamens,
            root,
            delta.normalize(),
            out,
            delta.length(),
            0.00031,
        );
        let long = (upper * 0.58 + out).normalize();
        blade(
            &mut mesh.stamens,
            root + delta - long * 0.00018,
            long * 0.00115,
            upper,
            0.00063,
            0.0,
        );
    }
}

fn petal(part: &mut PartGeometry, root: Vec3, dir: Vec3, upper: Vec3, length: f32) {
    // Use the shared blade's closed sections as an obovate, spoon-shaped
    // petal. The final section lies just inside the lip, rounding the tip.
    let first = part.vertices.len();
    let first_index = part.indices.len();
    let across = upper.cross(dir).normalize();
    let normal = dir.cross(across);
    let width = length * 0.73;
    blade(part, root, dir * length, upper, width, 0.0);
    for p in &mut part.vertices[first..] {
        let old_t = p.uv[1];
        let (t, source_breadth, breadth) = match old_t {
            0.0 => (0.0, 1.0, 1.0),
            0.16 => (0.18, 0.48, 0.43),
            0.39 => (0.46, 1.0, 0.83),
            0.65 => (0.75, 0.88, 1.0),
            0.85 => (0.95, 0.47, 0.77),
            _ => (1.0, 1.0, 1.0),
        };
        let x = (p.uv[0] - 0.5) * breadth / source_breadth;
        let depth = (Vec3::from(p.pos) - root).dot(normal) * 0.42;
        let cup = length * (0.10 * t * t + 0.18 * x * x);
        p.pos =
            (root + dir * (length * t) + across * (x * width) + normal * (depth + cup)).to_array();
    }
    update_normals(part, first, first_index);
}
