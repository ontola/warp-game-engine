//! Vegetative shoot of mountain-grown Juniperus communis. Dimensions in metres.
use super::{identify, stalk, update_normals};
use crate::{
    cluster::Vertex,
    foliage_assembly::PartGeometry,
    plant_spray::{Shoot, unit},
};
use glam::Vec3;
use std::f32::consts::{FRAC_PI_3, TAU};

pub(crate) struct ShootParts {
    pub wood: PartGeometry,
    pub needles: PartGeometry,
    pub bands: PartGeometry,
}

pub(crate) fn shoot(variant: u32) -> ShootParts {
    let empty = || PartGeometry {
        id: 0,
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let mut parts = ShootParts {
        wood: empty(),
        needles: empty(),
        bands: empty(),
    };
    // Both leaf variants grow on the same woody fork. Leave old brown
    // internodes exposed below the short, densely needled lateral shoots.
    let leader = stalk(
        &mut parts.wood,
        Vec3::ZERO,
        Vec3::new(0.003, 0.052, 0.001),
        Vec3::new(-0.002, 0.0, 0.001),
        0.0010,
        8,
    );
    foliage(&mut parts, leader, 0.24, 12, variant, 0);
    for (i, (site, delta, whorls)) in [
        (0.18, Vec3::new(0.024, 0.021, 0.008), 7),
        (0.49, Vec3::new(-0.017, 0.025, 0.015), 6),
        (0.65, Vec3::new(0.007, 0.013, -0.016), 5),
    ]
    .into_iter()
    .enumerate()
    {
        let twig = stalk(
            &mut parts.wood,
            leader.point(site),
            delta,
            Vec3::new(-delta.x, 0.0, -delta.z) * 0.08,
            0.00082,
            5,
        );
        foliage(&mut parts, twig, 0.12, whorls, variant, i as u32 + 1);
    }
    for part in [&mut parts.wood, &mut parts.needles, &mut parts.bands] {
        identify(part);
    }
    parts
}

fn foliage(parts: &mut ShootParts, twig: Shoot, start: f32, nodes: u32, v: u32, branch: u32) {
    let rotation = TAU * unit(v, 1, branch);
    for node in 0..nodes {
        let f = node as f32 / (nodes - 1) as f32;
        let t = start + (0.98 - start) * f;
        let tangent = (twig.delta + twig.bend * (4.0 - 8.0 * t)).normalize();
        let side = tangent.cross(Vec3::Z).normalize();
        let depth = tangent.cross(side);
        let azimuth = rotation + FRAC_PI_3 * node as f32;
        for rank in 0..3 {
            let key = branch * 64 + node * 3 + rank;
            let angle = azimuth + TAU * rank as f32 / 3.0;
            let out = side * angle.cos() + depth * angle.sin();
            let tilt = 1.23 - 0.63 * f * f + 0.22 * (unit(v, 2, key) - 0.5);
            let dir = tangent * tilt.cos() + out * tilt.sin();
            let across = tangent.cross(out);
            let length = (0.0075 + 0.0030 * unit(v, 3, key)) * (1.0 - 0.48 * f.powi(5));
            let width = (0.00145 + 0.00040 * unit(v, 4, key)) * (1.0 - 0.34 * f.powi(5));
            let awl = Awl {
                root: twig.point(t) + out * twig.radius * (1.0 - 0.86 * t) * 0.80,
                dir,
                across,
                upper: dir.cross(across).normalize(),
                length,
                width,
            };
            // The adaxial groove faces the growing end of the twig. A broad
            // pale band lies between its two green margins; the keeled back is green.
            awl.solid(
                &mut parts.needles,
                &[[-0.5, 0.0], [0.0, -0.30], [0.5, 0.0], [0.0, -0.055]],
            );
            awl.solid(
                &mut parts.bands,
                &[[-0.32, -0.016], [0.0, -0.17], [0.32, -0.016], [0.0, -0.031]],
            );
        }
    }
}

struct Awl {
    root: Vec3,
    dir: Vec3,
    across: Vec3,
    upper: Vec3,
    length: f32,
    width: f32,
}

impl Awl {
    /// Close both coloured solids, including their inserted bases. The pale
    /// section straddles the groove surface and has its own physical underside.
    fn solid(&self, part: &mut PartGeometry, section: &[[f32; 2]]) {
        let first = part.vertices.len() as u32;
        let first_index = part.indices.len();
        let sides = section.len() as u32;
        let centre =
            |t: f32| self.root + (self.dir * t + self.upper * (0.10 * t * t)) * self.length;
        let vertex = |p: Vec3, uv| Vertex {
            pos: p.to_array(),
            normal: [0.0; 3],
            uv,
        };
        for (station, breadth) in [(0.0, 0.78), (0.50, 0.96)] {
            let axis = (self.dir + self.upper * (0.20 * station)).normalize();
            let upper = axis.cross(self.across).normalize();
            for &[x, y] in section {
                let pos = centre(station) + (self.across * x + upper * y) * (self.width * breadth);
                part.vertices.push(vertex(pos, [x + 0.5, station]));
            }
        }
        for i in 0..sides {
            let j = (i + 1) % sides;
            part.indices.extend([
                first + i,
                first + j,
                first + sides + j,
                first + i,
                first + sides + j,
                first + sides + i,
            ]);
        }
        let tip = part.vertices.len() as u32;
        part.vertices.push(vertex(centre(1.0), [0.5, 1.0]));
        for i in 0..sides {
            let j = (i + 1) % sides;
            part.indices
                .extend([first + sides + i, first + sides + j, tip]);
        }
        // Fan from the deepest point on the keel so the insert end does not
        // create nearly collinear triangles across the shallow upper groove.
        let anchor = section
            .iter()
            .enumerate()
            .min_by(|(_, p), (_, q)| p[1].total_cmp(&q[1]))
            .unwrap()
            .0 as u32;
        for i in 1..sides - 1 {
            part.indices.extend([
                first + anchor,
                first + (anchor + i + 1) % sides,
                first + (anchor + i) % sides,
            ]);
        }
        // Shared base-ring vertices also shade the cut face against the twig.
        // Its outward normal contributes alongside the keel and groove faces.
        update_normals(part, first as usize, first_index);
    }
}
