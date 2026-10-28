//! Serialized raster-pool addressing. The source/brick plant ABI stays intact.
use super::{meadow_pool_bytes, meadow_pool_plant_words, meadow_pool_profile, Settings};
use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

#[test]
fn compact_pool_requires_homogeneous_windswept_and_keeps_mixed_branches() {
    use crate::{foliage_assembly::MeadowBladeBudget, meadow_data::MeadowGeometry};
    let mut classic =
        MeadowGeometry::cook_with_blade_budget(73, 0.0, MeadowBladeBudget::Blades120).unwrap();
    let mut windswept =
        MeadowGeometry::cook_windswept(73, 0.0, MeadowBladeBudget::Blades120).unwrap();
    assert_eq!(meadow_pool_profile(&classic), 0.0);
    assert_eq!(meadow_pool_profile(&windswept), 2.0);
    classic.vertices[0].anchor[3] = 0.5;
    windswept.vertices[0].anchor[3] = 0.0;
    for mixed in [&classic, &windswept] {
        assert_eq!(meadow_pool_profile(mixed), 1.0);
        assert_eq!(meadow_pool_plant_words(meadow_pool_profile(mixed)), 28);
    }
    // A frame update must preserve the layout chosen when the pool was
    // allocated, including the compact discriminator rather than just >0.
    for profile in [0.0, 1.0, 2.0] {
        let mut settings = Settings::zeroed();
        settings.shape[3] = 25.0;
        settings.range[0] = 35.0;
        settings.wind_history[3] = profile;
        let wind = crate::wind::WindState::default();
        let frame = settings
            .for_frame(
                Default::default(),
                &super::Clock {
                    current: wind,
                    previous: wind,
                    valid: true,
                },
            )
            .unwrap();
        assert_eq!(frame.wind_history[3], profile);
    }
}

#[test]
fn compact_pool_ranges_are_disjoint_and_250m_fits_128mib_binding() {
    let dimension = ((250.0 / crate::meadow_data::ROOT_CELL).ceil() as u32 + 3) * 2 + 1;
    let capacity = dimension * dimension;
    assert_eq!((dimension, capacity), (1119, 1_252_161));
    assert_eq!(meadow_pool_bytes(capacity, 0.0), 155_267_964);
    assert_eq!(meadow_pool_bytes(capacity, 2.0), 75_129_660);
    assert!(meadow_pool_bytes(capacity, 2.0) < 128 * 1024 * 1024);
    assert!(meadow_pool_bytes(capacity, 0.0) > 128 * 1024 * 1024);
    for count in [1, 3, 7, capacity] {
        for (profile, stride) in [(0.0, 28u32), (1.0, 28), (2.0, 12)] {
            assert_eq!(meadow_pool_plant_words(profile), stride);
            let list_start = count * stride;
            assert_eq!((count - 1) * stride + stride - 1, list_start - 1);
            for lod in 0..3 {
                let first = list_start + lod * count;
                let last = first + count - 1;
                assert!(first >= list_start && last < count * (stride + 3));
                if lod > 0 {
                    assert_eq!(first, list_start + (lod - 1) * count + count);
                }
            }
            assert_eq!(
                u64::from(list_start + 3 * count) * 4,
                meadow_pool_bytes(count, profile)
            );
        }
    }
}

fn item<'a>(source: &'a str, marker: &str) -> &'a str {
    let start = source
        .find(marker)
        .unwrap_or_else(|| panic!("missing WGSL item {marker}"));
    let body = source[start..].find('{').unwrap() + start;
    let mut depth = 0;
    for (index, byte) in source.as_bytes().iter().enumerate().skip(body) {
        if *byte == b'{' {
            depth += 1;
        }
        if *byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return &source[start..=index];
            }
        }
    }
    panic!("unterminated WGSL item {marker}");
}

