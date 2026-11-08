//! Warp's portable temporal upscaler (`UpscaleMode::Temporal*`).
//!
//! The scene renders at a lower resolution with the same Halton jitter the
//! DLSS path uses; one compute pass per view turns that frame, the motion
//! vectors and depth from `motion.rs`, and its own output-resolution history
//! into the output image (`shaders/temporal_upscale.wgsl`). It needs no
//! vendor runtime, no storage buffers and one `rgba16float` storage texture,
//! so it runs on every adapter wgpu supports, including WebGPU.
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

pub const SHADER: &str = include_str!("shaders/temporal_upscale.wgsl");

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    render: [f32; 4],
    output: [f32; 4],
    jitter: [f32; 4],
    params: [f32; 4],
}

/// Byte offset of the exposure (params.x), for a GPU copy of a metered value.
pub const EXPOSURE_OFFSET: u64 = std::mem::offset_of!(Uniform, params) as u64;

pub struct Inputs<'a> {
    /// Render-resolution HDR colour (`Rgba16Float`).
    pub color: &'a wgpu::Texture,
    /// `MotionOutput::depth`, `vectors` and `bias` at the render resolution.
    pub depth: &'a wgpu::Texture,
    pub motion: &'a wgpu::Texture,
    pub bias: &'a wgpu::Texture,
    /// Where render pixel (0, 0) sampled the unjittered image, in render
    /// pixels relative to its centre: (jx, -jy) for a projection offset by
    /// (jx, jy) as in frame.rs.
    pub jitter: [f32; 2],
    pub reset: bool,
    pub exposure: f32,
    /// Smallest share of a fully covered pixel's new sample (TAA's blend).
    pub min_blend: f32,
    pub clip_gamma: f32,
    /// A GPU-side exposure (auto exposure) copied over `exposure`.
    pub exposure_state: Option<&'a wgpu::Buffer>,
}

struct View {
    output: (u32, u32),
    input: (u32, u32),
    tex: [wgpu::Texture; 2],
    cur: usize,
}

pub struct TemporalUpscaler {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    views: Vec<Option<View>>,
}

