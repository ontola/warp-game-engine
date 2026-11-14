//! Stochastic screen-space reflections (Stachowiak, SIGGRAPH 2015): shade
//! traces one GGX visible-normal ray per pixel (`sssr_write` in shade.wgsl)
//! into a per-view ray target; this module resolves the rays with spatial
//! reuse (BRDF over pdf, ratio estimator, filtered importance sampling of
//! last frame's image), filters them over time (reprojected by hit
//! distance, neighbourhood-clamped) and adds them onto the shaded image.
//! See engine/docs/dynamic-gi.md.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Resolve radius in pixels at 1080 lines (scaled with the view height):
/// neighbours within it share their rays.
pub const RADIUS_PX: f32 = 6.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    prev_view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    size: [f32; 4],
    motion: [f32; 4],
    params: [f32; 4],
}

struct Target {
    size: (u32, u32),
    rays_view: wgpu::TextureView,
    resolved_view: wgpu::TextureView,
    temporal_views: [wgpu::TextureView; 2],
    uniform: wgpu::Buffer,
    cur: usize,
    frames: u32,
}

pub(crate) struct Sssr {
    resolve: wgpu::ComputePipeline,
    temporal: wgpu::ComputePipeline,
    composite: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    pub fallback: wgpu::TextureView,
    targets: std::sync::Mutex<Vec<Option<Target>>>,
}

/// What one view's passes need beyond its own targets.
pub(crate) struct ViewInput<'a> {
    pub index: usize,
    pub depth: &'a wgpu::TextureView,
    pub history: &'a wgpu::TextureView,
    pub history_levels: u32,
    pub hdr: &'a wgpu::TextureView,
    pub view_proj: Mat4,
    pub prev_view_proj: Mat4,
    pub camera: Vec3,
    pub proj_scale: f32,
    pub frame: u32,
    /// The temporal pass's surface motion (sssr.wgsl group 1).
    pub motion: Motion<'a>,
}

/// What the temporal pass reads to tell how the surface under a pixel moved:
/// the view's visibility target and visible clusters, the instances and
/// wind now and, once motion.rs holds them, last frame's. Without those
/// every surface counts as one whose motion is unknown.
pub(crate) struct Motion<'a> {
    pub visibility: &'a wgpu::TextureView,
    pub visible: &'a wgpu::Buffer,
    pub instances: &'a wgpu::Buffer,
    pub wind: &'a wgpu::Buffer,
    pub previous: Option<(&'a wgpu::Buffer, &'a wgpu::Buffer)>,
}

