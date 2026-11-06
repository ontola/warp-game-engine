//! Shared physical-lighting inputs for terrain/foliage volume passes.
//! The caller retains one group per camera and reuses it across frames/passes.
//! Scene/volume-specific data and timestamps remain owned by each renderer.
pub(crate) struct VolumeViewInputs<'a> {
    pub camera: &'a wgpu::Buffer,
    pub light: &'a wgpu::Buffer,
    pub opaque_depth: &'a wgpu::TextureView,
    pub sun_shadow: &'a wgpu::TextureView,
    pub shadow_sampler: &'a wgpu::Sampler,
    pub sky: &'a wgpu::TextureView,
    pub heightfield: &'a wgpu::TextureView,
    pub gi: &'a wgpu::Buffer,
    pub canopy: &'a wgpu::TextureView,
    pub canopy_sampler: &'a wgpu::Sampler,
    pub near_shadow: &'a wgpu::TextureView,
}
pub(crate) fn create_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
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
    let unfiltered = tex(
        wgpu::TextureSampleType::Float { filterable: false },
        wgpu::TextureViewDimension::D2,
    );
    let depth = tex(
        wgpu::TextureSampleType::Depth,
        wgpu::TextureViewDimension::D2,
    );
    let bindings = [
        (0, uniform),
        (2, uniform),
        (4, depth),
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
        // The sun clipmap (frame.rs near_levels): one layer per level.
        (15, tex(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2Array)),
    ];
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shared volume lighting view"),
        entries: &bindings
            .iter()
            .map(|&(binding, ty)| wgpu::BindGroupLayoutEntry {
                binding,
                ty,
                visibility: wgpu::ShaderStages::COMPUTE,
                count: None,
            })
            .collect::<Vec<_>>(),
    })
}
impl VolumeViewInputs<'_> {
    pub(crate) fn bind_cached(
        &self,
        gpu: &crate::Gpu,
        layout: &wgpu::BindGroupLayout,
        cache: &mut crate::frame::resource_cache::BindGroups<1>,
    ) -> wgpu::BindGroup {
        cache.entries(gpu, layout, "shared volume lighting view", &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.light.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(self.opaque_depth),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(self.sun_shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(self.shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(self.sky),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(self.heightfield),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: self.gi.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 13,
                    resource: wgpu::BindingResource::TextureView(self.canopy),
                },
                wgpu::BindGroupEntry {
                    binding: 14,
                    resource: wgpu::BindingResource::Sampler(self.canopy_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 15,
                    resource: wgpu::BindingResource::TextureView(self.near_shadow),
                },
            ],
        )
    }
}
