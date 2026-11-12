use super::Rng;
use glam::Vec3;
use render::{
    Scene,
    cluster::{ClusterMesh, Part},
    crowd::{Crowd, CrowdHits},
    debris::Debris,
    primitives::{Primitive, hewn_block},
    scene::Material,
};

const PIECES: usize = 12;
const MAX_BURST: usize = 192;

// Stored in the same colour space as Warp Humans' generated skin.
const SKIN: [[f32; 3]; 8] = [
    [0.33, 0.18, 0.12],
    [0.44, 0.25, 0.17],
    [0.54, 0.34, 0.23],
    [0.61, 0.40, 0.30],
    [0.69, 0.50, 0.38],
    [0.76, 0.55, 0.43],
    [0.82, 0.61, 0.48],
    [0.87, 0.66, 0.53],
];

#[derive(Default)]
pub(super) struct GibMeshes {
    pub(super) cut: [usize; 3],
    skin: [[usize; 3]; SKIN.len()],
}

impl GibMeshes {
    pub(super) fn new(scene: &mut Scene) -> Self {
        // Bake both finishes into mesh UVs once so the markings turn with the piece.
        let mut noise = fastnoise_lite::FastNoiseLite::with_seed(9041);
        noise.set_frequency(Some(0.035));
        let sample = |x: u32, y: u32| noise.get_noise_2d(x as f32, y as f32) * 0.5 + 0.5;
        let tile = 128;
        let picture = image::RgbaImage::from_fn(tile * 2, tile, |x, y| {
            let skin = x >= tile;
            let x = x % tile;
            let n = sample(2 * x, 2 * y);
            let fine = sample(4 * x + 37, 10 * y + 11);
            let ridge = (1. - ((fine + 0.35 * n - 0.69) * 9.).abs()).max(0.);
            let color = if skin {
                let edge = x.min(tile - 1 - x).min(y.min(tile - 1 - y)) as f32;
                let border = (1. - edge / (3. + 10. * n)).clamp(0., 1.);
                Vec3::splat(0.91 + 0.09 * fine).lerp(Vec3::new(0.64, 0.30, 0.30), border)
            } else {
                Vec3::new(0.16, 0.016, 0.028).lerp(
                    Vec3::new(0.49, 0.085, 0.094),
                    ((n - 0.18) * 1.55 + 0.18 * (fine - 0.5)).clamp(0., 1.),
                ) * (1. - 0.54 * ridge)
            };
            let [r, g, b] = (color * 255.).to_array().map(|v| v as u8);
            image::Rgba([r, g, b, 255])
        });
        let layer = scene.add_image("stadium gib cut and skin", picture);
        let mut material = |tint: [f32; 3]| {
            scene.add_material(Material {
                tint: super::lin(tint),
                layer,
                roughness: 0.90,
                flags: 0,
                scale: 1.,
                metal: 0.,
            })
        };
        let cuts = [[0.96, 0.88, 0.90], [0.78, 0.70, 0.80], [1.0, 0.93, 0.86]].map(&mut material);
        let skins = SKIN.map(material);
        let mut meshes = Self::default();
        for (i, cut) in cuts.into_iter().enumerate() {
            let mut shape = unit_shard(cut, i);
            for v in &mut shape.vertices {
                v.uv[0] = 0.01 + 0.48 * v.uv[0];
            }
            meshes.cut[i] = scene.add_mesh(&shape.mesh());
            let direction = [Vec3::Z, Vec3::NEG_X, Vec3::NEG_Z][i];
            let mut inner = Vec::new();
            let mut outer = Vec::new();
            for tri in shape.indices.chunks_exact(3) {
                let v = shape.vertices[tri[0] as usize];
                if Vec3::from(v.normal).dot(direction) > 0.7 {
                    outer.extend_from_slice(tri);
                } else {
                    inner.extend_from_slice(tri);
                }
            }
            let mut exterior = shape.vertices.clone();
            for v in &mut exterior {
                v.uv[0] += 0.5;
            }
            for (tone, &skin) in skins.iter().enumerate() {
                meshes.skin[tone][i] = scene.add_mesh(&ClusterMesh::build(&[
                    Part {
                        vertices: &shape.vertices,
                        indices: &inner,
                        material: cut,
                    },
                    Part {
                        vertices: &exterior,
                        indices: &outer,
                        material: skin,
                    },
                ]));
            }
        }
        meshes
    }
}

fn volume(mesh: &Primitive) -> f32 {
    mesh.indices
        .chunks_exact(3)
        .map(|tri| {
            let [a, b, c] = [0, 1, 2].map(|i| Vec3::from(mesh.vertices[tri[i] as usize].pos));
            a.dot(b.cross(c)) / 6.
        })
        .sum::<f32>()
        .abs()
}