fn texture(gpu: &Gpu, label: &str, (w, h): (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
}

impl Sssr {
    pub fn new(gpu: &Gpu) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sssr"), source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sssr.wgsl").into()) });
        let compute = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: None, module: &module,
            entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let keep = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
        let composite = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("sssr composite"), layout: None,
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: Some(wgpu::BlendState { color: add, alpha: keep }), write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: gpu.pipeline_cache() });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("sssr"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let fallback = texture(gpu, "sssr rays (none)", (1, 1), wgpu::TextureFormat::Rgba32Uint, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING)
            .create_view(&Default::default());
        Self { resolve: compute("resolve"), temporal: compute("temporal"), composite, sampler, fallback, targets: std::sync::Mutex::new(Vec::new()) }
    }

    pub fn set_view_count(&self, count: usize) { self.targets.lock().unwrap().truncate(count); }

    /// This view's ray target (cleared), for shade's binding 22; resized
    /// with the view.
    pub fn rays(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, index: usize, size: (u32, u32)) -> wgpu::TextureView {
        let mut targets = self.targets.lock().unwrap();
        if targets.len() <= index { targets.resize_with(index + 1, || None); }
        if targets[index].as_ref().is_none_or(|t| t.size != size) {
            let rays = texture(gpu, "sssr rays", size, wgpu::TextureFormat::Rgba32Uint,
                wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT);
            let float = |label| texture(gpu, label, size, wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING)
                .create_view(&Default::default());
            let uniform = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("sssr view"), size: std::mem::size_of::<Uniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            targets[index] = Some(Target { size, rays_view: rays.create_view(&Default::default()), resolved_view: float("sssr resolved"),
                temporal_views: [float("sssr temporal a"), float("sssr temporal b")], uniform, cur: 0, frames: 0 });
        }
        let t = targets[index].as_ref().unwrap();
        // Cleared with an empty render pass (clear_texture needs a device
        // feature): pixels shade does not write stay "no ray".
        drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("sssr rays clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.rays_view, resolve_target: None, depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None }));
        t.rays_view.clone()
    }

    /// Resolve, filter and add this view's reflections onto `hdr`.
    pub fn render(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, v: ViewInput, times: Option<(&wgpu::QuerySet, u32)>) {
        let mut targets = self.targets.lock().unwrap();
        let Some(Some(t)) = targets.get_mut(v.index) else { return };
        let (w, h) = t.size;
        let u = Uniform {
            view_proj: v.view_proj.to_cols_array_2d(),
            inv_view_proj: v.view_proj.inverse().to_cols_array_2d(),
            prev_view_proj: v.prev_view_proj.to_cols_array_2d(),
            cam: v.camera.extend(v.frame as f32).to_array(),
            size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
            motion: [if v.motion.previous.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            params: [v.proj_scale, v.history_levels as f32, RADIUS_PX * h as f32 / 1080.0, if t.frames > 0 { 1.0 } else { 0.0 }],
        };
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&u));
        let (prev, next) = (t.cur, 1 - t.cur);
        let e = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let tv = wgpu::BindingResource::TextureView;
        let resolve_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sssr resolve"), layout: &self.resolve.get_bind_group_layout(0), entries: &[
            e(0, t.uniform.as_entire_binding()), e(1, tv(&t.rays_view)), e(2, tv(v.depth)), e(3, tv(v.history)),
            e(4, wgpu::BindingResource::Sampler(&self.sampler)), e(5, tv(&t.resolved_view))] });
        let temporal_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sssr temporal"), layout: &self.temporal.get_bind_group_layout(0), entries: &[
            e(0, t.uniform.as_entire_binding()), e(1, tv(&t.rays_view)), e(2, tv(v.depth)), e(4, wgpu::BindingResource::Sampler(&self.sampler)),
            e(6, tv(&t.resolved_view)), e(7, tv(&t.temporal_views[prev])), e(8, tv(&t.temporal_views[next]))] });
        let (previous_instances, previous_wind) = v.motion.previous.unwrap_or((v.motion.instances, v.motion.wind));
        let motion_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sssr temporal motion"), layout: &self.temporal.get_bind_group_layout(1), entries: &[
            e(0, tv(v.motion.visibility)), e(1, v.motion.visible.as_entire_binding()), e(2, v.motion.instances.as_entire_binding()),
            e(3, previous_instances.as_entire_binding()), e(4, v.motion.wind.as_entire_binding()), e(5, previous_wind.as_entire_binding())] });
        let composite_bind =gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sssr composite"), layout: &self.composite.get_bind_group_layout(0), entries: &[
            e(1, tv(&t.rays_view)), e(6, tv(&t.temporal_views[next]))] });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sssr resolve"),
                timestamp_writes: times.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: None }) });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &resolve_bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            pass.set_pipeline(&self.temporal);
            pass.set_bind_group(0, &temporal_bind, &[]);
            pass.set_bind_group(1, &motion_bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("sssr composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: v.hdr, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: times.map(|(query_set, i)| wgpu::RenderPassTimestampWrites { query_set, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(i + 1) }),
                occlusion_query_set: None, multiview_mask: None });
            pass.set_pipeline(&self.composite);
            pass.set_bind_group(0, &composite_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        t.cur = next;
        t.frames += 1;
    }

    /// Forget this view's history (a cut).
    pub fn reset(&self) { for t in self.targets.lock().unwrap().iter_mut().flatten() { t.frames = 0; } }
}
