//! Explicit native-only experiment: a depth-aware 3x3 filter of the rendered
//! source's premultiplied shading, immediately before its existing composite.
//! Alpha, source geometry, opaque foreground and temporal history are unchanged.
//! This is not a prefiltered terrain/volume replacement or geometric antialiasing.
//! Filter onset uses the coarse triangle's mean width (half its basal width).
use crate::{meadow_data::MeadowGeometry, Gpu};

pub(super) const MAX_STRENGTH: f32 = 0.65;
const SOURCE: &str = include_str!("shaders/meadow_spatial_filter.wgsl");

/// Actual basal diameter, not a game-specific hardcoded width. Single-form
/// sources retain their original average exactly. A mixed-form field uses
/// the narrowest blade so its subpixel shading cannot escape the shared
/// filter threshold; broader forms can consequently soften earlier. Only
/// homogeneous authored-t sources qualify; legacy metadata has no t contract.
pub(super) fn source_dimensions(geometry: &MeadowGeometry) -> Option<[f32; 2]> {
    if geometry.vertices.is_empty() || geometry.vertices.iter().any(|v| v.anchor[3] <= 0.0) {
        return None;
    }
    let mut diameter = 0.0f64;
    let mut minimum_diameter = f32::INFINITY;
    let mut maximum_diameter = 0.0f32;
    let mut count = 0;
    let mut height = 0.0f32;
    for vertex in &geometry.vertices {
        height = height.max(vertex.anchor[3]);
        if vertex.normal[3] == 0.0 {
            let offset = glam::Vec3::from_slice(&vertex.position) - glam::Vec3::from_slice(&vertex.anchor);
            diameter += 2.0 * f64::from(offset.length());
            minimum_diameter = minimum_diameter.min(2.0 * offset.length());
            maximum_diameter = maximum_diameter.max(2.0 * offset.length());
            count += 1;
        }
    }
    if count == 0 || diameter <= 0.0 || !diameter.is_finite() { return None; }
    let average = (diameter / f64::from(count)) as f32;
    // Quantized roots plus f32 yaw produce tiny subtraction differences even
    // for one authored width. Preserve that source's exact previous average.
    // A genuinely varied source uses the minimum regardless of its spatial
    // variant selection mode (callers may still choose independent hashes).
    let width = if maximum_diameter - minimum_diameter > average * 1e-4 { minimum_diameter } else { average };
    Some([width, height])
}

pub(super) struct Filter {
    layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
    pub blade_width: f32,
}
impl Filter {
    pub fn new(gpu: &Gpu, dimensions: [f32; 2]) -> Self {
        let layout = super::layout(gpu, "meadow spatial filter", &[
            (0, super::texture(wgpu::TextureSampleType::Float {filterable:false}, wgpu::TextureViewDimension::D2)),
            (1, super::texture(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2)),
            (2, super::texture(wgpu::TextureSampleType::Depth, wgpu::TextureViewDimension::D2)),
            (3, super::uniform()), (4, super::uniform()),
        ], wgpu::ShaderStages::FRAGMENT);
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meadow spatial filter"), bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let shader = super::shader(gpu, "meadow spatial filter", SOURCE.into());
        let constants = [("SOURCE_BLADE_WIDTH", f64::from(dimensions[0])), ("SOURCE_BLADE_HEIGHT", f64::from(dimensions[1]))];
        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("meadow spatial filter composite"), layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader, entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions {constants:&constants,..Default::default()},
                targets: &[Some(wgpu::ColorTargetState {format:wgpu::TextureFormat::Rgba16Float,
                    blend:Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})],
            }),
            multiview_mask: None, cache: gpu.pipeline_cache(),
        });
        Self {layout,pipeline,blade_width:dimensions[0]}
    }
    pub fn bind(&self, gpu: &Gpu, color: &wgpu::TextureView, depth: &wgpu::TextureView,
        opaque: &wgpu::TextureView, view: wgpu::BindingResource<'_>, settings: &wgpu::Buffer) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow spatial filter inputs"), layout: &self.layout, entries: &[
                wgpu::BindGroupEntry {binding:0,resource:wgpu::BindingResource::TextureView(color)},
                wgpu::BindGroupEntry {binding:1,resource:wgpu::BindingResource::TextureView(depth)},
                wgpu::BindGroupEntry {binding:2,resource:wgpu::BindingResource::TextureView(opaque)},
                wgpu::BindGroupEntry {binding:3,resource:view},
                wgpu::BindGroupEntry {binding:4,resource:settings.as_entire_binding()},
            ],
        })
    }
}

#[cfg(test)]
#[path = "meadow_spatial_filter_tests.rs"]
mod tests;
