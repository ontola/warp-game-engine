//! Pinus sylvestris short shoots. A pair of twisted needles emerges from a
//! common sheath. The woody shoot ends in an ovoid, imbricate winter bud.
use super::shoots::{identify, radial, update_normals};
use crate::{
    cluster::Vertex,
    foliage_assembly::PartGeometry,
    plant_growth_assembly::{Look, needle_base_uv},
    plant_spray::{Shoot, tube, unit},
};
use glam::Vec3;
use std::f32::consts::TAU;

pub(super) fn parts(look: &Look) -> Vec<PartGeometry> {
    let (needles_a, twig_a) = shoot(look, 0);
    let (needles_b, twig_b) = shoot(look, 1);
    vec![needles_a, needles_b, twig_a, twig_b]
}

pub(super) fn shoot(look: &Look, variant: u32) -> (PartGeometry, PartGeometry) {
    let empty = || PartGeometry {
        id: 0,
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let (mut needles, mut wood) = (empty(), empty());
    let reach = look.brush_reach.clamp(0.01, 1.0);
    let leader = Shoot {
        root: -Vec3::Y * reach,
        delta: Vec3::Y * reach,
        bend: Vec3::new(0.035 * (1.0 - 2.0 * variant as f32), 0.0, 0.024) * reach,
        radius: 0.0028,
    };
    let pairs = look.brush_needles.clamp(6, 2000).div_ceil(2);
    let lateral_pairs = pairs * 3 / 10;
    let phase = TAU * unit(variant, 26, 0);
    // The previous year's whorl carries a leading shoot and two shorter laterals.
    // All three axes keep their own fascicles, right back to the junction.
    for i in 0..3 {
        let (axis, count) = if i == 0 {
            (leader, pairs - 2 * lateral_pairs)
        } else {
            let radial = radial(phase + (i - 1) as f32 * 2.74);
            let side = radial * (0.71 + 0.09 * unit(variant, 25, i));
            let direction = (side + Vec3::Y * (0.57 + 0.11 * unit(variant, 28, i))).normalize();
            (
                Shoot {
                    root: leader.point(0.025 + 0.055 * i as f32),
                    delta: direction * reach * (0.76 + 0.12 * unit(variant, 27, i)),
                    bend: Vec3::Y * (-0.034 * reach),
                    radius: 0.0022,
                },
                lateral_pairs,
            )
        };
        tube(&mut wood, axis, 4, 5);
        let direction = axis.delta.normalize();
        terminal_bud(
            &mut wood,
            axis.point(1.0),
            direction,
            (0.0065 + 0.0015 * unit(variant, 8, i)) * if i == 0 { 1.20 } else { 0.86 },
        );
        if i == 0 {
            for side in [-1.0, 1.0] {
                let origin = Vec3::Y * -0.0015 + Vec3::X * (side * 0.0007);
                terminal_bud(
                    &mut wood,
                    origin,
                    (Vec3::Y + Vec3::X * (side * 0.55)).normalize(),
                    0.0044,
                );
            }
        }
        fascicles(&mut needles, &mut wood, look, axis, count, 3 * variant + i);
    }
    update_normals(&mut needles, 0, 0);
    identify(&mut needles);
    identify(&mut wood);
    (needles, wood)
}

fn fascicles(
    needles: &mut PartGeometry,
    wood: &mut PartGeometry,
    look: &Look,
    axis: Shoot,
    pairs: u32,
    variant: u32,
) {
    let direction = axis.delta.normalize();
    let rotate = glam::Quat::from_rotation_arc(Vec3::Y, direction);
    let phase = TAU * unit(variant, 2, 0);
    for i in 0..pairs {
        let t = (i as f32 + 0.32 + 0.36 * unit(variant, 3, i)) / pairs as f32;
        let t = 0.035 + 0.90 * t.powf(1.0 / look.ball.clamp(0.5, 8.0));
        let side = rotate * radial(phase + 2.399_963 * i as f32 + 0.45 * unit(variant, 4, i));
        let tangent = direction.cross(side);
        let tilt = look.tip_spread + (look.brush_spread - look.tip_spread) * (1.0 - t).sqrt();
        let out = (side * tilt.sin() + direction * tilt.cos()).normalize();
        let root = axis.point(t) + side * axis.radius * (1.0 - 0.86 * t);
        let sheath = Shoot {
            root,
            delta: out * (0.0035 + 0.0018 * unit(variant, 5, i)),
            bend: Vec3::ZERO,
            radius: 0.00095,
        };
        fascicle_sheath(wood, sheath, tangent);
        let start = sheath.point(1.0);
        let open = 0.11 + 0.24 * unit(variant, 7, i);
        let length = look.needle_length.clamp(0.01, 1.0) * (0.79 + 0.34 * unit(variant, 9, i));
        for sign in [-1.0, 1.0] {
            let dir = (out + tangent * (sign * open)).normalize();
            let width = look.needle_width.clamp(0.0005, 0.02);
            needle(
                needles,
                start + tangent * (sign * width * 0.3),
                dir,
                tangent,
                length * (1.0 + sign * 0.035),
                width,
                sign,
            );
        }
    }
}

/// A few millimetres of papery sheath flare from the woody axis to a shared mouth.
fn fascicle_sheath(wood: &mut PartGeometry, sheath: Shoot, side: Vec3) {
    let first = wood.vertices.len();
    let index = wood.indices.len();
    let direction = sheath.delta.normalize();
    let across = direction.cross(side);
    for j in 0..3 {
        let angle = TAU * j as f32 / 3.0;
        let radial = side * angle.cos() + across * angle.sin();
        wood.vertices.push(Vertex {
            pos: (sheath.point(1.0) + radial * sheath.radius).to_array(),
            normal: radial.to_array(),
            uv: [0.0; 2],
        });
    }
    wood.vertices.push(Vertex {
        pos: sheath.root.to_array(),
        normal: (-direction).to_array(),
        uv: [0.0; 2],
    });
    let b = first as u32;
    wood.indices.extend([
        b,
        b + 1,
        b + 2,
        b + 3,
        b + 1,
        b,
        b + 3,
        b + 2,
        b + 1,
        b + 3,
        b,
        b + 2,
    ]);
    update_normals(wood, first, index);
}

fn needle(
    part: &mut PartGeometry,
    base: Vec3,
    dir: Vec3,
    tangent: Vec3,
    length: f32,
    radius: f32,
    sign: f32,
) {
    let side = (tangent - dir * dir.dot(tangent)).normalize();
    let cross = dir.cross(side);
    let first = part.vertices.len() as u32;
    // A needle narrows inside its sheath; a triangular section carries its two
    // faces and narrow edge. The skewed tips give the pairs their slight twist.
    part.vertices.push(Vertex {
        pos: base.to_array(),
        normal: (-dir).to_array(),
        uv: [0.5, 1.0],
    });
    for j in 0..3 {
        let angle = 0.57 * sign + TAU * j as f32 / 3.0;
        let outward = side * angle.cos() + cross * (0.79 * angle.sin());
        let centre = base + dir * (0.36 * length);
        part.vertices.push(Vertex {
            pos: (centre + outward * radius).to_array(),
            normal: outward.normalize().to_array(),
            uv: needle_base_uv(outward.normalize()),
        });
    }
    let tip = base + (dir + cross * 0.075 + side * (sign * 0.055)).normalize() * length;
    part.vertices.push(Vertex {
        pos: tip.to_array(),
        normal: dir.to_array(),
        uv: [0.5, 1.0],
    });
    for j in 0..3 {
        let (a, b) = (first + 1 + j, first + 1 + (j + 1) % 3);
        part.indices.extend([first, b, a, a, b, first + 4]);
    }
}
fn terminal_bud(part: &mut PartGeometry, base: Vec3, direction: Vec3, height: f32) {
    // Staggered, overlapping scale rows form a swollen base and tapered tip.
    let first = part.vertices.len();
    let first_index = part.indices.len();
    let rotate = glam::Quat::from_rotation_arc(Vec3::Y, direction);
    let mut vertex = |point: Vec3| {
        let i = part.vertices.len() as u32;
        part.vertices.push(Vertex {
            pos: (base + rotate * point).to_array(),
            normal: [0.0; 3],
            uv: [0.0; 2],
        });
        i
    };
    let at = |row: usize, s: usize| {
        let (y, radius) = [(0.0, 0.18), (0.31, 0.32), (0.63, 0.24), (0.87, 0.13)][row];
        (Vec3::Y * y + radial(TAU * (s as f32 + 0.5 * row as f32) / 7.0) * radius) * height
    };
    let bottom = vertex(Vec3::ZERO);
    let top = vertex(Vec3::Y * height);
    let ring: [[u32; 7]; 4] =
        std::array::from_fn(|row| std::array::from_fn(|s| vertex(at(row, s))));
    for s in 0..7 {
        let next = (s + 1) % 7;
        part.indices.extend([bottom, ring[0][s], ring[0][next]]);
        for row in 0..3 {
            let (a, b, c, d) = (
                ring[row][s],
                ring[row][next],
                ring[row + 1][s],
                ring[row + 1][next],
            );
            // A little lifted tip makes the lip of each scale catch the light.
            let centre = (at(row, s) + at(row, next) + at(row + 1, s) + at(row + 1, next)) * 0.25;
            if height < 0.0066 || row != 1 {
                part.indices.extend([a, c, b, b, c, d]);
                continue;
            }
            let ridge =
                vertex(centre + Vec3::new(centre.x, 0.0, centre.z).normalize() * (0.048 * height));
            part.indices
                .extend([a, ridge, b, b, ridge, d, d, ridge, c, c, ridge, a]);
        }
        part.indices.extend([ring[3][s], top, ring[3][next]]);
    }
    update_normals(part, first, first_index);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needle_pairs_share_a_sheath_and_end_below_the_scaled_bud() {
        let model = crate::plant_species::grown("scots_pine").unwrap();
        let library = parts(&model.look);
        let ids: std::collections::BTreeSet<_> = library.iter().map(|part| part.id).collect();
        assert_eq!(ids.len(), 4);
        for (needles, twig) in library[..2].iter().zip(&library[2..]) {
            let pairs = needles.vertices.chunks_exact(10);
            assert!(pairs.remainder().is_empty());
            assert_eq!(pairs.len() * 2, model.look.brush_needles as usize);
            for pair in pairs {
                let (a, b) = (Vec3::from(pair[0].pos), Vec3::from(pair[5].pos));
                assert!(a.distance(b) < 0.001 && a.y < 0.0);
                for (base, tip) in [(a, pair[4].pos), (b, pair[9].pos)] {
                    let length = base.distance(tip.into());
                    assert!((0.025..=0.07).contains(&length), "{length}");
                }
                assert!(Vec3::from(pair[4].pos).distance(pair[9].pos.into()) > 0.008);
            }
            let top = twig.vertices.iter().map(|v| v.pos[1]).fold(0.0, f32::max);
            assert!((0.007..=0.011).contains(&top));
        }
    }
}
