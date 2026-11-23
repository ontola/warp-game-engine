//! Exact index streams into the existing flat meadow recipe. Triangle order,
//! winding, normals and LOD cuts remain unchanged. A representative is shared
//! only when its required attributes agree bit-for-bit in EVERY variant.
use crate::meadow_data::{GpuMeadowVertex, MeadowGeometry, LODS, VARIANTS};
use std::collections::BTreeMap;

pub(crate) struct MeadowIndices {
    pub depth: [Vec<u16>; LODS],
    pub color: [Vec<u16>; LODS],
    pub depth_unique: [u32; LODS],
    pub color_unique: [u32; LODS],
}

fn depth_key(v: &GpuMeadowVertex) -> [u32; 8] {
    std::array::from_fn(|i| if i < 4 { v.position[i].to_bits() } else { v.anchor[i - 4].to_bits() })
}
fn color_key(v: &GpuMeadowVertex) -> [u32; 16] { bytemuck::cast(*v) }

fn stream<const N: usize>(geometry: &MeadowGeometry, lod: usize,
    key: impl Fn(&GpuMeadowVertex) -> [u32; N]) -> (Vec<u16>, u32) {
    let count = geometry.lod_ranges[0][lod][1] as usize;
    let mut representatives = BTreeMap::<[[u32; N]; VARIANTS], u16>::new();
    let mut indices = Vec::with_capacity(count);
    for i in 0..count {
        let common_key = std::array::from_fn(|variant| {
            let first = geometry.lod_ranges[variant][lod][0] as usize;
            key(&geometry.vertices[first + i])
        });
        let representative = *representatives.entry(common_key).or_insert(i as u16);
        indices.push(representative);
    }
    (indices, representatives.len() as u32)
}

impl MeadowIndices {
    pub fn build(geometry: &MeadowGeometry) -> Result<Self, String> {
        // Indices are relative to each variant's original flat LOD range.
        // No truncation, float tolerance, normal averaging or variant-local
        // equivalence is allowed by the shared hardware index stream.
        if geometry.lod_ranges.iter().flatten().any(|r| r[1] > u16::MAX as u32 + 1) {
            return Err("meadow flat LOD exceeds u16 indexed vertex address space".into());
        }
        geometry.validate()?;
        let depth: [(Vec<u16>, u32); LODS] = std::array::from_fn(|lod| stream(geometry, lod, depth_key));
        let color: [(Vec<u16>, u32); LODS] = std::array::from_fn(|lod| stream(geometry, lod, color_key));
        let depth_unique = std::array::from_fn(|lod| depth[lod].1);
        let color_unique = std::array::from_fn(|lod| color[lod].1);
        Ok(Self { depth: depth.map(|v| v.0), color: color.map(|v| v.0), depth_unique, color_unique })
    }

