//! Small, deterministic forward pass for continuous-alpha and additive surfaces.
//!
//! Only source triangles carrying blend metadata enter this pass. They reuse
//! the opaque renderer's geometry, materials and live instance transforms,
//! and sort by camera-space triangle depth per view. There is no stochastic
//! coverage, opaque depth write, shadow-map draw or ambient proxy for glass.

use crate::Gpu;
use crate::frame::Camera;
use crate::scene::{GpuInstance, Scene, HIDE_IN_VIEW_SHIFT};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::HashMap;
use std::sync::Mutex;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Triangle {
    vertices: [u32; 3],
    owner: u32,
    material: u32,
    alpha: f32,
    double_sided: u32,
    additive: u32,
}

struct SortTriangle {
    owner: u32,
    center: Vec3,
}

struct OrderBuffer {
    buffer: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

pub(crate) struct Transparency {
    pipelines: [wgpu::RenderPipeline; 2],
    /// Kept to rebuild both native and DLSS pipelines when source changes.
    layout: wgpu::PipelineLayout,
    source: String,
    view_layout: wgpu::BindGroupLayout,
    geometry_layout: wgpu::BindGroupLayout,
    triangles: wgpu::Buffer,
    sort_triangles: Vec<SortTriangle>,
    // Only owners of blended geometry: do not mirror a whole forest on CPU.
    owners: Mutex<HashMap<u32, GpuInstance>>,
    // One reusable order buffer per view avoids rewriting a preceding view's
    // indices before its commands execute at the shared queue submission.
    orders: Mutex<Vec<OrderBuffer>>,
    motion_targets: Mutex<Vec<Option<wgpu::TextureView>>>,
}

pub(crate) struct Reload {
    pipelines: [wgpu::RenderPipeline; 2],
    source: String,
}

/// Compose the real production lighting helpers with the forward entry points.
/// Unused compute bindings are absent from the forward pipeline layout: HDR is
/// a render attachment here, never also a bound writable storage texture.
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}",
        crate::wind::shade_source(),
        crate::shaders::source("transparency.wgsl")
    )
}

/// Native glass and glass with DLSS motion metadata from one composed source.
fn pipelines(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str) -> [wgpu::RenderPipeline; 2] {
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("transparent car surfaces"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    [false, true].map(|motion| {
        let mut targets = vec![Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        if motion {
            targets.push(Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rg32Uint,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            }));
        }
        gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("transparent car surfaces"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("transparent_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if motion { "transparent_motion_fs" } else { "transparent_fs" }),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: gpu.pipeline_cache(),
        })
    })
}

fn source_triangles(
    scene: &Scene,
) -> (Vec<Triangle>, Vec<SortTriangle>, HashMap<u32, GpuInstance>) {
    let mut triangles = Vec::new();
    let mut sorting = Vec::new();
    let mut owners = HashMap::new();
    for (owner, instance) in scene.instances.iter().enumerate() {
        // Zero-cluster owners (virtual branches pack templates in first_cluster).
        if instance.cluster_count == 0 { continue; }
        let start = instance.first_cluster as usize;
        let end = start + instance.cluster_count as usize;
        for cluster in &scene.clusters[start..end] {
            if cluster.own_error != 0.0 {
                continue;
            }
            let Some(blend) = scene.blended_materials.get(&cluster.material) else {
                continue;
            };
            for triangle in cluster.first_tri..cluster.first_tri + cluster.tri_count {
                let packed = scene.tris[triangle as usize];
                let indices = std::array::from_fn(|corner| {
                    scene.cluster_verts
                        [cluster.first_vert as usize + ((packed >> (corner * 8)) & 255) as usize]
                });
                let center = indices
                    .iter()
                    .map(|&index| Vec3::from(scene.vertices[index as usize].pos))
                    .sum::<Vec3>()
                    / 3.0;
                triangles.push(Triangle {
                    vertices: indices,
                    owner: owner as u32,
                    material: cluster.material,
                    alpha: blend.alpha,
                    double_sided: u32::from(blend.double_sided),
                    additive: u32::from(blend.additive),
                });
                sorting.push(SortTriangle {
                    owner: owner as u32,
                    center: if blend.object_sort { Vec3::ZERO } else { center },
                });
                owners.insert(owner as u32, *instance);
            }
        }
    }
    (triangles, sorting, owners)
}