impl TemporalUpscaler {
    pub fn new(gpu: &Gpu) -> Result<Self, String> {
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
        };
        let raw = wgpu::TextureSampleType::Float { filterable: false };
        let entries = [
            texture(0, raw), texture(1, raw), texture(2, raw), texture(3, raw),
            texture(4, wgpu::TextureSampleType::Float { filterable: true }),
            wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2 } },
            wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None } },
            wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering) },
        ];
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("temporal upscale"), entries: &entries });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("temporal upscale"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal upscale"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("temporal upscale"), layout: Some(&pipeline_layout), module: &module, entry_point: Some("main"),
            compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("temporal upscale history"),
            address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Ok(Self { pipeline, layout, sampler, views: Vec::new() })
    }

    /// Forget every view's history (mode change, layout change).
    pub fn reset(&mut self) { self.views.clear(); }

    /// Record one view's reconstruction; returns the output-resolution HDR
    /// image, which stays valid until this view's next frame.
    pub fn evaluate(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, index: usize,
        input: (u32, u32), output: (u32, u32), inputs: Inputs<'_>,
        timestamp: Option<(&wgpu::QuerySet, u32)>) -> Result<wgpu::Texture, String> {
        if index >= 4 { return Err("temporal upscaler tracks at most four views".into()); }
        if input.0 == 0 || input.1 == 0 || output.0 < input.0 || output.1 < input.1 {
            return Err("temporal upscaler needs a non-empty input no larger than its output".into());
        }
        let size = |t: &wgpu::Texture| (t.width(), t.height());
        if [inputs.color, inputs.depth, inputs.motion, inputs.bias].iter().any(|t| size(t) != input) {
            return Err("temporal upscaler inputs must all match the render size".into());
        }
        if !inputs.jitter.iter().all(|j| j.is_finite() && j.abs() <= 0.5) || !inputs.exposure.is_finite() || inputs.exposure <= 0.0 {
            return Err("temporal upscaler needs a jitter within half a pixel and a positive exposure".into());
        }
        while self.views.len() <= index { self.views.push(None); }
        let fresh = self.views[index].as_ref().is_none_or(|v| v.output != output || v.input != input);
        if fresh {
            let make = |label| gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label), size: wgpu::Extent3d { width: output.0, height: output.1, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[] });
            self.views[index] = Some(View { output, input, tex: [make("temporal upscale a"), make("temporal upscale b")], cur: 0 });
        }
        let view = self.views[index].as_mut().unwrap();
        let (src, dst) = (&view.tex[view.cur], &view.tex[1 - view.cur]);
        let uniform = Uniform {
            render: [input.0 as f32, input.1 as f32, 1.0 / input.0 as f32, 1.0 / input.1 as f32],
            output: [output.0 as f32, output.1 as f32, 1.0 / output.0 as f32, 1.0 / output.1 as f32],
            jitter: [inputs.jitter[0], inputs.jitter[1], output.0 as f32 / input.0 as f32, output.1 as f32 / input.1 as f32],
            params: [inputs.exposure, 1.0 / inputs.min_blend.clamp(0.01, 1.0), (fresh || inputs.reset) as u32 as f32, inputs.clip_gamma],
        };
        let buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("temporal upscale"), contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        if let Some(state) = inputs.exposure_state { encoder.copy_buffer_to_buffer(state, 0, &buffer, EXPOSURE_OFFSET, 4); }
        let views = [inputs.color, inputs.depth, inputs.motion, inputs.bias, src, dst].map(|t| t.create_view(&Default::default()));
        let mut entries = views.iter().enumerate()
            .map(|(binding, v)| wgpu::BindGroupEntry { binding: binding as u32, resource: wgpu::BindingResource::TextureView(v) })
            .collect::<Vec<_>>();
        entries.push(wgpu::BindGroupEntry { binding: 6, resource: buffer.as_entire_binding() });
        entries.push(wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&self.sampler) });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("temporal upscale"), layout: &self.layout, entries: &entries });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("temporal upscale"),
                timestamp_writes: timestamp.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set,
                    beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(output.0.div_ceil(8), output.1.div_ceil(8), 1);
        }
        let result = dst.clone();
        view.cur = 1 - view.cur;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shader_validates_and_needs_no_storage_buffers() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        let storage = module.global_variables.iter().filter(|(_, v)| matches!(v.space, naga::AddressSpace::Storage { .. })).count();
        assert_eq!(storage, 0, "the portable upscaler must not spend storage buffers");
        let images = module.global_variables.iter().filter(|(_, v)| matches!(module.types[v.ty].inner, naga::TypeInner::Image { .. })).count();
        assert_eq!(images, 6, "five sampled textures and one storage texture");
        assert_eq!(std::mem::size_of::<Uniform>(), 64);
    }

    /// Sharp HDR test image: a diagonal edge, thin stripes and a bright dot.
    fn truth(u: f32, v: f32) -> f32 {
        let edge = if u * 0.8 + v * 0.35 > 0.55 { 2.0 } else { 0.1 };
        let stripes = if u < 0.4 && ((v * 64.0).floor() as i32) % 3 == 0 { 1.5 } else { 0.0 };
        let dot = if (u - 0.7).hypot(v - 0.25) < 0.02 { 8.0 } else { 0.0 };
        edge + stripes + dot
    }

    fn f16(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = ((bits >> 10) & 31) as i32;
        let frac = (bits & 1023) as f32;
        sign * if exp == 0 { frac * 2f32.powi(-24) } else { (1.0 + frac / 1024.0) * 2f32.powi(exp - 15) }
    }

    fn read_red(gpu: &Gpu, tex: &wgpu::Texture) -> Vec<f32> {
        let (w, h) = (tex.width(), tex.height());
        let row = (w * 8).div_ceil(256) * 256;
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(tex.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
        gpu.queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.wait();
        let data = buf.slice(..).get_mapped_range().expect("mapped readback");
        (0..h).flat_map(|y| (0..w).map(move |x| (y, x)))
            .map(|(y, x)| { let i = (y * row + x * 8) as usize; f16(u16::from_le_bytes([data[i], data[i + 1]])) }).collect()
    }

    fn halton(mut i: u32, b: u32) -> f32 {
        let (mut f, mut r) = (1.0f32, 0.0f32);
        while i > 0 { f /= b as f32; r += f * (i % b) as f32; i /= b; }
        r - 0.5
    }

    /// cargo test -p render --lib temporal_upscale -- --ignored
    /// A still image rendered at half resolution with the Halton jitter
    /// converges much closer to the full-resolution truth than a bilinear
    /// upscale of one unjittered frame, and a reset frame keeps no history.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn still_image_converges_past_bilinear() {
        let gpu = Gpu::new().unwrap();
        let (input, output) = ((48u32, 32u32), (96u32, 64u32));
        let make = |format, size: (u32, u32)| gpu.device.create_texture(&wgpu::TextureDescriptor { label: None,
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
        let color = make(wgpu::TextureFormat::Rgba32Float, input);
        let depth = make(wgpu::TextureFormat::R32Float, input);
        let motion = make(wgpu::TextureFormat::Rg16Float, input);
        let bias = make(wgpu::TextureFormat::R32Float, input);
        let render = |jitter: [f32; 2]| -> Vec<f32> {
            (0..input.1).flat_map(|y| (0..input.0).map(move |x| (x, y))).flat_map(|(x, y)| {
                let v = truth((x as f32 + 0.5 + jitter[0]) / input.0 as f32, (y as f32 + 0.5 + jitter[1]) / input.1 as f32);
                [v, v, v, 1.0]
            }).collect()
        };
        let mut upscaler = TemporalUpscaler::new(&gpu).unwrap();
        let mut run = |frame: u32, reset: bool| {
            let jitter = [halton(frame, 2), halton(frame, 3)];
            gpu.queue.write_texture(color.as_image_copy(), bytemuck::cast_slice(&render(jitter)),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(input.0 * 16), rows_per_image: None },
                wgpu::Extent3d { width: input.0, height: input.1, depth_or_array_layers: 1 });
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            let out = upscaler.evaluate(&gpu, &mut enc, 0, input, output, Inputs { color: &color, depth: &depth, motion: &motion,
                bias: &bias, jitter, reset, exposure: 0.5, min_blend: 0.05, clip_gamma: 1.5, exposure_state: None }, None).unwrap();
            gpu.queue.submit([enc.finish()]);
            read_red(&gpu, &out)
        };
        let truth_out: Vec<f32> = (0..output.1).flat_map(|y| (0..output.0).map(move |x|
            truth((x as f32 + 0.5) / output.0 as f32, (y as f32 + 0.5) / output.1 as f32))).collect();
        // Mean absolute error in range-compressed colour, as seen after tone mapping.
        let error = |image: &[f32]| image.iter().zip(&truth_out).map(|(a, b)| (a / (1.0 + a) - b / (1.0 + b)).abs()).sum::<f32>() / image.len() as f32;
        // Bilinear upscale of one unjittered frame.
        let flat = render([0.0, 0.0]);
        let at = |x: i32, y: i32| flat[((y.clamp(0, input.1 as i32 - 1) as u32 * input.0 + x.clamp(0, input.0 as i32 - 1) as u32) * 4) as usize];
        let bilinear: Vec<f32> = (0..output.1).flat_map(|y| (0..output.0).map(move |x| (x, y))).map(|(x, y)| {
            let (px, py) = ((x as f32 + 0.5) * 0.5 - 0.5, (y as f32 + 0.5) * 0.5 - 0.5);
            let (fx, fy) = (px - px.floor(), py - py.floor());
            let (ix, iy) = (px.floor() as i32, py.floor() as i32);
            (at(ix, iy) * (1.0 - fx) + at(ix + 1, iy) * fx) * (1.0 - fy) + (at(ix, iy + 1) * (1.0 - fx) + at(ix + 1, iy + 1) * fx) * fy
        }).collect();
        let first = run(1, true);
        let mut last = first.clone();
        for frame in 2..=48 { last = run(frame, false); }
        let (e_first, e_last, e_bilinear) = (error(&first), error(&last), error(&bilinear));
        eprintln!("mean error: first frame {e_first:.4}, after 48 frames {e_last:.4}, bilinear {e_bilinear:.4}");
        assert!(e_last < 0.6 * e_bilinear, "converged {e_last} vs bilinear {e_bilinear}");
        assert!(e_last < 0.6 * e_first, "history must add detail: {e_last} vs first {e_first}");
        // Reset: the result is this frame alone again.
        let reset = run(49, true);
        assert!(error(&reset) > 1.5 * e_last, "reset must drop history");
    }
}
