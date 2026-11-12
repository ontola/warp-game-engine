//! Weighted mesh deformation, evaluated once before all views and shadows.
//!
//! Every actor owns a destination mesh range, but materials and immutable clip
//! samples can be shared by the asset caller. Palettes are final object-space
//! skin matrices (animated global joint * inverse bind). Eight influences are
//! retained. Cluster and owner spheres must enclose every submitted pose.
use crate::{
    Gpu,
    cluster::Vertex,
    scene::{GpuInstance, SKINNED},
};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

// Eight playable seats plus their burning corpse counterparts.
pub const MAX_ACTORS: usize = 64;
pub const MAX_JOINTS: usize = 256;
pub const MAX_VERTICES: usize = 1_600_000;

#[derive(Clone, Copy, Debug)]
pub struct Influence {
    pub joints: [u32; 8],
    pub weights: [f32; 8],
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub owner: u32,
    /// Initial final skin matrices. Later palettes must stay inside the mesh's
    /// declared all-pose bounds; clip sampling and convex crossfades do so.
    pub palette: Vec<Mat4>,
    pub influences: Vec<Influence>,
}

#[derive(Clone, Debug, Default)]
pub struct Skinning {
    pub actors: Vec<Actor>,
}

pub fn valid_matrix(matrix: Mat4) -> bool {
    matrix.is_finite()
        && matrix.row(3).abs_diff_eq(Vec4::W, 1e-5)
        && matrix.to_cols_array().iter().all(|v| v.abs() < 1e6)
}

impl Skinning {
    pub fn validate(&self, vertices: &[Vertex], instances: &[GpuInstance]) -> Result<(), String> {
        if self.actors.len() > MAX_ACTORS {
            return Err("skinned actor capacity exceeded".into());
        }
        let mut ranges = Vec::new();
        let mut total = 0usize;
        let mut owners = std::collections::HashSet::new();
        for actor in &self.actors {
            let range = actor.first_vertex as usize
                ..actor.first_vertex as usize + actor.vertex_count as usize;
            if actor.vertex_count == 0
                || range.end > vertices.len()
                || ranges
                    .iter()
                    .any(|r: &std::ops::Range<usize>| r.start < range.end && range.start < r.end)
                || !owners.insert(actor.owner)
            {
                return Err("invalid or overlapping skinned vertex/owner range".into());
            }
            let owner = instances
                .get(actor.owner as usize)
                .ok_or("missing skinned owner")?;
            if owner.flags & SKINNED == 0
                || owner.flags
                    & (crate::scene::FOLIAGE
                        | crate::scene::PART_WIND
                        | crate::scene::WIND_GRASS
                        | crate::scene::LIVE_GRASS)
                    != 0
            {
                return Err(
                    "skinned owner flag missing or conflicts with other deformation".into(),
                );
            }
            if actor.palette.is_empty()
                || actor.palette.len() > MAX_JOINTS
                || actor.palette.iter().any(|m| !valid_matrix(*m))
                || actor.influences.len() != actor.vertex_count as usize
            {
                return Err("invalid skinning palette/influence count".into());
            }
            for (vertex, influence) in vertices[range.clone()].iter().zip(&actor.influences) {
                let sum: f32 = influence.weights.iter().sum();
                if !vertex
                    .pos
                    .iter()
                    .chain(&vertex.normal)
                    .chain(&vertex.uv)
                    .all(|f| f.is_finite())
                    || Vec3::from(vertex.pos).abs().max_element() > 1e6
                    || !(1e-12..=1e12).contains(&Vec3::from(vertex.normal).length_squared())
                    || !sum.is_finite()
                    || (sum - 1.0).abs() > 0.001
                    || influence.weights.iter().any(|w| !w.is_finite() || *w < 0.0)
                    || influence
                        .joints
                        .iter()
                        .any(|j| *j as usize >= actor.palette.len())
                {
                    return Err("invalid weighted vertex, joint index or normalized weights".into());
                }
            }
            total += actor.vertex_count as usize;
            ranges.push(range);
        }
        if total > MAX_VERTICES {
            return Err("skinned vertex capacity exceeded".into());
        }
        Ok(())
    }
}

/// CPU reference for focused validation and animation bounds cooking.
pub fn skin_position(position: Vec3, influence: &Influence, palette: &[Mat4]) -> Vec3 {
    (0..8)
        .map(|i| {
            palette[influence.joints[i] as usize].transform_point3(position) * influence.weights[i]
        })
        .sum()
}

/// Element-wise interpolation of final matrices; position results are convex
/// combinations of the source poses, preserving their conservative bounds.
pub fn blend_palette(a: &[Mat4], b: &[Mat4], blend: f32) -> Result<Vec<Mat4>, String> {
    let mut out = vec![Mat4::IDENTITY; a.len()];
    blend_palette_into(a, b, blend, &mut out)?;
    Ok(out)
}