fn sorted_indices(
    triangles: &[SortTriangle],
    owners: &HashMap<u32, GpuInstance>,
    camera: &Camera,
    view_index: usize,
    excluded_categories: u32,
) -> Vec<u32> {
    let forward = (camera.target - camera.pos).normalize_or(Vec3::NEG_Z);
    let mut depths: Vec<_> = triangles
        .iter()
        .enumerate()
        .filter(|(_, triangle)| visible_in_view(&owners[&triangle.owner], view_index, excluded_categories))
        .map(|(index, triangle)| {
            let rows = owners[&triangle.owner].rows;
            let p = triangle.center.extend(1.0);
            let world = Vec3::new(
                glam::Vec4::from(rows[0]).dot(p),
                glam::Vec4::from(rows[1]).dot(p),
                glam::Vec4::from(rows[2]).dot(p),
            );
            (index as u32, (world - camera.pos).dot(forward))
        })
        .collect();
    // Stable tie-break prevents equal-depth coplanar decals flipping order.
    depths.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    depths.into_iter().map(|(index, _)| index).collect()
}

fn visible_in_view(instance: &GpuInstance, view_index: usize, excluded_categories: u32) -> bool {
    instance.cluster_count != 0
        && instance.scale > 0.0
        && (view_index >= 8 || (instance.flags >> HIDE_IN_VIEW_SHIFT) & (1 << view_index) == 0)
        && instance.category().exclusion_bit() & excluded_categories == 0
}

impl Transparency {
    pub(crate) fn triangle_count(&self) -> u32 {
        self.sort_triangles.len() as u32
    }

