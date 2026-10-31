//! Shared source-defined stage meshes. CPU generation once, instanced thereafter.
use super::visuals::{material, quad};
use glam::{Affine3A, Quat, Vec3};
use render::{
    Scene,
    cluster::{ClusterMesh, Vertex},
    primitives::{self, Primitive},
};
use std::f32::consts::{PI, TAU};
fn triangle(p: &mut Primitive, a: Vec3, b: Vec3, c: Vec3) {
    let n = (b - a).cross(c - a).normalize_or_zero();
    let k = p.vertices.len() as u32;
    for v in [a, b, c] {
        p.vertices.push(Vertex {
            pos: v.to_array(),
            normal: n.to_array(),
            uv: [0., 0.],
        });
    }
    p.indices.extend([k, k + 1, k + 2]);
}
fn append(p: &mut Primitive, q: Primitive, xf: Affine3A) {
    let k = p.vertices.len() as u32;
    let normal = xf.matrix3.inverse().transpose();
    for mut v in q.vertices {
        v.pos = xf.transform_point3(v.pos.into()).to_array();
        v.normal = (normal * Vec3::from(v.normal))
            .normalize_or_zero()
            .to_array();
        p.vertices.push(v);
    }
    p.indices.extend(q.indices.iter().map(|i| i + k));
}
fn scaled(p: &mut Primitive, q: Primitive, at: Vec3, size: Vec3) {
    append(
        p,
        q,
        Affine3A::from_scale_rotation_translation(size, Quat::IDENTITY, at),
    );
}
pub fn tube(p: &mut Primitive, points: &[Vec3], radius: impl Fn(f32) -> f32, sides: usize) {
    if points.len() < 2 {
        return;
    }
    let mut axes = Vec::new();
    for i in 0..points.len() {
        let f = (points[(i + 1).min(points.len() - 1)] - points[i.saturating_sub(1)])
            .normalize_or_zero();
        let x = f
            .cross(if f.y.abs() < 0.95 { Vec3::Y } else { Vec3::X })
            .normalize_or_zero();
        axes.push((x, f.cross(x)));
    }
    for i in 0..points.len() - 1 {
        for side in 0..sides {
            let t0 = i as f32 / (points.len() - 1) as f32;
            let t1 = (i + 1) as f32 / (points.len() - 1) as f32;
            let a = side as f32 * TAU / sides as f32;
            let b = (side + 1) as f32 * TAU / sides as f32;
            let n0 = axes[i].0 * a.cos() + axes[i].1 * a.sin();
            let n1 = axes[i].0 * b.cos() + axes[i].1 * b.sin();
            let n2 = axes[i + 1].0 * b.cos() + axes[i + 1].1 * b.sin();
            let n3 = axes[i + 1].0 * a.cos() + axes[i + 1].1 * a.sin();
            quad(
                p,
                [
                    points[i] + n0 * radius(t0),
                    points[i] + n1 * radius(t0),
                    points[i + 1] + n2 * radius(t1),
                    points[i + 1] + n3 * radius(t1),
                ],
                [n0, n1, n2, n3],
                [
                    [side as f32 / sides as f32, t0],
                    [(side + 1) as f32 / sides as f32, t0],
                    [(side + 1) as f32 / sides as f32, t1],
                    [side as f32 / sides as f32, t1],
                ],
            );
        }
    }
}
fn blade(p: &mut Primitive, root: Vec3, out: Vec3, length: f32, width: f32, lift: f32, droop: f32) {
    let across = out.cross(Vec3::Y).normalize();
    let point = |t: f32| root + out * length * t + Vec3::Y * length * (lift * t - droop * t * t);
    for i in 0..6 {
        let a = i as f32 / 6.;
        let b = (i + 1) as f32 / 6.;
        let w0 = width * (PI * (a * 0.92 + 0.08).min(1.)).sin();
        let w1 = width * (PI * (b * 0.92 + 0.08).min(1.)).sin();
        let q = [
            point(a) - across * w0,
            point(a) + across * w0,
            point(b) + across * w1,
            point(b) - across * w1,
        ];
        let n = (q[3] - q[0]).cross(q[1] - q[0]).normalize();
        quad(p, q, [n; 4], [[0., a], [1., a], [1., b], [0., b]]);
        quad(p, q, [-n; 4], [[0., a], [1., a], [1., b], [0., b]]);
    }
}
fn normalized_sphere(m: u32) -> Primitive {
    let mut p = primitives::sphere(m, 12, 24);
    for v in &mut p.vertices {
        v.pos = (Vec3::from(v.pos) * 0.5).to_array();
    }
    p
}
pub fn build(scene: &mut Scene, name: &str, m: u32) -> (usize, Vec3, Vec3) {
    let mut p = Primitive {
        material: m,
        ..Default::default()
    };
    let mut parts = Vec::new();
    match name {
        "box" => p = primitives::cube(m),
        "octagon" => p = primitives::cylinder(m, 0.5, 0.5, 8),
        "rod" => p = primitives::cylinder(m, 0.5, 0.5, 8),
        "sphere" | "mitochondrion" | "bubble" => p = normalized_sphere(m),
        "pore" => p = primitives::torus(m, 0.23, 24, 8),
        "dna" => {
            for strand in 0..2 {
                let c = if strand == 0 {
                    [0.15, 0.7, 0.57]
                } else {
                    [0.85, 0.38, 0.29]
                };
                let mat = material(scene, c, 0.45, 0.);
                let mut q = Primitive {
                    material: mat,
                    ..Default::default()
                };
                let path: Vec<_> = (0..=144)
                    .map(|i| {
                        let t = i as f32 / 144.;
                        let a = t * TAU * 3. + strand as f32 * PI;
                        Vec3::new(a.cos() * 0.115, t - 0.5, a.sin() * 0.115)
                    })
                    .collect();
                tube(&mut q, &path, |_| 0.016, 8);
                parts.push(q);
            }
            for i in 0..37 {
                let t = i as f32 / 36.;
                let a = t * TAU * 3.;
                let edge = Vec3::new(a.cos() * 0.115, 0., a.sin() * 0.115);
                let center = Vec3::new(0., t - 0.5, 0.);
                tube(&mut p, &[center - edge, center + edge], |_| 0.008, 6);
            }
        }
        "protein" => {
            let path: Vec<_> = (0..=200)
                .map(|i| {
                    let t = TAU * i as f32 / 200.;
                    let r = 0.29 + 0.105 * (3. * t).cos();
                    Vec3::new(
                        r * (2. * t).cos(),
                        (3. * t).sin() * 0.16,
                        r * (2. * t).sin(),
                    )
                })
                .collect();
            tube(&mut p, &path, |_| 0.04, 9);
            let path: Vec<_> = (0..=80)
                .map(|i| {
                    let t = i as f32 / 80.;
                    Vec3::new(
                        (t - 0.5) * 0.55,
                        (t * TAU * 6.).cos() * 0.075 + 0.16,
                        (t * TAU * 6.).sin() * 0.075,
                    )
                })
                .collect();
            tube(&mut p, &path, |_| 0.025, 7);
        }
        "cristae" => {
            for fold in 0..9 {
                let z = (fold as f32 - 4.) * 0.085;
                let w = (1. - (z / 0.53).powi(2)).sqrt();
                let path: Vec<_> = (0..25)
                    .map(|i| {
                        let a = PI * i as f32 / 24.;
                        let x = a.cos() * 0.43 * w;
                        let zz = z + (a * 3.).sin() * 0.025;
                        Vec3::new(x, (0.25 - x * x - zz * zz).max(0.001).sqrt() + 0.009, zz)
                    })
                    .collect();
                tube(&mut p, &path, |_| 0.024, 6);
            }
        }
        "membrane" => {
            p = primitives::sphere(m, 32, 64);
            for v in &mut p.vertices {
                v.pos = (Vec3::from(v.pos) * 0.5).to_array();
                v.normal = (-Vec3::from(v.normal)).to_array();
            }
            for t in p.indices.chunks_exact_mut(3) {
                t.swap(1, 2);
            }
        }
        "reticulum" => {
            scene.materials[m as usize].flags |= render::scene::TWO_SIDED;
            let point = |a: f32, v: f32, layer: f32| {
                let r = 0.12 + v * (0.28 + (a * 3. + layer).sin() * 0.025);
                Vec3::new(
                    a.cos() * r,
                    (a * 5. + v * 2. + layer * 0.7).sin() * 0.045 + (layer - 1.5) * 0.07,
                    a.sin() * r,
                )
            };
            for l in 0..4 {
                for i in 0..64 {
                    for band in 0..5 {
                        let a = i as f32 * TAU / 64.;
                        let b = (i + 1) as f32 * TAU / 64.;
                        let u = band as f32 / 5.;
                        let v = (band + 1) as f32 / 5.;
                        let coords = [(a, u), (b, u), (b, v), (a, v)];
                        let q = coords.map(|(a, v)| point(a, v, l as f32));
                        let normals = coords.map(|(a, v)| {
                            (point(a + 0.001, v, l as f32) - point(a - 0.001, v, l as f32))
                                .cross(
                                    point(a, v + 0.001, l as f32) - point(a, v - 0.001, l as f32),
                                )
                                .normalize()
                        });
                        quad(&mut p, q, normals, [[0.; 2]; 4]);
                    }
                }
            }
        }
        "fern" => {
            for frond in 0..9 {
                let a = frond as f32 * TAU / 9.;
                let along = Vec3::new(a.cos(), 0., a.sin());
                let across = Vec3::new(-a.sin(), 0., a.cos());
                for row in 1..13 {
                    let t = row as f32 / 13.;
                    let center = along * t + Vec3::Y * (0.13 + (t * PI * 0.9).sin() * 0.65);
                    for side in [-1., 1.] {
                        let tip = center + across * side * (t * PI).sin() * 0.3 + along * 0.13;
                        triangle(&mut p, center - along * 0.055, tip, center + along * 0.055);
                        triangle(&mut p, center + along * 0.055, tip, center - along * 0.055);
                    }
                }
            }
        }
        "cactus" => {
            for (path, r) in [
                (vec![Vec3::ZERO, Vec3::Y], 0.055),
                (
                    vec![
                        Vec3::new(0.04, 0.42, 0.),
                        Vec3::new(0.2, 0.44, 0.),
                        Vec3::new(0.23, 0.52, 0.),
                        Vec3::new(0.23, 0.78, 0.),
                    ],
                    0.04,
                ),
                (
                    vec![
                        Vec3::new(-0.04, 0.55, 0.),
                        Vec3::new(-0.17, 0.56, 0.),
                        Vec3::new(-0.19, 0.63, 0.),
                        Vec3::new(-0.19, 0.84, 0.),
                    ],
                    0.036,
                ),
            ] {
                tube(&mut p, &path, |_| r, 12);
                scaled(
                    &mut p,
                    primitives::sphere(m, 6, 12),
                    *path.last().unwrap(),
                    Vec3::splat(r),
                );
            }
        }
        "arch" => {
            let path: Vec<_> = (0..=32)
                .map(|i| {
                    let t = i as f32 * PI / 32.;
                    Vec3::new(t.cos() * 0.5, t.sin(), 0.)
                })
                .collect();
            tube(&mut p, &path, |t| 0.09 + 0.025 * (t * 19.).sin(), 10);
        }
        "kelp" => {
            let path: Vec<_> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.;
                    Vec3::new((t * 3.).sin() * 0.012, t, (t * 2.3).cos() * 0.008)
                })
                .collect();
            tube(&mut p, &path, |t| 0.007 - 0.004 * t, 5);
            for i in 0..23 {
                let h = 0.07 + i as f32 * 0.04;
                let a = i as f32 * 2.4;
                blade(
                    &mut p,
                    Vec3::new((h * 3.).sin() * 0.012, h, (h * 2.3).cos() * 0.008),
                    Vec3::new(a.cos(), 0., a.sin()),
                    (0.13 + (0.07 - 0.13) * h) * 1.3,
                    0.03,
                    0.9,
                    0.5,
                );
            }
        }
        "lantern" => {
            let mut path = vec![Vec3::ZERO];
            let mut at = Vec3::ZERO;
            for i in 1..25 {
                let t = i as f32 / 24.;
                let a = t * t * 2.5;
                at += Vec3::new(a.sin(), a.cos(), 0.) * (1.15 / 24.);
                path.push(at);
            }
            tube(&mut p, &path, |t| 0.024 - 0.015 * t, 7);
            let glow = material(scene, [0.2, 0.9, 0.65], 0.5, 90.);
            let mut bulb = Primitive {
                material: glow,
                ..Default::default()
            };
            let dir = (path[24] - path[23]).normalize();
            scaled(
                &mut bulb,
                primitives::sphere(glow, 8, 12),
                at + dir * 0.09,
                Vec3::new(0.1, 0.115, 0.1),
            );
            parts.push(bulb);
            for i in 0..6 {
                let a = i as f32 * TAU / 6. + 0.3;
                blade(
                    &mut p,
                    Vec3::new(0., 0.02, 0.),
                    Vec3::new(a.cos(), 0., a.sin()),
                    0.22,
                    0.05,
                    0.6,
                    0.7,
                );
            }
        }
        "fan" => {
            tube(&mut p, &[Vec3::ZERO, Vec3::Y * 0.8], |_| 0.012, 6);
            for branch in 0..18 {
                let a = -PI * 0.48 + branch as f32 * PI * 0.96 / 17.;
                let end = Vec3::new(a.sin() * 0.5, 0.18 + a.cos() * 0.8, -0.04);
                tube(
                    &mut p,
                    &[Vec3::Y * 0.05, Vec3::Y * 0.18, end],
                    |t| 0.011 - 0.007 * t,
                    5,
                );
                for k in 1..5 {
                    let at = Vec3::Y * 0.18 + (end - Vec3::Y * 0.18) * (k as f32 / 5.);
                    tube(&mut p, &[at, at + Vec3::new(0.075, 0.07, 0.)], |_| 0.004, 4);
                }
            }
        }
        "sponge" => {
            let mut rng = gdcompat::Pcg::new(9917);
            for k in 0..8 {
                let a = k as f32 * 2.4;
                let spot = Vec3::new(a.cos(), 0., a.sin()) * (0.08 + 0.32 * (k as f32 / 8.).sqrt());
                let h = rng.randf_range_f32(0.35, 1.) * (1. - k as f32 * 0.05);
                let r = rng.randf_range_f32(0.06, 0.12);
                let path: Vec<_> = (0..7)
                    .map(|i| spot + spot * 0.25 * h * i as f32 / 6. + Vec3::Y * h * i as f32 / 6.)
                    .collect();
                tube(&mut p, &path, |t| r * (1. + 0.35 * t * t), 9);
                tube(
                    &mut p,
                    &path[4..],
                    |t| r * (1. + 0.35 * ((t + 2.) / 3.).powi(2)) * 0.8,
                    9,
                );
            }
        }
        "fish" => {
            let mut path = Vec::new();
            for i in 0..9 {
                path.push(Vec3::new(0., 0., -0.5 + 0.82 * i as f32 / 8.));
            }
            tube(
                &mut p,
                &path,
                |t| (PI * (t * 0.95 + 0.05).min(1.)).sin() * (1. - t * 0.55) * 0.13,
                8,
            );
            for v in &mut p.vertices {
                v.pos[0] *= 0.6;
            }
            for q in [
                [
                    Vec3::new(0., 0., 0.3),
                    Vec3::new(0., 0.2, 0.56),
                    Vec3::new(0., 0.02, 0.46),
                ],
                [
                    Vec3::new(0., 0., 0.3),
                    Vec3::new(0., 0.02, 0.46),
                    Vec3::new(0., -0.18, 0.55),
                ],
                [
                    Vec3::new(0., 0.15, -0.18),
                    Vec3::new(0., 0.3, 0.),
                    Vec3::new(0., 0.12, 0.12),
                ],
            ] {
                triangle(&mut p, q[0], q[1], q[2]);
                triangle(&mut p, q[2], q[1], q[0]);
            }
        }
        "jelly" => {
            let mut q = normalized_sphere(m);
            for v in &mut q.vertices {
                v.pos[1] = v.pos[1].max(0.) * 0.55;
            }
            p = q;
            for i in 0..12 {
                let a = i as f32 * TAU / 12.;
                let path: Vec<_> = (0..20)
                    .map(|j| {
                        let t = j as f32 / 19.;
                        Vec3::new(
                            a.cos() * (0.3 - 0.1 * t) + (t * 8. + a).sin() * 0.04,
                            -t,
                            a.sin() * (0.3 - 0.1 * t),
                        )
                    })
                    .collect();
                tube(&mut p, &path, |t| 0.012 * (1. - 0.7 * t), 5);
            }
        }
        "manta" => {
            scaled(
                &mut p,
                normalized_sphere(m),
                Vec3::ZERO,
                Vec3::new(0.5, 0.16, 0.7),
            );
            for side in [-1., 1.] {
                let q = [
                    Vec3::new(0., 0., -0.3),
                    Vec3::new(side * 0.9, 0.04, 0.05),
                    Vec3::new(side * 0.25, 0., 0.3),
                ];
                triangle(&mut p, q[0], q[1], q[2]);
                triangle(&mut p, q[2], q[1], q[0]);
            }
            tube(
                &mut p,
                &[Vec3::new(0., 0., 0.25), Vec3::new(0., -0.02, 0.95)],
                |t| 0.018 * (1. - t * 0.8),
                5,
            );
        }
        _ if name.starts_with("rock") => {
            let kind = name[4..].parse::<usize>().unwrap_or(0);
            let mut rng = gdcompat::Pcg::new(7100 + kind as u64);
            let (rings, sides) = if kind < 2 { (14, 9) } else { (6, 8) };
            let profile = |t: f32| {
                if kind < 2 {
                    let smooth = |a: f32, b: f32| {
                        let x = ((t - a) / (b - a)).clamp(0., 1.);
                        x * x * (3. - 2. * x)
                    };
                    (
                        0.09 + 0.035 * (t * TAU * (2. + kind as f32 * 0.5) + kind as f32).sin()
                            + 0.17 * (1. - smooth(0., 0.25))
                            + 0.09 * smooth(0.8, 0.88) * (1. - smooth(0.95, 1.)),
                        t,
                    )
                } else {
                    ((PI * (0.08 + t * 0.84)).sin() * 0.5, t * 0.7 - 0.15)
                }
            };
            let mut rows = Vec::new();
            for ring in 0..=rings {
                let (r, y) = profile(ring as f32 / rings as f32);
                let twist = rng.randf() * TAU;
                let mut row = Vec::new();
                for side in 0..sides {
                    let a = side as f32 * TAU / sides as f32 + twist * 0.08;
                    let rr = r * rng.randf_range_f32(0.82, 1.12);
                    row.push(Vec3::new(
                        a.cos() * rr,
                        y + rng.randf_range_f32(-0.008, 0.008),
                        a.sin() * rr,
                    ));
                }
                rows.push(row);
            }
            for ring in 0..rings {
                for side in 0..sides {
                    let a = rows[ring][side];
                    let b = rows[ring][(side + 1) % sides];
                    let c = rows[ring + 1][side];
                    let d = rows[ring + 1][(side + 1) % sides];
                    triangle(&mut p, a, c, b);
                    triangle(&mut p, b, c, d);
                }
            }
            for side in 0..sides {
                triangle(
                    &mut p,
                    rows[rings][(side + 1) % sides],
                    rows[rings][side],
                    Vec3::new(0., profile(1.).1 + 0.01, 0.),
                );
            }
        }
        _ if name.starts_with("traffic") => {
            let variant = name[7..].parse::<usize>().unwrap_or(0);
            let size = [
                Vec3::new(3.7, 2.1, 7.),
                Vec3::new(4.7, 2.2, 10.),
                Vec3::new(4.4, 1.7, 9.),
                Vec3::new(5.2, 3.8, 14.),
            ][variant];
            let w = size.x * 0.5;
            let l = size.z * 0.5;
            let mut hull = |sections: &[[f32; 4]], mat: u32| {
                let mut q = Primitive {
                    material: mat,
                    ..Default::default()
                };
                let ring = [
                    [-0.72, -0.5],
                    [0.72, -0.5],
                    [1., -0.15],
                    [0.68, 0.5],
                    [-0.68, 0.5],
                    [-1., -0.15],
                ];
                let at = |j: usize, k: usize| {
                    let s = sections[j];
                    Vec3::new(ring[k][0] * s[0], ring[k][1] * s[1] + s[3], s[2])
                };
                for j in 0..sections.len() - 1 {
                    for k in 0..6 {
                        let v = [
                            at(j, k),
                            at(j, (k + 1) % 6),
                            at(j + 1, (k + 1) % 6),
                            at(j + 1, k),
                        ];
                        let n = (v[1] - v[0]).cross(v[2] - v[0]).normalize();
                        quad(&mut q, v, [n; 4], [[0.; 2]; 4]);
                    }
                }
                for j in [0, sections.len() - 1] {
                    for k in 0..6 {
                        let a = at(j, k);
                        let b = at(j, (k + 1) % 6);
                        let c = Vec3::new(0., sections[j][3], sections[j][2]);
                        if j == 0 {
                            triangle(&mut q, a, c, b)
                        } else {
                            triangle(&mut q, a, b, c)
                        }
                    }
                }
                parts.push(q);
            };
            hull(
                &[
                    [w * 0.85, 0.85, -l, 0.],
                    [w, 1.1, -l * 0.55, 0.],
                    [w, 1.1, l * 0.45, 0.],
                    [w * 0.7, 0.6, l, -0.1],
                ],
                m,
            );
            let glass = material(scene, [0.008, 0.018, 0.03], 0.25, 0.);
            let roof = size.y - 0.55;
            hull(
                &[
                    [w * 0.75, 0.28, -l * 0.6, 0.58],
                    [w * 0.65, roof, -l * 0.22, 0.65],
                    [w * 0.61, roof, l * 0.2, 0.65],
                    [w * 0.63, 0.18, l * 0.63, 0.55],
                ],
                glass,
            );
            for side in [-1., 1.] {
                for end in [-1., 1.] {
                    let light = material(
                        scene,
                        if end > 0. {
                            [0.65, 0.82, 1.]
                        } else {
                            [1., 0.025, 0.008]
                        },
                        0.5,
                        90.,
                    );
                    let mut q = Primitive {
                        material: light,
                        ..Default::default()
                    };
                    let at = Vec3::new(side * w * 0.5, 0.04, end * (l + 0.04));
                    let v = [
                        at + Vec3::new(-w * 0.2, -0.13, 0.),
                        at + Vec3::new(w * 0.2, -0.13, 0.),
                        at + Vec3::new(w * 0.2, 0.13, 0.),
                        at + Vec3::new(-w * 0.2, 0.13, 0.),
                    ];
                    quad(&mut q, v, [Vec3::Z * end; 4], [[0.; 2]; 4]);
                    parts.push(q);
                }
            }
        }
        _ => p = primitives::cube(m),
    }
    if !p.indices.is_empty() {
        parts.push(p);
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for p in &parts {
        for v in &p.vertices {
            lo = lo.min(v.pos.into());
            hi = hi.max(v.pos.into());
        }
    }
    let mesh = scene.add_mesh(&ClusterMesh::build(
        &parts.iter().map(Primitive::part).collect::<Vec<_>>(),
    ));
    (mesh, lo, hi)
}