/// Sample into a caller-owned palette; no allocation after actor creation.
pub fn blend_palette_into(a: &[Mat4], b: &[Mat4], blend: f32, out: &mut [Mat4]) -> Result<(), String> {
    if a.len() != b.len() || a.len() != out.len() || !blend.is_finite() || !(0.0..=1.0).contains(&blend) {
        return Err("skin palette blend needs equal lengths and a 0..1 weight".into());
    }
    for ((a, b), dst) in a.iter().zip(b).zip(out) {
        *dst = *a * (1.0 - blend) + *b * blend;
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Source {
    position: [f32; 4],
    normal: [f32; 4],
    joints: [[u32; 4]; 2],
    weights: [[f32; 4]; 2],
    /// Destination global vertex index, first palette matrix, reserved.
    address: [u32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Matrix {
    rows: [[f32; 4]; 3],
}
impl Matrix {
    fn new(m: Mat4) -> Self {
        Self {
            rows: [
                m.row(0).to_array(),
                m.row(1).to_array(),
                m.row(2).to_array(),
            ],
        }
    }
}

pub(crate) struct SkinningRenderer {
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    palette: wgpu::Buffer,
    actors: Vec<(u32, usize)>,
    packed: std::sync::Mutex<Vec<Matrix>>,
    count: u32,
}
impl SkinningRenderer {
    pub fn new(
        gpu: &Gpu,
        skinning: &Skinning,
        vertices: &[Vertex],
        instances: &[GpuInstance],
        output: &crate::topology::Topology,
        owners: &wgpu::Buffer,
    ) -> Result<Self, String> {
        skinning.validate(vertices, instances)?;
        let mut palette = Vec::new();
        let mut sources = Vec::new();
        let mut actors = Vec::new();
        for actor in &skinning.actors {
            let offset = palette.len() as u32;
            actors.push((offset, actor.palette.len()));
            palette.extend(actor.palette.iter().map(|m| Matrix::new(*m)));
            for (index, influence) in actor.influences.iter().enumerate() {
                let destination = actor.first_vertex + index as u32;
                let vertex = vertices[destination as usize];
                sources.push(Source {
                    position: Vec3::from(vertex.pos).extend(1.0).to_array(),
                    normal: Vec3::from(vertex.normal).extend(0.0).to_array(),
                    joints: [
                        influence.joints[..4].try_into().unwrap(),
                        influence.joints[4..].try_into().unwrap(),
                    ],
                    weights: [
                        influence.weights[..4].try_into().unwrap(),
                        influence.weights[4..].try_into().unwrap(),
                    ],
                    address: [destination, offset, actor.owner, 0],
                });
            }
        }
        if sources.is_empty() {
            return Err("cannot construct empty GPU skinning".into());
        }
        let limit = gpu.limits.max_storage_buffer_binding_size as usize;
        if std::mem::size_of_val(sources.as_slice()) > limit
            || std::mem::size_of_val(palette.as_slice()) > limit
        {
            return Err("skinning storage exceeds device capacity".into());
        }
        let source = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("immutable weighted bind vertices"),
                contents: bytemuck::cast_slice(&sources),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let packed = std::sync::Mutex::new(palette);
        let palette = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("actor skin palettes"),
                contents: bytemuck::cast_slice(&packed.lock().unwrap()),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("weighted skinning"),
                source: wgpu::ShaderSource::Wgsl(SHADER.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("weighted skinning"),
                layout: None,
                module: &module,
                entry_point: Some("skin_vertices"),
                compilation_options: wgpu::PipelineCompilationOptions {constants:&[("SOURCE_COUNT",sources.len()as f64)],..Default::default()},
                cache: gpu.pipeline_cache(),
            });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("weighted skinning"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: source.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palette.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: output.vertices(),
                },
                wgpu::BindGroupEntry { binding: 3, resource: owners.as_entire_binding() },
            ],
        });
        Ok(Self {
            pipeline,
            bind,
            palette,
            actors,
            packed,
            count: sources.len() as u32,
        })
    }

    pub(crate) fn palette_buffer(&self) -> &wgpu::Buffer { &self.palette }

    pub fn set_palette(&self, gpu: &Gpu, actor: usize, matrices: &[Mat4]) -> Result<(), String> {
        let &(offset, count) = self.actors.get(actor).ok_or("skinned actor out of range")?;
        if matrices.len() != count || matrices.iter().any(|m| !valid_matrix(*m)) {
            return Err("invalid skinning palette size or non-affine matrix".into());
        }
        let mut packed = self.packed.lock().unwrap();
        let slice = &mut packed[offset as usize..offset as usize + count];
        for (dst, m) in slice.iter_mut().zip(matrices) { *dst = Matrix::new(*m); }
        gpu.queue.write_buffer(
            &self.palette,
            offset as u64 * std::mem::size_of::<Matrix>() as u64,
            bytemuck::cast_slice(slice),
        );
        Ok(())
    }

    /// Upload every actor in scene order with a single staging write. Validate
    /// the complete batch before changing any retained palette.
    pub fn set_palettes<'a>(&self, gpu: &Gpu,
        palettes: impl ExactSizeIterator<Item = &'a [Mat4]> + Clone) -> Result<(), String> {
        if palettes.len() != self.actors.len() {
            return Err("skinning batch must contain every actor in scene order".into());
        }
        for (matrices, &(_, count)) in palettes.clone().zip(&self.actors) {
            if matrices.len() != count || matrices.iter().any(|m| !valid_matrix(*m)) {
                return Err("invalid skinning palette size or non-affine matrix".into());
            }
        }
        let mut packed = self.packed.lock().unwrap();
        for (matrices, &(offset, count)) in palettes.zip(&self.actors) {
            for (dst, m) in packed[offset as usize..offset as usize + count].iter_mut().zip(matrices) {
                *dst = Matrix::new(*m);
            }
        }
        if !packed.is_empty() { gpu.queue.write_buffer(&self.palette, 0, bytemuck::cast_slice(&packed)); }
        Ok(())
    }

    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp: Option<(&wgpu::QuerySet, u32)>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shared weighted character skinning"),
            timestamp_writes: timestamp.map(|(query_set, start)| {
                wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: Some(start),
                    end_of_pass_write_index: Some(start + 1),
                }
            }),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
    }
}