pub(crate) fn unit_shard(material: u32, variant: usize) -> Primitive {
    let mut mesh = hewn_block(material, 2, 0.30, 0.17, 9 + variant as u32 * 37);
    let shear = [-0.24, 0.16, 0.36][variant];
    for v in &mut mesh.vertices {
        let p = Vec3::from(v.pos);
        let taper = 0.80 - 0.52 * p.y;
        v.pos = [p.x * taper + shear * p.y, p.y, p.z * taper];
        let n = Vec3::from(v.normal);
        let normal = Vec3::new(
            n.x / taper,
            n.y - ((shear - 0.52 * p.x) * n.x - 0.52 * p.z * n.z) / taper,
            n.z / taper,
        );
        v.normal = normal.normalize().into();
    }
    let side = volume(&mesh).cbrt();
    for v in &mut mesh.vertices {
        v.pos = (Vec3::from(v.pos) / side).into();
    }
    mesh
}

fn sizes(rng: &mut Rng, body_scale: Vec3, count: usize) -> [Vec3; PIECES] {
    // About 75 litres for a 1.80 m reference body. Two larger pieces carry
    // much of its silhouette; the smaller elongated ones break it up.
    let body_volume = 0.075 * body_scale.element_product() / 1.8_f32.powi(3);
    let mut sizes = [Vec3::ZERO; PIECES];
    for (i, size) in sizes[..count].iter_mut().enumerate() {
        let major = if i < 2 { 1.55 } else { 1.0 };
        *size = Vec3::new(
            rng.range(0.62, 1.0),
            rng.range(0.9, 1.6),
            rng.range(0.55, 0.95),
        ) * major;
    }
    let total: f32 = sizes[..count].iter().map(|s| s.element_product()).sum();
    let scale = (body_volume / total).cbrt();
    for size in &mut sizes[..count] {
        *size *= scale;
    }
    sizes
}

pub(super) fn emit(
    scene: &Scene,
    debris: &mut Debris,
    meshes: &GibMeshes,
    crowd: &Crowd,
    hit: &CrowdHits,
) {
    let victims = hit.seats.len();
    let groups = victims.min(MAX_BURST);
    for group in 0..groups {
        let i = group * victims / groups;
        let seat = hit.seats[i] as usize;
        let scale = crowd.body_scale(seat);
        let tone = crowd.human(seat).map_or(4, |person| {
            let color = person.skin_color();
            SKIN.iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    color
                        .distance_squared(Vec3::from(**a))
                        .total_cmp(&color.distance_squared(Vec3::from(**b)))
                })
                .unwrap()
                .0
        });
        let center = hit.positions[i].with_y(crowd.position(seat).y + scale.y * 0.52);
        let push = if hit.radius > 0. {
            (center - hit.origin).normalize_or(Vec3::Y)
        } else {
            hit.direction
        };
        let count = (MAX_BURST / groups).min(PIECES);
        let mut rng = Rng(((hit.time.to_bits() as u64) << 32 | seat as u64) | (1u64 << 63));
        for (piece, size) in sizes(&mut rng, scale, count)[..count].iter().enumerate() {
            let offset = Vec3::new(
                rng.range(-0.10, 0.10),
                rng.range(-0.28, 0.28),
                rng.range(-0.09, 0.09),
            );
            let dir = offset.normalize_or(Vec3::Y);
            let pos = center + offset * scale;
            let vel = dir * rng.range(1.3, 4.1)
                + push * rng.range(1.0, 3.3)
                + Vec3::Y * rng.range(2.2, 4.5);
            let variant = rng.below(3);
            let mesh = if (piece + seat) % 2 == 0 {
                meshes.skin[tone][variant]
            } else {
                meshes.cut[variant]
            };
            debris.spawn(
                scene,
                mesh,
                pos,
                vel,
                *size,
                rng.range(-11., 11.),
                rng.range(2.1, 2.6),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_volume_tracks_the_body_at_each_burst_budget() {
        for variant in 0..3 {
            assert!((volume(&unit_shard(0, variant)) - 1.).abs() < 1e-5);
        }
        let unit = volume(&unit_shard(0, 0));
        for count in 1..=PIECES {
            let scale = Vec3::new(1.56, 1.83, 1.56);
            let parts = sizes(&mut Rng(48371), scale, count);
            let total = parts
                .iter()
                .map(|p| p.element_product() * unit)
                .sum::<f32>();
            assert!((total - 0.075 * scale.element_product() / 1.8_f32.powi(3)).abs() < 1e-5);
        }
    }
}
