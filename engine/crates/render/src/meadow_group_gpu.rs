//! GPU producer/consumer for the experimental shared meadow surface pages.
//! Kept separate from the reference path. The page producer executes the same
//! root function and same relightable source atlas; it never cooks lighting.
use super::group_cache as cache;
use super::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Page {
    vp: [[f32; 4]; 4],
    inverse: [[f32; 4]; 4],
    lower: [f32; 4],
    upper: [f32; 4],
    direction: [f32; 4],
    identity: [i32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Table {
    control: [u32; 4],
    lookup: [[u32; 4]; 256],
    active: [[u32; 4]; 64],
    pages: [Page; 256],
}
const DECISION_WORDS: u64 = 780;
pub(super) const DIAGNOSTIC_BYTES: u64 = 8 * 4;
pub(super) const PAGE_DRAW_OFFSET: u64 = 776 * 4;
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PageView {
    vp: [[f32; 4]; 4],
    inverse: [[f32; 4]; 4],
    camera: [f32; 4],
    planes: [[f32; 4]; 6],
    params: [f32; 4],
    size: [f32; 4],
    extra: [f32; 4],
    grass: [f32; 4],
    fog: [f32; 4],
}

/// Page scheduling plus explicit diagnostic readback. Replacement counters
/// certify the current page consumer, not source-image optical equivalence.
/// Normal frame rendering never waits for these GPU counters.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub enabled: bool,
    pub replacement_enabled: bool,
    pub frame: u64,
    pub requested: u32,
    pub hits: u32,
    pub misses: u32,
    pub cooked: u32,
    pub deferred: u32,
    pub resident: u32,
    pub evictions: u32,
    pub active_pages: u32,
    pub page_triangles: u64,
    /// Actual reference root quads submitted per phase. Replacement mode has
    /// already removed whole certified groups; shade-only mode masks pixels.
    pub reference_roots_submitted: u32,
    /// Explicit GPU readback: complete current-view certificates, not cache hits.
    pub certificate_pages: u32,
    pub certified_pages: u32,
    pub useful_pages: u32,
    pub fallback_pages: u32,
    pub certificate_pixels: u32,
    pub replaced_roots: u32,
    /// Two quad triangles per removed root, per depth/material pass.
    pub saved_root_triangles: u64,
    pub producer_root_candidates: u32,
    pub update_budget: u32,
    pub capacity: u32,
    pub gpu_bytes: u64,
    pub cpu_bounds_bytes: u64,
    pub request_cpu_ms: f64,
}
struct Prepared {
    frame: u64,
    tables: Vec<wgpu::Buffer>,
    decisions: Vec<wgpu::Buffer>,
    stats: Vec<Stats>,
    pending: Vec<usize>,
}
struct State {
    requested_frame: u64,
    views: Vec<crate::ViewRect>,
    residency: cache::Residency,
    prepared: Option<Prepared>,
    signature: Option<[u32; 9]>,
}
pub(super) struct GpuGroups {
    replacement: bool,
    terrain: cache::TerrainBounds,
    state: Mutex<State>,
    texture: wgpu::TextureView,
    material: wgpu::TextureView,
    material_layers: [wgpu::TextureView; 3],
    nearest: wgpu::TextureView,
    furthest: wgpu::TextureView,
    zero_depth: wgpu::TextureView,
    dummy_depth: wgpu::TextureView,
    motion: wgpu::TextureView,
    draw_layout: wgpu::BindGroupLayout,
    view_layout: wgpu::BindGroupLayout,
    pub empty_scene: wgpu::BindGroup,
    pub reference_depth: [wgpu::RenderPipeline; 2],
    pub reference_material: wgpu::RenderPipeline,
    pub page_depth: [wgpu::RenderPipeline; 2],
    pub page_material: wgpu::RenderPipeline,
    far_depth: wgpu::RenderPipeline,
    generate: wgpu::ComputePipeline,
    replacement_layout: wgpu::BindGroupLayout,
    replacement_generate: wgpu::ComputePipeline,
    certificate_layout: wgpu::BindGroupLayout,
    certificate: wgpu::ComputePipeline,
    finalize: wgpu::ComputePipeline,
    pack_layout: wgpu::BindGroupLayout,
    pack: wgpu::ComputePipeline,
    pool: wgpu::Buffer,
    args: wgpu::Buffer,
    initial: wgpu::Buffer,
    empty_table: wgpu::Buffer,
    empty_decisions: wgpu::Buffer,
    producer_table: wgpu::Buffer,
    bytes: u64,
}

pub fn source() -> String {
    let base = super::shader_source();
    let original = "let offset=in.owner*MEADOW_PLANT_WORDS;let root=meadow_read4(offset);let yaw=meadow_read4(offset+4u);";
    let changed =
        format!("{original}\n    if meadow_group_replaces(in.position.xy,root.xyz) {{discard;}}");
    assert_eq!(
        base.matches(original).count(),
        1,
        "page fallback must guard the unique aggregate sampler"
    );
    format!(
        "{}\n{}",
        base.replace(original, &changed),
        include_str!("shaders/meadow_groups.wgsl")
    )
}
pub fn certificate_source() -> String {
    let page = include_str!("shaders/meadow_groups.wgsl")
        .split("@vertex fn meadow_page_vs")
        .next()
        .unwrap()
        .replace(
            "var<storage,read> meadow_page_decisions:array<u32>",
            "var<storage,read_write> meadow_page_decisions:array<atomic<u32>>",
        )
        .replace(
            "return meadow_page_decisions[index];",
            "return atomicLoad(&meadow_page_decisions[index]);",
        );
    format!(
        "{}\n{page}\n{}",
        super::shader_source(),
        include_str!("shaders/meadow_group_certificate.wgsl")
    )
}
pub fn replacement_source() -> String {
    let original = "fn meadow_retain_root(cell:vec2<i32>,root:vec3<f32>)->bool {return true;}";
    let generated = super::generate_source();
    assert_eq!(
        generated.matches(original).count(),
        1,
        "shared root retention hook changed"
    );
    format!("{}\n{}",generated.replace(original,
        "fn meadow_retain_root(cell:vec2<i32>,root:vec3<f32>)->bool {return meadow_retain_certified_root(cell,root);}"),
        include_str!("shaders/meadow_group_generate.wgsl"))
}
fn make_texture(
    gpu: &Gpu,
    label: &str,
    size: (u32, u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: size.2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
impl GpuGroups {
    pub fn new(
        gpu: &Gpu,
        field: &crate::meadow_data::MeadowField,
        generate_layout: &wgpu::BindGroupLayout,
        generate_view_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, String> {
        let replacement = std::env::var("KERN_MEADOW_GROUP_REPLACE").as_deref() == Ok("1");
        if std::mem::size_of::<Table>() as u64 > gpu.limits.max_uniform_buffer_binding_size
            || gpu.limits.max_texture_array_layers < 256
        {
            return Err(
                "meadow group cache requires a 54288-byte uniform and 256 texture layers".into(),
            );
        }
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        let target = sampled | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let page_texture = make_texture(
            gpu,
            "meadow shared world pages",
            (256, 64, 256),
            wgpu::TextureFormat::Rgba16Float,
            sampled | wgpu::TextureUsages::STORAGE_BINDING,
        )
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let material_texture = make_texture(
            gpu,
            "meadow page OIT scratch",
            (64, 64, 3),
            wgpu::TextureFormat::Rgba16Float,
            target,
        );
        let material_layers = std::array::from_fn(|i| {
            material_texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: i as u32,
                array_layer_count: Some(1),
                ..Default::default()
            })
        });
        let material = material_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let nearest = make_texture(
            gpu,
            "meadow page nearest",
            (64, 64, 1),
            wgpu::TextureFormat::Depth32Float,
            target,
        )
        .create_view(&Default::default());
        let furthest = make_texture(
            gpu,
            "meadow page farthest",
            (64, 64, 1),
            wgpu::TextureFormat::Depth32Float,
            target,
        )
        .create_view(&Default::default());
        let zero_depth = make_texture(
            gpu,
            "meadow page material depth",
            (64, 64, 1),
            wgpu::TextureFormat::Depth32Float,
            target,
        )
        .create_view(&Default::default());
        let dummy_depth = make_texture(
            gpu,
            "meadow producer unused opaque",
            (1, 1, 1),
            wgpu::TextureFormat::Depth32Float,
            target,
        )
        .create_view(&Default::default());
        let motion = make_texture(
            gpu,
            "meadow page motion",
            (64, 64, 1),
            wgpu::TextureFormat::Rgba32Float,
            target,
        )
        .create_view(&Default::default());
        let d2 = wgpu::TextureViewDimension::D2;
        let array = wgpu::TextureViewDimension::D2Array;
        let draw_layout = layout(
            gpu,
            "meadow page-backed draw",
            &[
                (0, uniform()),
                (1, storage(true)),
                (2, storage(true)),
                (3, uniform()),
                (
                    4,
                    texture(wgpu::TextureSampleType::Float { filterable: true }, array),
                ),
                (
                    5,
                    texture(wgpu::TextureSampleType::Float { filterable: false }, array),
                ),
                (
                    6,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                (
                    7,
                    texture(wgpu::TextureSampleType::Float { filterable: false }, array),
                ),
                (8, texture(wgpu::TextureSampleType::Depth, d2)),
                (9, uniform()),
                (10, storage(true)),
            ],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        // Unlit production/accumulation needs no scene lighting resources.
        // A narrow layout avoids exceeding the portable 16 sampled textures
        // when page/depth inputs are added to the already-full scene layout.
        let empty_layout = layout(
            gpu,
            "meadow page empty scene",
            &[],
            wgpu::ShaderStages::VERTEX_FRAGMENT,
        );
        let empty_scene = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow page empty scene"),
            layout: &empty_layout,
            entries: &[],
        });
        let view_layout = layout(
            gpu,
            "meadow page camera",
            &[(0, uniform())],
            wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
        );
        let pipeline_layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow pages"),
                bind_group_layouts: &[Some(&empty_layout), Some(&view_layout), Some(&draw_layout)],
                immediate_size: 0,
            });
        let module = shader(gpu, "meadow world pages", source());
        let pipeline = |page: bool, depth: bool, motion: bool, far: bool| {
            let add = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            };
            let reveal = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            };
            let targets = if depth {
                if motion {
                    vec![Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })]
                } else {
                    vec![]
                }
            } else {
                (0..3)
                    .map(|i| {
                        Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba16Float,
                            blend: Some(wgpu::BlendState {
                                color: if i == 2 { reveal } else { add },
                                alpha: if i == 2 { reveal } else { add },
                            }),
                            write_mask: wgpu::ColorWrites::ALL,
                        })
                    })
                    .collect()
            };
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("meadow world page pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(if page {
                            "meadow_page_vs"
                        } else {
                            "meadow_aggregate_vs"
                        }),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: wgpu::TextureFormat::Depth32Float,
                        depth_write_enabled: Some(depth),
                        depth_compare: Some(if far {
                            wgpu::CompareFunction::LessEqual
                        } else {
                            wgpu::CompareFunction::GreaterEqual
                        }),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(match (page, depth, motion) {
                            (true, true, true) => "meadow_page_motion_fs",
                            (true, true, false) => "meadow_page_depth_fs",
                            (true, false, _) => "meadow_page_fs",
                            (false, true, true) => "meadow_aggregate_depth_motion_fs",
                            (false, true, false) => "meadow_aggregate_depth_fs",
                            (false, false, _) => "meadow_aggregate_fs",
                        }),
                        compilation_options: Default::default(),
                        targets: &targets,
                    }),
                    multiview_mask: None,
                    cache: gpu.pipeline_cache(),
                })
        };
        let reference_depth = [
            pipeline(false, true, false, false),
            pipeline(false, true, true, false),
        ];
        let reference_material = pipeline(false, false, false, false);
        let page_depth = [
            pipeline(true, true, false, false),
            pipeline(true, true, true, false),
        ];
        let page_material = pipeline(true, false, false, false);
        let far_depth = pipeline(false, true, false, true);
        let gl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow page roots"),
                bind_group_layouts: &[Some(generate_layout), Some(generate_view_layout)],
                immediate_size: 0,
            });
        let generate = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meadow page roots"),
                layout: Some(&gl),
                module: &shader(gpu, "meadow page roots", super::generate_source()),
                entry_point: Some("generate_meadow_page"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        let replacement_layout = layout(
            gpu,
            "meadow certified root selection",
            &[(0, uniform()), (1, storage(false))],
            wgpu::ShaderStages::COMPUTE,
        );
        let replacement_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("meadow certified root selection"),
                    bind_group_layouts: &[
                        Some(generate_layout),
                        Some(generate_view_layout),
                        Some(&replacement_layout),
                    ],
                    immediate_size: 0,
                });
        let replacement_generate =
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("meadow certified roots"),
                    layout: Some(&replacement_pipeline_layout),
                    module: &shader(gpu, "meadow certified roots", replacement_source()),
                    entry_point: Some("generate_meadow"),
                    compilation_options: Default::default(),
                    cache: gpu.pipeline_cache(),
                });
        let certificate_layout = layout(
            gpu,
            "meadow whole-page certificate",
            &[
                (0, uniform()),
                (
                    7,
                    texture(wgpu::TextureSampleType::Float { filterable: false }, array),
                ),
                (8, texture(wgpu::TextureSampleType::Depth, d2)),
                (9, uniform()),
                (10, storage(false)),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let certificate_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("meadow whole-page certificate"),
                    bind_group_layouts: &[
                        Some(&empty_layout),
                        Some(&view_layout),
                        Some(&certificate_layout),
                    ],
                    immediate_size: 0,
                });
        let certificate_module = shader(gpu, "meadow whole-page certificate", certificate_source());
        let make_certificate = |entry| {
            gpu.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&certificate_pipeline_layout),
                    module: &certificate_module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: gpu.pipeline_cache(),
                })
        };
        let certificate = make_certificate("certify_meadow_pages");
        let finalize = make_certificate("finalize_meadow_pages");
        let pack_layout = layout(
            gpu,
            "meadow page packing",
            &[
                (
                    0,
                    texture(wgpu::TextureSampleType::Float { filterable: false }, array),
                ),
                (1, texture(wgpu::TextureSampleType::Depth, d2)),
                (2, texture(wgpu::TextureSampleType::Depth, d2)),
                (
                    3,
                    texture(wgpu::TextureSampleType::Float { filterable: false }, d2),
                ),
                (
                    4,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: array,
                    },
                ),
                (5, uniform()),
            ],
            wgpu::ShaderStages::COMPUTE,
        );
        let pl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meadow page packing"),
                bind_group_layouts: &[Some(&pack_layout)],
                immediate_size: 0,
            });
        let pack = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meadow page packing"),
                layout: Some(&pl),
                module: &shader(
                    gpu,
                    "meadow page packing",
                    include_str!("shaders/meadow_group_pack.wgsl").into(),
                ),
                entry_point: Some("pack_page"),
                compilation_options: Default::default(),
                cache: gpu.pipeline_cache(),
            });
        let pool = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meadow page root scratch"),
            size: 64 * 31 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let args = buffer(
            gpu,
            "meadow page draws",
            &[0; ARGS_BYTES as usize],
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST,
        );
        let initial = buffer(
            gpu,
            "meadow page draw reset",
            bytemuck::cast_slice(&initial_draw_args([6, 0, 0])),
            wgpu::BufferUsages::COPY_SRC,
        );
        let empty_table = buffer(
            gpu,
            "meadow disabled pages",
            bytemuck::bytes_of(&Table::zeroed()),
            wgpu::BufferUsages::UNIFORM,
        );
        let empty_decisions = buffer(
            gpu,
            "meadow disabled certificate decisions",
            &[0; DECISION_WORDS as usize * 4],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        );
        let mut producer = Table::zeroed();
        producer.control[0] = 2;
        let producer_table = buffer(
            gpu,
            "meadow page producer control",
            bytemuck::bytes_of(&producer),
            wgpu::BufferUsages::UNIFORM,
        );
        let bytes = 256 * 64 * 256 * 8
            + 64 * 64 * (3 * 8 + 3 * 4 + 16)
            + 4
            + 64 * 31 * 4
            + ARGS_BYTES * 2
            + std::mem::size_of::<Table>() as u64 * 6
            + 4 * (std::mem::size_of::<Settings>() as u64 + 320 + 16 + 16)
            + 4 * 16
            + DECISION_WORDS * 4 * 5;
        if bytes > 64 * 1024 * 1024 {
            return Err("meadow group cache exceeds its 64 MiB cap".into());
        }
        Ok(Self {
            replacement,
            terrain: cache::TerrainBounds::new(field),
            state: Mutex::new(State {
                requested_frame: 0,
                views: Vec::new(),
                residency: cache::Residency::default(),
                prepared: None,
                signature: None,
            }),
            texture: page_texture,
            material,
            material_layers,
            nearest,
            furthest,
            zero_depth,
            dummy_depth,
            motion,
            draw_layout,
            view_layout,
            empty_scene,
            reference_depth,
            reference_material,
            page_depth,
            page_material,
            far_depth,
            generate,
            replacement_layout,
            replacement_generate,
            certificate_layout,
            certificate,
            finalize,
            pack_layout,
            pack,
            pool,
            args,
            initial,
            empty_table,
            empty_decisions,
            producer_table,
            bytes,
        })
    }
    pub fn set_views(&self, views: &[crate::ViewRect], frame: u64) {
        let mut state = self.state.lock().unwrap();
        state.views = views.iter().copied().take(4).collect();
        state.requested_frame = frame;
    }
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap();
        state.residency.clear();
        state.prepared = None;
        state.signature = None;
    }
    pub fn submitted(&self) {
        let mut state = self.state.lock().unwrap();
        let pending = state
            .prepared
            .as_mut()
            .map(|p| std::mem::take(&mut p.pending))
            .unwrap_or_default();
        state.residency.commit(&pending);
    }
    pub fn stats(&self, view: usize) -> Stats {
        self.state
            .lock()
            .unwrap()
            .prepared
            .as_ref()
            .and_then(|p| p.stats.get(view))
            .cloned()
            .unwrap_or_else(|| Stats {
                enabled: true,
                replacement_enabled: self.replacement,
                capacity: 256,
                update_budget: 4,
                gpu_bytes: self.bytes,
                cpu_bounds_bytes: self.terrain.bytes(),
                ..Default::default()
            })
    }

    fn bind(
        &self,
        gpu: &Gpu,
        settings: &wgpu::Buffer,
        pool: &wgpu::Buffer,
        recipe: &wgpu::Buffer,
        atlas: &AtlasTextures,
        capacity: u32,
        opaque: &wgpu::TextureView,
        table: &wgpu::Buffer,
        decisions: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let draw = buffer(
            gpu,
            "meadow page draw parameters",
            bytemuck::cast_slice(&[0u32, capacity, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow page-backed draw"),
            layout: &self.draw_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: recipe.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: draw.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&atlas.material),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&atlas.surface),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&atlas.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&self.texture),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(opaque),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: decisions.as_entire_binding(),
                },
            ],
        })
    }
    pub fn draw_bind(
        &self,
        gpu: &Gpu,
        view: usize,
        settings: &wgpu::Buffer,
        pool: &wgpu::Buffer,
        recipe: &wgpu::Buffer,
        atlas: &AtlasTextures,
        capacity: u32,
        opaque: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let state = self.state.lock().unwrap();
        let table = state
            .prepared
            .as_ref()
            .and_then(|p| p.tables.get(view))
            .unwrap_or(&self.empty_table);
        let decisions = state
            .prepared
            .as_ref()
            .and_then(|p| p.decisions.get(view))
            .unwrap_or(&self.empty_decisions);
        self.bind(
            gpu, settings, pool, recipe, atlas, capacity, opaque, table, decisions,
        )
    }
    pub fn page_draw_args(&self, view: usize) -> Option<wgpu::Buffer> {
        if !self.replacement {
            return None;
        }
        self.state
            .lock()
            .unwrap()
            .prepared
            .as_ref()
            .and_then(|p| p.decisions.get(view))
            .cloned()
    }
    pub fn copy_diagnostics(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: usize,
        destination: &wgpu::Buffer,
        offset: u64,
    ) {
        let state = self.state.lock().unwrap();
        let data = state
            .prepared
            .as_ref()
            .and_then(|p| p.decisions.get(view))
            .unwrap_or(&self.empty_decisions);
        encoder.copy_buffer_to_buffer(data, 0, destination, offset, DIAGNOSTIC_BYTES);
    }
    pub fn apply_diagnostics(stats: &mut Stats, words: &[u32]) -> Result<(), String> {
        if words.len() < 8 {
            return Err("meadow group certificate diagnostics truncated".into());
        }
        stats.certificate_pages = words[0];
        stats.certified_pages = words[1];
        stats.useful_pages = words[2];
        stats.certificate_pixels = words[3];
        stats.replaced_roots = words[4];
        stats.fallback_pages = words[5];
        stats.saved_root_triangles = u64::from(words[4]) * 2;
        if words[0] > 256
            || u64::from(words[1]) + u64::from(words[5]) != u64::from(words[0])
            || words[2] > words[1]
            || u64::from(words[3]) > u64::from(words[0]) * 4096
            || u64::from(words[4]) > u64::from(words[1]) * 64
        {
            return Err("meadow group certificate counters exceed bounded work".into());
        }
        if stats.replacement_enabled {
            stats.page_triangles = u64::from(words[2]) * 2;
        }
        Ok(())
    }
    pub fn view_bind(
        &self,
        gpu: &Gpu,
        bindings: &[(u32, wgpu::BindingResource)],
    ) -> Result<wgpu::BindGroup, String> {
        let resource = bindings
            .iter()
            .find(|(b, _)| *b == 0)
            .ok_or("meadow page missing camera")?
            .1
            .clone();
        Ok(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow page camera"),
            layout: &self.view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource,
            }],
        }))
    }

    /// Ordered current-frame certification -> reference member suppression ->
    /// compact useful-page draw. No GPU readback or per-frame CPU root list.
    pub fn generate_replacement(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        settings: &wgpu::Buffer,
        generation: &wgpu::BindGroup,
        generation_view: &wgpu::BindGroup,
        bindings: &[(u32, wgpu::BindingResource)],
        opaque: &wgpu::TextureView,
        dimension: u32,
        timing: Option<(&wgpu::QuerySet, u32)>,
    ) -> Result<bool, String> {
        if !self.replacement {
            return Ok(false);
        }
        let state = self.state.lock().unwrap();
        let prepared = state
            .prepared
            .as_ref()
            .ok_or("meadow page replacement has no current plan")?;
        let table = prepared
            .tables
            .get(index)
            .ok_or("meadow page replacement view missing")?;
        let decisions = prepared
            .decisions
            .get(index)
            .ok_or("meadow page replacement decisions missing")?;
        let count = prepared.stats[index].active_pages;
        // Reset is in command order, never a queue.write_buffer competing with
        // another view/frame. The decision buffer is unique to this view/frame.
        encoder.clear_buffer(decisions, 0, None);
        let camera = self.view_bind(gpu, bindings)?;
        let certificate = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow current-view certificate inputs"),
            layout: &self.certificate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: settings.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&self.texture),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(opaque),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: decisions.as_entire_binding(),
                },
            ],
        });
        if count > 0 {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meadow whole-page certification"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.certificate);
            pass.set_bind_group(0, &self.empty_scene, &[]);
            pass.set_bind_group(1, &camera, &[]);
            pass.set_bind_group(2, &certificate, &[]);
            pass.dispatch_workgroups(count, 1, 1);
        }
        let replacement = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow root replacement decisions"),
            layout: &self.replacement_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: decisions.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meadow remaining far roots"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.replacement_generate);
            pass.set_bind_group(0, generation, &[]);
            pass.set_bind_group(1, generation_view, &[]);
            pass.set_bind_group(2, &replacement, &[]);
            pass.dispatch_workgroups(dimension.div_ceil(8), dimension.div_ceil(8), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meadow useful-page compact draw"),
                timestamp_writes: timing.map(|(query_set, i)| wgpu::ComputePassTimestampWrites {
                    query_set,
                    beginning_of_pass_write_index: None,
                    end_of_pass_write_index: Some(i + 1),
                }),
            });
            pass.set_pipeline(&self.finalize);
            pass.set_bind_group(0, &self.empty_scene, &[]);
            pass.set_bind_group(1, &camera, &[]);
            pass.set_bind_group(2, &certificate, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        Ok(true)
    }

    pub fn prepare(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        renderer: &MeadowRenderer,
        settings: Settings,
        ranges: [f32; 4],
    ) -> Result<(), String> {
        let started = std::time::Instant::now();
        let mut state = self.state.lock().unwrap();
        let frame = state.requested_frame;
        if state.prepared.as_ref().is_some_and(|p| p.frame == frame) || state.views.is_empty() {
            return Ok(());
        }
        let signature = [
            settings.shape[0].to_bits(),
            settings.edits[0].to_bits(),
            settings.edits[1].to_bits(),
            settings.edits[2].to_bits(),
            settings.edits[3].to_bits(),
            settings.range[1].to_bits(),
            settings.range[2].to_bits(),
            settings.previous[1].to_bits(),
            settings.previous[2].to_bits(),
        ];
        if state.signature != Some(signature) {
            state.residency.clear();
            state.signature = Some(signature);
        }
        let margin = settings.support[0]
            + settings.support[1] * settings.shape[0] * 1.3 * (1.0 + 0.22 * 1.5);
        let mut requests =
            cache::requests(&self.terrain, &state.views, [ranges[0], ranges[3]], margin);
        // Hysteresis is geometric rather than camera-cell hysteresis: retain
        // an already cooked world basis while its actual projection bound is
        // still valid. Crossing a direction-bin boundary alone is not a miss.
        for request in &mut requests {
            if let Some(page) = state.residency.pages.iter().flatten().find(|p| {
                p.ready
                    && p.key.x == request.key.x
                    && p.key.z == request.key.z
                    && state
                        .views
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| request.views & (1 << i) != 0)
                        .all(|(_, v)| {
                            let focal = v.h as f32 / (2.0 * (v.cam.fov_y * 0.5).tan());
                            cache::projection_error(p.bounds, p.key, v.cam, focal)
                                <= cache::MAX_ERROR_PIXELS * 0.8
                        })
            }) {
                request.key = page.key;
            }
        }
        let max_focal = state
            .views
            .iter()
            .map(|v| v.h as f32 / (2.0 * (v.cam.fov_y * 0.5).tan()))
            .fold(1.0f32, f32::max);
        // All branch harmonic derivatives have sum 9.2. The full supported
        // slope shear is bounded by two, and anchored growth by 1.3.
        let metres_per_phase = settings.support[1]
            * settings.shape[0]
            * 1.3
            * 2.0
            * 0.22
            * settings.range[1]
            * settings.range[2]
            * 9.2;
        let minimum_distance = requests
            .iter()
            .flat_map(|r| {
                state
                    .views
                    .iter()
                    .enumerate()
                    .filter(move |(i, _)| r.views & (1 << i) != 0)
                    .map(move |(_, v)| (v.cam.pos - r.bounds.center()).length() - r.bounds.radius())
            })
            .fold(ranges[3], f32::min);
        let pose_limit = if metres_per_phase > 1e-8 {
            0.2 * minimum_distance.max(1.0) / max_focal / metres_per_phase
        } else {
            std::f32::consts::PI
        };
        let old: std::collections::BTreeMap<_, _> = state
            .residency
            .pages
            .iter()
            .flatten()
            .filter(|p| p.ready)
            .map(|p| (p.key, p.phase))
            .collect();
        let plan = state.residency.plan(
            &requests,
            frame,
            settings.range[3],
            settings.range[1],
            pose_limit,
        );
        let cpu_ms = started.elapsed().as_secs_f64() * 1000.0;
        for &slot in &plan.updates {
            let page = state.residency.pages[slot].as_ref().unwrap();
            self.cook(
                gpu,
                enc,
                renderer,
                settings,
                slot,
                page,
                old.get(&page.key).copied(),
            )?;
        }
        let mut tables = Vec::new();
        let mut decisions = Vec::new();
        let mut stats = Vec::new();
        for view_index in 0..state.views.len() {
            let mut table = Table::zeroed();
            table.control = [1, 0, frame as u32, u32::from(self.replacement)];
            let mut active = 0;
            for request in requests.iter().filter(|r| r.views & (1 << view_index) != 0) {
                let Some(&slot) = plan.slots.get(&request.key) else {
                    continue;
                };
                let resident = state.residency.pages[slot].as_ref().unwrap();
                let (vp, _, _) = resident.bounds.projection(resident.key);
                let hash = ((resident.key.x as u32).wrapping_mul(0x9e3779b1)
                    ^ (resident.key.z as u32).wrapping_mul(0x85ebca77))
                    & 1023;
                let Some(address) = (0..16)
                    .map(|p| (hash + p) & 1023)
                    .find(|&a| table.lookup[a as usize / 4][a as usize % 4] == 0)
                else {
                    continue;
                };
                table.lookup[address as usize / 4][address as usize % 4] = slot as u32 + 1;
                table.pages[slot] = Page {
                    vp: vp.to_cols_array_2d(),
                    inverse: vp.inverse().to_cols_array_2d(),
                    lower: resident.bounds.lo.extend(0.0).to_array(),
                    upper: resident
                        .bounds
                        .hi
                        .extend(if resident.cooked_frame == frame {
                            1.0
                        } else {
                            0.0
                        })
                        .to_array(),
                    direction: resident.key.direction().extend(0.0).to_array(),
                    identity: [resident.key.x, resident.key.z, slot as i32, 1],
                };
                table.active[active as usize / 4][active as usize % 4] = slot as u32;
                active += 1;
            }
            table.control[1] = active;
            tables.push(buffer(
                gpu,
                "meadow view page table",
                bytemuck::bytes_of(&table),
                wgpu::BufferUsages::UNIFORM,
            ));
            decisions.push(buffer(
                gpu,
                "meadow current-view page decisions",
                &[0; DECISION_WORDS as usize * 4],
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::INDIRECT
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            ));
            stats.push(Stats {
                enabled: true,
                replacement_enabled: self.replacement,
                frame,
                requested: plan.stats.requested,
                hits: plan.stats.hits,
                misses: plan.stats.misses,
                cooked: plan.stats.cooked,
                deferred: plan.stats.deferred,
                resident: plan.stats.resident,
                evictions: plan.stats.evictions,
                active_pages: active,
                page_triangles: u64::from(active) * 2,
                reference_roots_submitted: 0,
                producer_root_candidates: plan.stats.cooked * 64,
                update_budget: 4,
                capacity: 256,
                gpu_bytes: self.bytes,
                cpu_bounds_bytes: self.terrain.bytes(),
                request_cpu_ms: cpu_ms,
                ..Default::default()
            });
        }
        // A new payload is available to following passes in this encoder, but
        // cannot be reused by a later frame until the queue submission hook.
        state.prepared = Some(Prepared {
            frame,
            tables,
            decisions,
            stats,
            pending: plan.updates,
        });
        Ok(())
    }

    fn cook(
        &self,
        gpu: &Gpu,
        enc: &mut wgpu::CommandEncoder,
        renderer: &MeadowRenderer,
        mut settings: Settings,
        slot: usize,
        page: &cache::Resident,
        old_phase: Option<f32>,
    ) -> Result<(), String> {
        let aggregate = renderer
            .aggregate
            .as_ref()
            .ok_or("group cache requires source aggregate")?;
        let (vp, eye, focal) = page.bounds.projection(page.key);
        let view = PageView {
            vp: vp.to_cols_array_2d(),
            inverse: vp.inverse().to_cols_array_2d(),
            camera: eye.extend(focal).to_array(),
            planes: [[0.0; 4]; 6],
            params: [1.0, 0.01, 1.0, 0.0],
            size: [64.0, 64.0, 1.0 / 64.0, 1.0 / 64.0],
            extra: [0.0; 4],
            grass: [0.0; 4],
            fog: [0.0; 4],
        };
        let origin = page.key.root_origin();
        settings.dispatch = [origin[0], origin[1], 8, 64];
        settings.support[2] = 2.0;
        settings.support[3] = f32::from_bits(aggregate.atlas_base);
        settings.range[3] = page.phase;
        settings.previous[0] = old_phase.unwrap_or(page.phase);
        settings.previous[3] = if old_phase.is_some() { 1.0 } else { 0.0 };
        let settings_buffer = buffer(
            gpu,
            "meadow page immutable settings",
            bytemuck::bytes_of(&settings),
            wgpu::BufferUsages::UNIFORM,
        );
        let view_buffer = buffer(
            gpu,
            "meadow page immutable projection",
            bytemuck::bytes_of(&view),
            wgpu::BufferUsages::UNIFORM,
        );
        let generation = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow page producer"),
            layout: &renderer.generate_layout,
            entries: &[
                (0, &settings_buffer),
                (1, &renderer.ground),
                (2, &self.pool),
                (3, &self.args),
                (4, &renderer.recipe),
            ]
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            }),
        });
        // The page producer deliberately bypasses camera Hi-Z, so the valid
        // bound texture is never sampled. All root eligibility stays shared.
        let gen_view = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow page orthographic view"),
            layout: &renderer.generate_view_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.material_layers[0]),
                },
            ],
        });
        enc.copy_buffer_to_buffer(&self.initial, 0, &self.args, 0, ARGS_BYTES);
        {
            let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("meadow cached page roots"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.generate);
            p.set_bind_group(0, &generation, &[]);
            p.set_bind_group(1, &gen_view, &[]);
            p.dispatch_workgroups(1, 1, 1);
        }
        let lighting = self.view_bind(gpu, &[(0, view_buffer.as_entire_binding())])?;
        let draw = self.bind(
            gpu,
            &settings_buffer,
            &self.pool,
            &renderer.recipe,
            &aggregate.atlas,
            64,
            &self.dummy_depth,
            &self.producer_table,
            &self.empty_decisions,
        );
        for phase in 0..3 {
            let attachments = if phase == 0 {
                vec![Some(wgpu::RenderPassColorAttachment {
                    view: &self.motion,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })]
            } else if phase == 1 {
                vec![]
            } else {
                (0..3)
                    .map(|i| {
                        Some(wgpu::RenderPassColorAttachment {
                            view: &self.material_layers[i],
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(if i == 2 {
                                    wgpu::Color::WHITE
                                } else {
                                    wgpu::Color::TRANSPARENT
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                        })
                    })
                    .collect()
            };
            // Material pass has a fresh zero-depth attachment: every source
            // layer contributes, independent of its nearest representative.
            let depth = if phase == 1 {
                &self.furthest
            } else if phase == 2 {
                &self.zero_depth
            } else {
                &self.nearest
            };
            let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("meadow page material cook"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if phase == 1 { 1.0 } else { 0.0 }),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            p.set_pipeline(if phase == 0 {
                &self.reference_depth[1]
            } else if phase == 1 {
                &self.far_depth
            } else {
                &self.reference_material
            });
            p.set_bind_group(0, &self.empty_scene, &[]);
            p.set_bind_group(1, &lighting, &[]);
            p.set_bind_group(2, &draw, &[]);
            p.draw_indirect(&self.args, 0);
        }
        let destination = buffer(
            gpu,
            "meadow page destination",
            bytemuck::cast_slice(&[slot as u32, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        let pack = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meadow page pack"),
            layout: &self.pack_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.material),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.nearest),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.furthest),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.motion),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.texture),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: destination.as_entire_binding(),
                },
            ],
        });
        let mut p = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meadow page packing"),
            timestamp_writes: None,
        });
        p.set_pipeline(&self.pack);
        p.set_bind_group(0, &pack, &[]);
        p.dispatch_workgroups(8, 8, 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_shader_and_cpu_abi_match_and_entrypoints_validate() {
        assert_eq!(std::mem::size_of::<Page>(), 192);
        assert_eq!(std::mem::size_of::<PageView>(), 320);
        assert_eq!(std::mem::size_of::<Table>(), 54288);
        for source in [
            source(),
            certificate_source(),
            replacement_source(),
            include_str!("shaders/meadow_group_pack.wgsl").into(),
        ] {
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            for (name, expected) in [("MeadowPage", 192), ("MeadowPages", 54288)] {
                if let Some((_, ty)) = module
                    .types
                    .iter()
                    .find(|(_, t)| t.name.as_deref() == Some(name))
                {
                    let naga::TypeInner::Struct { span, .. } = ty.inner else {
                        panic!("struct ABI")
                    };
                    assert_eq!(span, expected);
                }
            }
        }
    }
    #[test]
    fn replacement_counter_layout_and_bounds_are_checked() {
        assert_eq!(DECISION_WORDS, 8 + 256 * 3 + 4);
        assert_eq!(PAGE_DRAW_OFFSET + 16, DECISION_WORDS * 4);
        let mut stats = Stats {
            replacement_enabled: true,
            ..Default::default()
        };
        GpuGroups::apply_diagnostics(&mut stats, &[3, 2, 1, 8193, 65, 1, 0, 0]).unwrap();
        assert_eq!(stats.page_triangles, 2);
        assert_eq!(stats.saved_root_triangles, 130);
        // Missing finalization, truncated data, overflow and impossible useful
        // groups must fail diagnostic acceptance rather than inflate savings.
        for bad in [
            vec![3, 2, 1, 8193, 65, 0, 0, 0],
            vec![3, 2, 3, 0, 1, 1, 0, 0],
            vec![1, 1, 1, 4097, 1, 0, 0, 0],
            vec![1, 1, 1, 1, 65, 0, 0, 0],
            vec![0, u32::MAX, 0, 0, 0, 1, 0, 0],
            vec![0; 7],
        ] {
            assert!(GpuGroups::apply_diagnostics(&mut stats, &bad).is_err());
        }
        GpuGroups::apply_diagnostics(&mut stats, &[0; 8]).unwrap();
        assert_eq!(stats.page_triangles, 0);
    }
    #[test]
    fn root_suppression_uses_world_cells_after_visibility_before_pose_work() {
        // Signed shift used by WGSL must agree with the CPU page cooker even
        // on negative cell boundaries, independently of jitter and camera.
        for cell in -1025i32..=1025 {
            assert_eq!(cell >> 3, cell.div_euclid(cache::GROUP_SIDE));
        }
        let source = replacement_source();
        let body = source.split("fn meadow_generate_cell").nth(1).unwrap();
        let visibility = body.find("meadow_occluded(root,radius)").unwrap();
        let retain = body.find("!meadow_retain_root(cell,root)").unwrap();
        let wind = body.find("meadow_branch_angle(xz,").unwrap();
        assert!(visibility < retain && retain < wind);
        assert!(body.contains("if !page && meadow.support.z>1.5 && !meadow_retain_root"));
        assert!(
            super::super::generate_source().contains(
                "fn meadow_retain_root(cell:vec2<i32>,root:vec3<f32>)->bool {return true;}"
            )
        );
    }
    #[test]
    fn whole_page_certificate_does_not_hide_partial_failure_or_truncate_work() {
        // Independent Boolean oracle: one uncertain sample anywhere in the
        // complete footprint retains ALL members. Visibility is not validity.
        #[derive(Clone, Copy)]
        enum Pixel {
            Empty,
            Visible,
            Unknown,
        }
        let certify = |width: usize, height: usize, pixels: &[Pixel]| -> Option<bool> {
            if width == 0
                || height == 0
                || width > 64
                || height > 64
                || pixels.len() != width * height
            {
                return None;
            }
            let mut useful = false;
            for pixel in pixels {
                match pixel {
                    Pixel::Unknown => return None,
                    Pixel::Visible => useful = true,
                    Pixel::Empty => {}
                }
            }
            Some(useful)
        };
        let mut samples = vec![Pixel::Empty; 64 * 64];
        assert_eq!(certify(64, 64, &samples), Some(false));
        samples[2047] = Pixel::Visible;
        assert_eq!(certify(64, 64, &samples), Some(true));
        for failed in [0, 63, 64, 2048, 4095] {
            let mut cut = samples.clone();
            cut[failed] = Pixel::Unknown;
            assert_eq!(certify(64, 64, &cut), None);
        }
        assert_eq!(certify(65, 1, &vec![Pixel::Visible; 65]), None);
        let shader = include_str!("shaders/meadow_group_certificate.wgsl");
        assert!(shader.contains("any(size>vec2<i32>(64))"));
        assert!(
            shader.contains("if !sample.valid {atomicStore(&meadow_certificate_failed,1u);break;}")
        );
        assert!(shader.contains("let sample=meadow_page_geometry(slot,vec2<f32>(pixel)+0.5)"));
        assert!(
            shader.contains("if flags==3u && atomicLoad(&meadow_page_decisions[264u+slot])>0u")
        );
    }
    #[test]
    fn padded_rectangle_certificate_contains_every_raster_pixel_centre() {
        // An odd view and clipped/subpixel boundaries exercise the extra
        // scan border; no centre potentially covered by the page quad is lost.
        let size = [639i32, 359];
        for (low, high) in [
            ([-4.2f64, 0.1], [19.2f64, 32.7]),
            ([610.9, 330.4], [650.2, 370.0]),
            ([100.499, 120.5], [101.501, 121.6]),
        ] {
            let low = low.map(|x| x - 1.001);
            let high = high.map(|x| x + 1.001);
            let first: [i32; 2] =
                std::array::from_fn(|i| (low[i].floor() as i32).clamp(0, size[i] - 1));
            let last: [i32; 2] =
                std::array::from_fn(|i| (high[i].ceil() as i32).clamp(0, size[i] - 1));
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let pixel = [x, y];
                    if (0..2).all(|i| {
                        pixel[i] as f64 + 0.5 >= low[i] && pixel[i] as f64 + 0.5 <= high[i]
                    }) {
                        assert!((0..2).all(|i| pixel[i] >= first[i] && pixel[i] <= last[i]));
                    }
                }
            }
        }
        let shader = include_str!("shaders/meadow_groups.wgsl");
        assert_eq!(
            shader
                .matches("let rectangle=meadow_page_rectangle(slot)")
                .count(),
            1
        );
        assert!(shader.contains("c.w<=1e-5 || c.z<0.0 || c.z>=c.w-1e-6"));
    }
    #[test]
    fn grouped_oit_keeps_raw_moments_and_reveal_product() {
        // Differently coloured layers with different coverage, including an
        // empty member, must combine identically under any group partition.
        #[derive(Clone, Copy)]
        struct Sum {
            rgb: [f64; 3],
            weight: f64,
            reveal: f64,
        }
        let fold = |layers: &[([f64; 3], f64)]| {
            let mut s = Sum {
                rgb: [0.0; 3],
                weight: 0.0,
                reveal: 1.0,
            };
            for &(rgb, a) in layers {
                for k in 0..3 {
                    s.rgb[k] += rgb[k] * a;
                }
                s.weight += a;
                s.reveal *= 1.0 - a;
            }
            s
        };
        let input = [
            ([1.0, 0.1, 0.2], 0.2),
            ([0.1, 0.9, 0.3], 0.9),
            ([20.0, 1.0, 1.0], 0.0),
            ([0.2, 0.3, 0.8], 0.4),
        ];
        let all = fold(&input);
        for cut in 0..=input.len() {
            let a = fold(&input[..cut]);
            let b = fold(&input[cut..]);
            for k in 0..3 {
                assert!((all.rgb[k] - a.rgb[k] - b.rgb[k]).abs() < 1e-12);
            }
            assert!((all.weight - a.weight - b.weight).abs() < 1e-12);
            assert!((all.reveal - a.reveal * b.reveal).abs() < 1e-12);
        }
        // Multiplying mean colour by union alpha would destroy this identity.
        assert!((all.weight - (1.0 - all.reveal)).abs() > 0.1);
    }
    #[test]
    fn opaque_cut_requires_member_fallback_not_whole_group_depth_test() {
        let layers = [(10.0f64, 0.2f64), (12.0, 0.9), (18.0, 0.4)];
        let coverage = |opaque: f64| {
            1.0 - layers
                .iter()
                .filter(|(t, _)| *t < opaque)
                .map(|(_, a)| 1.0 - a)
                .product::<f64>()
        };
        assert_eq!(coverage(9.0), 0.0);
        assert!((coverage(11.0) - 0.2).abs() < 1e-12);
        assert!((coverage(13.0) - 0.92).abs() < 1e-12);
        assert!((coverage(20.0) - 0.952).abs() < 1e-12);
        for opaque in [10.0, 11.0, 13.0, 18.0] {
            assert!(
                opaque >= layers[0].0 && opaque <= layers[2].0,
                "a single nearest-depth test cannot reproduce an opaque cut"
            );
        }
        assert!(
            include_str!("shaders/meadow_groups.wgsl")
                .contains("if t>=lo-0.01 && t<=hi+0.01 {return out;}")
        );
    }
    #[test]
    fn fixed_budget_includes_page_atlas_and_all_shared_scratch() {
        let atlas = 256u64 * 64 * 256 * 8;
        assert_eq!(atlas, 32 * 1024 * 1024);
        let scratch = 64u64 * 64 * (3 * 8 + 3 * 4 + 16) + 4 + 64 * 31 * 4 + ARGS_BYTES * 2;
        assert!(atlas + scratch + std::mem::size_of::<Table>() as u64 * 6 < 34 * 1024 * 1024);
        assert_eq!(
            cache::UPDATE_BUDGET * cache::GROUP_SIDE as usize * cache::GROUP_SIDE as usize,
            256
        );
    }
}
