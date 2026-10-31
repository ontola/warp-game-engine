//! Native ribbon construction from validated Luau recipes and seeded samples.
use gdcompat::Pcg;
use glam::{Vec2, Vec3};
use std::f64::consts::{PI, TAU};
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Dimensions {
    pub radius: [f32; 2],
    pub lobes: [i32; 2],
    pub amplitude: [f32; 2],
    pub stretch: [f32; 2],
    pub hills: [i32; 2],
    pub climb: [f32; 2],
    pub corner_a: [f32; 2],
    pub corner_b: [f32; 2],
    pub corner_shift: [f32; 2],
    pub loop_radius: [f32; 2],
    pub road_width: [f64; 3],
}
#[derive(Clone, Debug)]
pub struct Recipe {
    pub dimensions: Dimensions,
    pub name: &'static str,
    pub stadium: bool,
    pub corner_scale: f64,
    pub width_scale: f64,
    pub boosts: Vec<[f64; 2]>,
    pub loops: Vec<f64>,
    pub tunnels: Vec<[f64; 2]>,
    pub corners: Vec<[f64; 3]>,
    pub sections: Vec<(&'static str, f64, f64)>,
    pub jumps: Vec<[f64; 2]>,
    pub hard_flight: bool,
    pub exposed: Vec<[f64; 2]>,
}
impl Recipe {
    pub fn index(seed: i64) -> usize {
        (seed - 31).rem_euclid(6) as usize
    }
    pub fn for_seed(seed: i64) -> Self {
        crate::rules::Rules::new(None)
            .expect("bundled Luau rules")
            .track(seed, "normal")
            .expect("bundled track recipe")
    }
}
#[derive(Clone, Debug)]
pub struct Centreline {
    pub recipe: Recipe,
    pub difficulty: String,
    pub biome: String,
    pub theme: usize,
    pub radius: f64,
    pub lobes: i32,
    pub amplitude: f64,
    pub phase: f64,
    pub stretch: f64,
    pub hills: i32,
    pub climb: f64,
    pub corner_a_width: f64,
    pub corner_b_width: f64,
    pub corner_shift: f64,
    pub loops: Vec<Loop>,
    pub jumps: Vec<Jump>,
    pub components: Vec<Component>,
}
#[derive(Clone, Debug)]
pub struct Loop {
    pub start: f64,
    pub end: f64,
    pub radius: f64,
    pub forward: Vec3,
    pub right: Vec3,
}
fn range(rng: &mut Pcg, a: f32, b: f32) -> f64 {
    rng.randf_range_f32(a, b) as f64
}
pub fn smooth_phase(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}
fn angle_difference(a: f64, b: f64) -> f64 {
    (b - a + PI).rem_euclid(TAU) - PI
}
impl Centreline {
    pub fn new(seed: i64, difficulty: &str, biome: &str) -> Self {
        Self::with_recipe(seed, difficulty, biome, Recipe::for_seed(seed))
    }
    pub fn with_recipe(seed: i64, difficulty: &str, biome: &str, recipe: Recipe) -> Self {
        let difficulty = if ["easy", "normal", "hard"].contains(&difficulty) {
            difficulty
        } else {
            "normal"
        };
        let biome = if ["city", "forest", "cell", "desert", "ocean"].contains(&biome) {
            biome
        } else {
            "city"
        };
        let mut rng = Pcg::new(seed as u64);
        let d = recipe.dimensions.clone();
        let mut c = Self {
            recipe,
            difficulty: difficulty.into(),
            biome: biome.into(),
            theme: rng.randi_range(0, 3) as usize,
            radius: range(&mut rng, d.radius[0], d.radius[1]),
            lobes: rng.randi_range(d.lobes[0], d.lobes[1]),
            amplitude: range(&mut rng, d.amplitude[0], d.amplitude[1]),
            phase: rng.randf() as f64 * TAU,
            stretch: range(&mut rng, d.stretch[0], d.stretch[1]),
            hills: rng.randi_range(d.hills[0], d.hills[1]),
            climb: range(&mut rng, d.climb[0], d.climb[1]),
            corner_a_width: range(&mut rng, d.corner_a[0], d.corner_a[1]),
            corner_b_width: range(&mut rng, d.corner_b[0], d.corner_b[1]),
            corner_shift: range(&mut rng, d.corner_shift[0], d.corner_shift[1]),
            loops: vec![],
            jumps: vec![],
            components: vec![],
        };
        if difficulty == "easy" {
            c.corner_a_width *= 1.35;
            c.corner_b_width *= 1.35;
        } else if difficulty == "hard" {
            c.corner_a_width *= 0.62;
            c.corner_b_width *= 0.70;
        }
        for &start in &c.recipe.loops {
            let center = start + 0.052;
            let mut f = c.base_position(center + 0.001) - c.base_position(center - 0.001);
            f.y = 0.0;
            f = f.normalize();
            c.loops.push(Loop {
                start,
                end: start + 0.105,
                radius: range(&mut rng, d.loop_radius[0], d.loop_radius[1]),
                forward: f,
                right: f.cross(Vec3::Y),
            });
        }
        if difficulty != "easy" {
            let mut jump_rng = Pcg::new((seed + 62041) as u64);
            let shift = range(&mut jump_rng, -0.002, 0.002);
            for span in &c.recipe.jumps {
                c.jumps.push(Jump {
                    kind: "jump",
                    start: span[0] + shift,
                    end: span[1] + shift,
                    lip: 0.38,
                    land: if difficulty == "normal" { 0.56 } else { 0.61 },
                    rise: if difficulty == "normal" { 12.0 } else { 25.0 },
                    drop: 0.0,
                    offset: 34.0 * if seed % 2 == 0 { 1.0 } else { -1.0 },
                    turbo: false,
                });
            }
            if difficulty == "hard" && c.recipe.hard_flight {
                c.jumps.push(Jump {
                    kind: "flight",
                    start: 0.927,
                    end: 0.994,
                    lip: 0.27,
                    land: 0.58,
                    rise: 28.0,
                    drop: 0.0,
                    offset: 46.0 * if seed % 2 == 0 { -1.0 } else { 1.0 },
                    turbo: false,
                });
            }
            let family = Recipe::index(seed);
            if family == 0 {
                c.components.push(c.component("spiral", 0.33, 0.405));
            }
            if family == 2 && difficulty == "hard" {
                c.components.push(c.component("hairpin", 0.19, 0.29));
            }
            if difficulty == "hard" && [0, 3, 4].contains(&family) && !c.jumps.is_empty() {
                let jump = &mut c.jumps[0];
                jump.turbo = true;
                jump.rise = 38.0;
                jump.drop = -38.0;
                jump.land = 0.68;
                jump.offset *= 1.4;
            }
        }
        c
    }
    pub fn base_position(&self, u: f64) -> Vec3 {
        let a = u * TAU;
        let c = self;
        let corner_a =
            (-(angle_difference(a, 2.7 + c.corner_shift) / c.corner_a_width).powi(2)).exp();
        let corner_b =
            (-(angle_difference(a, 5.45 - c.corner_shift) / c.corner_b_width).powi(2)).exp();
        let mut r = c.radius
            + c.amplitude * (c.lobes as f64 * a + c.phase).sin()
            + (260.0 * corner_a - 175.0 * corner_b) * c.recipe.corner_scale;
        for corner in &c.recipe.corners {
            let width = corner[2]
                * if c.difficulty == "easy" {
                    1.4
                } else if c.difficulty == "hard" {
                    0.72
                } else {
                    1.0
                };
            let center = corner[0] * TAU + c.corner_shift;
            r += corner[1]
                * ((-(angle_difference(a, center - 0.11) / width).powi(2)).exp()
                    - (-(angle_difference(a, center + 0.11) / width).powi(2)).exp());
        }
        let ridge = 170.0 * (-((u - 0.55) / 0.075).powi(2)).exp();
        let wave = (c.hills as f64 * a + c.phase).sin();
        let small = (3.0 * a + c.phase * 0.4).sin();
        let altitude = match c.biome.as_str() {
            "forest" => 80.0 + c.climb * 0.42 * wave + 6.0 * small + ridge * 0.3,
            "desert" | "ocean" => 90.0 + c.climb * 0.5 * wave + 8.0 * small + ridge * 0.35,
            _ => 200.0 + c.climb * wave + 12.0 * small + ridge,
        };
        if c.recipe.stadium {
            let h = stadium(u, c.radius, c.stretch);
            Vec3::new(h.x, altitude as f32, h.y)
        } else {
            Vec3::new(
                (a.sin() * r * c.stretch) as f32,
                altitude as f32,
                (a.cos() * r) as f32,
            )
        }
    }
}
pub fn stadium(u: f64, radius: f64, stretch: f64) -> Vec2 {
    let r = radius * 0.65;
    let straight = radius * 1.4;
    let mut d = u.rem_euclid(1.0) * (2.0 * straight + TAU * r);
    if d < straight * 0.5 {
        return Vec2::new((-r * stretch) as f32, d as f32);
    }
    d -= straight * 0.5;
    if d < PI * r {
        let a = PI - d / r;
        return Vec2::new(
            (a.cos() * r * stretch) as f32,
            (straight * 0.5 + a.sin() * r) as f32,
        );
    }
    d -= PI * r;
    if d < straight {
        return Vec2::new((r * stretch) as f32, (straight * 0.5 - d) as f32);
    }
    d -= straight;
    if d < PI * r {
        let a = -d / r;
        return Vec2::new(
            (a.cos() * r * stretch) as f32,
            (-straight * 0.5 + a.sin() * r) as f32,
        );
    }
    Vec2::new((-r * stretch) as f32, (-straight * 0.5 + d - PI * r) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_base_centrelines_all_layouts_biomes_difficulties() {
        let cases: serde_json::Value = serde_json::from_str(&crate::oracle("generation")).unwrap();
        for c in cases.as_array().unwrap() {
            let seed = c["seed"].as_i64().unwrap();
            let difficulty = c["difficulty"].as_str().unwrap();
            let biome = c["biome"].as_str().unwrap();
            let line = Centreline::new(seed, difficulty, biome);
            assert!(
                (line.radius - c["radius"].as_f64().unwrap()).abs() < 0.0001,
                "seed {seed} radius {} expected {}",
                line.radius,
                c["radius"]
            );
            for (i, point) in c["points"].as_array().unwrap().iter().enumerate() {
                let expected: Vec3 = serde_json::from_value(point.clone()).unwrap();
                let actual = line.base_position(i as f64 / 128.0);
                let raw: Vec3 = serde_json::from_value(c["raw"][i].clone()).unwrap();
                let actual_raw = line.raw_position(i as f64 / 128.0);
                assert!(
                    raw.distance(actual_raw) < 0.02,
                    "raw {seed} {difficulty} {biome} point {i}: {actual_raw:?} != {raw:?}"
                );
                assert!(
                    actual.distance(expected) < 0.002,
                    "{seed} {difficulty} {biome} point {i}: {actual:?} != {expected:?}"
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Jump {
    pub kind: &'static str,
    pub start: f64,
    pub end: f64,
    pub lip: f64,
    pub land: f64,
    pub rise: f64,
    pub drop: f64,
    pub offset: f64,
    pub turbo: bool,
}
#[derive(Clone, Debug)]
pub struct Component {
    pub kind: &'static str,
    pub start: f64,
    pub end: f64,
    pub curve: crate::curve::Curve,
}
impl Centreline {
    fn component(&self, kind: &'static str, start: f64, end: f64) -> Component {
        let origin = self.base_position(start);
        let finish = self.base_position(end);
        let mut forward = finish - origin;
        forward.y = 0.0;
        forward = forward.normalize();
        let right = forward.cross(Vec3::Y);
        let span = Vec2::new(finish.x - origin.x, finish.z - origin.z).length();
        let mut points = vec![origin];
        if kind == "spiral" {
            let radius = if self.difficulty == "hard" {
                95.0
            } else {
                120.0
            };
            let entry = origin + forward * span * 0.30;
            for i in 0..25 {
                let theta = TAU * i as f64 / 24.0;
                points.push(
                    entry
                        + right * radius * (1.0 - theta.cos()) as f32
                        + forward * radius * theta.sin() as f32
                        + Vec3::Y * 170.0 * i as f32 / 24.0,
                );
            }
            points.push(origin + forward * span * 0.76 + Vec3::Y * 125.0);
        } else {
            let radius = 72.0;
            for i in 0..13 {
                let theta = PI * i as f64 / 12.0;
                points.push(
                    origin
                        + forward * (180.0 + radius * theta.sin()) as f32
                        + right * (radius * (1.0 - theta.cos())) as f32,
                );
            }
            for i in 0..13 {
                let theta = PI * i as f64 / 12.0;
                points.push(
                    origin
                        + forward * (60.0 - radius * theta.sin()) as f32
                        + right * (radius * 3.0 - radius * theta.cos()) as f32
                        + Vec3::Y * 12.0,
                );
            }
            points.push(
                origin + forward * span * 0.65 + right * radius as f32 * 4.0 + Vec3::Y * 12.0,
            );
        }
        points.push(finish);
        let mut tangents = Vec::new();
        for i in 0..points.len() {
            let tangent = if i == 0 {
                (self.base_position(start + 0.0001) - origin).normalize()
                    * origin.distance(points[1])
                    * 0.33
            } else if i == points.len() - 1 {
                (self.base_position(end + 0.0001) - finish).normalize()
                    * finish.distance(points[i - 1])
                    * 0.33
            } else {
                (points[i + 1] - points[i - 1]).normalize()
                    * points[i]
                        .distance(points[i - 1])
                        .min(points[i].distance(points[i + 1]))
                    * 0.34
            };
            tangents.push(tangent);
        }
        Component {
            kind,
            start,
            end,
            curve: crate::curve::Curve::bake(&points, &tangents),
        }
    }
    pub fn raw_position(&self, u: f64) -> Vec3 {
        for c in &self.components {
            if u >= c.start && u < c.end {
                return c
                    .curve
                    .sample(((u - c.start) / (c.end - c.start) * c.curve.length as f64) as f32);
            }
        }
        let mut p = self.base_position(u);
        for jump in &self.jumps {
            if u <= jump.start || u >= jump.end {
                continue;
            }
            let t = (u - jump.start) / (jump.end - jump.start);
            let line = self
                .base_position(jump.start)
                .lerp(self.base_position(jump.end), t as f32);
            let blend = smooth_phase((t.min(1.0 - t) / 0.22).clamp(0.0, 1.0));
            let altitude = p.y;
            p = p.lerp(line, blend as f32);
            p.y = altitude;
            let height = if t < jump.lip {
                jump.rise
                    * if jump.turbo {
                        (t / jump.lip).powi(2)
                    } else {
                        smooth_phase(t / jump.lip)
                    }
            } else if t < jump.land {
                jump.rise
                    + (jump.drop - jump.rise)
                        * smooth_phase((t - jump.lip) / (jump.land - jump.lip))
            } else {
                jump.drop * (1.0 - smooth_phase((t - jump.land) / (1.0 - jump.land)))
            };
            p.y = (p.y as f64 + height) as f32;
            if self.difficulty == "hard" && t > jump.lip {
                let side = (self.base_position(jump.start + 0.001)
                    - self.base_position(jump.start))
                .normalize()
                .cross(Vec3::Y)
                .normalize();
                let mut across =
                    smooth_phase(((t - jump.lip) / (jump.land - jump.lip)).clamp(0.0, 1.0));
                if t > jump.land + 0.20 {
                    across = 1.0
                        - smooth_phase(
                            ((t - jump.land - 0.20) / (1.0 - jump.land - 0.20)).clamp(0.0, 1.0),
                        );
                }
                p += side * jump.offset as f32 * across as f32;
            }
        }
        for l in &self.loops {
            if u <= l.start || u >= l.end {
                continue;
            }
            let q = (u - l.start) / (l.end - l.start);
            let theta = TAU * smooth_phase(q);
            p += l.forward * l.radius as f32 * theta.sin() as f32;
            p += Vec3::Y * l.radius as f32 * (1.0 - theta.cos()) as f32;
            p += l.right * 160.0 * theta.sin() as f32 * (PI * q).sin().powi(2) as f32;
        }
        p
    }
}

use crate::track::{Geometry, Node, Track};
use glam::{Mat3, Quat};
#[derive(Clone, Debug)]
pub struct Generated {
    pub track: Track,
    pub centreline: Centreline,
    pub recharge: Vec<[f64; 3]>,
    pub jump_distances: Vec<[f64; 3]>,
}
fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}
fn orthonormal(m: Mat3) -> Mat3 {
    let x = m.x_axis.normalize();
    let y = (m.y_axis - x * x.dot(m.y_axis)).normalize();
    let z = (m.z_axis - x * x.dot(m.z_axis) - y * y.dot(m.z_axis)).normalize();
    Mat3::from_cols(x, y, z)
}
impl Generated {
    pub fn new(seed: i64, difficulty: &str, biome: &str) -> Self {
        Self::with_recipe(seed, difficulty, biome, Recipe::for_seed(seed))
    }
    pub fn with_recipe(seed: i64, difficulty: &str, biome: &str, recipe: Recipe) -> Self {
        let c = Centreline::with_recipe(seed, difficulty, biome, recipe);
        let difficulty = c.difficulty.as_str();
        let mut raw = Vec::with_capacity(4097);
        let mut distances = vec![0.0];
        let mut length = 0.0;
        for i in 0..=4096 {
            let p = c.raw_position(i as f64 / 4096.0);
            if i > 0 {
                length += p.distance(raw[i - 1]) as f64;
                distances.push(length);
            }
            raw.push(p);
        }
        let count = (length / 9.0).ceil() as usize;
        let step = length / count as f64;
        let mut j = 0;
        let mut nodes = Vec::with_capacity(count);
        let mut hints = Vec::with_capacity(count);
        for i in 0..count {
            let distance = i as f64 * step;
            while distances[j + 1] < distance {
                j += 1;
            }
            let f = (distance - distances[j]) / (distances[j + 1] - distances[j]);
            let u = (j as f64 + f) / 4096.0;
            let is_loop = c.loops.iter().any(|l| u > l.start && u < l.end);
            let zone = if u < 0.065 {
                "repair"
            } else if c.recipe.boosts.iter().any(|s| u > s[0] && u < s[1]) {
                "boost"
            } else {
                ""
            };
            let mut section = if is_loop {
                "MAGNETIC LOOP"
            } else if u > 0.49 && u < 0.64 {
                "SKYLINE DIVE"
            } else if u > 0.38 && u < 0.48 {
                "TECHNICAL SECTOR"
            } else {
                "HIGH SPEED SWEEP"
            };
            let mut hint = c.base_position(u + 0.001) - c.base_position(u - 0.001);
            hint.y = 0.0;
            hints.push(hint.normalize().cross(Vec3::Y));
            let mut tunnel = c.recipe.tunnels.iter().any(|s| u > s[0] && u < s[1]);
            if tunnel && ["desert", "ocean"].contains(&c.biome.as_str()) {
                tunnel = false;
                section = if c.biome == "desert" {
                    "SLOT CANYON"
                } else {
                    "DEEP TRENCH"
                };
            }
            nodes.push(Node {
                u,
                geometry: Geometry {
                    p: raw[j].lerp(raw[j + 1], f as f32),
                    frame: Mat3::IDENTITY.to_cols_array_2d(),
                    width: (c.recipe.dimensions.road_width[0]
                        + c.recipe.dimensions.road_width[1]
                            * (u * TAU * 3.0 + c.phase).sin().powi(2))
                        * c.recipe.width_scale
                        + if difficulty == "easy" {
                            c.recipe.dimensions.road_width[2]
                        } else {
                            0.0
                        },
                    shape_angle: 0.0,
                },
                heading: 0.0,
                slope: 0.0,
                crest: 0.0,
                curve: 0.0,
                bank: 0.0,
                split_gap: 0.0,
                dead_side: 0.0,
                dead_gap: false,
                preferred_route: 0.0,
                air_gap: false,
                rails: true,
                feature: "ribbon".into(),
                zone: zone.into(),
                tunnel,
                is_loop,
                section: section.into(),
                before: None,
                after: None,
            });
        }
        let mut forwards = Vec::with_capacity(count);
        for i in 0..count {
            let delta =
                nodes[(i + 1) % count].geometry.p - nodes[(i + count - 1) % count].geometry.p;
            let forward = delta.normalize();
            forwards.push(forward);
            nodes[i].heading = (-delta.x as f64).atan2(delta.z as f64);
            nodes[i].slope = forward.y as f64;
            if c.components
                .iter()
                .any(|v| nodes[i].u >= v.start && nodes[i].u < v.end)
            {
                hints[i] = forward.cross(Vec3::Y).normalize();
            }
        }
        let limit = if difficulty == "easy" {
            0.20
        } else if difficulty == "hard" {
            0.34
        } else {
            0.28
        };
        let derivatives: Vec<Vec3> = (0..count)
            .map(|i| {
                (forwards[(i + 1) % count] - forwards[(i + count - 1) % count])
                    / (2.0 * step) as f32
            })
            .collect();
        let targets: Vec<f64> = (0..count)
            .map(|i| {
                if nodes[i].is_loop {
                    0.0
                } else {
                    (-derivatives[i].dot(hints[i]) as f64 * 48.0).clamp(-limit, limit)
                }
            })
            .collect();
        for i in 0..count {
            let f = forwards[i];
            let mut right = (hints[i] - f * hints[i].dot(f)).normalize();
            let mut total = 0.0;
            let mut weight = 0.0;
            for offset in -12_i64..=12 {
                let w = 13.0 - offset.abs() as f64;
                total += targets[(i as i64 + offset).rem_euclid(count as i64) as usize] * w;
                weight += w;
            }
            let bank = total / weight;
            right = Quat::from_axis_angle(f, -bank as f32) * right;
            let up = right.cross(f).normalize();
            nodes[i].bank = bank;
            nodes[i].geometry.frame =
                orthonormal(Mat3::from_cols(-right, up, f)).to_cols_array_2d();
            nodes[i].curve = derivatives[i].dot(right) as f64;
            nodes[i].crest = derivatives[i].dot(up) as f64;
        }
        let mut rng = Pcg::new((seed + 91837) as u64);
        let features: Vec<_> = c
            .recipe
            .sections
            .iter()
            .map(|&(kind, start, end)| {
                let shift = range(&mut rng, -0.006, 0.006);
                (
                    kind,
                    start + shift,
                    end + shift,
                    range(&mut rng, 0.90, 1.10),
                )
            })
            .collect();
        for n in &mut nodes {
            for &(kind, start, end, size) in &features {
                if n.u <= start || n.u >= end {
                    continue;
                }
                let q = (n.u - start) / (end - start);
                let blend = smooth_phase((q.min(1.0 - q) / 0.24).clamp(0.0, 1.0));
                n.feature = kind.into();
                match kind {
                    "chicane" => n.section = "CHICANE".into(),
                    "narrows" => {
                        n.geometry.width = mix(
                            n.geometry.width,
                            if difficulty == "easy" {
                                25.0
                            } else if difficulty == "hard" {
                                15.0
                            } else {
                                18.0
                            },
                            blend,
                        );
                        n.section = "NARROWS".into();
                    }
                    "open" => {
                        n.rails = difficulty == "easy";
                        n.section = if n.rails {
                            "SKYWAY"
                        } else {
                            "OPEN SKY Ãƒâ€šÃ‚Â· NO RAILS"
                        }
                        .into();
                    }
                    "halfpipe" => {
                        n.geometry.shape_angle =
                            if difficulty == "easy" { 1.1 } else { 1.48 } * blend;
                        n.geometry.width = mix(n.geometry.width, 52.0 * size, blend);
                        n.rails = difficulty == "easy";
                        n.section = "HALF PIPE".into();
                    }
                    "tube" => {
                        n.geometry.shape_angle = PI * blend;
                        n.geometry.width = mix(n.geometry.width, PI * 30.0 * size, blend);
                        n.rails = difficulty == "easy";
                        n.section = "360Ãƒâ€šÃ‚Â° MAGNETIC TUBE".into();
                    }
                    "split" => {
                        n.split_gap = 18.0 * size * blend;
                        n.geometry.width += n.split_gap;
                        n.section = "SPLIT ROUTE".into();
                        if difficulty == "hard" {
                            n.dead_side = if seed % 2 == 0 { 1.0 } else { -1.0 };
                            n.preferred_route = -n.dead_side;
                            n.dead_gap = q >= 0.59;
                            n.section = if n.dead_side > 0.0 {
                                "DEAD END Ãƒâ€šÃ‚Â· KEEP LEFT"
                            } else {
                                "DEAD END Ãƒâ€šÃ‚Â· KEEP RIGHT"
                            }
                            .into();
                        }
                    }
                    _ => {}
                }
                break;
            }
            for v in &c.components {
                if n.u < v.start || n.u >= v.end {
                    continue;
                }
                n.feature = v.kind.into();
                n.geometry.shape_angle = 0.0;
                n.split_gap = 0.0;
                n.tunnel = false;
                n.zone.clear();
                n.section = if v.kind == "spiral" {
                    "SPIRAL ASCENT"
                } else {
                    "HAIRPIN Ãƒâ€šÃ‚Â· BRAKE"
                }
                .into();
                if v.kind == "hairpin" {
                    let q = (n.u - v.start) / (v.end - v.start);
                    n.geometry.width = mix(
                        n.geometry.width,
                        22.0,
                        smooth_phase((q.min(1.0 - q) / 0.12).clamp(0.0, 1.0)),
                    );
                    n.rails = q < 0.12 || q > 0.88;
                }
            }
            for v in &c.jumps {
                if n.u <= v.start || n.u >= v.end {
                    continue;
                }
                let t = (n.u - v.start) / (v.end - v.start);
                n.feature = v.kind.into();
                n.air_gap = t >= v.lip && t < v.land;
                n.rails = !n.air_gap;
                n.zone.clear();
                n.geometry.width += smooth_phase((t.min(1.0 - t) / 0.2).clamp(0.0, 1.0))
                    * if difficulty == "normal" { 18.0 } else { 2.0 };
                if v.turbo && t > v.lip - 0.19 && t < v.lip - 0.025 {
                    n.zone = "boost".into();
                }
                n.section = if n.air_gap {
                    "FLIGHT GAP"
                } else if t < v.lip {
                    "JUMP Ãƒâ€šÃ‚Â· KEEP SPEED"
                } else {
                    "LANDING ZONE"
                }
                .into();
                if v.turbo {
                    n.section = if t < v.land {
                        "TURBO JUMP Ãƒâ€šÃ‚Â· NOSE DOWN"
                    } else {
                        "LANDING ZONE"
                    }
                    .into();
                }
            }
            if difficulty == "hard"
                && !n.is_loop
                && !n.tunnel
                && n.feature == "ribbon"
                && c.recipe.exposed.iter().any(|s| n.u > s[0] && n.u < s[1])
            {
                n.rails = false;
                n.section = "EXPOSED SKYWAY".into();
            }
        }
        let jump_distances: Vec<_> = c
            .jumps
            .iter()
            .map(|v| {
                let ids: Vec<_> = nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| {
                        n.feature == v.kind && n.air_gap && n.u > v.start && n.u < v.end
                    })
                    .map(|(i, _)| i)
                    .collect();
                let takeoff = ids[0] as f64 * step;
                [
                    takeoff,
                    (*ids.last().unwrap() + 1) as f64 * step,
                    takeoff - 260.0,
                ]
            })
            .collect();
        for i in 0..count {
            nodes[i].before = Some(nodes[(i + count - 1) % count].geometry.clone());
            nodes[i].after = Some(nodes[(i + 1) % count].geometry.clone());
        }
        let mut result = Self {
            track: Track {
                seed,
                difficulty: difficulty.into(),
                length,
                step,
                nodes,
            },
            centreline: c,
            recharge: vec![],
            jump_distances,
        };
        result.install_recharge();
        result
    }
    fn recharge_eligible(&self, i: usize) -> bool {
        let n = &self.track.nodes[i];
        let d = i as f64 * self.track.step;
        d > 180.0
            && n.zone != "boost"
            && !n.is_loop
            && !n.air_gap
            && n.split_gap < 0.01
            && n.geometry.shape_angle < 0.01
            && n.geometry.width >= 22.0
            && n.curve.abs() < 0.006
            && n.slope.abs() < 0.32
            && ["ribbon", "open", "chicane", "narrows"].contains(&n.feature.as_str())
            && !self
                .jump_distances
                .iter()
                .any(|j| d >= j[0] - 160.0 && d < j[1] + 120.0)
    }
    fn install_recharge(&mut self) {
        let count = self.track.nodes.len();
        let step = self.track.step;
        let mut invalid = vec![0usize];
        let mut curvature = vec![0.0];
        for i in 0..count {
            if self.track.nodes[i].zone == "repair" {
                self.track.nodes[i].zone.clear();
            }
            invalid.push(invalid[i] + usize::from(!self.recharge_eligible(i)));
            curvature.push(curvature[i] + self.track.nodes[i].curve.abs());
        }
        for fraction in [0.055, 0.40, 0.73] {
            let mut chosen = None;
            let mut best = f64::INFINITY;
            for length in [300.0, 220.0, 160.0] {
                let span = (length / step).ceil() as usize;
                for start in 0..count - span - 1 {
                    if invalid[start + span + 1] != invalid[start] {
                        continue;
                    }
                    let center = (start as f64 + span as f64 * 0.5) * step;
                    if self.recharge.iter().any(|s| (center - s[2]).abs() < 850.0) {
                        continue;
                    }
                    let score = (center - self.track.length * fraction).abs()
                        + (curvature[start + span] - curvature[start]) / span as f64 * 60000.0
                        + (300.0 - length) * 1.7;
                    if score < best {
                        best = score;
                        chosen = Some((start, span));
                    }
                }
            }
            let Some((start, span)) = chosen else {
                continue;
            };
            for i in start..start + span {
                self.track.nodes[i].zone = "repair".into();
                self.track.nodes[i].rails = true;
            }
            for i in start.saturating_sub(6)..(start + span + 6).min(count) {
                if self.recharge_eligible(i) {
                    self.track.nodes[i].rails = true;
                }
            }
            self.recharge.push([
                start as f64 * step,
                (start + span) as f64 * step,
                (start as f64 + span as f64 * 0.5) * step,
            ]);
        }
        self.recharge.sort_by(|a, b| a[0].total_cmp(&b[0]));
    }
}

#[cfg(test)]
mod generated_tests {
    use super::*;
    #[test]
    fn generated_ribbons_match_source_nodes_and_gameplay_features() {
        let cases: serde_json::Value = serde_json::from_str(&crate::oracle("tracks")).unwrap();
        for case in cases.as_array().unwrap() {
            let source: Track = serde_json::from_value(case.clone()).unwrap();
            let generated = Generated::new(source.seed, &source.difficulty, "city");
            let actual = &generated.track;
            assert_eq!(
                actual.nodes.len(),
                source.nodes.len(),
                "{} {} node count",
                source.seed,
                source.difficulty
            );
            assert!(
                (actual.length - source.length).abs() < 0.03,
                "{} {} length {} vs {}",
                source.seed,
                source.difficulty,
                actual.length,
                source.length
            );
            for (i, (a, b)) in actual.nodes.iter().zip(&source.nodes).enumerate() {
                let label = format!("{} {} node {i}", source.seed, source.difficulty);
                assert!(
                    a.geometry.p.distance(b.geometry.p) < 0.03,
                    "{label}: position {:?} vs {:?}",
                    a.geometry.p,
                    b.geometry.p
                );
                assert!(
                    (a.geometry.width - b.geometry.width).abs() < 0.002,
                    "{label}: width"
                );
                assert!((a.curve - b.curve).abs() < 0.00002, "{label}: curve");
                assert!(
                    (a.geometry.shape_angle - b.geometry.shape_angle).abs() < 0.001,
                    "{label}: shape"
                );
                assert_eq!(
                    (&a.feature, &a.zone, a.rails, a.air_gap, a.dead_gap),
                    (&b.feature, &b.zone, b.rails, b.air_gap, b.dead_gap),
                    "{label}: gameplay"
                );
            }
        }
    }
}

impl Generated {
    pub fn checkpoints(&self) -> Vec<crate::race_rules::Gate> {
        let mut spans: Vec<(f64, bool, f64)> = self
            .centreline
            .loops
            .iter()
            .map(|l| ((l.start + l.end) * 0.5, true, l.start))
            .collect();
        spans.extend(
            self.centreline
                .components
                .iter()
                .filter(|c| c.kind == "spiral")
                .map(|c| (mix(c.start, c.end, 0.48), true, c.start)),
        );
        let mut selected: Vec<(f64, crate::race_rules::Gate)> = Vec::new();
        for u in [0.25, 0.55, 0.80] {
            spans.push((u, false, u));
        }
        for (u, is_loop, entry) in spans {
            if !is_loop && selected.iter().any(|(v, _)| (v - u).abs() < 0.14) {
                continue;
            }
            let Some((i, n)) = self
                .track
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| !n.air_gap && (is_loop || !n.is_loop))
                .min_by(|(_, a), (_, b)| (a.u - u).abs().total_cmp(&(b.u - u).abs()))
            else {
                continue;
            };
            let mut respawn = i as f64 * self.track.step - 100.0;
            if is_loop {
                if let Some(k) = self.track.nodes[..i].iter().position(|n| n.u >= entry) {
                    respawn = k as f64 * self.track.step - 100.0;
                }
            }
            let height = if n.tunnel {
                13.0
            } else if n.geometry.shape_angle > 2.0 {
                2.0 * n.geometry.width / n.geometry.shape_angle + 6.0
            } else if is_loop {
                24.0
            } else {
                32.0
            };
            selected.push((
                n.u,
                crate::race_rules::Gate {
                    pose: crate::race_rules::Pose {
                        origin: n.geometry.p,
                        basis: n.geometry.basis(),
                    },
                    distance: i as f64 * self.track.step,
                    width: n.geometry.width + 6.0,
                    height,
                    respawn: respawn.max(0.0),
                },
            ));
        }
        selected.sort_by(|a, b| a.1.distance.total_cmp(&b.1.distance));
        selected.into_iter().map(|(_, g)| g).collect()
    }
    pub fn safe_respawn(&self, distance: f64) -> f64 {
        let local = distance.rem_euclid(self.track.length);
        let u = self.track.nodes[(local / self.track.step) as usize].u;
        if self.track.difficulty == "hard" {
            let mut rng = Pcg::new((self.track.seed + 91837) as u64);
            for &(kind, start, end) in &self.centreline.recipe.sections {
                let shift = range(&mut rng, -0.006, 0.006);
                let _size = range(&mut rng, 0.90, 1.10);
                if kind == "split" && u >= start + shift && u < end + shift {
                    if let Some(i) = self.track.nodes.iter().position(|n| n.u >= start + shift) {
                        return (distance / self.track.length).floor() * self.track.length
                            + i as f64 * self.track.step
                            - 180.0;
                    }
                }
            }
        }
        for j in &self.jump_distances {
            if local >= j[0] - 260.0 && local < j[1] {
                return (distance / self.track.length).floor() * self.track.length + j[2];
            }
        }
        distance
    }
}
