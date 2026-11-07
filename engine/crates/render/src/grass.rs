//! Nearest-layer opacity for live-grass transition bands. Opaque cores stay in
//! the visibility buffer. No CPU sort, readback or per-frame target allocation.
//! Only the nearest covered transition fragment survives; deeper partial layers
//! are deliberately omitted to preserve the frontmost colour at the opaque limit.
//! An alpha/depth prepass resolves that winner before expensive lighting.
//! KERN_GRASS_DEPTH_PREPASS=0 retains the original path for matched diagnostics.
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
    depth_pipeline: Option<wgpu::RenderPipeline>,
    pipeline_layout: wgpu::PipelineLayout,
    source: String,
    selection: wgpu::ComputePipeline,
    selection_layout: wgpu::PipelineLayout,
    selection_source: String,
    selection_scene: wgpu::BindGroup,
    selection_view_layout: wgpu::BindGroupLayout,
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
        "{}\n{}\n{}\n{}",
        crate::shaders::game_mask(),
        crate::wind::common_source(),
        include_str!("shaders/grass_common.wgsl"),
        concat!(include_str!("shaders/virtual_member.wgsl"), "\n", include_str!("shaders/raster.wgsl"))
    )
}
pub(crate) fn shader_source() -> String {
    format!(
        "{}\n{}\n{}",
        crate::wind::shade_source(),
        include_str!("shaders/grass_common.wgsl"),
        crate::shaders::source("grass_transition.wgsl")
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

pub(crate) struct Reload {
    pipelines: [wgpu::RenderPipeline; 2],
    depth_pipeline: Option<wgpu::RenderPipeline>,
    selection: wgpu::ComputePipeline,
    source: String,
    selection_source: String,
}

fn transition_pipelines(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str, prepass: bool)
    -> ([wgpu::RenderPipeline; 2], Option<wgpu::RenderPipeline>) {
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("grass transition"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let make = |motion: bool, depth_only: bool| {
        let mut targets=if depth_only {vec![]} else {vec![Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,blend:None,write_mask:wgpu::ColorWrites::ALL})]};
        if motion {targets.push(Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rg32Uint,blend:None,write_mask:wgpu::ColorWrites::ALL}));}
        gpu
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if depth_only {"grass alpha depth"} else {"grass nearest transition"}),
        layout: Some(layout),
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
            depth_write_enabled: Some(depth_only || !prepass),
            depth_compare: Some(if prepass && !depth_only {wgpu::CompareFunction::Equal} else {wgpu::CompareFunction::GreaterEqual}),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some(if depth_only {"grass_depth_fs"} else if motion {"grass_motion_fs"}else{"grass_fs"}),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: gpu.pipeline_cache(),
        })
    };
    ([false,true].map(|motion|make(motion,false)), prepass.then(||make(false,true)))
}

fn selection_pipeline(gpu: &Gpu, layout: &wgpu::PipelineLayout, source: &str) -> wgpu::ComputePipeline {
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("grass transition selection"), source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("grass transition selection"), layout: Some(layout), module: &module,
        entry_point: Some("grass_compact"), compilation_options: Default::default(), cache: gpu.pipeline_cache(),
    })
}

// Selection never shades a plant. Keep its compute-visible descriptors
// independent of material, GI, track and shadow-page scene dependencies.
fn selection_scene_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    [4,12].map(|binding| wgpu::BindGroupLayoutEntry {
        binding, visibility:wgpu::ShaderStages::COMPUTE,
        ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {read_only:true},
            has_dynamic_offset:false,min_binding_size:None},count:None,
    })
}
fn selection_view_entries() -> [wgpu::BindGroupLayoutEntry; 1] {
    [wgpu::BindGroupLayoutEntry {binding:0,visibility:wgpu::ShaderStages::COMPUTE,
        ty:wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Uniform,
            has_dynamic_offset:false,min_binding_size:None},count:None}]
}