    pub fn bytes(&self) -> u64 {
        self.depth.iter().chain(&self.color).map(|v| v.len() as u64 * 2).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    fn square() -> MeadowGeometry {
        let a = [-0.1, 0.1, -0.1, 0.0];let b = [0.1, 0.1, -0.1, 0.0];
        let c = [0.1, 0.1, 0.1, 0.0];let d = [-0.1, 0.1, 0.1, 0.0];
        let vertices: Vec<_> = (0..VARIANTS * LODS).flat_map(|_| [a,b,c,a,c,d].map(|position|
            GpuMeadowVertex { position, normal: [0.0, 1.0, 0.0, 0.0],
                color: [0.1, 0.2, 0.1, 0.9], anchor: [0.0; 4] })).collect();
        MeadowGeometry { vertices, mown_turf: false, variant_patch_metres: 0.0,
            lod_ranges: std::array::from_fn(|v| std::array::from_fn(|l| [((v * LODS + l) * 6) as u32, 6])),
            lod_errors: [[0.0; LODS]; VARIANTS], branch_pivots: [[[0.0; 4]; 4]; VARIANTS],
            max_tuft_radius: 0.2, max_tuft_height: 0.1, max_anchor_radius: 0.1, max_blade_radius: 1.0 }
    }

    #[test]
    fn common_indices_never_merge_other_variant_normals_anchors_or_branches() {
        let source = square();let indices = MeadowIndices::build(&source).unwrap();
        assert_eq!(indices.depth[0], [0,1,2,0,2,5]);
        assert_eq!(indices.color[0], indices.depth[0]);
        assert_eq!(indices.depth_unique, [4; LODS]);
        assert_eq!(indices.color_unique, [4; LODS]);
        let changed = source.lod_ranges[2][0][0] as usize + 3;
        let mut normals = source.clone();normals.vertices[changed].normal = [1.0, 0.0, 0.0, 0.0];
        let n = MeadowIndices::build(&normals).unwrap();
        assert_eq!(n.depth[0][3], 0);
        assert_eq!(n.color[0][3], 3, "a difference in only variant2 must split the COMMON color index");
        let mut anchors = source.clone();anchors.vertices[changed].anchor[0] = 0.01;
        let a = MeadowIndices::build(&anchors).unwrap();
        assert_eq!(a.depth[0][3], 3, "equal positions with different wind pivots are not interchangeable");
        let mut branches = source;branches.vertices[changed].position[3] = 1.0;
        assert_eq!(MeadowIndices::build(&branches).unwrap().depth[0][3], 3);
    }

    #[test]
    fn actual_recipe_expands_exactly_in_order_for_all_variants_lods_and_wind_poses() {
        let source = MeadowGeometry::cook(17, 0.0).unwrap();
        let indices = MeadowIndices::build(&source).unwrap();
        // Independent rigid-pose oracle: exact anchor/position/branch matches
        // must remain exact through current and previous rotations, growth,
        // yaw and a sloping terrain plane. No production shader helper used.
        let pose = |v: &GpuMeadowVertex, growth: f32, time: f32| {
            let anchor = Vec3::from_slice(&v.anchor);
            let local = (Vec3::from_slice(&v.position) - anchor) * growth;
            let yaw = Quat::from_rotation_y(0.73);
            let rotation = Quat::from_axis_angle(Vec3::new(0.6, 0.0, -0.8), time + v.position[3] * 0.03);
            let p = yaw * anchor + rotation * (yaw * local);
            Vec3::new(53.0 + p.x, 7.0 + p.y + 0.45 * p.x - 0.17 * p.z, -83.0 + p.z)
        };
        for lod in 0..LODS {
            let count = source.lod_ranges[0][lod][1] as usize;
            assert_eq!(indices.depth[lod].len(), count);assert_eq!(indices.color[lod].len(), count);
            assert!(indices.depth_unique[lod] <= indices.color_unique[lod]);
            assert!(indices.color_unique[lod] <= count as u32);
            for variant in 0..VARIANTS {
                let start = source.lod_ranges[variant][lod][0] as usize;
                let flat = &source.vertices[start..start + count];
                for i in 0..count {
                    let d = &flat[indices.depth[lod][i] as usize];
                    let c = &flat[indices.color[lod][i] as usize];
                    assert_eq!(depth_key(&flat[i]), depth_key(d));
                    assert_eq!(color_key(&flat[i]), color_key(c));
                    for (growth, phase) in [(0.35, -0.12), (1.45, 0.19)] {
                        assert_eq!(pose(&flat[i], growth, phase), pose(d, growth, phase));
                    }
                }
            }
            eprintln!("meadow index LOD{lod}: flat {count}, depth unique {}, color unique {}",
                indices.depth_unique[lod], indices.color_unique[lod]);
        }
        assert!(indices.depth_unique[0] < source.lod_ranges[0][0][1]);
        assert!(indices.color_unique[0] < source.lod_ranges[0][0][1]);
        assert_eq!(indices.bytes(), source.lod_ranges[0].iter().map(|r| u64::from(r[1]) * 4).sum::<u64>());
    }

    #[test]
    fn rejects_index_overflow_instead_of_truncating() {
        let mut source = square();
        source.lod_ranges[0][0][1] = 65_538;
        assert!(MeadowIndices::build(&source).err().unwrap().contains("u16"));
    }
}