fn sentinel_source() -> String {
    let generate = include_str!("shaders/meadow_generate.wgsl");
    let render = include_str!("shaders/meadow_render.wgsl");
    let visibility = include_str!("shaders/meadow_visibility.wgsl");
    let visible_variant = visibility
        .lines()
        .find(|line| {
            line.trim_start()
                .starts_with("let variant=meadow_pool[owner*")
        })
        .unwrap();
    [
        include_str!("shaders/meadow_field.wgsl").to_owned(),
        include_str!("shaders/meadow_common.wgsl").to_owned(),
        item(generate, "struct MeadowDrawArgs ").to_owned() + ";",
        item(generate, "struct MeadowCandidate ").to_owned() + ";",
        r#"
@group(0) @binding(0) var<uniform> meadow:MeadowSettings;
@group(0) @binding(1) var<storage,read_write> meadow_pool:array<u32>;
@group(0) @binding(2) var<storage,read_write> meadow_args:array<MeadowDrawArgs,3>;
"#
        .to_owned(),
        item(generate, "fn meadow_write4(").to_owned(),
        item(generate, "fn meadow_store_candidate(").to_owned(),
        item(render, "fn meadow_read4(").to_owned(),
        item(render, "fn meadow_source_owner(").to_owned(),
        format!("fn visible_variant(owner:u32)->u32 {{{visible_variant}\nreturn variant;}}"),
        r#"
@compute @workgroup_size(1) fn verify() {
    let capacity=u32(meadow.dispatch.w);let stride=meadow_plant_words(meadow.wind_history.w);
    for(var slot=0u;slot<capacity;slot++) {
        var c:MeadowCandidate;c.valid=true;c.lod=slot%3u;
        let base=slot*100u;
        c.plant.root_scale=vec4<f32>(f32(base),f32(base+1u),f32(base+2u),f32(base+3u));
        c.plant.yaw=vec4<f32>(f32(base+4u),f32(base+5u),f32(base+6u),f32(base+7u));
        c.plant.road_variant_tint=vec4<u32>(base+8u,base+9u,base+10u,base+11u);
        for(var branch=0u;branch<4u;branch++) {
            let n=base+12u+branch*4u;
            c.plant.branches[branch]=vec4<f32>(f32(n),f32(n+1u),f32(n+2u),f32(n+3u));
        }
        meadow_store_candidate(c,slot,capacity);
    }
    // One invocation deliberately serializes writes and dependent reads.
    // Test the real draw-list consumer plus the real visibility variant read.
    let footer=meadow_pool_words(capacity,meadow.wind_history.w);var index=0u;
    for(var lod=0u;lod<3u;lod++) {
        let count=atomicLoad(&meadow_args[lod].instance_count);
        for(var draw=0u;draw<count;draw++) {
            let owner=meadow_source_owner(draw,lod,capacity);
            let root=meadow_read4(owner*stride);
            meadow_pool[footer+index*3u]=owner;
            meadow_pool[footer+index*3u+1u]=bitcast<u32>(root.x);
            meadow_pool[footer+index*3u+2u]=visible_variant(owner);index++;
        }
        meadow_pool[footer+capacity*3u+lod]=count;
    }
    meadow_pool[footer+capacity*3u+3u]=meadow_source_owner(capacity,0u,capacity);
    meadow_pool[footer+capacity*3u+4u]=meadow_source_owner(0u,3u,capacity);
}
"#
        .to_owned(),
    ]
    .join("\n")
}

