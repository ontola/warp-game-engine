//! Bounded, source-accepted basal contact approximation. No blade shadow map.
//! A small GPU cache reuses the exact source habitat and road-anchor decisions;
//! opaque terrain/road lighting reads soft kernels around those occupied roots.
use super::{buffer, layout, shader, storage, uniform, Settings};
use crate::{Gpu, meadow_data::{MeadowField, ROOT_CELL, VARIANTS}};
use glam::Vec3;

const DIMENSION: u32 = 151;
const HEADER: u32 = 4;
const VIEWS: u32 = 4;
const BLOCK: u32 = HEADER + DIMENSION * DIMENSION;
const BYTES: u64 = BLOCK as u64 * VIEWS as u64 * 16;
const EDGE_EXTENSION: f32 = 0.18;
const MAX_RADIUS: f32 = 0.55;
const SUN_REACH: f32 = 0.35;
const SKY_STRENGTH: f32 = 0.24;
const SUN_STRENGTH: f32 = 0.48;

#[derive(Clone, Copy, Debug)]
pub struct Stats {
    pub enabled: bool,
    pub shared_bytes: u64,
    pub radius_m: f32,
    pub sky_strength: f32,
    pub sun_strength: f32,
    pub sun_reach_m: f32,
    pub fade_m: [f32; 2],
    pub grid_dimension: u32,
}

pub(super) struct Renderer {
    pub buffer: wgpu::Buffer,
    pipeline: wgpu::ComputePipeline,
    view_layout: wgpu::BindGroupLayout,
    settings: [wgpu::Buffer; VIEWS as usize],
    generation: [wgpu::BindGroup; VIEWS as usize],
    output: [wgpu::BindGroup; VIEWS as usize],
    empty_hiz: wgpu::TextureView,
    radius: f32,
}

pub(super) fn source() -> String {
    format!("{}\n{}", super::generate_source(), include_str!("shaders/meadow_contact_generate.wgsl"))
}

fn contact_radius(anchor_radius: f32) -> f32 {
    // The two lighting queries share a five-by-five scan around their
    // midpoint. Even the closest omitted root, with maximal jitter and
    // accepted-centroid shift toward either query, must have zero support.
    let support = 2.5 * ROOT_CELL - 0.04 * ROOT_CELL - anchor_radius - SUN_REACH * 0.5 - 0.002;
    (anchor_radius + EDGE_EXTENSION).min(MAX_RADIUS).min(support).max(0.0)
}

