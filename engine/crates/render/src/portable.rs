//! Portable renderer: procedural terrain and sky for WebGPU devices with
//! fewer than 16 storage buffers per shader stage, which the main pipeline
//! needs (most Macs offer 10, Firefox 9, the spec's minimum is 8). It uses
//! three. Terrain patches are drawn straight from the pool the procedural
//! terrain pass writes; nothing else in the scene is drawn yet. On with
//! such a device, or with `KERN_PORTABLE=1` (renderer-switches.md).
use crate::{Gpu, Lighting, ViewRect, FrameStats};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform { vp: [[f32; 4]; 4], inv_vp: [[f32; 4]; 4], camera: [f32; 4], sun: [f32; 4], sun_color: [f32; 4], horizon: [f32; 4], sky: [f32; 4], pool: [u32; 4] }

struct Target { size: [u32; 2], color: wgpu::Texture, depth: wgpu::Texture, uniform: wgpu::Buffer, bind: wgpu::BindGroup }

pub(crate) struct Portable {
    terrain: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    vertices: wgpu::Buffer,
    instances: wgpu::Buffer,
    materials: wgpu::Buffer,
    albedo: wgpu::TextureView,
    sampler: wgpu::Sampler,
    /// First instance, first vertex, terrain material, slots.
    pool: [u32; 4],
    /// Each slot's bounding sphere while it holds a patch (radius < 0: free).
    spheres: Vec<[f32; 4]>,
    /// This frame's visible slots, one per instance (a vertex buffer).
    visible: wgpu::Buffer,
    targets: Vec<Target>,
}

/// Storage buffers per stage and sampled textures per stage the full
/// renderer needs without trees (every pass fits 8, shade needs 21
/// textures; browser.md). The GPU forest still needs 16 storage buffers.
pub const FULL_STORAGE_BUFFERS: u32 = 8;
pub const FULL_SAMPLED_TEXTURES: u32 = 24;

/// Whether a device with these limits needs the portable renderer.
pub fn needed(limits: &wgpu::Limits) -> bool {
    limits.max_storage_buffers_per_shader_stage < FULL_STORAGE_BUFFERS
        || limits.max_sampled_textures_per_shader_stage < FULL_SAMPLED_TEXTURES
        || std::env::var("KERN_PORTABLE").as_deref() == Ok("1")
}

pub(crate) fn wanted(gpu: &Gpu) -> bool { needed(&gpu.limits) }

