//! Kern's renderer. Headless by design: it renders into textures and reads
//! them back, so frames and benchmarks run the same on a desktop GPU and on
//! a software rasteriser, and never open a window unless a game asks.
//!
//! Geometry is Nanite-style: clusters in a LOD DAG (cluster.rs), culled and
//! LOD-selected per cluster on the GPU, rastered into a visibility buffer
//! and shaded once per pixel in HDR (frame.rs).

pub mod cluster;
pub mod frame;
pub mod static_sun;
pub mod debug_settings;
pub mod bloom;
pub mod debug_ui;
pub mod debug_timing;
pub mod cpu_time;
pub mod cpu_spans;
pub mod gpu;
pub mod gpu_profile;
pub mod geometry_diagnostics;
pub mod startup_profile;
mod texture_cache;
mod pipeline_cache;
pub mod grass;
pub mod meadow;
pub mod meadow_data;
pub(crate) mod meadow_shadow_receivers;
pub mod receiver_shadow_demand;
pub mod meadow_aggregate;
pub mod meadow_voxel;
pub mod meadow_voxel_samples;
pub mod meadow_brick_field;
pub mod vegetation_brick_render;
mod meadow_substrate;
mod meadow_index;
pub mod tree_occlusion;
pub mod cull_groups;
pub mod cluster_traversal;
pub mod shadow_diagnostics;
mod shadow_validation;
mod depth_pyramid;
#[cfg(test)]
mod meadow_occlusion_tests;
mod grass_lighting;
pub mod far_grass;
pub mod hud;
pub mod picture;
pub mod screen;
pub mod sprites;
pub mod hud_profile;
pub mod present;
pub mod scene;
pub mod primitives;
pub mod slots;
pub mod particles;
pub mod shade_features;
pub mod shaders;
pub mod atmosphere;
pub mod radiance_cache;
pub mod radiance_cache_gpu;
pub mod headlights;
pub mod exhaust;
pub mod wheel_particles;
pub mod tracks;
pub mod foliage_voxel;
pub mod foliage_shadow;
pub mod foliage_assembly;
pub mod vegetation_asset;
pub mod vegetation_accounting;
pub mod vegetation_clod;
pub mod foliage_volume_data;
pub mod foliage_groups;
pub mod foliage_representation;
pub mod foliage_compact;
pub mod source_replacement;
pub mod foliage_world;
pub mod foliage_world_render;
pub mod vegetation_detail;
pub mod voxel_forest;
pub mod wind;
pub mod part_wind;
pub mod baked_animation;
pub mod skinning;
pub mod debris;
pub mod fx;
pub mod crowd;
pub mod motion;
pub mod dlss;
pub mod upscale;
mod stability;
mod far_sun;
mod foliage_sun_pages;
mod world_visibility;
mod transparency;
mod volume_view;
mod canopy;
mod terrain_visibility;
pub mod terrain_trace;
#[cfg(test)]
mod fog_tests;
#[cfg(test)]
mod transmittance_tests;
#[cfg(test)]
mod rock_mapping;

pub use frame::{Camera, FrameStats, Lighting, Renderer, SplitLayout, ViewRect, split_rects, split_rects_with};
pub use gpu::{Gpu, Stopwatch, write_png};
pub use scene::Scene;
/// A test frame: a compute shader fills an RGBA8 image with a gradient.
/// Proves the device, compute, readback and PNG path end to end.
pub fn test_frame(gpu: &Gpu, w: u32, h: u32) -> Vec<u8> {
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("test"),
        source: wgpu::ShaderSource::Wgsl(
            "@group(0) @binding(0) var out: texture_storage_2d<rgba8unorm, write>;
             @compute @workgroup_size(8, 8) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                 let s = vec2<f32>(textureDimensions(out));
                 let uv = vec2<f32>(id.xy) / s;
                 textureStore(out, id.xy, vec4<f32>(uv.x, uv.y, 0.5, 1.0));
             }"
            .into(),
        ),
    });
    let pipe = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("test"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: gpu.pipeline_cache(),
    });
    let view = tex.create_view(&Default::default());
    let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipe.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
    });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }
    gpu.queue.submit([enc.finish()]);
    gpu.read_rgba8(&tex)
}

/// Every shader parses and validates (naga, no GPU): `cargo test -p render`
/// catches a shader mistake in a second instead of at run time.
#[cfg(test)]
mod shader_tests {
    #[test]
    fn shaders_validate() {
        let raster=crate::grass::raster_source();
        let cull=crate::frame::cull_source();
        let shade=crate::wind::shade_source();
        let shaders = [
            ("cull", cull.as_str()),
            ("raster", raster.as_str()),
            ("shade", shade.as_str()),
            ("wind", include_str!("shaders/wind.wgsl")),
            ("tonemap", include_str!("shaders/tonemap.wgsl")),
            ("bloom", include_str!("shaders/bloom.wgsl")),
            ("foliage", include_str!("shaders/foliage.wgsl")),
            ("density_filter", include_str!("shaders/density_filter.wgsl")),
            ("taa", include_str!("shaders/taa.wgsl")),
            ("present", include_str!("shaders/present.wgsl")),
            ("hud", include_str!("shaders/hud.wgsl")),
        ];
        let mut failed = Vec::new();
        for (name, src) in shaders {
            match naga::front::wgsl::parse_str(src) {
                Err(e) => failed.push(format!("{name}: {}", e.emit_to_string(src))),
                Ok(module) => {
                    let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
                    if let Err(e) = v.validate(&module) {
                        failed.push(format!("{name}: {}", e.emit_to_string(src)));
                    }
                }
            }
        }
        assert!(failed.is_empty(), "\n{}", failed.join("\n"));
    }

    #[test]
    fn grass_transition_shaders_validate() {
        for src in [crate::grass::shader_source(),crate::grass::composite_source(),crate::grass::compact_source()] {
            let module=naga::front::wgsl::parse_str(&src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&src)));
        }
    }

    #[test]
    fn voxel_forest_shaders_validate() {
        for src in [crate::voxel_forest::shader_source(),include_str!("shaders/voxel_composite.wgsl").to_owned()] {
            let module=naga::front::wgsl::parse_str(&src).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module).unwrap_or_else(|e|panic!("{}",e.emit_to_string(&src)));
        }
    }

    #[test]
    fn composed_transparency_shader_validates() {
        let src = crate::transparency::shader_source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let mut validator = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
        validator.validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    }
}
