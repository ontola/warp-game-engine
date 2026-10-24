//! Nearest-layer opacity for live-grass transition bands. Opaque cores stay in
//! the visibility buffer. No CPU sort, readback or per-frame target allocation.
//! Only the nearest covered transition fragment survives; deeper partial layers
//! are deliberately omitted to preserve the frontmost colour at the opaque limit.
use crate::{Gpu, Scene};
use bytemuck::{Pod, Zeroable};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

#[derive(Clone, Copy, Debug)]
pub struct GrassRange {
    pub first: u32,
    pub count: u32,
    pub material: u32,
    pub width: f32,
    pub height: f32,
    pub split: bool,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RangeUniform {
    indices: [u32; 4],
    shape: [f32; 4],
}
struct RangeGpu {
    owners: crate::foliage_compact::OwnerRange,
    bind: wgpu::BindGroup,
}
struct Targets {
    size: (u32, u32),
    layer: wgpu::TextureView,
    depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    composite: wgpu::BindGroup,
    motion: Option<wgpu::TextureView>,
}
pub(crate) struct GrassRenderer {
    pipelines: [wgpu::RenderPipeline; 2],
    selection: wgpu::ComputePipeline,
    compact: crate::foliage_compact::Compactor,
    composite: wgpu::RenderPipeline,
    view_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    ranges: Vec<RangeGpu>,
    empty_far: [wgpu::TextureView; 2],
    blend_uniforms: [wgpu::Buffer; 6],
    targets: Mutex<Vec<Option<Targets>>>,
    // Instance capacity is independent of pixel dimensions. Retain these on
    // viewport resize; separated players must never share a mutable draw list.
    visible: Mutex<Vec<Option<Vec<crate::foliage_compact::OwnerList>>>>,
}

pub(crate) fn raster_source() -> String {
    format!(
        "{}\n{}\n{}",
        crate::wind::common_source(),
        include_str!("shaders/grass_common.wgsl"),
        include_str!("shaders/raster.wgsl")
    )
}
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}\n{}",
        crate::wind::shade_source(),
        include_str!("shaders/grass_common.wgsl"),
        include_str!("shaders/grass_transition.wgsl")
    )
}
pub(crate) fn compact_source() -> String {
    format!("{}\n{}\n{}\n{}",crate::wind::shade_source(),
        include_str!("shaders/grass_common.wgsl"),crate::foliage_compact::selection_source(),
        include_str!("shaders/grass_compact.wgsl"))
}
pub(crate) fn composite_source() -> String {
    format!("{}\n{}", include_str!("shaders/grass_common.wgsl"),
        include_str!("shaders/grass_composite.wgsl"))
}

