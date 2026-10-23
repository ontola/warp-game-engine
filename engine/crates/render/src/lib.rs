//! Kern's renderer. Headless by design: it renders into textures and reads
//! them back, so frames and benchmarks run the same on a desktop GPU and on
//! a software rasteriser, and never open a window unless a game asks.
//!
//! Geometry is Nanite-style: clusters in a LOD DAG (cluster.rs), culled and
//! LOD-selected per cluster on the GPU, rastered into a visibility buffer
//! and shaded once per pixel in HDR (frame.rs).

pub mod cluster;
pub mod frame;
pub mod gpu;
pub mod grass;
pub mod far_grass;
pub mod hud;
pub mod hud_profile;
pub mod present;
pub mod scene;
pub mod foliage_voxel;
pub mod foliage_volume_data;
pub mod voxel_forest;
mod stability;
mod transparency;
mod volume_view;
mod canopy;

pub use frame::{Camera, FrameStats, Lighting, Renderer, ViewRect, split_rects};
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
        cache: None,
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
        let shaders = [
            ("cull", include_str!("shaders/cull.wgsl")),
            ("raster", raster.as_str()),
            ("shade", include_str!("shaders/shade.wgsl")),
            ("tonemap", include_str!("shaders/tonemap.wgsl")),
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
        for src in [crate::grass::shader_source(),crate::grass::composite_source()] {
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
