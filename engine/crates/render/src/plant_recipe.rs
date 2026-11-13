//! Woody silhouettes assembled from shared detailed twig and foliage sprays.
//!
//! A recipe changes placement, scale and branch layout, never the primitive
//! vertex bytes. Cook once per selected recipe/seed; runtime wind and rendering
//! use shared primitives and GPU branch poses. These are authored
//! silhouettes, not a botanical growth simulation.
use crate::{
    cluster::Vertex,
    foliage_assembly::{
        Archetype, Assembly, BranchGroup, Limits, Parameters, PartGeometry, PartInstance,
    },
    scene::{Material, THIN_FOLIAGE, TWO_SIDED},
};
use glam::{Mat3, Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoliageKind {
    Needle,
    Broadleaf,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrownProfile {
    Conical,
    Columnar,
    Rounded,
    Umbrella,
    Spreading,
}

#[derive(Clone, Copy, Debug)]
pub struct WoodyRecipe {
    /// Stable author-chosen identity; distinct recipes must not reuse it.
    pub id: u64,
    pub height: f32,
    pub trunk_radius: f32,
    /// Fraction of trunk height at the lowest branch; 0..0.9.
    pub crown_base: f32,
    pub crown_radius: f32,
    pub crown_profile: CrownProfile,
    pub tiers: u32,
    pub branches_per_tier: u32,
    /// Vertical rise per metre of horizontal branch length.
    pub branch_rise: f32,
    pub branch_radius: f32,
    pub stiffness: f32,
    pub foliage: FoliageKind,
    /// Number of detailed woody/leafy spray pairs on each primary branch.
    pub sprays_per_branch: u32,
    /// Uniform scale of the metre-long immutable spray, not individual leaves.
    pub spray_scale: f32,
    /// Positive axial component of spray direction along its supporting branch.
    pub spray_axial_bias: f32,
    pub foliage_colour: [f32; 3],
    pub wood_colour: [f32; 3],
    /// A collision-bearing trunk keeps this above-root height/radius under
    /// growth, and extends 0.5 m below the root to match a trunk collider.
    /// Zero selects decorative shrubs with uniformly scaled stems.
    pub protected_trunk_height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct BuildOptions {
    pub seed: u64,
    /// Static rest-size parameter. Branch and leaf identities stay stable.
    pub growth: f32,
    /// 0 = authored colour, 1 = muted/dry colour. Does not remove leaves.
    pub season: f32,
}
impl BuildOptions {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            growth: 1.0,
            season: 0.0,
        }
    }
}

/// Bounded authoring budget, separate from total runtime branch-pose limits.
pub fn recipe_limits() -> Limits {
    Limits {
        max_parts: 14,
        max_branches: 64,
        max_instances: 8192,
        // Expanded arithmetic limits protect optional offline flatten/bake callers.
        // Runtime cook_source stores only the unique meshes and transforms.
        max_vertices: 32_000_000,
        max_indices: 120_000_000,
        max_source_bytes: 1536 * 1024 * 1024,
    }
}

fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}
fn key(recipe: u64, seed: u64, tag: u64, index: u64) -> u64 {
    mix(recipe
        ^ seed.rotate_left(21)
        ^ tag.wrapping_mul(0x9e3779b97f4a7c15)
        ^ index.wrapping_mul(0xd1b54a32d192ed03))
}
fn unit(recipe: u64, seed: u64, tag: u64, index: u64) -> f32 {
    (key(recipe, seed, tag, index) >> 40) as f32 / 16_777_216.0
}
fn finite_range(value: f32, lo: f32, hi: f32) -> bool {
    value.is_finite() && (lo..=hi).contains(&value)
}

