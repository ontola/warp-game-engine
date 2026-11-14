// Exercise the production reservation with partial subgroups, zero survivors,
// several concurrent workgroups, and a capacity that ends within a packet.
#[test]
#[ignore = "exclusive GPU subgroup reservation readback"]
fn gpu_splat_subgroup_append_preserves_records_and_overflow() {
    let gpu = crate::Gpu::new().unwrap();
    if !gpu.features.contains(wgpu::Features::SUBGROUP) {
        eprintln!("subgroup append unavailable on {}", gpu.info.name);
        return;
    }
    let options = super::IntegratorOptions {
        subgroup_append: true,
        ..Default::default()
    };
    let production = options.specialize(&super::shader_source()).unwrap();
    let start = production.find("fn volume_splat_store(").unwrap();
    let end = start
        + production[start..]
            .find("// Brick occlusion culling")
            .unwrap();
    let reservations = &production[start..end];
    // Count reservations in a spare stats word for this fixture only.
    let reserve = "base = atomicAdd(&volume_stats[VOLUME_DRAW_WORD + 1u], count);";
    assert_eq!(reservations.matches(reserve).count(), 1);
    let reservations = reservations.replace(
        reserve,
        &format!("atomicAdd(&volume_stats[0],1u); {reserve}"),
    );
    let expected: Vec<u32> = (0..217).filter(|i| i % 19 < 11 || i % 55 == 0).collect();
    for capacity in [0, 13, expected.len()] {
        let shader = format!(
            r#"
            @group(0) @binding(0) var<storage,read_write> volume_splats:array<vec4<u32>>;
            @group(0) @binding(1) var<storage,read_write> volume_stats:array<atomic<u32>>;
            const VOLUME_DRAW_WORD:u32 = 0u;
            fn volume_splat_first()->u32 {{ return 2u; }}
            fn volume_splat_limit()->u32 {{ return 2u + {capacity}u; }}
            {reservations}
            @compute @workgroup_size(64)
            fn main(@builtin(global_invocation_id) id:vec3<u32>, @builtin(local_invocation_index) lane:u32) {{
                let i=id.x;
                let keep=i<217u && (i%19u<11u || i%55u==0u);
                let rec=select(vec4<u32>(0u),vec4<u32>(i,i^789u,i+77u,5u),keep);
                volume_splat_append_packet(rec,lane);
            }}"#
        );
        let module = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("splat packet test"),
                source: wgpu::ShaderSource::Wgsl(shader.into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let record_bytes = ((capacity + 3) * 16) as u64;
        let output = |size| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let records = output(record_bytes);
        let counts = output(8);
        let binding = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: records.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: counts.as_entire_binding(),
                },
            ],
        });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: record_bytes + 8,
            mapped_at_creation: false,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(5, 1, 1);
        }
        enc.copy_buffer_to_buffer(&records, 0, &read, 0, record_bytes);
        enc.copy_buffer_to_buffer(&counts, 0, &read, record_bytes, 8);
        gpu.queue.submit([enc.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        gpu.wait();
        let bytes = read.slice(..).get_mapped_range().unwrap();
        let (cells, stats) = bytes.split_at(record_bytes as usize);
        let stats = bytemuck::cast_slice::<u8, u32>(stats);
        assert_eq!(stats[1], expected.len() as u32);
        assert!((1..=stats[1]).contains(&stats[0]));
        eprintln!(
            "{}: {} global reservations for {} splats with capacity {capacity}",
            gpu.info.name, stats[0], stats[1]
        );
        let cells = bytemuck::cast_slice::<u8, [u32; 4]>(cells);
        assert_eq!(cells[0..2], [[0; 4]; 2]);
        assert_eq!(cells[2 + capacity], [0; 4], "queue guard");
        let mut surviving_ids = Vec::new();
        for &[i, x, y, z] in &cells[2..2 + capacity] {
            assert_eq!([x, y, z], [i ^ 789, i + 77, 5]);
            assert!(expected.binary_search(&i).is_ok());
            surviving_ids.push(i);
        }
        surviving_ids.sort_unstable();
        surviving_ids.dedup();
        assert_eq!(surviving_ids.len(), capacity);
        if capacity == expected.len() {
            assert_eq!(surviving_ids, expected);
        }
    }
}