    pub(crate) fn new(
        gpu: &Gpu,
        scene: &Scene,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        let (triangles, sort_triangles, owners) = source_triangles(scene);
        if triangles.is_empty() {
            return None;
        }
        let read_buffer = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let texture = |sample_type, view_dimension| wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        };
        let all = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let entries = [
            (0, uniform),
            (2, uniform),
            (
                5,
                texture(
                    wgpu::TextureSampleType::Depth,
                    wgpu::TextureViewDimension::D2,
                ),
            ),
            (
                6,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            ),
            (
                7,
                texture(
                    wgpu::TextureSampleType::Float { filterable: false },
                    wgpu::TextureViewDimension::D2,
                ),
            ),
            (
                10,
                texture(
                    wgpu::TextureSampleType::Float { filterable: false },
                    wgpu::TextureViewDimension::D2,
                ),
            ),
            (11, uniform),
            (12, read_buffer),
            (
                13,
                texture(
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D3,
                ),
            ),
            (
                14,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            (
                15,
                texture(
                    wgpu::TextureSampleType::Depth,
                    wgpu::TextureViewDimension::D2,
                ),
            ),
        ];
        let view_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("transparent view"),
                entries: &entries
                    .iter()
                    .map(|&(binding, ty)| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: all,
                        ty,
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            });
        let geometry_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("transparent triangles"),
                    entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: all,
                        ty: read_buffer,
                        count: None,
                    }),
                });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("transparent car surfaces"),
                bind_group_layouts: &[
                    Some(scene_layout),
                    Some(&view_layout),
                    Some(&geometry_layout),
                ],
                immediate_size: 0,
            });
        let source = shader_source();
        let pipelines = pipelines(gpu, &layout, &source);
        let triangles = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("transparent source triangles"),
                contents: bytemuck::cast_slice(&triangles),
                usage: wgpu::BufferUsages::STORAGE,
            });
        Some(Self {
            pipelines,
            layout,
            source,
            view_layout,
            geometry_layout,
            triangles,
            sort_triangles,
            owners: Mutex::new(owners),
            orders: Mutex::new(Vec::new()),
            motion_targets: Mutex::new(Vec::new()),
        })
    }

    /// Preparation never changes a live pipeline; Renderer commits all
    /// material consumers together, including both motion variants.
    pub(crate) fn prepare_reload(&self, gpu: &Gpu, source: String) -> Result<Option<Reload>, String> {
        if source == self.source {
            return Ok(None);
        }
        let pipelines = crate::shaders::checked(gpu, || pipelines(gpu, &self.layout, &source))?;
        Ok(Some(Reload { pipelines, source }))
    }
    pub(crate) fn commit_reload(&mut self, reload: Reload) {
        self.pipelines = reload.pipelines;
        self.source = reload.source;
    }

    pub(crate) fn update_instances(&self, first: usize, instances: &[GpuInstance]) {
        let mut owners = self.owners.lock().unwrap();
        for (&owner, value) in owners.iter_mut() {
            if let Some(offset) = (owner as usize).checked_sub(first) {
                if let Some(instance) = instances.get(offset) {
                    *value = *instance;
                }
            }
        }
    }

    pub(crate) fn render(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        view_index: usize,
        camera: &Camera,
        scene_bind: &wgpu::BindGroup,
        bindings: &[(u32, wgpu::BindingResource<'_>)],
        hdr: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
        motion: bool,
        excluded_categories: u32,
    ) {
        let owners=self.owners.lock().unwrap();
        let indices = sorted_indices(&self.sort_triangles, &owners, camera, view_index, excluded_categories);
        drop(owners);
        let mut orders = self.orders.lock().unwrap();
        while orders.len() <= view_index {
            let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("transparent view order"),
                size: (self.sort_triangles.len() as u64 * 4).max(4),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("transparent triangles"),
                layout: &self.geometry_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.triangles.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            });
            orders.push(OrderBuffer { buffer, bind });
        }
        let order = &orders[view_index];
        if !indices.is_empty() {gpu.queue.write_buffer(&order.buffer, 0, bytemuck::cast_slice(&indices));}
        let view_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transparent view"),
            layout: &self.view_layout,
            entries: &bindings
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.clone(),
                })
                .collect::<Vec<_>>(),
        });
        let mut motion_targets=self.motion_targets.lock().unwrap();
        while motion_targets.len()<=view_index {motion_targets.push(None);}
        let size=hdr.texture().size();
        if motion && motion_targets[view_index].as_ref().is_none_or(|v|v.texture().size()!=size) {
            let tex=gpu.device.create_texture(&wgpu::TextureDescriptor {label:Some("transparent motion owner"),size,
                mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rg32Uint,
                usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[]});
            motion_targets[view_index]=Some(tex.create_view(&Default::default()));
        }
        let mut attachments=vec![Some(wgpu::RenderPassColorAttachment {
            view:hdr,resolve_target:None,depth_slice:None,ops:wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store},
        })];
        if motion {attachments.push(Some(wgpu::RenderPassColorAttachment {
            view:motion_targets[view_index].as_ref().unwrap(),resolve_target:None,depth_slice:None,
            ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store},
        }));}
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transparency"),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipelines[usize::from(motion)]);
        pass.set_bind_group(0, scene_bind, &[]);
        pass.set_bind_group(1, &view_bind, &[]);
        pass.set_bind_group(2, &order.bind, &[]);
        pass.draw(0..3, 0..indices.len() as u32);
    }

    pub(crate) fn motion_layer(&self,view:usize)->Option<wgpu::TextureView> {
        self.motion_targets.lock().unwrap().get(view).and_then(Clone::clone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_tracks_updated_owner_pose_and_keeps_equal_depth_stable() {
        let camera = Camera {
            pos: Vec3::ZERO,
            target: Vec3::NEG_Z,
            fov_y: 1.0,
            near: 0.1,
        };
        let triangles = [
            SortTriangle {
                owner: 7,
                center: Vec3::new(0.0, 0.0, -2.0),
            },
            SortTriangle {
                owner: 3,
                center: Vec3::new(0.0, 0.0, -4.0),
            },
            SortTriangle {
                owner: 3,
                center: Vec3::new(0.0, 0.0, -4.0),
            },
        ];
        let identity = GpuInstance {
            rows: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            sphere: [0.0; 4],
            first_cluster: 0,
            cluster_count: 1,
            scale: 1.0,
            flags: 0,
        };
        let mut owners = HashMap::from([(7, identity), (3, identity)]);
        assert_eq!(sorted_indices(&triangles, &owners, &camera, 0, 0), vec![1, 2, 0]);
        owners.get_mut(&7).unwrap().rows[2][3] = -8.0;
        assert_eq!(sorted_indices(&triangles, &owners, &camera, 0, 0), vec![0, 1, 2]);
    }

    #[test]
    fn ordering_omits_empty_collapsed_hidden_and_excluded_owners() {
        let camera = Camera { pos: Vec3::ZERO, target: Vec3::NEG_Z, fov_y: 1.0, near: 0.1 };
        let triangles = [SortTriangle { owner: 0, center: Vec3::ZERO }];
        let instance = GpuInstance { cluster_count: 1, scale: 1.0, ..GpuInstance::zeroed() };
        let mut owners = HashMap::from([(0, instance)]);
        let visible = |owners: &HashMap<u32, GpuInstance>, view, categories| {
            sorted_indices(&triangles, owners, &camera, view, categories)
        };
        assert_eq!(visible(&owners, 0, 0), vec![0]);
        owners.get_mut(&0).unwrap().cluster_count = 0;
        assert!(visible(&owners, 0, 0).is_empty());
        owners.get_mut(&0).unwrap().cluster_count = 1;
        owners.get_mut(&0).unwrap().scale = 0.0;
        assert!(visible(&owners, 0, 0).is_empty());
        owners.get_mut(&0).unwrap().scale = 1.0;
        owners.get_mut(&0).unwrap().hide_in_views(1 << 1);
        assert_eq!(visible(&owners, 0, 0), vec![0]);
        assert!(visible(&owners, 1, 0).is_empty());
        owners.get_mut(&0).unwrap().set_category(crate::scene::GeometryCategory::Cars);
        assert!(visible(&owners, 0, crate::scene::GeometryCategory::Cars.exclusion_bit()).is_empty());
        owners.get_mut(&0).unwrap().hide_in_views(0);
        assert_eq!(visible(&owners, 1, 0), vec![0]);
    }

    #[test]
    fn blended_triangle_layout_is_two_vec4_slots() {
        assert_eq!(std::mem::size_of::<Triangle>(), 32);
    }

    #[test]
    fn blend_geometry_preserves_badges_and_glass_but_excludes_opaque_and_lod_copies() {
        use crate::cluster::{ClusterMesh, GpuCluster, Vertex};
        use crate::scene::{BLEND, EMISSIVE, Material, NO_AMBIENT_OCCLUSION};
        let mut scene = Scene::default();
        let material = Material {
            tint: [1.0; 3],
            layer: -1,
            roughness: 0.2,
            flags: 0,
            scale: 1.0,
            metal: 0.0,
        };
        let opaque = scene.add_material(material);
        let glass = scene.add_material(material);
        let badge = scene.add_material(material);
        let energy = scene.add_material(Material { flags: EMISSIVE, ..material });
        scene.set_material_blend(glass, 0.8, true);
        scene.set_material_blend(badge, 1.0, false);
        scene.set_material_additive(energy, 0.2, true);
        let vertices = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
            .map(|pos| Vertex {
                pos,
                normal: [0.0, 0.0, 1.0],
                uv: [0.0; 2],
            })
            .to_vec();
        let clusters = [(opaque, 0.0), (glass, 0.0), (badge, 0.0), (energy, 0.0), (glass, 0.1)]
            .map(|(material, own_error)| GpuCluster {
                tri_count: 1,
                material,
                own_error,
                ..GpuCluster::zeroed()
            })
            .to_vec();
        let mesh = scene.add_mesh(&ClusterMesh {
            vertices,
            cluster_verts: vec![0, 1, 2],
            tris: vec![0 | (1 << 8) | (2 << 16)],
            clusters,
            sphere: [0.0, 0.0, 0.0, 2.0],
            levels: 2,
        });
        scene.add_instance(mesh, glam::Affine3A::IDENTITY);
        // Collapsed pool entries still need their geometry registered at startup.
        scene.add_instance(mesh, glam::Affine3A::from_scale(Vec3::ZERO));
        let (triangles, _, owners) = source_triangles(&scene);
        assert_eq!(triangles.len(), 6);
        assert_eq!(owners.len(), 2);
        for group in triangles.chunks_exact(3) {
            assert_eq!(
                (group[0].material, group[0].alpha, group[0].double_sided, group[0].additive),
                (glass, 0.8, 1, 0)
            );
            assert_eq!(
                (group[1].material, group[1].alpha, group[1].double_sided, group[1].additive),
                (badge, 1.0, 0, 0)
            );
            assert_eq!(
                (group[2].material, group[2].alpha, group[2].double_sided, group[2].additive),
                (energy, 0.2, 1, 1)
            );
        }
        assert_eq!(
            scene.materials[glass as usize].flags & (BLEND | NO_AMBIENT_OCCLUSION),
            BLEND | NO_AMBIENT_OCCLUSION
        );
        assert_eq!(scene.materials[opaque as usize].flags & BLEND, 0);
        assert_eq!(scene.materials[energy as usize].flags, EMISSIVE | BLEND | NO_AMBIENT_OCCLUSION);
        scene.set_material_blend(energy, 2.0, false);
        assert!(!scene.blended_materials[&energy].additive);
        assert_eq!(scene.blended_materials[&energy].alpha, 1.0);
        scene.set_material_additive(energy, -1.0, true);
        assert!(scene.blended_materials[&energy].additive);
        assert_eq!(scene.blended_materials[&energy].alpha, 0.0);
    }
}