impl WoodyRecipe {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0
            || !finite_range(self.height, 0.3, 40.0)
            || !finite_range(self.trunk_radius, 0.005, 1.0)
            || !finite_range(self.crown_base, 0.0, 0.9)
            || !finite_range(self.crown_radius, 0.1, 12.0)
            || !(1..=12).contains(&self.tiers)
            || !(1..=12).contains(&self.branches_per_tier)
            || 2 * self.tiers * self.branches_per_tier + 1 > 64
            || !finite_range(self.branch_rise, -0.35, 2.0)
            || !finite_range(self.branch_radius, 0.002, 0.25)
            || !finite_range(self.stiffness, 0.0, 1.0)
            || !(4..=96).contains(&self.sprays_per_branch)
            || !finite_range(self.spray_scale, 0.1, 1.5)
            || !finite_range(self.spray_axial_bias, 0.05, 4.0)
            || self
                .foliage_colour
                .iter()
                .chain(&self.wood_colour)
                .any(|&v| !finite_range(v, 0.0, 1.0))
            || !finite_range(self.protected_trunk_height, 0.0, 8.0)
            || self.protected_trunk_height >= self.height * 0.8
        {
            return Err("invalid woody species recipe or branch budget".into());
        }
        Ok(())
    }
}

fn triangle(part: &mut PartGeometry, a: Vec3, b: Vec3, c: Vec3) {
    let normal = (b - a).cross(c - a).normalize().to_array();
    for position in [a, b, c] {
        part.indices.push(part.vertices.len() as u32);
        part.vertices.push(Vertex {
            pos: position.to_array(),
            normal,
            uv: [position.x + 0.5, position.y],
        });
    }
}

/// Twelve-sided closed stem. The lower 80% has constant radius; the upper collar
/// tapers. Radius one is the apothem, so a radius-r trunk encloses an r cylinder.
fn stem() -> PartGeometry {
    let mut part = PartGeometry {
        id: 0x5750_5354_454d_0001,
        vertices: vec![],
        indices: vec![],
    };
    let ring = |i: usize, y: f32, radius: f32| {
        let a = i as f32 * std::f32::consts::TAU / 12.0;
        Vec3::new(
            a.cos() * radius / (std::f32::consts::PI / 12.0).cos(),
            y,
            a.sin() * radius / (std::f32::consts::PI / 12.0).cos(),
        )
    };
    for i in 0..12 {
        let next = (i + 1) % 12;
        for (y0, r0, y1, r1) in [(0.0, 1.0, 0.8, 1.0), (0.8, 1.0, 1.0, 0.16)] {
            let (a, b, c, d) = (
                ring(i, y0, r0),
                ring(i, y1, r1),
                ring(next, y1, r1),
                ring(next, y0, r0),
            );
            triangle(&mut part, a, b, c);
            triangle(&mut part, a, c, d);
        }
        triangle(
            &mut part,
            Vec3::ZERO,
            ring(i, 0.0, 1.0),
            ring(next, 0.0, 1.0),
        );
        triangle(
            &mut part,
            Vec3::Y,
            ring(next, 1.0, 0.16),
            ring(i, 1.0, 0.16),
        );
    }
    part
}

pub(crate) const STEM: u32 = 0;
pub(crate) const SEGMENT: u32 = 13;
pub(crate) const SEGMENT_TIP: f32 = 0.70;

/// Smooth tapered segment; joints are overlapped by a small collar. Used for
/// curved upper trunks and branch axes, while part zero protects the collider.
fn segment() -> PartGeometry {
    let mut part = PartGeometry {
        id: 0x5750_5345_474d_0001,
        vertices: vec![],
        indices: vec![],
    };
    let sides = 12;
    for i in 0..sides {
        let a = i as f32 * std::f32::consts::TAU / sides as f32;
        for (y, r) in [(0.0, 1.0), (1.0, SEGMENT_TIP)] {
            let normal = Vec3::new(a.cos(), 0.30, a.sin()).normalize();
            part.vertices.push(Vertex {
                pos: [r * a.cos(), y, r * a.sin()],
                normal: normal.to_array(),
                uv: [i as f32 / sides as f32, y],
            });
        }
    }
    for i in 0..sides {
        let a = (i * 2) as u32;
        let b = (((i + 1) % sides) * 2) as u32;
        part.indices
            .extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
        let ring = |i: usize, y: f32, r: f32| {
            let a = i as f32 * std::f32::consts::TAU / sides as f32;
            Vec3::new(r * a.cos(), y, r * a.sin())
        };
        triangle(
            &mut part,
            Vec3::ZERO,
            ring(i, 0.0, 1.0),
            ring((i + 1) % sides, 0.0, 1.0),
        );
        triangle(
            &mut part,
            Vec3::Y,
            ring((i + 1) % sides, 1.0, SEGMENT_TIP),
            ring(i, 1.0, SEGMENT_TIP),
        );
    }
    part
}