#[test]
fn compact_pool_shader_validates_and_source_brick_abi_is_unchanged() {
    let source = sentinel_source();
    let module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    let plant = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("MeadowPlant"))
        .unwrap()
        .1;
    let naga::TypeInner::Struct { span, members } = &plant.inner else {
        panic!("plant ABI must be a struct")
    };
    assert_eq!(*span, 112);
    assert_eq!(
        members
            .iter()
            .find(|member| member.name.as_deref() == Some("branches"))
            .unwrap()
            .offset,
        48
    );
    let source = format!(
        "{}\n{};",
        include_str!("shaders/meadow_common.wgsl"),
        item(
            include_str!("shaders/meadow_brick_field.wgsl"),
            "struct BrickRoot "
        )
    );
    // Parsing resolves layout independently of unrelated source-field helpers.
    let module = naga::front::wgsl::parse_str(&format!(
        "{}\n{source}",
        include_str!("shaders/meadow_field.wgsl")
    ))
    .unwrap();
    let brick = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("BrickRoot"))
        .unwrap()
        .1;
    assert!(matches!(
        brick.inner,
        naga::TypeInner::Struct { span: 128, .. }
    ));
}

#[test]
#[ignore = "serial GPU fixture; requires an idle adapter"]
fn compact_pool_gpu_serialization_preserves_legacy_and_drawlists() {
    const CAPACITY: u32 = 7;
    const SENTINEL: u32 = 0xa5a5a5a5;
    let gpu = crate::Gpu::new().expect("headless GPU");
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("actual meadow pool serialization"),
            source: wgpu::ShaderSource::Wgsl(sentinel_source().into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meadow pool sentinel"),
            layout: None,
            module: &shader,
            entry_point: Some("verify"),
            compilation_options: Default::default(),
            cache: None,
        });
    let mut classic = None;
    for profile in [0.0, 1.0, 2.0] {
        let stride = meadow_pool_plant_words(profile);
        let footer = (CAPACITY * (stride + 3)) as usize;
        let count = footer + (CAPACITY * 3) as usize + 8;
        let mut settings = Settings::zeroed();
        settings.dispatch[3] = CAPACITY as i32;
        settings.wind_history[3] = profile;
        let uniform = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("pool layout profile"),
                contents: bytemuck::bytes_of(&settings),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let pool = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sentinel meadow pool"),
                contents: bytemuck::cast_slice(&vec![SENTINEL; count]),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let args = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sentinel draw arguments"),
                contents: bytemuck::cast_slice(&[0u32; 15]),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pool sentinel readback"),
            size: (count * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pool sentinel"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: args.as_entire_binding(),
                },
            ],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&pool, 0, &read, 0, (count * 4) as u64);
        gpu.queue.submit([encoder.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("map pool sentinel"));
        gpu.wait();
        let bytes = read.slice(..).get_mapped_range().unwrap();
        let actual: &[u32] = bytemuck::cast_slice(&bytes);
        let mut expected = vec![SENTINEL; count];
        for owner in 0..CAPACITY {
            for word in 0..stride {
                let value = owner * 100 + word;
                expected[(owner * stride + word) as usize] = if (8..12).contains(&word) {
                    value
                } else {
                    (value as f32).to_bits()
                };
            }
            expected[(CAPACITY * stride + (owner % 3) * CAPACITY + owner / 3) as usize] = owner;
        }
        let mut draw = 0;
        for lod in 0..3u32 {
            let owners: Vec<_> = (0..CAPACITY).filter(|owner| owner % 3 == lod).collect();
            expected[footer + (CAPACITY * 3 + lod) as usize] = owners.len() as u32;
            for owner in owners {
                expected[footer + draw * 3] = owner;
                expected[footer + draw * 3 + 1] = ((owner * 100) as f32).to_bits();
                expected[footer + draw * 3 + 2] = owner * 100 + 10;
                draw += 1;
            }
        }
        expected[footer + (CAPACITY * 3 + 3) as usize] = u32::MAX;
        expected[footer + (CAPACITY * 3 + 4) as usize] = u32::MAX;
        assert_eq!(
            actual, expected,
            "profile={profile}: owners, branches, lists, visibility or guard words changed"
        );
        if profile == 0.0 {
            classic = Some(actual.to_vec());
        }
        if profile == 1.0 {
            assert_eq!(
                actual,
                classic.as_ref().unwrap(),
                "mixed source retains classic serialized ABI"
            );
        }
    }
}
