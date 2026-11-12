//! A prefiltered copy of last frame's resolved image (the TAA history) for
//! traced reflections: half resolution with a mip chain, so a glossy
//! reflection reads one sample at the mip that matches its cone instead of
//! many noisy jittered rays or taps (the cone-traced / prefiltered
//! approach). Built once per frame and view when traced reflections are on.
use crate::Gpu;

/// Levels: full resolution (a copy), half, quarter, ... 1/64.
pub const LEVELS: u32 = 7;

struct Target {
    size: [u32; 2],
    texture: wgpu::Texture,
    /// All levels, for the shade pass.
    view: wgpu::TextureView,
    /// Bind groups per history texture (TAA alternates two): one per level.
    cached: Vec<(wgpu::Texture, wgpu::TextureView, Vec<wgpu::BindGroup>)>,
}

pub(crate) struct ReflectionMips {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    plain: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    views: std::sync::Mutex<Vec<Target>>,
}

const SHADER: &str = "
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var samp: sampler;
// Last frame's shaded image, whose alpha is 0 on a light's face (shade.wgsl):
// read by level 0 only.
@group(0) @binding(3) var faces: texture_2d<f32>;
fn clean(c: vec3<f32>) -> vec3<f32> {
    // Keep NaN and overflow out of every level below.
    if any(c != c) { return vec3<f32>(0.0); }
    return min(c, vec3<f32>(65504.0));
}

// Level 0: a copy. Below: the four source texels, each weighted by
// 1 / (1 + its luminance over their mean) (Karis' average, made relative:
// the image is scene-linear, not exposed). A box filter let one small, very
// bright texel (a bulb) take over whole coarse texels, which a glossy
// reflection then showed as a bright square around the darker, sharper
// things in front of the bulb.
fn reduce(id: vec3<u32>, karis: bool) {
    let size = textureDimensions(dst);
    if id.x >= size.x || id.y >= size.y { return; }
    let src_size = textureDimensions(src);
    if all(src_size == size) {
        // A light's face (an emissive surface an analytic light stands for)
        // leaves the chain: its light reaches a glossy surface as that
        // light's specular, and a coarse level would spread the face's
        // radiance over everything a reflection reads near it. Alpha: the
        // share of each texel that is not a face.
        var cover = 1.0;
        if all(textureDimensions(faces) == size) { cover = select(1.0, 0.0, textureLoad(faces, id.xy, 0).a < 0.5); }
        textureStore(dst, id.xy, vec4<f32>(clean(textureLoad(src, id.xy, 0).rgb) * cover, cover));
        return;
    }
    let last = vec2<i32>(src_size) - 1;
    var c: array<vec3<f32>, 4>;
    var a: array<f32, 4>;
    var mean = 0.0;
    var cover = 0.0;
    for (var k = 0; k < 4; k++) {
        let s = textureLoad(src, min(vec2<i32>(id.xy) * 2 + vec2<i32>(k & 1, k >> 1u), last), 0);
        // Each texel's colour over its own cover: the mean of what is not a
        // light's face (level 0 stores colour times cover).
        a[k] = clamp(s.a, 0.0, 1.0);
        c[k] = clean(s.rgb) / max(a[k], 1e-6);
        mean += dot(c[k], vec3<f32>(0.2126, 0.7152, 0.0722)) * a[k];
        cover += a[k] * 0.25;
    }
    mean /= max(cover * 4.0, 1e-6);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var k = 0; k < 4; k++) {
        let w = a[k] * select(1.0, 1.0 / (1.0 + dot(c[k], vec3<f32>(0.2126, 0.7152, 0.0722)) / max(mean, 1e-6)), karis);
        sum += c[k] * w;
        weight += w;
    }
    // Colour times cover again, so the next level weighs it the same way.
    textureStore(dst, id.xy, vec4<f32>(sum / max(weight, 1e-6) * cover, cover));
}

// The first downsample is Karis-weighted (one very bright texel); below it a
// plain average keeps the energy: weighted at every level, a bright doorway
// lost most of its light by the coarse levels a semi-rough floor reads.
@compute @workgroup_size(8, 8)
fn down(@builtin(global_invocation_id) id: vec3<u32>) { reduce(id, true); }
@compute @workgroup_size(8, 8)
fn down_plain(@builtin(global_invocation_id) id: vec3<u32>) { reduce(id, false); }";

impl ReflectionMips {
    pub fn new(gpu: &Gpu) -> Self {
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("reflection mips"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false } },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::Rgba16Float, view_dimension: wgpu::TextureViewDimension::D2 } },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering) },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::COMPUTE, count: None,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false } },
        ]});
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("reflection mips"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("reflection mips"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("reflection mip"), layout: Some(&pipeline_layout),
            module: &module, entry_point: Some("down"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let plain = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("reflection mip (plain)"), layout: Some(&pipeline_layout),
            module: &module, entry_point: Some("down_plain"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor { label: Some("reflection mips"),
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        Self { layout, pipeline, plain, sampler, views: std::sync::Mutex::new(Vec::new()) }
    }

    pub fn set_view_count(&self, count: usize) { self.views.lock().unwrap().truncate(count); }

    /// Downsample `history` (full resolution) into this view's chain and
    /// return a view of all its levels (level 0 is full resolution).
    pub fn build(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, view: usize, history: &wgpu::Texture, faces: &wgpu::TextureView, timestamps: Option<(&wgpu::QuerySet, u32)>) -> wgpu::TextureView {
        let size = [history.width().max(1), history.height().max(1)];

        let mut views = self.views.lock().unwrap();
        let make = |size: [u32; 2]| {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("reflection mips"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: LEVELS.min(32 - size[0].max(size[1]).leading_zeros()), sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING, view_formats: &[] });
            let view = texture.create_view(&Default::default());
            Target { size, texture, view, cached: Vec::new() }
        };
        while views.len() <= view { views.push(make([1, 1])); }
        let target = &mut views[view];
        if target.size != size { *target = make(size); }
        // Keyed by history and the faces' image (a resized view replaces both).
        if !target.cached.iter().any(|(t, f, _)| t == history && f == faces) {
            let level = |l: u32| target.texture.create_view(&wgpu::TextureViewDescriptor { base_mip_level: l, mip_level_count: Some(1), ..Default::default() });
            let source = history.create_view(&Default::default());
            let binds = (0..target.texture.mip_level_count()).map(|l| {
                let src = if l == 0 { source.clone() } else { level(l - 1) };
                gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("reflection mip"), layout: &self.layout, entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&level(l)) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(faces) },
                ]})
            }).collect();
            if target.cached.len() >= 2 { target.cached.remove(0); }
            target.cached.push((history.clone(), faces.clone(), binds));
        }
        let (_, _, binds) = target.cached.iter().find(|(t, f, _)| t == history && f == faces).unwrap();
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("reflection mips"),
            timestamp_writes: timestamps.map(|(query_set, i)| wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }) });
        pass.set_pipeline(&self.pipeline);
        for (l, bind) in binds.iter().enumerate() {
            let w = (target.size[0] >> l).max(1);
            let h = (target.size[1] >> l).max(1);
            if l == 2 { pass.set_pipeline(&self.plain); }
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        drop(pass);
        target.view.clone()
    }
}