pub const SHADER: &str = include_str!("shaders/skinning.wgsl");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_influences_and_degenerate_normals_are_rejected() {
        let mut skinning = Skinning {
            actors: vec![Actor {
                first_vertex: 0,
                vertex_count: 1,
                owner: 0,
                palette: vec![Mat4::IDENTITY],
                influences: vec![Influence {
                    joints: [0; 8],
                    weights: [0.125; 8],
                }],
            }],
        };
        let instances = [GpuInstance {
            rows: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            sphere: [0.0, 0.0, 0.0, 1.0],
            first_cluster: 0,
            cluster_count: 1,
            scale: 1.0,
            flags: SKINNED,
        }];
        let mut vertices = [Vertex {
            pos: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
        }];
        assert!(skinning.validate(&vertices, &instances).is_ok());
        vertices[0].normal = [0.0; 3];
        assert!(skinning.validate(&vertices, &instances).is_err());
        vertices[0].normal = [0.0, 1.0, 0.0];
        skinning.actors[0].influences[0].joints[7] = 1;
        assert!(skinning.validate(&vertices, &instances).is_err());
        skinning.actors[0].influences[0].joints[7] = 0;
        skinning.actors[0].influences[0].weights[7] = -0.125;
        assert!(skinning.validate(&vertices, &instances).is_err());
    }
    #[test]
    fn weighted_skinning_retains_small_influences() {
        let weights = [0.32, 0.21, 0.14, 0.10, 0.08, 0.06, 0.05, 0.04];
        let influence = Influence {
            joints: [0, 1, 2, 3, 4, 5, 6, 7],
            weights,
        };
        let palette: Vec<_> = (0..8)
            .map(|i| Mat4::from_translation(Vec3::X * i as f32))
            .collect();
        let expected: f32 = weights.iter().enumerate().map(|(i, w)| i as f32 * w).sum();
        assert!(
            skin_position(Vec3::Y, &influence, &palette)
                .abs_diff_eq(Vec3::new(expected, 1.0, 0.0), 1e-6)
        );
    }
    #[test]
    fn interpolation_and_crossfade_stay_inside_sample_sphere() {
        let influence = Influence {
            joints: [0; 8],
            weights: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        };
        let a = [Mat4::from_rotation_z(-1.3)];
        let b = [Mat4::from_rotation_z(1.3)];
        for frame in 0..101 {
            let p = skin_position(
                Vec3::Y,
                &influence,
                &blend_palette(&a, &b, frame as f32 / 100.0).unwrap(),
            );
            assert!(p.length() <= 1.000001);
        }
        assert!(blend_palette(&a, &[], 0.5).is_err());
    }
    #[test]
    fn retained_palette_matches_previous_arithmetic_and_rejects_bad_batches() {
        let a: Vec<_> = (0..128).map(|i| Mat4::from_rotation_y(i as f32 * 0.07)).collect();
        let b: Vec<_> = (0..128).map(|i| Mat4::from_translation(Vec3::new(i as f32, 2.0, -3.0))).collect();
        let mut out = vec![Mat4::IDENTITY; a.len()];
        let pointer = out.as_ptr();
        for step in 0..101 {
            let weight = step as f32 / 100.0;
            blend_palette_into(&a, &b, weight, &mut out).unwrap();
            for ((a, b), value) in a.iter().zip(&b).zip(&out) {
                assert_eq!(value.to_cols_array(), (*a * (1.0 - weight) + *b * weight).to_cols_array());
            }
            assert_eq!(pointer, out.as_ptr());
        }
        let previous = out.clone();
        assert!(blend_palette_into(&a, &b, f32::NAN, &mut out).is_err());
        assert!(blend_palette_into(&a, &b[..127], 0.5, &mut out).is_err());
        assert_eq!(out, previous);
    }
    #[test]
    fn skinning_shader_and_layout_validate() {
        assert_eq!(std::mem::size_of::<Source>(), 112);
        assert_eq!(std::mem::size_of::<Matrix>(), 48);
        let module = naga::front::wgsl::parse_str(SHADER)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