/// Stable palette: collider stem, three needle wood/foliage pairs, three
/// broadleaf pairs, then tapered segment. Equal IDs always have equal bytes.
pub fn primitive_library() -> Vec<PartGeometry> {
    let mut parts = vec![stem()];
    for generator in [
        crate::plant_spray::needle_spray,
        crate::plant_spray::broadleaf_spray,
    ] {
        for variant in 0..3 {
            let (wood, foliage) = generator(variant);
            parts.push(wood);
            parts.push(foliage);
        }
    }
    parts.push(segment());
    parts
}

fn material(colour: [f32; 3], foliage: bool) -> Material {
    Material {
        tint: colour,
        layer: -1,
        roughness: if foliage { 0.86 } else { 0.96 },
        flags: if foliage { THIN_FOLIAGE | TWO_SIDED } else { 0 },
        scale: 1.0,
        metal: 0.0,
    }
}

fn profile(profile: CrownProfile, t: f32) -> f32 {
    match profile {
        CrownProfile::Conical => 1.0 - 0.82 * t,
        CrownProfile::Columnar => 0.70 + 0.28 * (t * std::f32::consts::PI).sin(),
        CrownProfile::Rounded => (1.0 - (1.72 * t - 0.86).powi(2)).sqrt(),
        CrownProfile::Umbrella => 0.65 + 0.35 * (t * std::f32::consts::FRAC_PI_2).sin(),
        CrownProfile::Spreading => 1.0 - 0.30 * t,
    }
}

