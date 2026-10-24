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
pub enum Archetype { GrassTuft, ConiferTwig }

#[derive(Clone, Copy, Debug)]
pub struct Parameters {
    pub seed: u64,
    pub archetype: Archetype,
    /// Rest-size multiplier, 0.1..=1.5. The assembly root remains the origin.
    pub growth: f32,
    /// 0 = green, 1 = dry/dormant palette. Does not erase source identities.
    pub season: f32,
    /// Exact blade/needle count, 1..=128, independent of growth and season.
    pub density: u32,
}

impl Parameters {
    pub fn grass(seed: u64) -> Self {
        Self { seed, archetype: Archetype::GrassTuft, growth: 1.0, season: 0.0, density: 36 }
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
    key(p.seed, tag + if p.archetype == Archetype::GrassTuft { 100 } else { 200 }, index)
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
    if !p.growth.is_finite() || !(0.1..=1.5).contains(&p.growth)
        || !p.season.is_finite() || !(0.0..=1.0).contains(&p.season)
        || !(1..=128).contains(&p.density) {
        return Err("invalid procedural foliage parameters".into());
    }
    let grass = p.archetype == Archetype::GrassTuft;
    let count = p.density as usize + if grass { 0 } else { 5 };
    if limits.max_parts < 3 || limits.max_branches < 5 || limits.max_instances < count {
        return Err("procedural foliage structure exceeds explicit limits".into());
    }
    let green = if grass { Vec3::new(0.105, 0.18, 0.039) } else { Vec3::new(0.045, 0.10, 0.032) };
    let dry = if grass { Vec3::new(0.24, 0.19, 0.075) } else { Vec3::new(0.072, 0.095, 0.036) };
    let colour = green.lerp(dry, p.season);
    let materials = vec![material(colour * 0.88, 0.88, true), material(colour, 0.9, true),
        material(colour * Vec3::new(1.08, 1.05, 0.98), 0.87, true),
        material(Vec3::new(0.105, 0.066, 0.033), 0.94, false)];
    let parts = if grass {
        (0..3).map(|i| blade(key(0, 300, i), 2.0 + i as f32 * 1.7, 0.075)).collect()
    } else {
        vec![blade(key(0, 400, 0), 0.2, 0.23), blade(key(0, 400, 1), 0.65, 0.23), stem(key(0, 400, 2))]
    };
    let mut assembly = Assembly { parameters: p, parts, materials, limits,
        branches: vec![BranchGroup { id: identity(p, 0, 0), parent: None, rest: Mat4::IDENTITY, stiffness: 1.0 }],
        instances: Vec::with_capacity(count) };
    for group in 0..4 {
        let angle = group as f32 * 2.399_963_1 + unit(p.seed, 1, 0) * std::f32::consts::TAU;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
        let (position, rotation, stiffness) = if grass {
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
        let group = i % 4 + 1;
        let angle = unit(p.seed, 10, i as u64) * std::f32::consts::TAU;
        let radial = Vec3::new(angle.cos(), 0.0, angle.sin());
        let (position, rotation, size, part) = if grass {
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
            material: (key(p.seed, 15, i as u64) % 3) as u32 });
    }
    assembly.validate()?;
    Ok(assembly)
}

fn valid_transform(m: Mat4) -> bool {
    m.is_finite() && m.x_axis.w == 0.0 && m.y_axis.w == 0.0 && m.z_axis.w == 0.0 && m.w_axis.w == 1.0
        && m.determinant().is_finite() && m.determinant() > 1e-12 && m.inverse().is_finite()
}

impl Assembly {
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
            if material.layer != -1 || material.flags & !(TWO_SIDED | THIN_FOLIAGE) != 0 || material.metal != 0.0
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

    pub fn bounds(&self) -> Result<[[f32; 3]; 2], String> {
        let mut lo = Vec3::splat(f32::INFINITY); let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for mesh in self.flatten_rest()? { for vertex in mesh.vertices { let p = Vec3::from(vertex.pos); lo = lo.min(p); hi = hi.max(p); } }
        if !lo.is_finite() { return Err("foliage assembly has no source geometry".into()); }
        Ok([lo.to_array(), hi.to_array()])
    }

    pub fn bake_rest(&self, config: BakeConfig) -> Result<VoxelAsset, String> {
        let meshes = self.flatten_rest()?;
        foliage_voxel::bake(&meshes.iter().map(SourceMesh::as_part).collect::<Vec<_>>(), &self.materials, &[], config)
    }

    /// Budgets are shared across all returned branches, not silently multiplied
    /// by the group count. Use ClosedSurfaceEntries for these closed source
    /// parts; SheetCrossings is retained only for explicit optical comparisons.
    pub fn bake_branches(&self, mut config: BakeConfig) -> Result<Vec<BakedBranch>, String> {
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
    fn opaque_sources_are_reused_deterministic_and_finite() {
        for p in [Parameters::grass(73), Parameters::conifer_twig(73)] {
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
        for base in [Parameters::grass(19),Parameters::conifer_twig(27)] {
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
        for p in [Parameters::grass(17), Parameters::conifer_twig(17)] {
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

    #[test]
    fn branch_local_source_reconstructs_exact_rest_and_bake_mass() {
        for p in [Parameters { density: 12, ..Parameters::grass(2) }, Parameters { density: 16, ..Parameters::conifer_twig(2) }] {
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
