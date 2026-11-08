//! Small deterministic foliage assemblies authored from opaque geometry.
//!
//! Shared tapered blades/needles are instanced in a parent-first branch rig.
//! Both source meshes and voxel assets are derived from these exact parts.
//! Growth changes rest dimensions; season changes material colour. Neither is
//! a biological simulation or a per-frame rebuild. Wind poses belong outside
//! this immutable source, and must retain the branch/part identities below.

use crate::{cluster::{Part, Vertex}, foliage_voxel::{self, BakeConfig, VoxelAsset}, scene::{Material, THIN_FOLIAGE, TWO_SIDED}};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Archetype { GrassTuft, ConiferTwig, MeadowGrass, MeadowPatch }

/// Authored blades inside each unchanged world-cell patch. Lower budgets
/// widen retained ribbons, not whole plants, and require a fresh aggregate
/// cook. This preserves neither exact silhouette union nor optical parity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeadowBladeBudget { Blades120, Blades180, Blades360 }
impl MeadowBladeBudget {
    pub const fn blades(self) -> usize {
        match self { Self::Blades120 => 120, Self::Blades180 => 180, Self::Blades360 => 360 }
    }
    pub fn width_multiplier(self) -> f32 { 360.0 / self.blades() as f32 }
}
impl TryFrom<u32> for MeadowBladeBudget {
    type Error = String;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value { 120 => Ok(Self::Blades120), 180 => Ok(Self::Blades180),
            360 => Ok(Self::Blades360), _ => Err("meadow blade budget must be 120, 180 or 360".into()) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Parameters {
    pub seed: u64,
    pub archetype: Archetype,
    /// Rest-size multiplier, 0.1..=1.5. The assembly root remains the origin.
    /// MeadowPatch scales blade dimensions about each fixed basal anchor.
    pub growth: f32,
    /// 0 = green, 1 = dry/dormant palette. Does not erase source identities.
    pub season: f32,
    /// Exact blade/needle count, 1..=128 (MeadowPatch: 1..=360) for built-in
    /// generators. Custom assemblies use their explicit placement limits.
    /// Independent
    /// of growth and season. MeadowPatch places 90 radial/angle strata per
    /// quadrant in a feathered, rounded footprint; it does not discard roots.
    pub density: u32,
}

impl Parameters {
    pub fn grass(seed: u64) -> Self {
        Self { seed, archetype: Archetype::GrassTuft, growth: 1.0, season: 0.0, density: 36 }
    }
    /// Fuller meadow silhouette at the reference tuft's placement/triangle
    /// budget: 36 blades, 22 triangles per blade, three reusable shapes and
    /// four independently animated groups plus the root. Opt-in recipe;
    /// `grass` remains the unchanged reference used by existing fixtures.
    pub fn meadow_grass(seed: u64) -> Self {
        Self { seed, archetype: Archetype::MeadowGrass, growth: 1.0, season: 0.0, density: 36 }
    }
    /// Dense patch for .45 m root spacing: 360 narrow, opaque five-triangle ribbons,
    /// three shared shapes and four spatial wind groups. Ribbons are open
    /// two-sided leaf surfaces, not textured rectangles or closed solids.
    /// Its rounded source footprint has a maximum .38 m basal radius with a
    /// full core and gradually thinner edge; neighbouring patches overlap.
    /// Use Limits::meadow_patch() and SheetCrossings for a voxel bake.
    pub fn meadow_patch(seed: u64) -> Self {
        Self { seed, archetype: Archetype::MeadowPatch, growth: 1.0, season: 0.0, density: 360 }
    }
    pub fn conifer_twig(seed: u64) -> Self {
        Self { seed, archetype: Archetype::ConiferTwig, growth: 1.0, season: 0.0, density: 64 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_parts: usize,
    pub max_branches: usize,
    pub max_instances: usize,
    /// Expanded source geometry, including every part instance.
    pub max_vertices: usize,
    pub max_indices: usize,
    /// Expanded vertex/index payload only. Voxel storage has BakeConfig caps.
    pub max_source_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_parts: 8, max_branches: 16, max_instances: 256,
            max_vertices: 32_768, max_indices: 98_304, max_source_bytes: 2 * 1024 * 1024 }
    }
}

impl Limits {
    /// Explicit higher placement budget for MeadowPatch. Other recipes retain
    /// their parameter limit of 128 even when given this resource allowance.
    pub fn meadow_patch() -> Self { Self { max_instances: 360, ..Self::default() } }
}

#[derive(Clone, Debug)]
pub struct PartGeometry {
    /// Geometry identity is independent of the placement seed and season.
    pub id: u64,
    /// +Y runs from root to tip. No alpha mask or broad cutout rectangle.
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct BranchGroup {
    pub id: u64,
    /// Parent index must be smaller than this group's index.
    pub parent: Option<u32>,
    /// Rigid pivot transform in parent space. Runtime poses compose with this
    /// rest. Growth scale is in part geometry/positions, never branch bases.
    pub rest: Mat4,
    pub stiffness: f32,
}

#[derive(Clone, Debug)]
pub struct PartInstance {
    pub id: u64,
    pub part: u32,
    pub branch: u32,
    /// Root/rotation/positive scale in branch-pivot space.
    pub rest: Mat4,
    pub material: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceIdentity { pub instance: u64, pub part: u64, pub branch: u64 }

#[derive(Clone, Debug)]
pub struct SourceMesh {
    pub identity: SourceIdentity,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: u32,
}

impl SourceMesh {
    pub fn as_part(&self) -> Part<'_> {
        Part { vertices: &self.vertices, indices: &self.indices, material: self.material }
    }
}

#[derive(Clone, Debug)]
pub struct Assembly {
    pub parameters: Parameters,
    pub parts: Vec<PartGeometry>,
    pub branches: Vec<BranchGroup>,
    pub instances: Vec<PartInstance>,
    pub materials: Vec<Material>,
    limits: Limits,
}

#[derive(Clone, Debug)]
pub struct BakedBranch {
    pub branch: u32,
    pub branch_id: u64,
    /// The volume and matching source vertices are in this pivot's space.
    pub rest_to_assembly: Mat4,
    pub asset: VoxelAsset,
    /// Provenance survives every mip of this volume; no new random placement.
    pub source_ids: Vec<u64>,
}

fn key(seed: u64, tag: u64, index: u64) -> u64 {
    let mut x = seed ^ tag.wrapping_mul(0x9e3779b97f4a7c15) ^ index.wrapping_mul(0xd1b54a32d192ed03);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
fn unit(seed: u64, tag: u64, index: u64) -> f32 { ((key(seed, tag, index) >> 40) as u32) as f32 / 16_777_216.0 }
fn identity(p: Parameters, tag: u64, index: u64) -> u64 {
    let archetype = match p.archetype {
        Archetype::GrassTuft => 100,
        Archetype::ConiferTwig => 200,
        Archetype::MeadowGrass => 300,
        Archetype::MeadowPatch => 400,
    };
    key(p.seed, tag + archetype, index)
}
fn material(colour: Vec3, roughness: f32, two_sided: bool) -> Material {
    Material { tint: colour.to_array(), layer: -1, roughness,
        flags: if two_sided { TWO_SIDED | THIN_FOLIAGE } else { 0 }, scale: 1.0, metal: 0.0 }
}
fn triangle(mesh: &mut PartGeometry, a: Vec3, b: Vec3, c: Vec3) {
    let normal = (b - a).cross(c - a).normalize().to_array();
    for p in [a, b, c] {
        mesh.indices.push(mesh.vertices.len() as u32);
        mesh.vertices.push(Vertex { pos: p.to_array(), normal, uv: [p.x + 0.5, p.y] });
    }
}

/// Tapered closed triangular cross-sections, with real thickness and a bent
/// centreline. The shape is thin, but its silhouette is entirely geometry.
fn blade(id: u64, curve: f32, thickness: f32) -> PartGeometry {
    let mut mesh = PartGeometry { id, vertices: Vec::new(), indices: Vec::new() };
    let heights = [0.0f32, 0.32, 0.68, 0.9];
    let rings: [[Vec3; 3]; 4] = heights.map(|y| std::array::from_fn(|j| {
        let angle = j as f32 * std::f32::consts::TAU / 3.0;
        Vec3::new(curve * y * y + 0.5 * (1.0 - y).sqrt() * angle.cos(),
            y, thickness * (1.0 - y).sqrt() * angle.sin())
    }));
    triangle(&mut mesh, rings[0][0], rings[0][1], rings[0][2]);
    for i in 0..rings.len() - 1 { for j in 0..3 {
        let next = (j + 1) % 3;
        triangle(&mut mesh, rings[i][j], rings[i + 1][j], rings[i + 1][next]);
        triangle(&mut mesh, rings[i][j], rings[i + 1][next], rings[i][next]);
    }}
    let tip = Vec3::new(curve, 1.0, 0.0);
    for j in 0..3 { triangle(&mut mesh, rings[3][j], tip, rings[3][(j + 1) % 3]); }
    mesh
}

/// Three authored narrow, curled silhouettes share the same four-ring closed
/// topology as the reference blade. Width increases above the narrow collar,
/// then tapers to the tip. A modest twist changes face orientation along the
/// blade; normals describe that geometry, not an artificial world-up vector.
/// Keeping every ring at one strictly increasing local Y preserves the source
/// CLOD recognizer, while the instance lean turns tip curvature into an arch.
/// The broad section lies across that arch (a positive 90-degree Y rotation),
/// so a low blade presents its physical ribbon face toward the ground.
fn meadow_blade(variant: u64) -> PartGeometry {
    let mut mesh = PartGeometry { id: key(0, 500, variant), vertices: Vec::new(), indices: Vec::new() };
    let (curve, twist, side_curl) = match variant {
        0 => (4.2, -0.12, -0.25),
        1 => (5.8, 0.10, 0.30),
        _ => (7.2, 0.18, -0.15),
    };
    let heights = [0.0f32, 0.32, 0.68, 0.9];
    let widths = [0.34f32, 0.56, 0.40, 0.16];
    let rings: [[Vec3; 3]; 4] = std::array::from_fn(|i| {
        let y = heights[i];
        let rotation = Quat::from_rotation_y(twist * y);
        let centre = Vec3::new(curve * y * y, y, side_curl * y * y);
        std::array::from_fn(|j| {
            let angle = j as f32 * std::f32::consts::TAU / 3.0;
            centre + rotation * Vec3::new(0.065 * (1.0 - y).sqrt() * angle.sin(), 0.0,
                -widths[i] * angle.cos())
        })
    });
    triangle(&mut mesh, rings[0][0], rings[0][1], rings[0][2]);
    for i in 0..rings.len() - 1 { for j in 0..3 {
        let next = (j + 1) % 3;
        triangle(&mut mesh, rings[i][j], rings[i + 1][j], rings[i + 1][next]);
        triangle(&mut mesh, rings[i][j], rings[i + 1][next], rings[i][next]);
    }}
    let tip = Vec3::new(curve, 1.0, side_curl);
    for j in 0..3 { triangle(&mut mesh, rings[3][j], tip, rings[3][(j + 1) % 3]); }
    mesh
}

fn meadow_group_angle(seed: u64, group: u32) -> f32 {
    unit(seed, 1, 0) * std::f32::consts::TAU
        + group as f32 * std::f32::consts::FRAC_PI_2
        + (unit(seed, 30, group as u64) - 0.5) * 0.75
}

/// Open physical leaf: three paired stations plus one tip. The root pair's
/// midpoint is exactly zero, so the placement origin is the animation anchor.
/// Pair order and increasing station Y define the ribbon CLOD contract.
fn meadow_patch_blade(variant: u64) -> PartGeometry {
    let mut mesh = PartGeometry { id: key(0, 600, variant), vertices: Vec::new(), indices: Vec::new() };
    let curve = [2.0f32, 4.0, 6.0][variant as usize];
    let pairs = [0.0f32, 0.45, 0.78].map(|y| {
        // Most displacement is lean, with a modest physical bend. Extreme
        // arches would make a cheap root-tip LOD lose substantial leaf area.
        let centre = Vec3::new(curve * (0.8 * y + 0.2 * y * y), y, 0.0);
        let across = Vec3::Z * (0.5 * (1.0 - y));
        [centre + across, centre - across]
    });
    for pair in pairs.windows(2) {
        triangle(&mut mesh, pair[0][0], pair[0][1], pair[1][1]);
        triangle(&mut mesh, pair[0][0], pair[1][1], pair[1][0]);
    }
    triangle(&mut mesh, pairs[2][0], pairs[2][1], Vec3::new(curve, 1.0, 0.0));
    mesh
}

fn meadow_patch_pivot(group: u32) -> Vec3 {
    Vec3::new(if group & 1 == 0 { -0.1125 } else { 0.1125 }, 0.0,
        if group & 2 == 0 { -0.1125 } else { 0.1125 })
}

const MEADOW_PATCH_CORE: f32 = 0.65;

/// Integral of r * density(r), normalized over the unit disk. Density is one
/// inside the core and 1-smoothstep(core, 1, r) outside it. Integrating area,
/// rather than radius alone, preserves a full centre without a dense ring.
fn meadow_patch_radial_cdf(radius: f32) -> f32 {
    let c = MEADOW_PATCH_CORE;
    let h = 1.0 - c;
    let total = 0.5 * c * c + 0.5 * h * c + 0.15 * h * h;
    let r = radius.clamp(0.0, 1.0);
    if r <= c { return 0.5 * r * r / total; }
    let u = (r - c) / h;
    let u2 = u * u;let u3 = u2 * u;let u4 = u2 * u2;let u5 = u4 * u;
    (0.5 * c * c + h * c * (u - u3 + 0.5 * u4)
        + h * h * (0.5 * u2 - 0.75 * u4 + 0.4 * u5)) / total
}

fn meadow_patch_feather_radius(area_quantile: f32) -> f32 {
    let q = area_quantile.clamp(0.0, 1.0);
    if q == 0.0 || q == 1.0 { return q; }
    let mut lo = 0.0;let mut hi = 1.0;
    // Cook-time only: no per-frame inversion, random deletion or screen fade.
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if meadow_patch_radial_cdf(mid) < q { lo = mid; } else { hi = mid; }
    }
    0.5 * (lo + hi)
}

fn meadow_patch_edge(seed: u64, azimuth: f32) -> f32 {
    0.36 + 0.012 * (3.0 * azimuth + unit(seed, 43, 0) * std::f32::consts::TAU).sin()
        + 0.008 * (5.0 * azimuth + unit(seed, 44, 0) * std::f32::consts::TAU).sin()
}

/// Stratified area quantiles remapped through a smooth radial density CDF.
/// Keep all 360 roots and their spatial quadrants (90 per wind group), but feather
/// the density of actual blades instead of fading a patch's opacity. The
/// nominal .36 m support has small smooth lobes and is bounded by .38 m.
/// Source LODs and the aggregate cook consume these exact same placements.
fn meadow_patch_anchor(seed: u64, index: u32) -> Vec3 {
    let local = (index / 20 % 9) * 10 + index % 10;
    // Each quadrant covers every radial and angular stratum. Starting from
    // bounded square-stratum radii would omit the final 3% of area quantiles
    // and truncate the entire thin outer ring after the inverse CDF.
    let quantile = (local as f32 + 0.5 + (unit(seed, 40, index as u64) - 0.5) * 0.7) / 90.0;
    // 37 and 90 are coprime: every angle stratum occurs once, independently
    // of radial rank, instead of making aligned spokes or concentric rings.
    let angle = (((local * 37) % 90) as f32 + 0.5
        + (unit(seed, 41, index as u64) - 0.5) * 0.7) / 90.0 * std::f32::consts::FRAC_PI_2;
    let direction = Vec3::new(angle.cos() * if index % 20 < 10 { -1.0 } else { 1.0 }, 0.0,
        angle.sin() * if index / 20 < 9 { -1.0 } else { 1.0 });
    let azimuth = direction.z.atan2(direction.x);
    direction * (meadow_patch_feather_radius(quantile) * meadow_patch_edge(seed, azimuth))
}

fn stem(id: u64) -> PartGeometry {
    let mut mesh = PartGeometry { id, vertices: Vec::new(), indices: Vec::new() };
    let ring = |j: usize, y: f32| {
        let angle = j as f32 * std::f32::consts::TAU / 6.0;
        Vec3::new(angle.cos(), 0.0, angle.sin()) * (1.0 - 0.45 * y) + Vec3::Y * y
    };
    for j in 0..6 {
        let k = (j + 1) % 6;
        triangle(&mut mesh, ring(j, 0.0), ring(j, 1.0), ring(k, 1.0));
        triangle(&mut mesh, ring(j, 0.0), ring(k, 1.0), ring(k, 0.0));
        triangle(&mut mesh, Vec3::ZERO, ring(j, 0.0), ring(k, 0.0));
        triangle(&mut mesh, Vec3::Y, ring(k, 1.0), ring(j, 1.0));
    }
    mesh
}

pub fn generate(p: Parameters, limits: Limits) -> Result<Assembly, String> {
    let patch = p.archetype == Archetype::MeadowPatch;
    let max_density = if patch { 360 } else { 128 };
    if !p.growth.is_finite() || !(0.1..=1.5).contains(&p.growth)
        || !p.season.is_finite() || !(0.0..=1.0).contains(&p.season)
        || !(1..=max_density).contains(&p.density) {
        return Err("invalid procedural foliage parameters".into());
    }
    let meadow = p.archetype == Archetype::MeadowGrass;
    let grass = p.archetype != Archetype::ConiferTwig;
    let count = p.density as usize + if grass { 0 } else { 5 };
    if limits.max_parts < 3 || limits.max_branches < 5 || limits.max_instances < count {
        return Err("procedural foliage structure exceeds explicit limits".into());
    }
    let green = if grass { Vec3::new(0.105, 0.18, 0.039) } else { Vec3::new(0.045, 0.10, 0.032) };
    let dry = if grass { Vec3::new(0.24, 0.19, 0.075) } else { Vec3::new(0.072, 0.095, 0.036) };
    let colour = green.lerp(dry, p.season);
    // Shared palette, never a unique random colour/material per blade. The
    // meadow does not compensate for silhouette changes by darkening albedo.
    let materials = if meadow || patch {
        vec![material(colour * Vec3::new(0.94, 1.0, 1.08), 0.9, true),
            material(colour, 0.9, true),
            material(colour * Vec3::new(1.08, 1.03, 0.96), 0.9, true)]
    } else {
        vec![material(colour * 0.88, 0.88, true), material(colour, 0.9, true),
            material(colour * Vec3::new(1.08, 1.05, 0.98), 0.87, true),
            material(Vec3::new(0.105, 0.066, 0.033), 0.94, false)]
    };
    let parts = if patch {
        (0..3).map(meadow_patch_blade).collect()
    } else if meadow {
        (0..3).map(meadow_blade).collect()
    } else if grass {
        (0..3).map(|i| blade(key(0, 300, i), 2.0 + i as f32 * 1.7, 0.075)).collect()
    } else {
        vec![blade(key(0, 400, 0), 0.2, 0.23), blade(key(0, 400, 1), 0.65, 0.23), stem(key(0, 400, 2))]
    };
    let mut assembly = Assembly { parameters: p, parts, materials, limits,
        branches: vec![BranchGroup { id: identity(p, 0, 0), parent: None, rest: Mat4::IDENTITY, stiffness: 1.0 }],
        instances: Vec::with_capacity(count) };
    for group in 0..4 {
        let angle = if meadow { meadow_group_angle(p.seed, group) }
            else { group as f32 * 2.399_963_1 + unit(p.seed, 1, 0) * std::f32::consts::TAU };
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
        let (position, rotation, stiffness) = if patch {
            // Group pivots classify shared wind coefficients. Runtime rotates
            // each ribbon about its own basal anchor, not this whole quadrant.
            (meadow_patch_pivot(group), Quat::IDENTITY, 0.23 + 0.05 * group as f32)
        } else if meadow {
            // Tight basal clusters fill the collar; the outward arches, not
            // widely scattered roots or extra instances, fill the upper tuft.
            (radial * (0.028 + 0.012 * unit(p.seed, 31, group as u64)) * p.growth,
                Quat::IDENTITY, 0.23 + 0.05 * group as f32)
        } else if grass {
            (radial * 0.075 * p.growth, Quat::IDENTITY, 0.22 + 0.05 * group as f32)
        } else {
            (Vec3::Y * (0.13 + group as f32 * 0.14) * p.growth,
                Quat::from_rotation_arc(Vec3::Y, (radial + Vec3::Y * 0.34).normalize()), 0.55 + group as f32 * 0.05)
        };
        assembly.branches.push(BranchGroup { id: identity(p, 0, group as u64 + 1), parent: Some(0),
            rest: Mat4::from_rotation_translation(rotation, position), stiffness });
    }
    if !grass {
        for group in 0..5 {
            let (radius, length) = if group == 0 { (0.014, 0.77) } else { (0.005, 0.30 - 0.025 * (group - 1) as f32) };
            assembly.instances.push(PartInstance { id: identity(p, 1, group as u64), part: 2,
                branch: group, rest: Mat4::from_scale(Vec3::new(radius, length, radius) * p.growth), material: 3 });
        }
    }
    for i in 0..p.density {
        let group = if patch { u32::from(i % 20 >= 10) + 2 * u32::from(i / 20 >= 9) + 1 }
            else { i % 4 + 1 };
        let angle = unit(p.seed, 10, i as u64) * std::f32::consts::TAU;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
        let (position, rotation, size, part) = if patch {
            // One anchor in every mapped stratum, never 360 roots in four
            // collars or a square tile. Keep the footprint fixed under growth;
            // the cooker supplies each
            // anchor separately for root-preserving GPU wind and world yaw.
            let anchor = meadow_patch_anchor(p.seed, i);
            let cohort = key(p.seed, 42, i as u64) % 10;
            let height = if cohort < 5 { 0.12 + 0.08 * unit(p.seed, 11, i as u64) }
                else if cohort < 9 { 0.20 + 0.10 * unit(p.seed, 11, i as u64) }
                else { 0.30 + 0.10 * unit(p.seed, 11, i as u64) };
            let width = 0.006 + 0.008 * unit(p.seed, 12, i as u64);
            (anchor - meadow_patch_pivot(group - 1), Quat::from_rotation_y(-angle),
                Vec3::new(width, height, width) * p.growth, i % 3)
        } else if meadow {
            // Per nine-blade group: two upright, three intermediate and four
            // low outward arches. The long low blades cover ground between
            // tufts instead of concentrating every tip in a tall palm-like fan.
            // The three narrow shared shapes, widths and triangle budget stay.
            let cohort = (i / 4) % 9;
            let middle = cohort >= 2;
            let low = cohort >= 5;
            let height = if low { 0.34 + unit(p.seed, 11, i as u64) * 0.09 }
                else if middle { 0.40 + unit(p.seed, 11, i as u64) * 0.11 }
                else { 0.45 + unit(p.seed, 11, i as u64) * 0.14 };
            let width = 0.028 + unit(p.seed, 12, i as u64) * 0.015;
            let fan_angle = meadow_group_angle(p.seed, group - 1)
                + (unit(p.seed, 10, i as u64) - 0.5) * (if low { 2.5 } else { 2.15 })
                + (unit(p.seed, 34, (group - 1) as u64) - 0.5) * 0.25;
            let fan_radial = Vec3::new(fan_angle.cos(), 0.0, fan_angle.sin());
            let lean = if middle {
                let tip = assembly.parts[(i % 3) as usize].vertices.iter()
                    .max_by(|a, b| a.pos[1].total_cmp(&b.pos[1])).unwrap();
                let tip_x = tip.pos[0] * width;
                let target = if low { 0.09 + 0.05 * unit(p.seed, 14, i as u64) }
                    else { 0.18 + 0.10 * unit(p.seed, 14, i as u64) };
                (height.atan2(tip_x) - (target / height.hypot(tip_x)).clamp(0.0, 0.95).asin()).max(0.0)
            } else { 0.05 + 0.15 * unit(p.seed, 14, i as u64) };
            // Keep collars compact on sloped terrain. Only the upper blade
            // reaches outward; no broad root scatter that would float uphill.
            let collar = 0.006 + (if middle { 0.012 } else { 0.0 }) + unit(p.seed, 13, i as u64) * 0.012;
            let mut position = fan_radial * collar;
            // Rotating the narrow root cap must not put local vertices below
            // the root plane. The scene still applies its normal soil inset.
            position.y = 0.065 * width * lean.sin() + 0.001;
            let rotation = Quat::from_rotation_y(-fan_angle) * Quat::from_rotation_z(-lean);
            (position * p.growth, rotation, Vec3::new(width, height, width) * p.growth, i % 3)
        } else if grass {
            let height = (0.30 + unit(p.seed, 11, i as u64) * 0.30) * p.growth;
            let width = (0.017 + unit(p.seed, 12, i as u64) * 0.014) * p.growth;
            let position = radial * unit(p.seed, 13, i as u64).sqrt() * 0.065 * p.growth;
            (position, Quat::from_rotation_y(-angle), Vec3::new(width, height, width), i % 3)
        } else {
            let length = (0.30 - 0.025 * (group - 1) as f32) * p.growth;
            let position = Vec3::Y * (0.10 + 0.86 * unit(p.seed, 13, i as u64)) * length;
            let direction = (radial + Vec3::Y * (0.20 + unit(p.seed, 14, i as u64) * 0.40)).normalize();
            let size = Vec3::new(0.0045, 0.08 + unit(p.seed, 11, i as u64) * 0.055, 0.0045) * p.growth;
            (position, Quat::from_rotation_arc(Vec3::Y, direction), size, i % 2)
        };
        assembly.instances.push(PartInstance { id: identity(p, 2, i as u64), part, branch: group,
            rest: Mat4::from_scale_rotation_translation(size, rotation, position),
            material: if meadow || patch {
                // Most blades share their basal group's palette. A stable
                // minority follows the next tone, avoiding a striped fan.
                let group_palette = (key(p.seed, 32, group as u64) % 3) as u32;
                (group_palette + u32::from(key(p.seed, 33, i as u64) % 5 == 0)) % 3
            } else { (key(p.seed, 15, i as u64) % 3) as u32 } });
    }
    assembly.validate()?;
    Ok(assembly)
}

/// Cook an explicit lower-cost MeadowPatch while retaining the same world
/// footprint and existing source blade identities. Each wind group/prototype
/// contributes the same count, sampled across all its radial strata. Other
/// recipes and the ordinary `generate` API are unchanged. Limits must permit
/// the reference360 placement cook, which is used only as immutable source.
pub fn generate_meadow_patch_budget(p: Parameters, budget: MeadowBladeBudget, limits: Limits) -> Result<Assembly, String> {
    if p.archetype != Archetype::MeadowPatch || p.density != 360 {
        return Err("meadow budget requires the complete MeadowPatch reference recipe".into());
    }
    let mut assembly = generate(p, limits)?;
    if budget == MeadowBladeBudget::Blades360 { return Ok(assembly); }
    let per_group_part = budget.blades() / 12;
    let stride = 30 / per_group_part;
    let mut keep = [false; 360];
    for group in 1..=4 {
        for part in 0..3 {
            let candidates: Vec<_> = assembly.instances.iter().enumerate()
                .filter(|(_, instance)| instance.branch == group && instance.part == part)
                .map(|(index, _)| index).collect();
            if candidates.len() != 30 { return Err("meadow budget reference distribution changed".into()); }
            for bin in 0..per_group_part {
                let pick = key(p.seed, 901 + u64::from(group * 3 + part), bin as u64) as usize % stride;
                keep[candidates[bin * stride + pick]] = true;
            }
        }
    }
    assembly.instances = assembly.instances.into_iter().enumerate().filter_map(|(i, mut blade)| {
        if !keep[i] { return None; }
        // The ribbon's transverse width is local Z. X is its centreline lean;
        // Y is height. Multiplying both X/Z would falsely enlarge the plant.
        blade.rest.z_axis *= budget.width_multiplier();
        Some(blade)
    }).collect();
    assembly.parameters.density = budget.blades() as u32;
    assembly.validate()?;
    Ok(assembly)
}

fn valid_transform(m: Mat4) -> bool {
    m.is_finite() && m.x_axis.w == 0.0 && m.y_axis.w == 0.0 && m.z_axis.w == 0.0 && m.w_axis.w == 1.0
        && m.determinant().is_finite() && m.determinant() > 1e-12 && m.inverse().is_finite()
}

impl Assembly {
    /// Construct an authored recipe from reusable primitives and placements.
    /// The same validation and resource limits apply as to built-in recipes.
    /// `parameters` stores seed/growth/season provenance; it does not dispatch
    /// generation again. Closed custom leaves use the ConiferTwig optical
    /// contract, while open sheets must use the MeadowPatch contract.
    pub fn from_parts(
        parameters: Parameters,
        parts: Vec<PartGeometry>,
        branches: Vec<BranchGroup>,
        instances: Vec<PartInstance>,
        materials: Vec<Material>,
        limits: Limits,
    ) -> Result<Self, String> {
        if !parameters.growth.is_finite() || !(0.1..=1.5).contains(&parameters.growth)
            || !parameters.season.is_finite() || !(0.0..=1.0).contains(&parameters.season)
            || parameters.density == 0
        { return Err("invalid authored assembly parameters".into()); }
        let assembly = Self { parameters, parts, branches, instances, materials, limits };
        assembly.validate()?;
        Ok(assembly)
    }

    /// Also validates public records after caller edits; no truncation/fallback.
    pub fn validate(&self) -> Result<(), String> {
        if self.parts.len() > self.limits.max_parts || self.branches.is_empty()
            || self.branches.len() > self.limits.max_branches || self.instances.len() > self.limits.max_instances {
            return Err("foliage assembly structure exceeds limits".into());
        }
        let mut ids = BTreeSet::new();
        for (i, group) in self.branches.iter().enumerate() {
            let basis = Mat3::from_mat4(group.rest);
            let rigid = (basis.transpose() * basis - Mat3::IDENTITY).to_cols_array().iter().all(|x| x.abs() < 1e-4);
            if !ids.insert(group.id) || !valid_transform(group.rest) || !group.stiffness.is_finite()
                || !rigid
                || !(0.0..=1.0).contains(&group.stiffness)
                || (i == 0 && (group.parent.is_some() || group.rest != Mat4::IDENTITY))
                || (i > 0 && !group.parent.is_some_and(|p| (p as usize) < i)) {
                return Err("invalid foliage branch hierarchy, pivot or identity".into());
            }
        }
        ids.clear();
        for part in &self.parts {
            if !ids.insert(part.id) || part.indices.is_empty() || part.indices.len() % 3 != 0 {
                return Err("invalid foliage geometry identity/triangles".into());
            }
            for vertex in &part.vertices {
                let normal = Vec3::from(vertex.normal);
                if !Vec3::from(vertex.pos).is_finite() || !normal.is_finite() || (normal.length_squared() - 1.0).abs() > 1e-3
                    || vertex.uv.iter().any(|x| !x.is_finite()) { return Err("invalid foliage geometry vertex".into()); }
            }
            for tri in part.indices.chunks_exact(3) {
                if tri.iter().any(|&i| i as usize >= part.vertices.len()) { return Err("foliage geometry index outside source".into()); }
                let [a, b, c] = std::array::from_fn(|i| Vec3::from(part.vertices[tri[i] as usize].pos));
                let area = (b - a).cross(c - a).length_squared();
                if !area.is_finite() || area <= 1e-16 { return Err("degenerate foliage source triangle".into()); }
            }
        }
        ids.clear();
        let mut vertices = 0usize; let mut indices = 0usize;
        for instance in &self.instances {
            let part = self.parts.get(instance.part as usize).ok_or("foliage part reference outside source")?;
            if instance.branch as usize >= self.branches.len() || !ids.insert(instance.id) || !valid_transform(instance.rest)
                || instance.material as usize >= self.materials.len() { return Err("invalid foliage part instance".into()); }
            vertices = vertices.checked_add(part.vertices.len()).ok_or("foliage vertex count overflow")?;
            indices = indices.checked_add(part.indices.len()).ok_or("foliage index count overflow")?;
        }
        let bytes = vertices.checked_mul(std::mem::size_of::<Vertex>()).and_then(|v|
            indices.checked_mul(4).and_then(|i| v.checked_add(i))).ok_or("foliage source byte count overflow")?;
        if vertices > self.limits.max_vertices || indices > self.limits.max_indices || bytes > self.limits.max_source_bytes {
            return Err("expanded foliage source exceeds explicit geometry/memory limits".into());
        }
        for material in &self.materials {
            let bark = material.layer == crate::scene::BARK_LAYER && material.flags & THIN_FOLIAGE == 0;
            if (material.layer != -1 && !bark) || material.flags & !(TWO_SIDED | THIN_FOLIAGE) != 0 || material.metal != 0.0
                || !material.roughness.is_finite() || !(0.0..=1.0).contains(&material.roughness)
                || material.tint.iter().any(|x| !x.is_finite() || !(0.0..=1.0).contains(x)) {
                return Err("procedural foliage requires finite opaque untextured materials".into());
            }
        }
        Ok(())
    }

    pub fn branch_transforms(&self) -> Result<Vec<Mat4>, String> {
        self.validate()?;
        let mut transforms = Vec::with_capacity(self.branches.len());
        for branch in &self.branches {
            let matrix = branch.parent.map_or(branch.rest, |p| transforms[p as usize] * branch.rest);
            if !valid_transform(matrix) { return Err("foliage branch world transform overflow".into()); }
            transforms.push(matrix);
        }
        Ok(transforms)
    }

    fn flatten(&self, branch: Option<u32>) -> Result<Vec<SourceMesh>, String> {
        let transforms = self.branch_transforms()?;
        if branch.is_some_and(|b| b as usize >= self.branches.len()) { return Err("foliage branch outside assembly".into()); }
        let mut meshes = Vec::new();
        for instance in &self.instances {
            if branch.is_some_and(|b| b != instance.branch) { continue; }
            let source = &self.parts[instance.part as usize];
            let matrix = if branch.is_some() { instance.rest } else { transforms[instance.branch as usize] * instance.rest };
            let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
            let vertices: Vec<_> = source.vertices.iter().map(|v| Vertex {
                pos: matrix.transform_point3(Vec3::from(v.pos)).to_array(),
                normal: (normal_matrix * Vec3::from(v.normal)).normalize().to_array(), uv: v.uv,
            }).collect();
            if vertices.iter().any(|v| !Vec3::from(v.pos).is_finite() || !Vec3::from(v.normal).is_finite()) {
                return Err("foliage transformed vertex overflow".into());
            }
            meshes.push(SourceMesh { identity: SourceIdentity { instance: instance.id, part: source.id,
                branch: self.branches[instance.branch as usize].id }, vertices,
                indices: source.indices.clone(), material: instance.material });
        }
        Ok(meshes)
    }

    /// Assembly-local source, suitable for source rendering or one static bake.
    pub fn flatten_rest(&self) -> Result<Vec<SourceMesh>, String> { self.flatten(None) }
    /// Only this group's geometry, in its pivot-local coordinates. Descendant
    /// groups remain separate so their future motion is not baked twice.
    pub fn flatten_branch(&self, branch: u32) -> Result<Vec<SourceMesh>, String> { self.flatten(Some(branch)) }

    /// Exact rest bounds without expanding shared geometry or copying indices.
    /// Retain flatten_rest's transformed-normal checks as well as its positions:
    /// a finite AABB must not hide an invalid source transform.
    pub fn bounds(&self) -> Result<[[f32; 3]; 2], String> {
        let transforms = self.branch_transforms()?;
        let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for instance in &self.instances {
            let matrix = transforms[instance.branch as usize] * instance.rest;
            let normal_matrix = Mat3::from_mat4(matrix).inverse().transpose();
            for vertex in &self.parts[instance.part as usize].vertices {
                let p = matrix.transform_point3(Vec3::from(vertex.pos));
                let normal = (normal_matrix * Vec3::from(vertex.normal)).normalize();
                if !p.is_finite() || !normal.is_finite() {
                    return Err("foliage transformed vertex overflow".into());
                }
                lo = lo.min(p); hi = hi.max(p);
            }
        }
        if !lo.is_finite() { return Err("foliage assembly has no source geometry".into()); }
        Ok([lo.to_array(), hi.to_array()])
    }

    /// Geometric optical measure, not a brightness correction. Dense meadow
    /// ribbons are open sheets; the other authored recipes enclose solids.
    pub fn projection_measure(&self) -> foliage_voxel::ProjectionMeasure {
        if self.parameters.archetype == Archetype::MeadowPatch { foliage_voxel::ProjectionMeasure::SheetCrossings }
        else { foliage_voxel::ProjectionMeasure::ClosedSurfaceEntries }
    }

    fn validate_projection(&self, config: &BakeConfig) -> Result<(), String> {
        if self.parameters.archetype == Archetype::MeadowPatch
            && config.projection_measure != foliage_voxel::ProjectionMeasure::SheetCrossings {
            return Err("MeadowPatch has open ribbons and requires SheetCrossings, not ClosedSurfaceEntries".into());
        }
        Ok(())
    }

    pub fn bake_rest(&self, config: BakeConfig) -> Result<VoxelAsset, String> {
        self.validate_projection(&config)?;
        let meshes = self.flatten_rest()?;
        foliage_voxel::bake(&meshes.iter().map(SourceMesh::as_part).collect::<Vec<_>>(), &self.materials, &[], config)
    }

    /// Budgets are shared across all returned branches, not silently multiplied
    /// by the group count. Use projection_measure() for the source contract:
    /// MeadowPatch requires SheetCrossings; the older recipes are closed.
    /// A voxel medium still approximates correlated opaque leaf visibility.
    pub fn bake_branches(&self, mut config: BakeConfig) -> Result<Vec<BakedBranch>, String> {
        self.validate_projection(&config)?;
        let transforms = self.branch_transforms()?;
        let mut output = Vec::new();
        for (index, branch) in self.branches.iter().enumerate() {
            let meshes = self.flatten_branch(index as u32)?;
            if meshes.is_empty() { continue; }
            let asset = foliage_voxel::bake(&meshes.iter().map(SourceMesh::as_part).collect::<Vec<_>>(), &self.materials, &[], config)?;
            config.max_samples -= asset.stats.samples;
            config.max_voxels -= asset.levels.iter().map(|level| level.voxels.len()).sum::<usize>();
            config.max_texture_texels -= asset.stats.prepared_texture_texels;
            output.push(BakedBranch { branch: index as u32, branch_id: branch.id, rest_to_assembly: transforms[index],
                asset, source_ids: meshes.iter().map(|mesh| mesh.identity.instance).collect() });
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> BakeConfig { BakeConfig { projection_measure:foliage_voxel::ProjectionMeasure::ClosedSurfaceEntries,
        cell_size: 0.08, levels: 3, samples_per_cell: 2,
        max_samples: 1_000_000, max_voxels: 32_768, max_texture_texels: 0, ..Default::default() } }

    #[test]
    fn meadow_blade_budget_preserves_anchors_height_lean_and_balanced_source_groups() {
        for seed in [2, 73, 707] {
            let parameters = Parameters::meadow_patch(seed);
            let reference = generate(parameters, Limits::meadow_patch()).unwrap();
            for budget in [MeadowBladeBudget::Blades120, MeadowBladeBudget::Blades180, MeadowBladeBudget::Blades360] {
                let patch = generate_meadow_patch_budget(parameters, budget, Limits::meadow_patch()).unwrap();
                let repeat = generate_meadow_patch_budget(parameters, budget, Limits::meadow_patch()).unwrap();
                assert_eq!(patch.instances.len(), budget.blades());
                assert_eq!(patch.parameters.density as usize, budget.blades());
                assert_eq!(patch.branches.len(), 5);
                let mut distribution = [[0; 3]; 4];
                let poses = patch.branch_transforms().unwrap();
                for (blade, again) in patch.instances.iter().zip(&repeat.instances) {
                    let old = reference.instances.iter().find(|v| v.id == blade.id).unwrap();
                    assert_eq!((blade.id,blade.part,blade.branch,blade.material),
                        (again.id,again.part,again.branch,again.material));
                    assert_eq!(blade.rest.to_cols_array(), again.rest.to_cols_array());
                    assert_eq!((blade.part,blade.branch,blade.material),(old.part,old.branch,old.material));
                    assert_eq!(blade.rest.x_axis, old.rest.x_axis, "lean must not grow with width");
                    assert_eq!(blade.rest.y_axis, old.rest.y_axis, "height must not grow with width");
                    assert_eq!(blade.rest.w_axis, old.rest.w_axis, "basal anchors must stay fixed");
                    assert_eq!(blade.rest.z_axis, old.rest.z_axis * budget.width_multiplier());
                    distribution[blade.branch as usize-1][blade.part as usize] += 1;
                    let transform = poses[blade.branch as usize] * blade.rest;
                    let root = transform.transform_point3(Vec3::ZERO);
                    assert_eq!(root.y, 0.0);assert!(root.length()<0.38);
                    for vertex in &patch.parts[blade.part as usize].vertices {
                        let p = transform.transform_point3(Vec3::from(vertex.pos));
                        assert!(p.is_finite() && p.y>=0.0 && p.y<=0.400001);
                        assert!(p.x.abs()<0.50 && p.z.abs()<0.50);
                    }
                }
                assert_eq!(distribution, [[budget.blades()/12;3];4]);
                for (old,new) in reference.parts.iter().zip(&patch.parts) {
                    assert_eq!(old.indices,new.indices);
                    assert_eq!(bytemuck::cast_slice::<_,u8>(&old.vertices),bytemuck::cast_slice::<_,u8>(&new.vertices));
                }
                if budget == MeadowBladeBudget::Blades360 {
                    assert!(reference.instances.iter().zip(&patch.instances).all(|(a,b)|
                        a.id==b.id && a.rest.to_cols_array()==b.rest.to_cols_array()));
                }
            }
        }
    }

    #[test]
    fn meadow_blade_budget_rejects_ambiguous_recipe_and_invalid_counts() {
        for count in [0,1,60,119,121,179,181,359,361] { assert!(MeadowBladeBudget::try_from(count).is_err()); }
        for count in [120,180,360] { assert_eq!(MeadowBladeBudget::try_from(count).unwrap().blades(),count as usize); }
        assert!(generate_meadow_patch_budget(Parameters::grass(0),MeadowBladeBudget::Blades120,Limits::meadow_patch()).is_err());
        assert!(generate_meadow_patch_budget(Parameters {density:120,..Parameters::meadow_patch(0)},
            MeadowBladeBudget::Blades120,Limits::meadow_patch()).is_err());
    }

    #[test]
    fn meadow_patch_distributes_360_real_anchors_in_an_overlapping_rounded_footprint() {
        for seed in [2, 73, 707] {
            let patch = generate(Parameters::meadow_patch(seed), Limits::meadow_patch()).unwrap();
            assert_eq!(patch.instances.len(), 360);
            assert_eq!(patch.parts.len(), 3);
            assert_eq!(patch.branches.len(), 5);
            assert!(patch.parts.iter().all(|part| part.indices.len() == 15));
            assert_eq!(patch.instances.iter().map(|p| patch.parts[p.part as usize].indices.len() / 3).sum::<usize>(), 1800);
            let poses = patch.branch_transforms().unwrap();
            let mut anchors = BTreeSet::new();let mut groups = [0; 4];let mut shapes = [0; 3];
            let mut lo = Vec3::splat(f32::INFINITY);let mut hi = Vec3::splat(f32::NEG_INFINITY);
            let mut overlapping = 0;
            for blade in &patch.instances {
                let transform = poses[blade.branch as usize] * blade.rest;
                let anchor = transform.transform_point3(Vec3::ZERO);
                assert_eq!(anchor.y, 0.0);
                assert!(anchor.length() < 0.38);
                assert!(anchors.insert(anchor.to_array().map(f32::to_bits)),
                    "mapping must preserve every distinct basal anchor");
                assert_eq!(blade.branch - 1,
                    u32::from(anchor.x > 0.0) + 2 * u32::from(anchor.z > 0.0),
                    "wind groups retain their spatial quadrant");
                overlapping += usize::from(anchor.x.abs() > 0.225 || anchor.z.abs() > 0.225);
                groups[blade.branch as usize - 1] += 1;shapes[blade.part as usize] += 1;
                lo = lo.min(anchor);hi = hi.max(anchor);
                let width = transform.transform_point3(Vec3::Z * 0.5)
                    .distance(transform.transform_point3(-Vec3::Z * 0.5));
                assert!((0.005999..=0.014001).contains(&width));
            }
            assert_eq!(groups, [90; 4]);assert_eq!(shapes, [120; 3]);
            assert!(lo.x < -0.25 && lo.z < -0.25 && hi.x > 0.25 && hi.z > 0.25,
                "rounded roots must overlap neighbouring cells, not shrink into isolated tufts");
            assert!(overlapping >= 40, "retain a substantial transition across the old square border");
            let bounds = patch.bounds().unwrap();
            assert_eq!(bounds[0][1], 0.0);
            assert!(bounds[1][1] <= 0.400001 && bounds[1][1] >= 0.30);
            // The existing blade's tip lean remains <=6*.014=.084 m. The
            // source cooker rebuilds exact bounds rather than assuming .45 m.
            assert!(bounds[0][0] > -0.465 && bounds[0][2] > -0.465
                && bounds[1][0] < 0.465 && bounds[1][2] < 0.465);
        }
    }

    #[test]
    fn meadow_patch_density_cdf_matches_integrated_smooth_feather() {
        // Independent midpoint quadrature of the intended area density checks
        // the polynomial and inverse, including the flat core and thin edge.
        let density = |r: f64| {
            let t = ((r - 0.65) / 0.35).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
        let integrate = |end: f64| (0..4096).map(|i| {
            let r = (i as f64 + 0.5) * end / 4096.0;
            r * density(r) * end / 4096.0
        }).sum::<f64>();
        let total = integrate(1.0);
        for q in [0.0, 0.01, 0.25, 0.5, 0.75, 0.9, 0.99, 1.0] {
            let r = meadow_patch_feather_radius(q);
            assert!((integrate(r as f64) / total - q as f64).abs() < 2e-6,
                "inverse area CDF must retain the intended density: q={q}, radius={r}");
        }
        assert_eq!(meadow_patch_feather_radius(0.0), 0.0);
        assert_eq!(meadow_patch_feather_radius(1.0), 1.0);
    }

    #[test]
    fn meadow_patch_actual_anchor_density_declines_toward_its_edge() {
        // Measure real generated placements per unit annulus area over seeds;
        // no blade is dropped, and density cannot form an opaque outer ring.
        let edges = [0.0f32, 0.65, 0.80, 0.90, 1.0];
        let mut counts = [0u32; 4];
        for seed in 0..64 {
            let mut strata: [BTreeSet<u32>; 4] = std::array::from_fn(|_| BTreeSet::new());
            for index in 0..360 {
                let anchor = meadow_patch_anchor(seed, index);
                let radius = anchor.length() / meadow_patch_edge(seed, anchor.z.atan2(anchor.x));
                let ring = (0..4).find(|&i| radius <= edges[i + 1]).unwrap();
                counts[ring] += 1;
                let group = usize::from(anchor.x > 0.0) + 2 * usize::from(anchor.z > 0.0);
                let stratum = (meadow_patch_radial_cdf(radius) * 90.0).floor() as u32;
                assert!(strata[group].insert(stratum), "every area stratum must occur once per quadrant");
            }
            for group in strata { assert_eq!(group, (0..90).collect()); }
        }
        assert_eq!(counts.iter().sum::<u32>(), 64 * 360);
        let density: [f32; 4] = std::array::from_fn(|i|
            counts[i] as f32 / (edges[i + 1].powi(2) - edges[i].powi(2)));
        assert!(density.windows(2).all(|pair| pair[0] > pair[1]));
        assert!(density[3] < density[0] * 0.15, "the outer edge must be feathered in source geometry");
        assert!(counts[3] > 0, "the support must retain actual overlapping edge blades");
    }

    #[test]
    fn meadow_patch_neighbours_do_not_turn_square_holes_into_round_holes() {
        // Independent spatial probe of actual anchors, including arbitrary
        // patch yaw and the generator's bounded .018 m per-axis centre jitter.
        // This measures basal spacing, not opaque screen coverage: leaves and
        // their wind remain the separate geometric/visual oracle.
        for seed in [2u64, 17, 73, 707] {
            let mut roots = Vec::new();
            for z in -1..=1 {
                for x in -1..=1 {
                    let cell = ((z + 1) * 3 + x + 1) as u64;
                    let source_seed = seed + [101, 303, 707][cell as usize % 3];
                    let patch = generate(Parameters::meadow_patch(source_seed), Limits::meadow_patch()).unwrap();
                    let poses = patch.branch_transforms().unwrap();
                    let yaw = Quat::from_rotation_y(unit(seed, 50, cell) * std::f32::consts::TAU);
                    let centre = Vec3::new(x as f32 * 0.45 + (unit(seed, 51, cell) - 0.5) * 0.036,
                        0.0, z as f32 * 0.45 + (unit(seed, 52, cell) - 0.5) * 0.036);
                    roots.extend(patch.instances.iter().map(|blade| {
                        let anchor = (poses[blade.branch as usize] * blade.rest).transform_point3(Vec3::ZERO);
                        centre + yaw * anchor
                    }));
                }
            }
            let mut worst = 0.0f32;
            let mut close = 0;
            for z in 0..=32 {
                for x in 0..=32 {
                    let point = Vec3::new((x as f32 / 32.0 - 0.5) * 0.45, 0.0,
                        (z as f32 / 32.0 - 0.5) * 0.45);
                    let nearest = roots.iter().map(|root| root.distance(point))
                        .fold(f32::INFINITY, f32::min);
                    worst = worst.max(nearest);
                    close += usize::from(nearest <= 0.03);
                }
            }
            assert!(worst < 0.045, "no missing-patch-sized basal gaps: seed={seed}, max={worst}");
            assert!(close as f32 / (33.0 * 33.0) > 0.97,
                "at least 97% of sampled interior ground must remain within 3 cm of a blade root");
        }
    }

    #[test]
    fn meadow_patch_ribbons_have_real_normals_open_edges_and_exact_root_midpoints() {
        let patch = generate(Parameters::meadow_patch(73), Limits::meadow_patch()).unwrap();
        assert!(patch.materials.iter().all(|m| m.flags == (THIN_FOLIAGE | TWO_SIDED) && m.layer == -1));
        for part in &patch.parts {
            let stations: BTreeSet<_> = part.vertices.iter().map(|v| v.pos[1].to_bits()).collect();
            assert_eq!(stations, [0.0f32, 0.45, 0.78, 1.0].map(f32::to_bits).into_iter().collect());
            let roots: BTreeSet<_> = part.vertices.iter().filter(|v| v.pos[1] == 0.0)
                .map(|v| v.pos.map(f32::to_bits)).collect();
            assert_eq!(roots.len(), 2);
            let root_sum: Vec3 = roots.iter().map(|p| Vec3::from(p.map(f32::from_bits))).sum();
            assert_eq!(root_sum, Vec3::ZERO);
            assert!(foliage_voxel::validate_closed_part(&Part {
                vertices: &part.vertices, indices: &part.indices, material: 0 }).is_err());
        }
        for mesh in patch.flatten_rest().unwrap() {
            for tri in mesh.indices.chunks_exact(3) {
                let [a, b, c] = std::array::from_fn(|i| Vec3::from(mesh.vertices[tri[i] as usize].pos));
                let cross = (b - a).cross(c - a);
                assert!(cross.length_squared() > 1e-16);
                for &i in tri { assert!(cross.normalize().dot(Vec3::from(mesh.vertices[i as usize].normal)) > 0.9999); }
            }
        }
    }

    #[test]
    fn meadow_patch_keeps_shared_geometry_determinism_and_growth_anchored() {
        let full = generate(Parameters::meadow_patch(17), Limits::meadow_patch()).unwrap();
        let repeat = generate(Parameters::meadow_patch(17), Limits::meadow_patch()).unwrap();
        let other = generate(Parameters::meadow_patch(91), Limits::meadow_patch()).unwrap();
        let young = generate(Parameters { growth: 0.5, season: 1.0, ..Parameters::meadow_patch(17) }, Limits::meadow_patch()).unwrap();
        for (part, shared) in full.parts.iter().zip(&other.parts) {
            assert_eq!(part.id, shared.id);assert_eq!(part.indices, shared.indices);
            assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&part.vertices), bytemuck::cast_slice::<Vertex, u8>(&shared.vertices));
        }
        let poses = full.branch_transforms().unwrap();let young_poses = young.branch_transforms().unwrap();
        let full_mesh = full.flatten_rest().unwrap();let young_mesh = young.flatten_rest().unwrap();
        for (index, ((a, b), c)) in full.instances.iter().zip(&repeat.instances).zip(&young.instances).enumerate() {
            assert_eq!(a.id, b.id);assert_eq!(a.id, c.id);assert_eq!(a.rest.to_cols_array(), b.rest.to_cols_array());
            let anchor = (poses[a.branch as usize] * a.rest).transform_point3(Vec3::ZERO);
            assert_eq!(anchor, (young_poses[c.branch as usize] * c.rest).transform_point3(Vec3::ZERO));
            for (v, w) in full_mesh[index].vertices.iter().zip(&young_mesh[index].vertices) {
                assert!((anchor + (Vec3::from(v.pos) - anchor) * 0.5 - Vec3::from(w.pos)).length() < 1e-6);
            }
        }
        assert_ne!(full.materials[1].tint, young.materials[1].tint);
        assert!(full.instances.iter().zip(&other.instances).any(|(a, b)| a.rest != b.rest));
    }

    #[test]
    fn meadow_patch_has_explicit_limits_and_open_sheet_bake_semantics() {
        assert!(generate(Parameters::meadow_patch(0), Limits::default()).is_err());
        assert!(generate(Parameters { density: 361, ..Parameters::meadow_patch(0) },
            Limits { max_instances: 512, ..Limits::default() }).is_err());
        for original in [Parameters::grass(0), Parameters::meadow_grass(0), Parameters::conifer_twig(0)] {
            assert!(generate(Parameters { density: 360, ..original }, Limits::meadow_patch()).is_err());
        }
        let patch = generate(Parameters { density: 1, ..Parameters::meadow_patch(0) }, Limits::meadow_patch()).unwrap();
        assert_eq!(patch.projection_measure(), foliage_voxel::ProjectionMeasure::SheetCrossings);
        assert!(patch.bake_rest(config()).unwrap_err().contains("SheetCrossings"));
        assert!(patch.bake_branches(config()).unwrap_err().contains("SheetCrossings"));
        let asset = patch.bake_rest(BakeConfig { projection_measure: patch.projection_measure(), ..config() }).unwrap();
        assert_eq!(asset.stats.closed_parts_checked, 0);
        assert_eq!(asset.stats.degenerate_triangles, 0);
        assert!(!asset.levels[0].voxels.is_empty());
    }

    #[test]
    fn opaque_sources_are_reused_deterministic_and_finite() {
        for p in [Parameters::grass(73), Parameters::conifer_twig(73), Parameters::meadow_grass(73)] {
            let a = generate(p, Limits::default()).unwrap();
            let b = generate(p, Limits::default()).unwrap();
            assert_eq!(a.parts.len(), 3);
            assert!(a.instances.len() > a.parts.len() * 8);
            assert!(a.materials.iter().all(|m| m.layer == -1 && m.flags & !(TWO_SIDED | THIN_FOLIAGE) == 0));
            let ma = a.flatten_rest().unwrap(); let mb = b.flatten_rest().unwrap();
            for (x, y) in ma.iter().zip(mb.iter()) {
                assert_eq!(x.identity, y.identity);
                assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&x.vertices), bytemuck::cast_slice::<Vertex, u8>(&y.vertices));
                for tri in x.indices.chunks_exact(3) {
                    let [a, b, c] = std::array::from_fn(|i| Vec3::from(x.vertices[tri[i] as usize].pos));
                    let normal = (b - a).cross(c - a).normalize();
                    for &i in tri { assert!(normal.dot(Vec3::from(x.vertices[i as usize].normal)) > 0.999); }
                }
            }
            let changed = generate(Parameters { seed: 74, ..p }, Limits::default()).unwrap();
            assert_ne!(a.bounds().unwrap(), changed.bounds().unwrap());
        }
    }

    #[test]
    fn blades_are_closed_outward_geometry_with_inclined_normals() {
        for p in [Parameters::grass(9), Parameters::conifer_twig(9)] {
            let a = generate(p, Limits::default()).unwrap();
            for part in &a.parts {
                let mut edges = std::collections::BTreeMap::new();
                let mut signed_volume = 0.0f64;
                for tri in part.indices.chunks_exact(3) {
                    let points: [Vec3; 3] = std::array::from_fn(|i| Vec3::from(part.vertices[tri[i] as usize].pos));
                    signed_volume += points[0].as_dvec3().dot(points[1].as_dvec3().cross(points[2].as_dvec3())) / 6.0;
                    for i in 0..3 {
                        let a = points[i].to_array().map(f32::to_bits);
                        let b = points[(i + 1) % 3].to_array().map(f32::to_bits);
                        let (key, direction) = if a < b { ((a, b), 1i32) } else { ((b, a), -1) };
                        let entry = edges.entry(key).or_insert((0u32, 0i32));
                        entry.0 += 1; entry.1 += direction;
                    }
                }
                assert!(signed_volume > 1e-6, "source must enclose positive outward volume");
                assert!(edges.values().all(|&e| e == (2, 0)), "each closed-mesh edge needs two opposite incidences");
            }
            let mut projected_up = 0.0f64; let mut area = 0.0f64;
            for mesh in a.flatten_rest().unwrap().iter().filter(|m| m.material != 3) {
                for tri in mesh.indices.chunks_exact(3) {
                    let [a, b, c] = std::array::from_fn(|i| Vec3::from(mesh.vertices[tri[i] as usize].pos));
                    let cross = (b - a).cross(c - a);
                    area += cross.length() as f64;
                    projected_up += cross.length() as f64 * cross.normalize().y.powi(2) as f64;
                }
            }
            assert!(projected_up / area < 0.5, "blade normals must describe their surface, not default world-up");
        }
    }

    #[test]
    fn generated_blades_needles_and_bark_are_closed_after_growth_and_branch_transforms() {
        for base in [Parameters::grass(19),Parameters::conifer_twig(27),Parameters::meadow_grass(19)] {
            for growth in [0.1,0.5,1.5] {
                let assembly=generate(Parameters {growth,season:0.8,density:128,..base},Limits::default()).unwrap();
                for mesh in assembly.flatten_rest().unwrap() {
                    assert_eq!(foliage_voxel::validate_closed_part(&mesh.as_part()).unwrap(),1);
                }
                for branch in &assembly.branches {
                    let index=assembly.branches.iter().position(|b|b.id==branch.id).unwrap() as u32;
                    for mesh in assembly.flatten_branch(index).unwrap() {
                        assert_eq!(foliage_voxel::validate_closed_part(&mesh.as_part()).unwrap(),1);
                    }
                }
            }
        }
    }

    #[test]
    fn growth_and_season_keep_identity_and_root() {
        for p in [Parameters::grass(17), Parameters::conifer_twig(17), Parameters::meadow_grass(17)] {
            let full = generate(p, Limits::default()).unwrap();
            let young = generate(Parameters { growth: 0.5, season: 1.0, ..p }, Limits::default()).unwrap();
            assert_eq!(young.branches[0].rest, Mat4::IDENTITY);
            for (a, b) in full.flatten_rest().unwrap().iter().zip(young.flatten_rest().unwrap().iter()) {
                assert_eq!(a.identity, b.identity);
                for (va, vb) in a.vertices.iter().zip(&b.vertices) {
                    assert!((Vec3::from(va.pos) * 0.5 - Vec3::from(vb.pos)).length() < 1e-6);
                }
            }
            assert_ne!(full.materials[1].tint, young.materials[1].tint);
        }
    }

    fn expanded_bounds_oracle(assembly: &Assembly) -> Result<[[f32; 3]; 2], String> {
        let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for mesh in assembly.flatten_rest()? { for vertex in mesh.vertices {
            let p = Vec3::from(vertex.pos); lo = lo.min(p); hi = hi.max(p);
        }}
        if !lo.is_finite() { return Err("foliage assembly has no source geometry".into()); }
        Ok([lo.to_array(), hi.to_array()])
    }

    #[test]
    fn shared_bounds_equal_expanded_oracle_after_growth_and_nested_rotations() {
        for base in [Parameters::grass(17), Parameters::conifer_twig(29), Parameters::meadow_grass(41),
            Parameters::meadow_patch(73)] {
            for growth in [0.1, 0.5, 1.5] {
                let mut assembly = generate(Parameters { growth, ..base }, Limits::meadow_patch()).unwrap();
                // Turn the ordinary root's four child groups into a deep rig.
                // Independent rotations exercise full parent composition, while
                // growth and nonuniform part scales remain in the source placements.
                for index in 1..assembly.branches.len() {
                    let branch = &mut assembly.branches[index];
                    branch.parent = Some(index as u32 - 1);
                    branch.rest = Mat4::from_rotation_translation(
                        Quat::from_rotation_x(0.17 * index as f32)
                            * Quat::from_rotation_z(-0.23 * index as f32),
                        Vec3::new(0.07 * index as f32, 0.11, -0.03) * growth) * branch.rest;
                }
                assert_eq!(assembly.bounds().unwrap().map(|v| v.map(f32::to_bits)),
                    expanded_bounds_oracle(&assembly).unwrap().map(|v| v.map(f32::to_bits)));
            }
        }
    }

    #[test]
    fn shared_bounds_preserve_expanded_oracle_validation_errors() {
        let base = generate(Parameters::grass(17), Limits::default()).unwrap();
        let mut invalid = base.clone();
        invalid.branches[1].parent = Some(1);
        assert_eq!(invalid.bounds(), expanded_bounds_oracle(&invalid));
        let mut over_budget = base.clone();
        over_budget.limits.max_source_bytes = 1;
        assert_eq!(over_budget.bounds(), expanded_bounds_oracle(&over_budget));
        let mut empty = base.clone();
        empty.instances.clear();
        assert_eq!(empty.bounds().unwrap_err(), "foliage assembly has no source geometry");
        assert_eq!(empty.bounds(), expanded_bounds_oracle(&empty));

        // Both local transforms are valid; their composition overflows only
        // when an instance is transformed, after branch validation succeeds.
        let mut overflow = base.clone();
        overflow.instances.truncate(1);
        overflow.instances[0].branch = 1;
        let shift = Vec3::X * (f32::MAX * 0.75);
        overflow.branches[1].rest = Mat4::from_translation(shift);
        overflow.instances[0].rest = Mat4::from_translation(shift);
        assert!(overflow.branch_transforms().is_ok());
        assert_eq!(overflow.bounds().unwrap_err(), "foliage transformed vertex overflow");
        assert_eq!(overflow.bounds(), expanded_bounds_oracle(&overflow));

        // Finite positions alone are insufficient: an extreme positive scale
        // can underflow a transformed normal's squared length before normalization.
        let mut invalid_normal = base;
        invalid_normal.instances.truncate(1);
        invalid_normal.instances[0].rest = Mat4::from_scale(Vec3::new(1e24, 1e-12, 1e-12));
        let part = invalid_normal.instances[0].part as usize;
        for vertex in &mut invalid_normal.parts[part].vertices { vertex.normal = Vec3::X.to_array(); }
        assert!(invalid_normal.branch_transforms().is_ok());
        assert_eq!(invalid_normal.bounds().unwrap_err(), "foliage transformed vertex overflow");
        assert_eq!(invalid_normal.bounds(), expanded_bounds_oracle(&invalid_normal));
    }

    #[test]
    fn branch_local_source_reconstructs_exact_rest_and_bake_mass() {
        for p in [Parameters { density: 12, ..Parameters::grass(2) }, Parameters { density: 16, ..Parameters::conifer_twig(2) },
            Parameters { density: 12, ..Parameters::meadow_grass(2) }] {
            let a = generate(p, Limits::default()).unwrap();
            let world = a.flatten_rest().unwrap(); let transforms = a.branch_transforms().unwrap();
            for (index, transform) in transforms.iter().enumerate() {
                for mesh in a.flatten_branch(index as u32).unwrap() {
                    let source = world.iter().find(|m| m.identity == mesh.identity).unwrap();
                    for (local, rest) in mesh.vertices.iter().zip(&source.vertices) {
                        assert!((transform.transform_point3(Vec3::from(local.pos)) - Vec3::from(rest.pos)).length() < 1e-6);
                    }
                }
            }
            let direct = a.bake_rest(config()).unwrap(); let branches = a.bake_branches(config()).unwrap();
            assert_eq!(direct.stats.degenerate_triangles, 0);
            assert_eq!(direct.stats.prepared_texture_texels, 0);
            let direct_area: f32 = direct.levels[0].voxels.iter().map(|v| v.surface_area).sum();
            let branch_area: f32 = branches.iter().map(|b| b.asset.levels[0].voxels.iter().map(|v| v.surface_area).sum::<f32>()).sum();
            assert!((direct_area - branch_area).abs() < direct_area * 1e-4);
            let ids: BTreeSet<_> = branches.iter().flat_map(|b| b.source_ids.iter().copied()).collect();
            assert_eq!(ids.len(), a.instances.len());
            assert_eq!(ids, a.instances.iter().map(|i| i.id).collect());
            for branch in branches { for level in &branch.asset.levels {
                let area: f32 = level.voxels.iter().map(|v| v.surface_area).sum();
                let fine: f32 = branch.asset.levels[0].voxels.iter().map(|v| v.surface_area).sum();
                assert!((area - fine).abs() < fine * 1e-4);
            }}
        }
    }

    #[test]
    fn meadow_keeps_explicit_reference_budget_and_reusable_source_lods() {
        let reference = generate(Parameters::grass(73), Limits::default()).unwrap();
        let meadow = generate(Parameters::meadow_grass(73), Limits::default()).unwrap();
        assert_eq!(reference.instances.len(), 36);
        assert_eq!(meadow.instances.len(), reference.instances.len());
        assert_eq!(meadow.branches.len(), 5);
        assert_eq!(meadow.parts.len(), 3);
        assert_eq!(meadow.materials.len(), 3);
        let triangles = |assembly: &Assembly| assembly.instances.iter().map(|instance|
            assembly.parts[instance.part as usize].indices.len() / 3).sum::<usize>();
        assert_eq!(triangles(&reference), 792);
        assert_eq!(triangles(&meadow), triangles(&reference));
        assert_eq!(meadow.parts.iter().map(|part| part.indices.len() / 3).sum::<usize>(), 66);
        let shared = generate(Parameters::meadow_grass(91), Limits::default()).unwrap();
        for (geometry, other) in meadow.parts.iter().zip(&shared.parts) {
            assert_eq!(geometry.id, other.id);
            assert_eq!(bytemuck::cast_slice::<Vertex, u8>(&geometry.vertices),
                bytemuck::cast_slice::<Vertex, u8>(&other.vertices));
            assert_eq!(geometry.indices, other.indices);
            let part = Part { vertices: &geometry.vertices, indices: &geometry.indices, material: 0 };
            assert_eq!(foliage_voxel::validate_closed_part(&part).unwrap(), 1);
            let mesh = crate::vegetation_clod::build(&part).expect("meadow retains source-derived CLOD");
            assert_eq!(mesh.leaf_triangles(), 22);
            assert!(mesh.levels > 1);
            assert!(mesh.clusters.iter().any(|cluster| cluster.tri_count < 22));
        }
        // Geometry and the small material palette are reusable across plant
        // seeds. Only placements and group identities vary, not asset bytes.
        assert_eq!(bytemuck::cast_slice::<Material, u8>(&meadow.materials),
            bytemuck::cast_slice::<Material, u8>(&shared.materials));
        assert!(meadow.instances.iter().all(|instance| instance.branch > 0));
        assert_eq!(meadow.instances.iter().map(|instance| instance.branch).collect::<BTreeSet<_>>().len(), 4);
        for density in [1, 7, 36, 128] {
            let custom = generate(Parameters { density, ..Parameters::meadow_grass(2) }, Limits::default()).unwrap();
            assert_eq!(custom.instances.len(), density as usize);
            assert_eq!(triangles(&custom), density as usize * 22);
        }
        assert!(generate(Parameters::meadow_grass(0), Limits { max_instances: 35, ..Limits::default() }).is_err());
    }

    /// Independent orthographic triangle-union mask. This measures occupied
    /// silhouette pixels, not summed leaf area (which counts overlaps twice).
    /// It intentionally has no lighting, normals, voxel density or GPU inputs.
    fn silhouette_pixels(meshes: &[SourceMesh], right: Vec3, up: Vec3) -> usize {
        use glam::Vec2;
        const RESOLUTION: usize = 128;
        const EXTENT: f32 = 1.2;
        let mut mask = vec![false; RESOLUTION * RESOLUTION];
        let project = |p: Vec3| (Vec2::new(p.dot(right), p.dot(up)) + Vec2::splat(EXTENT * 0.5))
            * (RESOLUTION as f32 / EXTENT);
        let edge = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
        for mesh in meshes { for tri in mesh.indices.chunks_exact(3) {
            let points: [Vec2; 3] = std::array::from_fn(|i| project(Vec3::from(mesh.vertices[tri[i] as usize].pos)));
            if edge(points[0], points[1], points[2]).abs() < 1e-8 { continue; }
            let lo = points[0].min(points[1]).min(points[2]).floor().max(Vec2::ZERO);
            let hi = points[0].max(points[1]).max(points[2]).ceil().min(Vec2::splat((RESOLUTION - 1) as f32));
            if lo.x > hi.x || lo.y > hi.y { continue; }
            for y in lo.y as usize..=hi.y as usize { for x in lo.x as usize..=hi.x as usize {
                let sample = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let edges = [edge(points[0], points[1], sample), edge(points[1], points[2], sample),
                    edge(points[2], points[0], sample)];
                if edges.iter().all(|&v| v >= 0.0) || edges.iter().all(|&v| v <= 0.0) {
                    mask[y * RESOLUTION + x] = true;
                }
            }}
        }}
        mask.into_iter().filter(|&occupied| occupied).count()
    }

    #[test]
    fn meadow_fills_more_real_silhouette_without_extra_blades_or_density_tint() {
        for seed in [2, 17, 73, 91] {
            let reference = generate(Parameters::grass(seed), Limits::default()).unwrap();
            let meadow = generate(Parameters::meadow_grass(seed), Limits::default()).unwrap();
            let source = reference.flatten_rest().unwrap();
            let fuller = meadow.flatten_rest().unwrap();
            assert_eq!(source.len(), fuller.len());
            assert_eq!(reference.materials[1].tint, meadow.materials[1].tint);
            let mut reference_side = 0;
            let mut meadow_side = 0;
            for quadrant in 0..4 {
                let angle = quadrant as f32 * std::f32::consts::FRAC_PI_4;
                let right = Vec3::new(angle.cos(), 0.0, angle.sin());
                reference_side += silhouette_pixels(&source, right, Vec3::Y);
                meadow_side += silhouette_pixels(&fuller, right, Vec3::Y);
            }
            // Ground-facing ribbons intentionally trade the old vertical fan
            // silhouette for low oblique cover (tested below), while retaining
            // at least the ordinary narrow-grass side silhouette within 10%.
            assert!(meadow_side as f32 > reference_side as f32 * 0.90,
                "ground-facing ribbons retain side silhouette: seed {seed}, {reference_side} -> {meadow_side}");
            let reference_top = silhouette_pixels(&source, Vec3::X, Vec3::Z);
            let meadow_top = silhouette_pixels(&fuller, Vec3::X, Vec3::Z);
            assert!(meadow_top > reference_top * 2,
                "the collar and arches must cover more ground without adding blades: seed {seed}, {reference_top} -> {meadow_top}");
            let bounds = meadow.bounds().unwrap();
            assert!(bounds[0][1] > -0.01 && bounds[1][1] < 0.65);
            assert!(bounds[0][0] > -0.6 && bounds[1][0] < 0.6
                && bounds[0][2] > -0.6 && bounds[1][2] < 0.6);
        }
    }

    /// Frozen r34 primitive geometry, including its former broad axis in the
    /// arch plane. Do not derive these vertices from the current recipe.
    fn r34_meadow_blade(variant: u64) -> PartGeometry {
        let mut mesh = PartGeometry { id: key(0, 500, variant), vertices: Vec::new(), indices: Vec::new() };
        let (curve, twist, side_curl) = match variant {
            0 => (4.2, -0.12, -0.25),
            1 => (5.8, 0.10, 0.30),
            _ => (7.2, 0.18, -0.15),
        };
        let heights = [0.0f32, 0.32, 0.68, 0.9];
        let widths = [0.34f32, 0.56, 0.40, 0.16];
        let rings: [[Vec3; 3]; 4] = std::array::from_fn(|i| {
            let y = heights[i];
            let rotation = Quat::from_rotation_y(twist * y);
            let centre = Vec3::new(curve * y * y, y, side_curl * y * y);
            std::array::from_fn(|j| {
                let angle = j as f32 * std::f32::consts::TAU / 3.0;
                centre + rotation * Vec3::new(widths[i] * angle.cos(), 0.0,
                    0.065 * (1.0 - y).sqrt() * angle.sin())
            })
        });
        triangle(&mut mesh, rings[0][0], rings[0][1], rings[0][2]);
        for i in 0..rings.len() - 1 { for j in 0..3 {
            let next = (j + 1) % 3;
            triangle(&mut mesh, rings[i][j], rings[i + 1][j], rings[i + 1][next]);
            triangle(&mut mesh, rings[i][j], rings[i + 1][next], rings[i][next]);
        }}
        let tip = Vec3::new(curve, 1.0, side_curl);
        for j in 0..3 { triangle(&mut mesh, rings[3][j], tip, rings[3][(j + 1) % 3]); }
        mesh
    }

    /// Retained r34 geometry and tall-fan placement oracle. The material
    /// palette is unchanged; the source cross-section is intentionally not.
    fn r34_meadow_reference(seed: u64) -> Assembly {
        let mut source = generate(Parameters::meadow_grass(seed), Limits::default()).unwrap();
        source.parts = (0..3).map(r34_meadow_blade).collect();
        let angle = |group: u32| unit(seed, 1, 0) * std::f32::consts::TAU
            + group as f32 * std::f32::consts::FRAC_PI_2
            + (unit(seed, 30, group as u64) - 0.5) * 0.45;
        for group in 0..4 {
            let a = angle(group);let radial = Vec3::new(a.cos(), 0.0, a.sin());
            source.branches[group as usize + 1].rest = Mat4::from_translation(
                radial * (0.028 + 0.012 * unit(seed, 31, group as u64)));
        }
        for (i, instance) in source.instances.iter_mut().enumerate() {
            let cohort = ((i / 4) % 3) as f32 * 0.5;
            let height = 0.31 + (1.0 - cohort) * 0.20 + unit(seed, 11, i as u64) * 0.10;
            let width = 0.028 + unit(seed, 12, i as u64) * 0.015;
            let fan = angle(i as u32 % 4) + (unit(seed, 10, i as u64) - 0.5) * 1.9;
            let position = Vec3::new(fan.cos(), 0.0, fan.sin())
                * (0.006 + cohort * 0.018 + unit(seed, 13, i as u64) * 0.012);
            let lean = 0.06 + cohort * 0.30 + unit(seed, 14, i as u64) * 0.10;
            instance.rest = Mat4::from_scale_rotation_translation(Vec3::new(width, height, width),
                Quat::from_rotation_y(-fan) * Quat::from_rotation_z(-lean), position);
        }
        source
    }

    /// Clip actual triangles at a horizontal plane before the independent
    /// projected union test. Summed leaf area would hide overlapping fans.
    fn source_below_height(source: &[SourceMesh], height: f32) -> Vec<SourceMesh> {
        source.iter().map(|mesh| {
            let mut result = SourceMesh { identity: mesh.identity, material: mesh.material,
                vertices: Vec::new(), indices: Vec::new() };
            for tri in mesh.indices.chunks_exact(3) {
                let points: [Vec3; 3] = std::array::from_fn(|i| Vec3::from(mesh.vertices[tri[i] as usize].pos));
                let mut clipped = Vec::with_capacity(4);
                for edge in 0..3 {
                    let a = points[edge];let b = points[(edge + 1) % 3];
                    let inside = a.y <= height;let next_inside = b.y <= height;
                    if inside { clipped.push(a); }
                    if inside != next_inside { clipped.push(a + (b - a) * ((height - a.y) / (b.y - a.y))); }
                }
                for corner in 1..clipped.len().saturating_sub(1) {
                    for p in [clipped[0], clipped[corner], clipped[corner + 1]] {
                        result.indices.push(result.vertices.len() as u32);
                        result.vertices.push(Vertex { pos: p.to_array(), normal: [0.0, 1.0, 0.0], uv: [0.0; 2] });
                    }
                }
            }
            result
        }).collect()
    }

    #[test]
    fn meadow_low_arches_increase_ground_union_without_spreading_root_collars() {
        for seed in [2, 17, 73, 91, 101, 303, 707] {
            let before = r34_meadow_reference(seed);
            let after = generate(Parameters::meadow_grass(seed), Limits::default()).unwrap();
            let old = before.flatten_rest().unwrap();let new = after.flatten_rest().unwrap();
            assert_eq!(old.len(), new.len());
            for (a, b) in before.parts.iter().zip(&after.parts) {
                assert_ne!(bytemuck::cast_slice::<Vertex, u8>(&a.vertices), bytemuck::cast_slice::<Vertex, u8>(&b.vertices));
                assert_eq!(a.vertices.len(), b.vertices.len());
                assert_eq!(a.indices, b.indices);
            }
            assert_eq!(bytemuck::cast_slice::<Material, u8>(&before.materials),
                bytemuck::cast_slice::<Material, u8>(&after.materials));
            let old_low = silhouette_pixels(&source_below_height(&old, 0.18), Vec3::X, Vec3::Z);
            let new_low = silhouette_pixels(&source_below_height(&new, 0.18), Vec3::X, Vec3::Z);
            let old_top = silhouette_pixels(&old, Vec3::X, Vec3::Z);
            let new_top = silhouette_pixels(&new, Vec3::X, Vec3::Z);
            assert!(new_low as f32 > old_low as f32 * 1.8,
                "real coverage below18cm: seed{seed} {old_low}->{new_low}");
            assert!(new_top as f32 > old_top as f32 * 1.12,
                "whole ground silhouette: seed{seed} {old_top}->{new_top}");
            // Chase looks down about ten degrees; closer ground is steeper
            // in screen space. Test real clipped blade surfaces at all three
            // inclinations, rather than asserting only a top-view footprint.
            // These are geometry masks, not a claim about terrain occlusion,
            // perspective, wind or final rendered pixel coverage.
            for degrees in [10.0f32, 20.0, 35.0] {
                let pitch = degrees.to_radians();
                let mut old_oblique = 0;let mut new_oblique = 0;
                let old_low_mesh = source_below_height(&old, 0.18);
                let new_low_mesh = source_below_height(&new, 0.18);
                for quadrant in 0..4 {
                    let angle = quadrant as f32 * std::f32::consts::FRAC_PI_4;
                    let right = Vec3::new(angle.cos(), 0.0, angle.sin());
                    let up = Vec3::new(-angle.sin() * pitch.sin(), pitch.cos(), angle.cos() * pitch.sin());
                    old_oblique += silhouette_pixels(&old_low_mesh, right, up);
                    new_oblique += silhouette_pixels(&new_low_mesh, right, up);
                }
                let minimum_ratio = if degrees == 10.0 { 1.0 } else if degrees == 20.0 { 1.20 } else { 1.60 };
                assert!(new_oblique as f32 > old_oblique as f32 * minimum_ratio,
                    "actual low ribbon cover at {degrees} degrees: seed {seed} {old_oblique}->{new_oblique}");
            }
            assert!(new.iter().flat_map(|m| &m.vertices).all(|v| v.pos[1] >= -1e-6));
            for instance in &after.instances {
                let root = (after.branches[instance.branch as usize].rest * instance.rest).w_axis;
                assert!(root.x.hypot(root.z) < 0.071, "compact basal roots avoid slope-floating");
            }
        }
    }

    #[test]
    fn invalid_hierarchy_and_budgets_fail_explicitly() {
        for p in [Parameters { growth: f32::NAN, ..Parameters::grass(0) },
            Parameters { density: 129, ..Parameters::grass(0) }, Parameters { season: -0.1, ..Parameters::grass(0) }] {
            assert!(generate(p, Limits::default()).is_err());
        }
        assert!(generate(Parameters::grass(0), Limits { max_instances: 1, ..Limits::default() }).is_err());
        assert!(generate(Parameters::grass(0), Limits { max_source_bytes: 8, ..Limits::default() }).is_err());
        let mut a = generate(Parameters::grass(0), Limits::default()).unwrap();
        a.branches[1].parent = Some(1);
        assert!(a.flatten_rest().is_err());
        a.branches[1].parent = Some(0);
        a.instances[0].rest = Mat4::from_scale(Vec3::ZERO);
        assert!(a.bake_rest(config()).is_err());
        let a = generate(Parameters::grass(0), Limits::default()).unwrap();
        assert!(a.bake_branches(BakeConfig { max_samples: 1, ..config() }).is_err());
    }
}
