//! A small offscreen target for a live display's alternate crowd camera.
use crate::{Camera, Gpu};
use glam::{Mat4, Vec3};
use std::{ops::Range, sync::Arc};
use wgpu::util::DeviceExt;

pub struct BroadcastShot {
    pub camera: Camera,
    pub size: [u32; 2],
    /// A seat range renders the audience. None renders the arena geometry.
    pub seats: Option<Range<u32>>,
    /// World-space backdrop triangles with linear display colour.
    pub backdrop: Vec<([f32; 3], [f32; 3])>,
}
impl BroadcastShot {
    pub(crate) fn matrices(&self) -> (Mat4, Mat4) {
        let c = self.camera;
        let view = Mat4::look_at_rh(c.pos, c.target, Vec3::Y);
        let projection = Mat4::perspective_infinite_reverse_rh(
            c.fov_y,
            self.size[0] as f32 / self.size[1] as f32,
            c.near,
        );
        (projection * view, view)
    }
}

struct Target {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    binding: wgpu::BindGroup,
}

pub(crate) struct Cutaway {
    pub shot: Option<(u32, Arc<BroadcastShot>, u32)>,
    rendered: Option<(Arc<BroadcastShot>, u32)>,
    pub revision: u64,
    target: Option<Target>,
    backdrop_pipeline: Option<wgpu::RenderPipeline>,
    backdrop: Option<wgpu::Buffer>,
}
impl Cutaway {
    pub fn new() -> Self {
        Self {
            shot: None,
            rendered: None,
            revision: 0,
            target: None,
            backdrop_pipeline: None,
            backdrop: None,
        }
    }
    pub fn pending(&self) -> Option<Arc<BroadcastShot>> {
        let (_, shot, tick) = self.shot.as_ref()?;
        self.rendered
            .as_ref()
            .is_none_or(|(old, t)| !Arc::ptr_eq(old, shot) || tick != t)
            .then(|| shot.clone())
    }
    pub fn binding(&self, layer: u32) -> Option<&wgpu::BindGroup> {
        let (id, shot, _) = self.shot.as_ref()?;
        let (ready, _) = self.rendered.as_ref()?;
        (*id == layer && Arc::ptr_eq(shot, ready)).then(|| &self.target.as_ref().unwrap().binding)
    }
    pub fn begin(
        &mut self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
    ) -> (&wgpu::TextureView, &wgpu::TextureView) {
        let (_, shot, tick) = self.shot.as_ref().unwrap();
        if self.target.as_ref().is_none_or(|t| t.size != shot.size) {
            let tex = |format, usage| {
                gpu.device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("broadcast camera"),
                        size: wgpu::Extent3d {
                            width: shot.size[0],
                            height: shot.size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&Default::default())
            };
            let color = tex(
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let depth = tex(
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            );
            let binding = super::bind(gpu, layout, sampler, &color, uniform);
            self.target = Some(Target {
                size: shot.size,
                color,
                depth,
                binding,
            });
        }
        if shot.seats.is_none() {
            // The scene raster clears these attachments itself.
            self.revision = self.revision.wrapping_add(1);
            self.rendered = Some((shot.clone(), *tick));
            let target = self.target.as_ref().unwrap();
            return (&target.color, &target.depth);
        }
        let pipeline = self.backdrop_pipeline.get_or_insert_with(|| {
            let module = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("TV backdrop"),
                    source: wgpu::ShaderSource::Wgsl(
                        r#"
struct V { @builtin(position) clip:vec4f, @location(0) color:vec4f }
@vertex fn vs(@location(0) clip:vec4f,@location(1) color:vec4f)->V { return V(clip,color); }
@fragment fn fs(v:V)->@location(0) vec4f { return v.color; }
"#
                        .into(),
                    ),
                });
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("TV backdrop"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[Some(wgpu::VertexBufferLayout {
                            array_stride: 32,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4],
                        })],
                    },
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Greater),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba16Float.into())],
                    }),
                    multiview_mask: None,
                    cache: gpu.pipeline_cache(),
                })
        });
        if self
            .rendered
            .as_ref()
            .is_none_or(|(old, _)| !Arc::ptr_eq(old, shot))
        {
            let (vp, _) = shot.matrices();
            let vertices: Vec<_> = shot
                .backdrop
                .iter()
                .map(|&(p, c)| {
                    [
                        (vp * Vec3::from(p).extend(1.)).to_array(),
                        [c[0], c[1], c[2], 1.],
                    ]
                })
                .collect();
            self.backdrop = (!vertices.is_empty()).then(|| {
                gpu.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("TV steps"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
            });
        }
        let target = self.target.as_ref().unwrap();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("TV terraces"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.045,
                        g: 0.048,
                        b: 0.056,
                        a: 1.,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(vertices) = &self.backdrop {
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..shot.backdrop.len() as u32, 0..1);
        }
        drop(pass);
        self.revision = self.revision.wrapping_add(1);
        self.rendered = Some((shot.clone(), *tick));
        (&target.color, &target.depth)
    }
}