impl GrassRenderer {
    pub(crate) fn new(
        gpu: &Gpu,
        scene: &Scene,
        scene_layout: &wgpu::BindGroupLayout,
        instances: &wgpu::Buffer,
        wind: &wgpu::Buffer,
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
        let source = shader_source();
        // Accepted four-view 4K-output DLSS Quality ABBA comparison. Both
        // paths retain the exact same compacted draw order and coverage;
        // constructor-only opt-out preserves the original measurement path.
        let prepass=!std::env::var("KERN_GRASS_DEPTH_PREPASS").is_ok_and(|v|v=="0");
        let (pipelines, depth_pipeline) = transition_pipelines(gpu, &layout, &source, prepass);
        let pipeline_layout = layout;
        let selection_scene_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:Some("grass selection instances and wind"),entries:&selection_scene_entries(),
        });
        let selection_scene=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:Some("grass selection instances and wind"),layout:&selection_scene_layout,
            entries:&[wgpu::BindGroupEntry {binding:4,resource:instances.as_entire_binding()},
                wgpu::BindGroupEntry {binding:12,resource:wind.as_entire_binding()}],
        });
        let selection_view_layout=gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:Some("grass selection camera"),entries:&selection_view_entries(),
        });
        let selection_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:Some("grass transition selection"),
            bind_group_layouts:&[Some(&selection_scene_layout),Some(&selection_view_layout),Some(&range_layout),Some(&compact.selection_layout)],
            immediate_size:0,
        });
        let selection_source = compact_source();
        let selection = selection_pipeline(gpu, &selection_layout, &selection_source);
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
                cache: gpu.pipeline_cache(),
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
            depth_pipeline,
            pipeline_layout,
            source,
            selection,
            selection_layout,
            selection_source,
            selection_scene,
            selection_view_layout,
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

    pub(crate) fn prepare_reload(&self, gpu: &Gpu, source: String, selection_source: String) -> Result<Option<Reload>, String> {
        if source == self.source && selection_source == self.selection_source { return Ok(None); }
        let ((pipelines, depth_pipeline), selection) = crate::shaders::checked(gpu, || (
            transition_pipelines(gpu, &self.pipeline_layout, &source, self.depth_pipeline.is_some()),
            selection_pipeline(gpu, &self.selection_layout, &selection_source),
        ))?;
        Ok(Some(Reload { pipelines, depth_pipeline, selection, source, selection_source }))
    }
    pub(crate) fn commit_reload(&mut self, reload: Reload) {
        self.pipelines = reload.pipelines;
        self.depth_pipeline = reload.depth_pipeline;
        self.selection = reload.selection;
        self.source = reload.source;
        self.selection_source = reload.selection_source;
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
        // Reuse the exact per-view uniform used by the transition draw. Scene
        // streaming and GPU wind update the same two retained live buffers.
        let selection_view=gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:Some("grass selection camera"),layout:&self.selection_view_layout,
            entries:&[wgpu::BindGroupEntry {binding:0,resource:bindings.iter()
                .find(|(binding,_)|*binding==0).expect("grass view uniform missing").1.clone()}],
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
            pass.set_bind_group(0,&self.selection_scene,&[]);pass.set_bind_group(1,&selection_view,&[]);
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
        if let Some(pipeline)=&self.depth_pipeline {
            // Alpha-only visibility resolution. Lighting below sees the final
            // frontmost depth, so hidden covered fragments do not run the sun,
            // sky, contact or grass-extinction queries. No extra texture or
            // per-view selection is required; retain equality and original
            // owner order so coincident cards keep the same last-writer winner.
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label:Some("grass alpha depth"),color_attachments:&[],
                depth_stencil_attachment:Some(wgpu::RenderPassDepthStencilAttachment {
                    view:&target.depth_view,
                    depth_ops:Some(wgpu::Operations {load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}),
                    stencil_ops:None,
                }),timestamp_writes:None,occlusion_query_set:None,multiview_mask:None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0,scene,&[]);pass.set_bind_group(1,&view_bind,&[]);
            for (range,list) in self.ranges.iter().zip(lists) {
                pass.set_bind_group(2,&range.bind,&[]);pass.set_bind_group(3,&list.draw,&[]);
                pass.draw_indirect(&list.indirect,0);
            }
        }
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
    fn grass_depth_entry_has_only_photo_material_dependencies() {
        // An inexpensive visibility prepass must not accidentally call shared
        // lighting helpers when a shader reload changes the material chain.
        for source in [shader_source(),crate::shaders::Snapshot::load().unwrap().lighting().grass] {
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let entry=module.entry_points.iter().position(|e|e.name=="grass_depth_fs").unwrap();
            let uses=info.get_entry_point(entry);
            let mut used=Vec::new();
            for (handle,global) in module.global_variables.iter() {
                if !uses[handle].is_empty() {used.push(global.name.as_deref().unwrap());}
            }
            used.sort_unstable();
            assert_eq!(used,["albedo","grass_range","plant_samp","scene_tables","view"]);
            assert!(module.entry_points[entry].function.result.is_none());
        }
    }
    #[test]
    fn grass_depth_prepass_preserves_nearest_rgba_motion_and_equal_depth_ties() {
        // Exhaust all 4-layer depth/coverage arrangements. Include clipped
        // texels, the exact cutoff, opaque-depth equality and both source draw
        // orders; final owner stands for colour AND packed motion identity.
        let values=[(0.1f32,1.0f32),(0.4,0.49999),(0.4,0.5),(0.6,1.0),(0.6,0.5)];
        for code in 0..625 {
            let mut n=code;
            let layers:Vec<_>=(0..4).map(|id|{let (z,a)=values[n%values.len()];n/=values.len();(z,a,id)}).collect();
            for opaque in [0.0,0.4,0.6,0.8] {
                for reverse in [false,true] {
                    let mut ordered=layers.clone();if reverse {ordered.reverse();}
                    let mut baseline_depth=opaque;let mut baseline=None;
                    for &(z,a,id) in &ordered {if a>=0.5&&z>=baseline_depth {baseline_depth=z;baseline=Some(id);}}
                    let final_depth=ordered.iter().filter(|&&(_,a,_)|a>=0.5)
                        .fold(opaque,|depth,&(z,_,_)|depth.max(z));
                    let candidate=ordered.iter().filter(|&&(z,a,_)|a>=0.5&&z==final_depth).last().map(|&(_,_,id)|id);
                    assert_eq!(candidate,baseline,"opaque={opaque} layers={ordered:?}");
                    assert_eq!(final_depth,baseline_depth);
                }
            }
        }
    }
    #[test]
    fn grass_selection_entrypoint_and_layout_fit_portable_storage_budget() {
        let sources=[compact_source(),crate::shaders::Snapshot::load().unwrap().lighting().grass_selection];
        for source in sources {
            let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
                .validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
            let entry=module.entry_points.iter().position(|e|e.name=="grass_compact").unwrap();
            let uses=info.get_entry_point(entry);
            let mut used_bindings=Vec::new();let mut used_storage=0;let mut compact_layout_storage=0;
            for (handle,global) in module.global_variables.iter() {
                let Some(binding)=global.binding.as_ref() else {continue;};
                let storage=matches!(global.space,naga::AddressSpace::Storage {..});
                // Finalization shares the compactor layout: its four storage
                // bindings all count against the selector pipeline limit,
                // even though selection itself writes just ranks and blocks.
                if binding.group==3 && storage {compact_layout_storage+=1;}
                if uses[handle].is_empty(){continue;}
                used_bindings.push((binding.group,binding.binding));
                used_storage+=usize::from(storage);
                let entries=match binding.group {0=>Some(selection_scene_entries().to_vec()),
                    1=>Some(selection_view_entries().to_vec()),_=>None};
                if let Some(entries)=entries {
                    let supplied=entries.iter().find(|e|e.binding==binding.binding)
                        .expect("selection added an undeclared scene/view dependency");
                    assert_eq!(supplied.visibility,wgpu::ShaderStages::COMPUTE);
                    assert_eq!(matches!(supplied.ty,wgpu::BindingType::Buffer {
                        ty:wgpu::BufferBindingType::Storage {..},..}),storage);
                }
            }
            used_bindings.sort_unstable();
            assert_eq!(used_bindings,[(0,4),(0,12),(1,0),(3,0),(3,1),(3,2)]);
            assert_eq!(used_storage,4);
            let layout_storage=selection_scene_entries().into_iter().chain(selection_view_entries())
                .filter(|e|matches!(e.ty,wgpu::BindingType::Buffer {ty:wgpu::BufferBindingType::Storage {..},..})).count()
                +compact_layout_storage;
            assert_eq!(layout_storage,6);
            assert!(layout_storage<=8,"selection must not depend on the raised material-pass storage limit");
        }
    }
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

#[cfg(test)]
mod game_visibility_tests {
    #[test]
    fn default_game_mask_keeps_raster_shader_valid() {
        let source=super::raster_source();
        let module=naga::front::wgsl::parse_str(&source).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap();
    }
}