pub fn build(
    recipe: &WoodyRecipe,
    options: BuildOptions,
    limits: Limits,
) -> Result<Assembly, String> {
    recipe.validate()?;
    if !finite_range(options.growth, 0.25, 1.5) || !finite_range(options.season, 0.0, 1.0) {
        return Err("plant growth must be 0.25..1.5 and season 0..1".into());
    }
    let count = recipe.tiers as usize * recipe.branches_per_tier as usize;
    let instances_count = 9 + count * (4 + 2 * recipe.sprays_per_branch as usize);
    if limits.max_parts < 14
        || limits.max_branches < 2 * count + 1
        || limits.max_instances < instances_count
    {
        return Err("woody recipe exceeds explicit primitive, branch or placement limits".into());
    }
    let identity = |tag, index| key(recipe.id, options.seed, tag, index);
    let random = |tag, index| unit(recipe.id, options.seed, tag, index);
    let growth = options.growth;
    let height = (recipe.height * growth).max(recipe.protected_trunk_height * 1.30);
    let protected = recipe.protected_trunk_height > 0.0;
    let embed = if protected { 0.5 } else { 0.0 };
    let trunk_radius = recipe.trunk_radius * if protected { 1.0 } else { growth };
    let lower_length = if protected {
        (recipe.protected_trunk_height + embed) / 0.8 + 0.01
    } else {
        height * 0.46
    };
    let join_y = lower_length * 0.78 - embed;
    let bend = Vec3::new(random(21, 0) - 0.5, 0.0, random(22, 0) - 0.5) * height * 0.045;
    let trunk_point = |y: f32| {
        let t = ((y - join_y) / (height - join_y)).clamp(0.0, 1.0);
        Vec3::Y * y + bend * t * t
    };
    let base = height * recipe.crown_base;
    let top = height * 0.94;
    let mut branches = vec![BranchGroup {
        id: identity(1, 0),
        parent: None,
        rest: Mat4::IDENTITY,
        stiffness: 1.0,
    }];
    let mut instances = vec![PartInstance {
        id: identity(2, 0),
        part: STEM,
        branch: 0,
        rest: Mat4::from_scale_rotation_translation(
            Vec3::new(trunk_radius, lower_length, trunk_radius),
            Quat::IDENTITY,
            -Vec3::Y * embed,
        ),
        material: 0,
    }];
    // Overlapping tapered sections give the upper trunk a continuous organic
    // axis. The protected lower cylinder still exactly encloses its collider.
    for j in 0..8 {
        let t = j as f32 / 8.0;
        let next = (j + 1) as f32 / 8.0;
        let a = trunk_point(join_y + (height - join_y) * t);
        let b = trunk_point(join_y + (height - join_y) * next);
        let delta = b - a;
        let radius = trunk_radius * SEGMENT_TIP.powi(j as i32);
        instances.push(PartInstance {
            id: identity(31, j),
            part: SEGMENT,
            branch: 0,
            material: 0,
            rest: Mat4::from_scale_rotation_translation(
                Vec3::new(radius, delta.length() * 1.04, radius),
                Quat::from_rotation_arc(Vec3::Y, delta.normalize()),
                a,
            ),
        });
    }
    for tier in 0..recipe.tiers {
        for ray in 0..recipe.branches_per_tier {
            let i = tier * recipe.branches_per_tier + ray;
            let branch = 2 * i + 1;
            // Break horizontal rings, keep a species-level silhouette. Golden
            // angles distribute branches without regularly repeated spokes.
            let t = ((tier as f32 + 0.18 + random(23, u64::from(i)) * 0.64) / recipe.tiers as f32)
                .min(0.98);
            let y = base + (top - base) * t;
            let angle = i as f32 * 2.399_963_1
                + random(3, 0) * std::f32::consts::TAU
                + (random(4, u64::from(i)) - 0.5) * 0.65;
            let spread = recipe.crown_radius
                * profile(recipe.crown_profile, t)
                * growth
                * (0.78 + 0.37 * random(5, u64::from(i)));
            let rise = recipe.branch_rise + (random(24, u64::from(i)) - 0.5) * 0.20;
            let direction = Vec3::new(angle.cos(), rise, angle.sin()).normalize();
            let side = direction.cross(Vec3::Y).normalize();
            let roll = match recipe.foliage {
                FoliageKind::Needle => (random(35, u64::from(i)) - 0.5) * 0.7,
                FoliageKind::Broadleaf => random(35, u64::from(i)) * std::f32::consts::TAU,
            };
            let rotation =
                Quat::from_mat3(&Mat3::from_cols(side, direction, side.cross(direction)))
                    * Quat::from_rotation_y(roll);
            branches.push(BranchGroup {
                id: identity(1, u64::from(branch)),
                parent: Some(0),
                rest: Mat4::from_rotation_translation(rotation, trunk_point(y)),
                stiffness: (recipe.stiffness + 0.10 * t).clamp(0.0, 1.0),
            });
            let length = spread * (1.0 + rise.powi(2)).sqrt();
            let radius = recipe.branch_radius * (1.0 - 0.58 * t) * growth;
            // Branch bends towards world up near its growing tip, with a
            // distinct stable sideways deviation rather than a straight rod.
            let up = rotation.inverse() * Vec3::Y;
            let sideways = Vec3::X * (random(25, u64::from(i)) - 0.5) * 0.10;
            let curve = |u: f32| Vec3::Y * length * u + (up * 0.13 + sideways) * length * u * u;
            let tip_branch = branch + 1;
            let tip_pivot = curve(0.5);
            branches.push(BranchGroup {
                id: identity(1, u64::from(tip_branch)),
                parent: Some(branch),
                rest: Mat4::from_translation(tip_pivot),
                stiffness: (recipe.stiffness - 0.12).max(0.0),
            });
            for section in 0..4 {
                let u = section as f32 / 4.0;
                let v = (section + 1) as f32 / 4.0;
                let a = curve(u);
                let delta = curve(v) - a;
                let r = radius * SEGMENT_TIP.powi(section as i32);
                let (pose, offset) = if section >= 2 {
                    (tip_branch, tip_pivot)
                } else {
                    (branch, Vec3::ZERO)
                };
                instances.push(PartInstance {
                    id: identity(32, u64::from(i) * 4 + section),
                    part: SEGMENT,
                    branch: pose,
                    material: 0,
                    rest: Mat4::from_scale_rotation_translation(
                        Vec3::new(r, delta.length() * 1.06, r),
                        Quat::from_rotation_arc(Vec3::Y, delta.normalize()),
                        a - offset,
                    ),
                });
            }
            for spray in 0..recipe.sprays_per_branch {
                let index = u64::from(i) * u64::from(recipe.sprays_per_branch) + u64::from(spray);
                let along = (spray as f32 + 0.25 + 0.50 * random(6, index))
                    / recipe.sprays_per_branch as f32;
                let u = 0.12 + 0.86 * along;
                // Alternating side shoots, with some upper shoots, form a
                // layered frond instead of a bottlebrush of giant needles.
                let theta = if spray % 3 == 2 {
                    0.4 + random(7, index) * 2.2
                } else if spray % 2 == 0 {
                    -0.6 + random(7, index) * 0.75
                } else {
                    std::f32::consts::PI + 0.2 - random(7, index) * 0.75
                };
                let axis = Vec3::new(
                    theta.cos(),
                    recipe.spray_axial_bias + along * 0.65,
                    theta.sin(),
                )
                .normalize();
                let orientation = Quat::from_rotation_arc(Vec3::Y, axis)
                    * Quat::from_rotation_y((random(8, index) - 0.5) * 1.8);
                // Shrubs scale whole sprays, preserving each component's
                // internal twig/leaf proportions. Top branches shorten gently.
                let scale = recipe.spray_scale
                    * growth
                    * (0.82 + 0.30 * random(9, index))
                    * (1.0 - 0.24 * t);
                let transform = Mat4::from_scale_rotation_translation(
                    Vec3::splat(scale),
                    orientation,
                    curve(u),
                );
                let first = if recipe.foliage == FoliageKind::Needle {
                    1
                } else {
                    7
                };
                let variant = (key(recipe.id, options.seed, 33, index) % 3) as u32;
                let (pose, transform) = if u >= 0.5 {
                    (tip_branch, Mat4::from_translation(-tip_pivot) * transform)
                } else {
                    (branch, transform)
                };
                for member in 0..2 {
                    instances.push(PartInstance {
                        id: identity(10, index * 2 + u64::from(member)),
                        part: first + variant * 2 + member,
                        branch: pose,
                        rest: transform,
                        material: member,
                    });
                }
            }
        }
    }
    let dry = match recipe.foliage {
        FoliageKind::Needle => [0.90, 0.86, 0.77],
        FoliageKind::Broadleaf => [1.16, 0.75, 0.44],
    };
    let colour =
        std::array::from_fn(|i| recipe.foliage_colour[i] * (1.0 + (dry[i] - 1.0) * options.season));
    Assembly::from_parts(
        Parameters {
            seed: options.seed,
            archetype: Archetype::ConiferTwig,
            growth,
            season: options.season,
            density: count as u32 * recipe.sprays_per_branch,
        },
        primitive_library(),
        branches,
        instances,
        vec![material(recipe.wood_colour, false), material(colour, true)],
        limits,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn recipe() -> WoodyRecipe {
        WoodyRecipe {
            id: 7,
            height: 14.0,
            trunk_radius: 0.22,
            crown_base: 0.24,
            crown_radius: 2.8,
            crown_profile: CrownProfile::Conical,
            tiers: 6,
            branches_per_tier: 5,
            branch_rise: 0.12,
            branch_radius: 0.065,
            stiffness: 0.65,
            foliage: FoliageKind::Needle,
            sprays_per_branch: 12,
            spray_scale: 1.0,
            spray_axial_bias: 0.6,
            foliage_colour: [0.07, 0.15, 0.045],
            wood_colour: [0.16, 0.10, 0.05],
            protected_trunk_height: 5.5,
        }
    }
    fn primitive_bits(assembly: &Assembly) -> Vec<u32> {
        assembly
            .parts
            .iter()
            .flat_map(|p| {
                p.vertices
                    .iter()
                    .flat_map(|v| {
                        v.pos
                            .into_iter()
                            .chain(v.normal)
                            .chain(v.uv)
                            .map(f32::to_bits)
                    })
                    .chain(p.indices.iter().copied())
            })
            .collect()
    }
    #[test]
    fn immutable_sprays_and_identities_survive_growth_and_season() {
        let r = recipe();
        let a = build(&r, BuildOptions::new(14), recipe_limits()).unwrap();
        let b = build(
            &r,
            BuildOptions {
                seed: 14,
                growth: 0.5,
                season: 1.0,
            },
            recipe_limits(),
        )
        .unwrap();
        assert_eq!(primitive_bits(&a), primitive_bits(&b));
        assert_eq!(
            a.instances.iter().map(|i| i.id).collect::<Vec<_>>(),
            b.instances.iter().map(|i| i.id).collect::<Vec<_>>()
        );
        assert_eq!(
            a.branches.iter().map(|i| i.id).collect::<Vec<_>>(),
            b.branches.iter().map(|i| i.id).collect::<Vec<_>>()
        );
        assert_ne!(a.materials[1].tint, b.materials[1].tint);
        assert!(b.bounds().unwrap()[1][1] < a.bounds().unwrap()[1][1]);
        for growth in [0.25, 0.5, 1.0, 1.5] {
            let a = build(
                &r,
                BuildOptions {
                    seed: 14,
                    growth,
                    season: 0.0,
                },
                recipe_limits(),
            )
            .unwrap();
            assert!(
                a.instances[0].rest.y_axis.y * 0.8 + a.instances[0].rest.w_axis.y
                    >= r.protected_trunk_height
            );
        }
    }
    #[test]
    fn species_share_mesh_bytes_and_spray_wood_follows_foliage() {
        let mut r = recipe();
        let a = build(&r, BuildOptions::new(1), recipe_limits()).unwrap();
        r.id = 8;
        r.foliage = FoliageKind::Broadleaf;
        r.crown_profile = CrownProfile::Umbrella;
        r.crown_radius = 3.5;
        let b = build(&r, BuildOptions::new(2), recipe_limits()).unwrap();
        assert_eq!(primitive_bits(&a), primitive_bits(&b));
        assert_ne!(a.bounds().unwrap(), b.bounds().unwrap());
        let again = build(&r, BuildOptions::new(2), recipe_limits()).unwrap();
        for (a, b) in b.instances.iter().zip(&again.instances) {
            assert_eq!(a.rest, b.rest);
            assert_eq!(a.id, b.id);
        }
        for a in [&a, &b] {
            for pair in a
                .instances
                .windows(2)
                .filter(|p| (1..=12).contains(&p[0].part) && p[0].material == 0)
            {
                assert_eq!(pair[0].rest, pair[1].rest);
                assert_eq!(pair[0].branch, pair[1].branch);
                assert_eq!(pair[0].part + 1, pair[1].part);
                assert_eq!(pair[1].material, 1);
            }
        }
    }
    #[test]
    fn malformed_recipes_and_capacity_fail_without_truncation() {
        let mut r = recipe();
        r.tiers = 12;
        r.branches_per_tier = 12;
        assert!(build(&r, BuildOptions::new(1), recipe_limits()).is_err());
        r = recipe();
        r.spray_scale = f32::NAN;
        assert!(r.validate().is_err());
        assert!(
            build(
                &recipe(),
                BuildOptions::new(0),
                Limits {
                    max_instances: 10,
                    ..recipe_limits()
                }
            )
            .is_err()
        );
        assert!(
            build(
                &recipe(),
                BuildOptions::new(0),
                Limits {
                    max_source_bytes: 1,
                    ..recipe_limits()
                }
            )
            .is_err()
        );
    }
}