fn source_area(field: &MeadowField) -> [f32; VARIANTS] {
    std::array::from_fn(|variant| {
        let [first, count] = field.geometry.lod_ranges[variant][0];
        field.geometry.vertices[first as usize..(first + count) as usize]
            .chunks_exact(3).map(|t| {
                let p: [Vec3; 3] = std::array::from_fn(|i| Vec3::from_slice(&t[i].position));
                (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
            }).sum::<f32>() / (ROOT_CELL * ROOT_CELL)
    })
}

impl Renderer {
    pub(super) fn new(gpu: &Gpu, field: &MeadowField, ground: &wgpu::Buffer,
        recipe: &wgpu::Buffer, generation_layout: &wgpu::BindGroupLayout,
        view_layout: &wgpu::BindGroupLayout) -> Option<Self> {
        if std::env::var("KERN_MEADOW_EDGE_SHADOW").as_deref() != Ok("1") { return None; }
        // Extend the fixed anchor footprint into the immediately adjacent
        // ground. Recipes too wide for the bounded scan disable this model.
        let radius = contact_radius(field.geometry.max_anchor_radius);
        let output_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meadow accepted contact field, four views"), size: BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let output_layout = layout(gpu, "meadow contact output",
            &[(0, storage(false)), (1, uniform())], wgpu::ShaderStages::COMPUTE);
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meadow accepted contact"),
            bind_group_layouts: &[Some(generation_layout), Some(view_layout), Some(&output_layout)],
            immediate_size: 0,
        });
        let module = shader(gpu, "meadow accepted contact", source());
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meadow accepted contact"), layout: Some(&pipeline_layout), module: &module,
            entry_point: Some("generate_meadow_contact"), compilation_options: Default::default(),
            cache: gpu.pipeline_cache(),
        });
        let unused_pool = buffer(gpu, "unused contact source pool", &[0; 16], wgpu::BufferUsages::STORAGE);
        let unused_args = buffer(gpu, "unused contact source args", &[0; 60], wgpu::BufferUsages::STORAGE);
        let settings = std::array::from_fn(|_| buffer(gpu, "meadow contact settings",
            &[0; std::mem::size_of::<Settings>()], wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST));
        let generation = std::array::from_fn(|i| gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow contact source"), layout: generation_layout,
            entries: &[(0, &settings[i]), (1, ground), (2, &unused_pool), (3, &unused_args), (4, recipe)]
                .map(|(binding, b)| wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }),
        }));
        let areas = source_area(field);
        let output = std::array::from_fn(|i| {
            let controls = [[i as f32, DIMENSION as f32, radius, SUN_REACH],
                [areas[0], areas[1], areas[2], 0.0],
                [SKY_STRENGTH, SUN_STRENGTH, 24.0, 32.0]];
            let values = buffer(gpu, "meadow contact controls", bytemuck::cast_slice(&controls), wgpu::BufferUsages::UNIFORM);
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("meadow contact output"), layout: &output_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: output_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: values.as_entire_binding() }],
            })
        });
        let empty_hiz = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("unused contact Hi-Z"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        Some(Self { buffer: output_buffer, pipeline, view_layout: view_layout.clone(), settings,
            generation, output, empty_hiz, radius })
    }

    pub(super) fn stats(&self) -> Stats {
        Stats { enabled: self.radius > 0.0, shared_bytes: BYTES, radius_m: self.radius,
            sky_strength: SKY_STRENGTH, sun_strength: SUN_STRENGTH,
            sun_reach_m: SUN_REACH, fade_m: [24.0, 32.0], grid_dimension: DIMENSION }
    }

    pub(super) fn prepare(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, index: usize,
        camera: &wgpu::Buffer, settings: Settings, enabled: bool,
        time: Option<(&wgpu::QuerySet, u32)>) -> Result<(), String> {
        if index >= VIEWS as usize { return Err("meadow edge contact supports at most four views".into()); }
        if !enabled || self.radius <= 0.0 {
            enc.clear_buffer(&self.buffer, index as u64 * BLOCK as u64 * 16, Some(BLOCK as u64 * 16));
            return Ok(());
        }
        gpu.queue.write_buffer(&self.settings[index], 0, bytemuck::bytes_of(&settings));
        let view = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow contact view"), layout: &self.view_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.empty_hiz) }],
        });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meadow edge contact"), timestamp_writes: time.map(|(query_set, i)|
                wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) }),
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.generation[index], &[]);
        pass.set_bind_group(1, &view, &[]);
        pass.set_bind_group(2, &self.output[index], &[]);
        pass.dispatch_workgroups(DIMENSION.div_ceil(8), DIMENSION.div_ceil(8), 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    #[test]
    fn accepted_contact_shader_validates_and_uses_source_habitat_and_blade_decisions() {
        let source = source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let body = include_str!("shaders/meadow_contact_generate.wgsl");
        assert!(body.contains("meadow_generate_cell(cell,true)"));
        assert!(body.contains("meadow_road_anchor_allowed("));
        assert!(body.contains("if accepted<=0.0 {return;}"));
        assert!(!body.contains("meadow_in_frustum"));
        assert!(!body.contains("meadow_occluded"));
        assert_eq!(BYTES, 1_459_520);
    }
    #[test]
    fn twenty_five_neighbours_bound_both_shifted_kernels_and_cache_guards() {
        let anchor = 0.315f32;
        assert!((contact_radius(anchor) - 0.495).abs() < 1e-6);
        let minimum_half_width = (DIMENSION / 2) as f32 * ROOT_CELL;
        for anchor in [0.0f32, 0.2, 0.315, 0.4, 0.6, 0.9, 0.94] {
            let radius = contact_radius(anchor);
            if radius <= 0.0 { continue; }
            // Each query is at most half the bounded sun offset from the
            // midpoint. Moving a root's centroid toward it is conservative.
            for fraction in [0.0f32, 0.001, 0.5, 0.999999] {
                for omitted in [-3i32, 3] {
                    let distance = ((omitted as f32 + 0.5 - fraction) * ROOT_CELL).abs()
                        - 0.04 * ROOT_CELL - anchor - SUN_REACH * 0.5;
                    assert!(distance > radius, "anchor={anchor}, radius={radius}");
                }
            }
            assert!(minimum_half_width > 32.0 + SUN_REACH + ROOT_CELL + anchor + radius);
        }
        assert_eq!(contact_radius(0.94), 0.0);
        let source = crate::wind::shade_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let shade = include_str!("shaders/shade.wgsl");
        assert!(shade.contains("let midpoint=(p.xz+sun_query)*0.5;"));
        assert!(shade.contains("for(var z=0i;z<5i;z++){for(var x=0i;x<5i;x++)"));
    }

    #[test]
    #[ignore = "serial GPU fixture; requires an idle adapter"]
    fn accepted_contact_gpu_rejects_empty_habitat_and_keeps_offscreen_sources_and_view_isolation() {
        use crate::{foliage_assembly::MeadowBladeBudget, meadow_data::MeadowGeometry};
        let gpu = Gpu::new().expect("headless GPU");
        let generation_layout = layout(&gpu, "contact oracle source", &[(0, uniform()),
            (1, storage(true)), (2, storage(false)), (3, storage(false)), (4, storage(true))], wgpu::ShaderStages::COMPUTE);
        let view_layout = layout(&gpu, "contact oracle view", &[(0, uniform()),
            (1, super::super::texture(wgpu::TextureSampleType::Float { filterable: false }, wgpu::TextureViewDimension::D2))], wgpu::ShaderStages::COMPUTE);
        let output_layout = layout(&gpu, "contact oracle output", &[(0, storage(false)), (1, uniform())], wgpu::ShaderStages::COMPUTE);
        let pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("contact oracle"), bind_group_layouts: &[Some(&generation_layout), Some(&view_layout), Some(&output_layout)], immediate_size: 0,
        });
        let module = shader(&gpu, "production contact producer", source());
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("contact oracle"), layout: Some(&pl), module: &module,
            entry_point: Some("generate_meadow_contact"), compilation_options: Default::default(), cache: None,
        });
        let unused_pool = buffer(&gpu, "oracle unused pool", &[0; 16], wgpu::BufferUsages::STORAGE);
        let unused_args = buffer(&gpu, "oracle unused args", &[0; 60], wgpu::BufferUsages::STORAGE);
        let mut camera = [[0.0f32; 4]; 20];
        camera[8] = [0.0, 0.0, 0.0, 1.0];
        // Every source sphere fails these planes. The contact producer must
        // nevertheless retain eligible occluders, including off-screen ones.
        camera[9..15].fill([0.0, 1.0, 0.0, -1000.0]);
        camera[15][1] = 0.01;
        let camera = buffer(&gpu, "oracle camera", bytemuck::cast_slice(&camera), wgpu::BufferUsages::UNIFORM);
        let hiz = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("oracle unused Hi-Z"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        }).create_view(&Default::default());
        let view = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oracle view"), layout: &view_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: camera.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&hiz) }],
        });
        const D: usize = 7;
        const STRIDE: usize = HEADER as usize + D * D;
        const ACTIVE: usize = 2;
        let controls = [[ACTIVE as f32, D as f32, contact_radius(0.315), SUN_REACH], [1.0f32; 4], [SKY_STRENGTH, SUN_STRENGTH, 24.0, 32.0]];
        let controls = buffer(&gpu, "oracle controls", bytemuck::cast_slice(&controls), wgpu::BufferUsages::UNIFORM);
        for mode in 0..3 {
            let profile=if mode==0 {0.0} else {2.0};
            let geometry = if mode==2 {
                use crate::meadow_windswept::WindsweptRecipe;
                MeadowGeometry::cook_windswept_recipes(73,0.0,MeadowBladeBudget::Blades120,
                    [WindsweptRecipe {height_scale:0.5,width_scale:0.9,..Default::default()},
                        WindsweptRecipe {height_scale:0.7,width_scale:1.4,..Default::default()},
                        WindsweptRecipe {height_scale:1.15,..Default::default()}],12.0)
            } else if profile > 0.0 { MeadowGeometry::cook_windswept(73, 0.0, MeadowBladeBudget::Blades120) }
                else { MeadowGeometry::cook_with_blade_budget(73, 0.0, MeadowBladeBudget::Blades120) }.unwrap();
            let mut recipe = vec![[0.0f32; 4]; 21];
            for variant in 0..VARIANTS {
                for lod in 0..3 {
                    let [offset, count] = geometry.lod_ranges[variant][lod];
                    recipe[variant * 3 + lod] = [f32::from_bits(offset), f32::from_bits(count), 0.0, 0.0];
                }
                for branch in 0..4 { recipe[9 + variant * 4 + branch] = geometry.branch_pivots[variant][branch]; }
            }
            recipe[0][2]=geometry.variant_patch_metres;
            for vertex in &geometry.vertices { recipe.extend([vertex.position, vertex.normal, vertex.color, vertex.anchor]); }
            let recipe = buffer(&gpu, "oracle recipe", bytemuck::cast_slice(&recipe), wgpu::BufferUsages::STORAGE);
            for case in 0..5 {
                let mut settings = Settings::zeroed();
                settings.field = [-8.0, -8.0, 4.0, ROOT_CELL]; settings.grid = [5, 5, 73, 3];
                settings.shape = [1.0, geometry.max_tuft_radius, geometry.max_tuft_height, 80.0];
                settings.range = [100.0, 0.0, 1.0, 0.0]; settings.distances = [0.0, 0.0, 1.0, 1.0];
                settings.support = [geometry.max_anchor_radius, geometry.max_blade_radius, 0.0, 0.0];
                settings.edits = [1.0, 1.0, 1.0, if case == 3 { 0.0 } else { 1.0 }];
                settings.wind_history[3] = profile;
                let settings = buffer(&gpu, "oracle settings", bytemuck::bytes_of(&settings), wgpu::BufferUsages::UNIFORM);
                let terrain: Vec<_> = (0..25).map(|i| {
                    let x = -8.0 + (i % 5) as f32 * 4.0;
                    [0.0f32, if case == 1 { 0.0 } else if case == 4 { x + 2.5 } else { 10.0 },
                        if case == 2 { 0.0 } else { 1.0 }, 0.0]
                }).collect();
                let terrain = buffer(&gpu, "oracle habitat", bytemuck::cast_slice(&terrain), wgpu::BufferUsages::STORAGE);
                let generation = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("oracle source"), layout: &generation_layout,
                    entries: &[(0, &settings), (1, &terrain), (2, &unused_pool), (3, &unused_args), (4, &recipe)]
                        .map(|(binding, b)| wgpu::BindGroupEntry { binding, resource: b.as_entire_binding() }),
                });
                let sentinel = vec![[-77.0f32; 4]; STRIDE * VIEWS as usize];
                let output = buffer(&gpu, "oracle contact field", bytemuck::cast_slice(&sentinel),
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
                let output_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("oracle output"), layout: &output_layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: output.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: controls.as_entire_binding() }],
                });
                let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("oracle readback"),
                    size: output.size(), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline); pass.set_bind_group(0, &generation, &[]);
                    pass.set_bind_group(1, &view, &[]); pass.set_bind_group(2, &output_group, &[]);
                    pass.dispatch_workgroups(1, 1, 1);
                }
                encoder.copy_buffer_to_buffer(&output, 0, &read, 0, output.size());
                gpu.queue.submit([encoder.finish()]);
                read.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map contact oracle"));
                gpu.wait();
                let bytes = read.slice(..).get_mapped_range().unwrap();
                let actual: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
                assert_eq!(&actual[..ACTIVE * STRIDE], &sentinel[..ACTIVE * STRIDE]);
                assert_eq!(&actual[(ACTIVE + 1) * STRIDE..], &sentinel[(ACTIVE + 1) * STRIDE..]);
                let records = &actual[ACTIVE * STRIDE + HEADER as usize..(ACTIVE + 1) * STRIDE];
                assert!(records.iter().flatten().all(|x| x.is_finite()));
                assert!(records.iter().all(|p| (0.0..=1.0).contains(&p[3])));
                let occupied = records.iter().filter(|p| p[3] > 0.0).count();
                match case {
                    0 => assert_eq!(occupied, D * D, "off-screen eligible roots, profile {profile}"),
                    1..=3 => assert_eq!(occupied, 0, "road/unknown/density empty case {case}, profile {profile}"),
                    4 => assert!(occupied > 0 && occupied < D * D, "real road cut must leave both occupied and empty cells: {occupied}, profile {profile}"),
                    _ => unreachable!(),
                }
            }
        }
    }
}