const SLOT: [Option<wgpu::VertexBufferLayout<'static>>; 1] = [Some(wgpu::VertexBufferLayout { array_stride: 4, step_mode: wgpu::VertexStepMode::Instance,
    attributes: &wgpu::vertex_attr_array![0 => Uint32] })];
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl Portable {
    pub fn new(gpu: &Gpu, scene: &crate::Scene, spec: &crate::procedural_terrain::TerrainSpec, vertices: &crate::topology::Topology, instances: &wgpu::Buffer,
        materials: &wgpu::Buffer, albedo: &wgpu::Texture, sampler: &wgpu::Sampler) -> Self {
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let storage = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("portable"), entries: &[
            entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            entry(1, wgpu::ShaderStages::VERTEX, storage),
            entry(2, wgpu::ShaderStages::VERTEX, storage),
            entry(3, wgpu::ShaderStages::FRAGMENT, storage),
            entry(4, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
            entry(5, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
        ] });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("portable"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/portable.wgsl").into()) });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("portable"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |label, vs, fs, depth_write| gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some(vs), compilation_options: Default::default(),
                buffers: if vs == "vs_terrain" { &SLOT } else { &[] } },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(depth_write),
                depth_compare: Some(if depth_write { wgpu::CompareFunction::GreaterEqual } else { wgpu::CompareFunction::Always }), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fs), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: COLOR, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache() });
        let terrain = pipeline("portable terrain", "vs_terrain", "fs_terrain", true);
        let sky = pipeline("portable sky", "vs_sky", "fs_sky", false);
        let material = scene.clusters.get(spec.first_cluster as usize).map_or(0, |c| c.material);
        Portable { terrain, sky, layout, vertices: vertices.buffer.clone(), instances: instances.clone(), materials: materials.clone(),
            albedo: albedo.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }),
            sampler: sampler.clone(), pool: [spec.first_instance, spec.first_vertex, material, spec.capacity],
            spheres: vec![[0.0, 0.0, 0.0, -1.0]; spec.capacity as usize], visible: Self::slots(gpu, 1024), targets: Vec::new() }
    }

    fn slots(gpu: &Gpu, count: u64) -> wgpu::Buffer {
        gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("portable visible slots"), size: count * 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    /// Instances the renderer uploads: the pool's slots keep their spheres.
    pub fn update(&mut self, first: usize, instances: &[crate::scene::GpuInstance]) {
        let start = self.pool[0] as usize;
        for (k, i) in instances.iter().enumerate() {
            if let Some(slot) = (first + k).checked_sub(start).filter(|s| *s < self.spheres.len()) {
                self.spheres[slot] = if i.cluster_count > 0 { i.sphere } else { [0.0, 0.0, 0.0, -1.0] };
            }
        }
    }

    fn target(&self, gpu: &Gpu, size: [u32; 2]) -> Target {
        let texture = |label, format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
        let color = texture("portable view", COLOR, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let depth = texture("portable depth", DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("portable view"), size: std::mem::size_of::<Uniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("portable"), layout: &self.layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: self.vertices.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: self.instances.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: self.materials.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&self.albedo) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) },
        ] });
        Target { size, color, depth, uniform, bind }
    }

    /// Draw each view into `output` at its rectangle. `enc` already holds
    /// this frame's procedural terrain build.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, gpu: &Gpu, mut enc: wgpu::CommandEncoder, views: &[ViewRect], matrices: &[Mat4], light: &Lighting, horizon: Vec3, output: &wgpu::Texture) -> FrameStats {
        let mut stats = FrameStats::default();
        let exposure = 1.0 / (1.2 * 2f32.powf(light.ev100));
        for (vi, view) in views.iter().enumerate() {
            let size = [view.w, view.h];
            if self.targets.len() <= vi { self.targets.push(self.target(gpu, size)); }
            if self.targets[vi].size != size { self.targets[vi] = self.target(gpu, size); }
            let vp = matrices[vi];
            // The slots in view (frustum test on each patch's sphere).
            let rows: [glam::Vec4; 4] = std::array::from_fn(|i| vp.row(i));
            let planes = [rows[3] + rows[0], rows[3] - rows[0], rows[3] + rows[1], rows[3] - rows[1], rows[3] - rows[2]];
            let slots: Vec<u32> = self.spheres.iter().enumerate().filter(|(_, s)| s[3] >= 0.0 && {
                let c = glam::Vec4::new(s[0], s[1], s[2], 1.0);
                planes.iter().all(|p| p.dot(c) >= -s[3] * p.truncate().length())
            }).map(|(i, _)| i as u32).collect();
            if slots.len() as u64 * 4 > self.visible.size() { self.visible = Self::slots(gpu, (slots.len() as u64).next_power_of_two()); }
            if !slots.is_empty() { gpu.queue.write_buffer(&self.visible, 0, bytemuck::cast_slice(&slots)); }
            let t = &self.targets[vi];
            let uniform = Uniform {
                vp: vp.to_cols_array_2d(), inv_vp: vp.inverse().to_cols_array_2d(),
                camera: view.cam.pos.extend(exposure).to_array(),
                sun: light.sun_dir.normalize_or_zero().extend(light.sun_lux * exposure).to_array(),
                sun_color: light.sun_color.extend(if light.sun_disc { 1.0 } else { 0.0 }).to_array(),
                horizon: (horizon * exposure).extend(light.fog_density.max(0.0)).to_array(),
                sky: [light.sky_lux * exposure, light.fog_start, light.fog_end, light.debug],
                pool: self.pool,
            };
            gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&uniform));
            let color = t.color.create_view(&Default::default());
            let depth = t.depth.create_view(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("portable view"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &color, depth_slice: None, resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &depth,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                    timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
                pass.set_bind_group(0, &t.bind, &[]);
                pass.set_pipeline(&self.sky);
                pass.draw(0..3, 0..1);
                pass.set_pipeline(&self.terrain);
                pass.set_vertex_buffer(0, self.visible.slice(..));
                pass.draw(0..128 * 3, 0..slots.len() as u32);
            }
            enc.copy_texture_to_texture(t.color.as_image_copy(), wgpu::TexelCopyTextureInfo { texture: output, mip_level: 0,
                origin: wgpu::Origin3d { x: view.x, y: view.y, z: 0 }, aspect: wgpu::TextureAspect::All }, t.color.size());
            stats.submitted_vertices += slots.len() as u64 * 384;
            stats.triangles += slots.len() as u32 * 128;
        }
        gpu.queue.submit([enc.finish()]);
        stats
    }
}
