//! Landscape equations from forest_terrain.gd and desert_terrain.gd.
//! Mesh, planting and collision sample the same triangle-interpolated field.
use fastnoise_lite::{FastNoiseLite, FractalType, NoiseType};
use glam::Vec2;
use render::{Scene, cluster::Vertex, primitives::Primitive};
use spaceracer::{environment::Landscape, race::Race};
use std::sync::Arc;
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn noise(seed: i32, f: f32, oct: i32, gain: f32) -> FastNoiseLite {
    let mut n = FastNoiseLite::with_seed(seed);
    n.set_noise_type(Some(NoiseType::OpenSimplex2S));
    n.set_frequency(Some(f));
    n.set_fractal_type(Some(FractalType::FBm));
    n.set_fractal_octaves(Some(oct));
    n.set_fractal_gain(Some(gain));
    n
}
pub fn generate(race: &Race) -> Arc<Landscape> {
    let nodes = &race.world.track.nodes;
    let lowest = nodes
        .iter()
        .map(|n| n.geometry.p.y)
        .fold(f32::INFINITY, f32::min);
    let forest = race.biome == "forest";
    let desert = race.biome == "desert";
    let ocean = race.biome == "ocean";
    let water = lowest - 24.;
    let floor = if forest { water } else { lowest - 48. };
    let seed = race.world.track.seed as i32;
    let hills = noise(seed + 49187, 0.00135, 3, 0.38);
    let dunes = noise(seed + 30211, 0.0042, 2, 0.5);
    let swell = noise(seed + 30212, 0.0011, 3, 0.5);
    let ranges = noise(seed + 30213, 0.0012, 3, 0.42);
    let skyline = noise(seed + 30215, 0.0016, 2, 0.5);
    let mask = noise(seed + 30214, 0.00055, 5, 0.5);
    let cells = 320;
    let step = 24.;
    let half = 3840.;
    let count = (cells + 1) * (cells + 1);
    let mut heights = vec![0.; count];
    for z in 0..=cells {
        for x in 0..=cells {
            let p = Vec2::new(x as f32 * step - half, z as f32 * step - half);
            let h = if forest {
                let h = water + 18. + hills.get_noise_2d(p.x, p.y) * 145.;
                let e = smooth(half - 650., half, p.abs().max_element());
                h + (water - 55. - h) * e
            } else if desert || ocean {
                let wx = p.x + dunes.get_noise_2d(p.y * 0.4, p.x * 0.4) * 180.;
                let crest = 1. - dunes.get_noise_2d(wx * 0.55, p.y * 1.6).abs();
                let mut h = floor + 6. + swell.get_noise_2d(p.x, p.y) * 38. + crest.powi(3) * 34.;
                let m =
                    ranges.get_noise_2d(p.x, p.y) * smooth(-0.15, 0.3, mask.get_noise_2d(p.x, p.y));
                h += if desert {
                    smooth(0.10, 0.16, m) * 110.
                        + smooth(0.27, 0.32, m) * 95.
                        + smooth(0.42, 0.46, m) * 80.
                } else {
                    smooth(0.04, 0.3, m) * 150. + smooth(0.28, 0.5, m) * 110.
                };
                let far = skyline.get_noise_2d(p.x, p.y);
                h += smooth(2200., 3300., p.length())
                    * (70.
                        + smooth(-0.25, -0.17, far) * 190.
                        + smooth(0.12, 0.19, far) * 170.
                        + smooth(0.4, 0.45, far) * 120.);
                h + (floor + 220. - h) * smooth(half - 500., half, p.abs().max_element())
            } else if race.biome == "city" {
                -180.
            } else {
                -1800.
            };
            heights[z * (cells + 1) + x] = h;
        }
    }
    if forest || desert || ocean {
        let mut near = vec![f32::INFINITY; count];
        let mut caps = near.clone();
        let mut wall = vec![0.; count];
        let mut anchor = vec![0.; count];
        let mut wall_distance = vec![f32::INFINITY; count];
        let mut low = vec![f32::INFINITY; count];
        for i in (0..nodes.len()).step_by(2) {
            let a = &nodes[i];
            let b = &nodes[(i + 2) % nodes.len()];
            let start = a.geometry.p.xz();
            let end = b.geometry.p.xz();
            let delta = end - start;
            let width = a.geometry.width.max(b.geometry.width) as f32;
            let inner = width
                + 36.
                + if a.air_gap || a.feature == "flight" || a.feature == "jump" {
                    45.
                } else {
                    0.
                };
            let reach = if forest { inner + 120. } else { 430. };
            let ceiling = a.geometry.p.y.min(b.geometry.p.y) - width - 16.;
            let lo = ((start.min(end) - Vec2::splat(reach + 0.1) + Vec2::splat(half)) / step)
                .floor()
                .max(Vec2::ZERO)
                .as_uvec2();
            let hi = ((start.max(end) + Vec2::splat(reach + 0.1) + Vec2::splat(half)) / step)
                .ceil()
                .min(Vec2::splat(cells as f32))
                .as_uvec2();
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let p = Vec2::new(x as f32 * step - half, z as f32 * step - half);
                    let t =
                        ((p - start).dot(delta) / delta.length_squared().max(0.001)).clamp(0., 1.);
                    let d = p.distance(start + delta * t);
                    if d >= reach {
                        continue;
                    }
                    let id = z as usize * (cells + 1) + x as usize;
                    let cap = ceiling
                        + smooth(inner, inner + if forest { 120. } else { 65. }, d)
                            * if forest { 180. } else { 900. };
                    caps[id] = caps[id].min(cap);
                    near[id] = near[id].min(d);
                    low[id] = low[id].min(ceiling);
                    if !a.is_loop
                        && !b.is_loop
                        && !a.air_gap
                        && !b.air_gap
                        && !matches!(a.feature.as_str(), "jump" | "flight")
                        && !matches!(b.feature.as_str(), "jump" | "flight")
                        && d < wall_distance[id]
                    {
                        wall_distance[id] = d;
                        anchor[id] = ceiling;
                        wall[id] = smooth(
                            0.08,
                            0.42,
                            (a.u as f32 * std::f32::consts::TAU * 3.).sin() * 0.5
                                + 0.5
                                + (a.u as f32 * std::f32::consts::TAU * 7. + 1.3).sin() * 0.18,
                        );
                    }
                }
            }
        }
        for i in 0..count {
            if !forest && near[i] < 430. {
                let mut top = if wall_distance[i] < 430. {
                    anchor[i]
                } else {
                    low[i]
                };
                // Reef shelves leave sightlines to schools and giant animals.
                // Desert keeps its tall enclosing canyon walls.
                top += if ocean {
                    10. + 65. * wall[i]
                } else {
                    40. + 175. * wall[i]
                };
                if desert {
                    let t = (top - floor) / 46.;
                    top = floor + (t.floor() + smooth(0.62, 1., t.fract())) * 46.;
                }
                let rise = smooth(60., 170., near[i]) * (1. - smooth(260., 430., near[i]));
                heights[i] = heights[i].max(heights[i] + (top - heights[i]) * rise);
            }
            heights[i] = heights[i].min(caps[i]);
        }
    }
    Arc::new(Landscape {
        heights,
        cells,
        step,
        half,
        outside: if forest { water - 55. } else { floor },
    })
}
use glam::Vec3Swizzles;
pub fn mesh(scene: &mut Scene, t: &Landscape, material: u32) {
    for cz in (0..t.cells).step_by(16) {
        for cx in (0..t.cells).step_by(16) {
            let mut p = Primitive {
                material,
                ..Default::default()
            };
            for z in 0..=16 {
                for x in 0..=16 {
                    let wx = (cx + x) as f32 * t.step - t.half;
                    let wz = (cz + z) as f32 * t.step - t.half;
                    p.vertices.push(Vertex {
                        pos: [wx, t.heights[(cz + z) * (t.cells + 1) + cx + x], wz],
                        normal: glam::Vec3::new(
                            t.height(wx - t.step, wz) - t.height(wx + t.step, wz),
                            2. * t.step,
                            t.height(wx, wz - t.step) - t.height(wx, wz + t.step),
                        )
                        .normalize()
                        .to_array(),
                        uv: [wx, wz],
                    });
                }
            }
            for z in 0..16 {
                for x in 0..16 {
                    let a = (z * 17 + x) as u32;
                    let b = a + 1;
                    let c = a + 17;
                    let d = c + 1;
                    p.indices.extend([a, c, b, b, c, d]);
                }
            }
            let m = scene.add_mesh(&p.mesh());
            scene.add_instance(m, glam::Affine3A::IDENTITY);
        }
    }
}