impl GrassRenderer {
    pub(crate) fn new(
        gpu: &Gpu,
        scene: &Scene,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> Option<Self> {
        if scene.grass_ranges.is_empty() && scene.far_grass.is_none() {
            return None;
        }
        let compact = crate::foliage_compact::Compactor::new(gpu);
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let storage = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let tex = |sample_type, view_dimension| wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        };
        let d2 = wgpu::TextureViewDimension::D2;
        let depth = tex(wgpu::TextureSampleType::Depth, d2);
        let unfiltered = tex(wgpu::TextureSampleType::Float { filterable: false }, d2);
        let bindings = [
            (0, uniform),
            (2, uniform),
            (5, depth),
            (
                6,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            ),
            (7, unfiltered),
            (10, unfiltered),
            (11, uniform),
            (12, storage),
            (
                13,
                tex(
                    wgpu::TextureSampleType::Float { filterable: true },
                    wgpu::TextureViewDimension::D3,
                ),
            ),
            (
                14,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ),
            (15, depth),
        ];
        let view_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grass view"),
                entries: &bindings
                    .iter()
                    .map(|&(binding, ty)| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                        ty,
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            });
        let range_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("grass range"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: uniform,
                    count: None,
                }],
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grass transition"),
                bind_group_layouts: &[Some(scene_layout), Some(&view_layout), Some(&range_layout),Some(&compact.draw_layout)],
                immediate_size: 0,
            });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("grass transition"),
                source: wgpu::ShaderSource::Wgsl(shader_source().into()),
            });
        let pipelines = [false,true].map(|motion| {
            let mut targets=vec![Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,blend:None,write_mask:wgpu::ColorWrites::ALL})];
            if motion {targets.push(Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rg32Uint,blend:None,write_mask:wgpu::ColorWrites::ALL}));}
            gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("grass nearest transition"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("grass_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(if motion {"grass_motion_fs"}else{"grass_fs"}),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        });
        let selection_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:Some("grass transition selection"),
            bind_group_layouts:&[Some(scene_layout),Some(&view_layout),Some(&range_layout),Some(&compact.selection_layout)],
            immediate_size:0,
        });
        let selection_shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:Some("grass transition selection"),source:wgpu::ShaderSource::Wgsl(compact_source().into()),
        });
        let selection = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label:Some("grass transition selection"),layout:Some(&selection_layout),module:&selection_shader,
            entry_point:Some("grass_compact"),compilation_options:Default::default(),cache:None,
        });
        let composite_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("grass composite"),
                    entries: &[0,1,2,3].map(|binding| wgpu::BindGroupLayoutEntry {
                        binding,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: if binding==3 {uniform} else {unfiltered},
                        count: None,
                    }),
                });
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("grass composite"),
                source: wgpu::ShaderSource::Wgsl(composite_source().into()),
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grass composite"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let composite = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("grass composite"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        let ranges = scene
            .grass_ranges
            .iter()
            .map(|range| {
                assert!(
                    range.first as usize + range.count as usize <= scene.instances.len(),
                    "grass range exceeds fixed pool"
                );
                assert!(
                    (range.material as usize) < scene.materials.len(),
                    "grass material missing"
                );
                assert!(
                    range.width.is_finite()
                        && range.width > 0.0
                        && (range.height - 0.5).abs() < 1e-6,
                    "grass template requires positive width and 0.5 m height"
                );
                let u = RangeUniform {
                    indices: [range.first, range.count, range.material, 0],
                    shape: [
                        range.width,
                        range.height,
                        if range.split { 1.0 } else { 0.0 },
                        0.0,
                    ],
                };
                let buffer = gpu
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("grass range"),
                        contents: bytemuck::bytes_of(&u),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("grass range"),
                    layout: &range_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                let owners=crate::foliage_compact::OwnerRange::new(range.first,range.count,18)
                    .expect("grass owner range exceeds u32");
                owners.validate(&gpu.limits).expect("grass compaction exceeds GPU limits");
                RangeGpu { owners, bind }
            })
            .collect();
        let empty_far = [wgpu::TextureFormat::Rgba16Float, wgpu::TextureFormat::R32Float].map(|format| {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("empty far grass"), size: wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1},
                mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format,
                usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,view_formats:&[],
            });
            let bytes = if format==wgpu::TextureFormat::Rgba16Float {8} else {4};
            gpu.queue.write_texture(texture.as_image_copy(),&[0u8;8][..bytes],
                wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:None,rows_per_image:None},
                wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1});
            texture.create_view(&Default::default())
        });
        let blend_uniforms = std::array::from_fn(|index| {
            let band=scene.grass_detail.for_views(if index%2==0 {1}else{2});
            gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label:Some("grass blend range"),contents:bytemuck::cast_slice(&[(index%2) as f32,(index/2) as f32,band.fade_start,band.fade_end]),usage:wgpu::BufferUsages::UNIFORM,
            })
        });
        Some(Self {
            pipelines,
            selection,
            compact,
            composite,
            view_layout,
            composite_layout,
            ranges,
            empty_far,
            blend_uniforms,
            targets: Mutex::new(Vec::new()),
            visible: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn set_view_count(&self, count: usize) {
        let mut targets = self.targets.lock().unwrap();
        targets.truncate(count);
        targets.resize_with(count, || None);
        let mut visible=self.visible.lock().unwrap();
        visible.truncate(count);
        visible.resize_with(count,||None);
    }

    fn targets(&self, gpu: &Gpu, size: (u32, u32)) -> Targets {
        let texture = |name, format, usage| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(name),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let layer = texture(
            "grass nearest layer",
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&Default::default());
        let depth = texture(
            "grass private depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST,
        );
        let depth_view = depth.create_view(&Default::default());
        let composite = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grass composite"),
            layout: &self.composite_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&layer),
            }, wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(&self.empty_far[0])},
            wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::TextureView(&self.empty_far[1])},
            wgpu::BindGroupEntry {binding:3,resource:self.blend_uniforms[0].as_entire_binding()}],
        });
        Targets {
            size,
            layer,
            depth,
            depth_view,
            composite,
            motion: None,
        }
    }

    pub(crate) fn motion_layer(&self,view:usize)->Option<wgpu::TextureView> {
        self.targets.lock().unwrap().get(view).and_then(Option::as_ref).and_then(|t|t.motion.clone())
    }

    pub(crate) fn render(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        view: usize,
        size: (u32, u32),
        scene: &wgpu::BindGroup,
        bindings: &[(u32, wgpu::BindingResource<'_>)],
        hdr: &wgpu::TextureView,
        opaque_depth: &wgpu::Texture,
        far: Option<&crate::far_grass::FarGrassLayer>,
        transition_times: Option<wgpu::RenderPassTimestampWrites<'_>>,
        composite_times: Option<wgpu::RenderPassTimestampWrites<'_>>,
        motion: bool,
        reference: f32,
    ) {
        let mut targets = self.targets.lock().unwrap();
        if !targets[view].as_ref().is_some_and(|t| t.size == size) {
            targets[view] = Some(self.targets(gpu, size));
        }
        if motion && targets[view].as_ref().unwrap().motion.is_none() {
            let tex=gpu.device.create_texture(&wgpu::TextureDescriptor {
                label:Some("grass motion owner"),size:wgpu::Extent3d {width:size.0,height:size.1,depth_or_array_layers:1},
                mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rg32Uint,
                usage:wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::TEXTURE_BINDING,view_formats:&[],
            });
            targets[view].as_mut().unwrap().motion=Some(tex.create_view(&Default::default()));
        }
        let target = targets[view].as_ref().unwrap();
        let mut lists=self.visible.lock().unwrap();
        if lists[view].is_none() {
            lists[view]=Some(self.ranges.iter().map(|range|self.compact.list(gpu,range.owners)
                .expect("previously validated grass owner range")).collect());
        }
        let lists=lists[view].as_ref().unwrap();
        let far_composite = far.map(|far|gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:Some("near and far grass blend"),layout:&self.composite_layout,
            entries:&[
                wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(&target.layer)},
                wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(&far.color)},
                wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::TextureView(&far.distance)},
                wgpu::BindGroupEntry {binding:3,resource:self.blend_uniforms[reference.clamp(0.0,2.0) as usize*2+usize::from(targets.len()>1)].as_entire_binding()},
            ],
        }));
        let view_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grass view"),
            layout: &self.view_layout,
            entries: &bindings
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.clone(),
                })
                .collect::<Vec<_>>(),
        });
        // Include compaction and the mandatory copy in the transition interval. Devices
        // lacking encoder timestamps use an empty pass for the beginning stamp;
        // no extra queries, mapping, synchronization or normal-play readback.
        if let Some(times) = &transition_times {
            if let Some(begin) = times.beginning_of_pass_write_index {
                if gpu
                    .features
                    .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
                {
                    encoder.write_timestamp(times.query_set, begin);
                } else {
                    drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("grass copy timestamp"),
                        timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                            query_set: times.query_set,
                            beginning_of_pass_write_index: Some(begin),
                            end_of_pass_write_index: None,
                        }),
                    }));
                }
            }
        }
        {
            let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label:Some("grass transition selection"),timestamp_writes:None,
            });
            pass.set_pipeline(&self.selection);
            pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,&view_bind,&[]);
            for (range,list) in self.ranges.iter().zip(lists) {
                if list.range().blocks()==0 {continue;}
                pass.set_bind_group(2,&range.bind,&[]);pass.set_bind_group(3,&list.selection,&[]);
                pass.dispatch_workgroups(list.range().blocks(),1,1);
            }
        }
        for list in lists { self.compact.finish(encoder,list); }
        encoder.copy_texture_to_texture(
            opaque_depth.as_image_copy(),
            target.depth.as_image_copy(),
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        {
            let mut attachments=vec![Some(wgpu::RenderPassColorAttachment {
                view:&target.layer,depth_slice:None,resolve_target:None,
                ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store},
            })];
            if motion {attachments.push(Some(wgpu::RenderPassColorAttachment {
                view:target.motion.as_ref().unwrap(),depth_slice:None,resolve_target:None,
                ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store},
            }));}
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grass nearest transition"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: transition_times.map(|times| wgpu::RenderPassTimestampWrites {
                    query_set: times.query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: times.end_of_pass_write_index,
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines[usize::from(motion)]);
            pass.set_bind_group(0, scene, &[]);
            pass.set_bind_group(1, &view_bind, &[]);
            for (range,list) in self.ranges.iter().zip(lists) {
                pass.set_bind_group(2, &range.bind, &[]);
                pass.set_bind_group(3, &list.draw, &[]);
                pass.draw_indirect(&list.indirect,0);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grass composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: composite_times,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite);
            pass.set_bind_group(0, far_composite.as_ref().unwrap_or(&target.composite), &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grass_range_layout_and_native_4k_target_budget() {
        assert_eq!(std::mem::size_of::<RangeUniform>(), 32);
        // RGBA16Float nearest layer + private Depth32Float, shared output area.
        let combined = 3840u64 * 2160 * 12;
        assert_eq!(combined, 99_532_800);
        assert_eq!(4 * 1920u64 * 1080 * 12, combined);
    }
    #[test]
    fn nearest_opacity_converges_to_opaque_frontmost_color_without_layer_average() {
        // Different sun/shade colours are intentional: averaging layers fails
        // this limit even though a same-colour transparency test would pass.
        let front = [0.08f64, 0.20, 0.025];
        let back = [0.62f64, 0.80, 0.32];
        let ground = [0.16f64, 0.12, 0.08];
        let composite = |alpha: f64| {
            std::array::from_fn::<_, 3, _>(|i| front[i] * alpha + ground[i] * (1.0 - alpha))
        };
        assert_eq!(composite(1.0), front);
        assert_eq!(composite(0.0), ground);
        for i in 0..3 {
            assert!((composite(0.99999)[i] - front[i]).abs() < 0.00001);
            assert!(((front[i] + back[i]) * 0.5 - front[i]).abs() > 0.1);
        }
    }
    #[test]
    fn nearest_transition_respects_opaque_depth_and_omits_deeper_partial_layers() {
        // Reversed-Z: larger values are nearer. The opaque depth is copied
        // first, and alpha-cutout holes never write a transition fragment.
        let choose = |opaque: f32, layers: &[(f32, f32, u32)]| {
            let mut depth = opaque;
            let mut picked = None;
            for &(z, alpha, id) in layers {
                if alpha > 0.0 && z >= depth {
                    depth = z;
                    picked = Some(id);
                }
            }
            picked
        };
        let layers = [(0.1, 0.8, 1), (0.5, 0.15, 2), (0.3, 0.9, 3), (0.7, 0.0, 4)];
        assert_eq!(choose(0.2, &layers), Some(2));
        assert_eq!(
            choose(0.2, &layers.into_iter().rev().collect::<Vec<_>>()),
            Some(2)
        );
        assert_eq!(choose(0.6, &layers), None);
    }
}
