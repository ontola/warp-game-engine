use super::*;
use crate::scene::GpuInstance;

#[test]
#[ignore = "needs a GPU adapter"]
fn repeated_gunfire_preserves_positions_and_deaths_across_draws() {
    let gpu = Gpu::new().unwrap();
    let positions = [
        (30., 0.),
        (32., 0.),
        (30., 1.6),
        (30., 10.),
        (31., 10.),
        (30., 22.),
    ];
    let seats: Vec<_> = positions
        .into_iter()
        .map(|(x, z)| GpuSeat {
            pos_yaw: [x, 0., z, 0.],
            params: [1.8, 0.7, 0., 0.],
            meshes: [1, 1, 1, 1],
            shape: [1., 0., 0., 0.],
        })
        .collect();
    let n = seats.len();
    let stride = std::mem::size_of::<GpuInstance>() as u64;
    let instances = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 2 * n as u64 * stride,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let crowd = CrowdRenderer::new(
        &gpu,
        &CrowdSpec {
            first: 0,
            capacity: n as u32,
        },
        &instances,
    );
    let uniform = |time| Uniform {
        mood: [time, 0., 0., 0.],
        ring: [0.; 4],
        counts: [n as u32, 0, 1, n as u32],
    };
    let alive = vec![1; n];
    crowd
        .update(
            &gpu,
            bytemuck::bytes_of(&uniform(0.)),
            Some(bytemuck::cast_slice(&seats)),
            Some((0, &alive)),
        )
        .unwrap();
    let run = |events: &[Stimulus], reset| {
        let receipts = crowd.reactions.dispatch(&gpu, reset, events, n);
        gpu.wait();
        let mut mask = alive.clone();
        for r in receipts {
            r.collect(&mut mask).unwrap().unwrap();
        }
        mask.into_iter()
            .enumerate()
            .filter_map(|(i, a)| (a == 0).then_some(i))
            .collect::<Vec<_>>()
    };
    let pose = |t| {
        crowd
            .update(&gpu, bytemuck::bytes_of(&uniform(t)), None, None)
            .unwrap();
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: instances.size(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        crowd.encode(&mut enc, None);
        enc.copy_buffer_to_buffer(&instances, 0, &read, 0, instances.size());
        gpu.queue.submit([enc.finish()]);
        read.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        gpu.wait();
        let bytes = read.slice(..).get_mapped_range().unwrap();
        let all: Vec<GpuInstance> = bytemuck::pod_collect_to_vec(&bytes);
        all.into_iter().step_by(2).collect::<Vec<_>>()
    };
    let shot =
        |from: Vec3, to: Vec3, time, id| Stimulus::new(1, from, to, 0., time, id, [0.; 2], 2.8);
    let a = Vec3::new(25., 0.9, 0.);
    let b = Vec3::new(36., 0.9, 0.);
    assert_eq!(run(&[shot(a, b, 0., 10)], true), [0]);
    assert!(
        run(&[shot(a, b, 0.01, 10)], false).is_empty(),
        "a later segment of that bullet"
    );
    let before = pose(0.35);
    assert!(before[2].rows[2][3] > 1.6, "near miss starts running");
    // A distinct shot at the actual position of the second spectator.
    let z = before[1].rows[2][3];
    assert_eq!(run(&[shot(a.with_z(z), b.with_z(z), 0.35, 11)], false), [1]);
    let after = pose(0.35);
    assert_eq!(
        before[2].rows, after[2].rows,
        "a second shot must not restart the survivor's motion"
    );
    assert!(after[0].sphere[3] == 0. && after[1].sphere[3] == 0.);
    let away = pose(1.6);
    let moved = away[2].rows[2][3];
    assert!(moved > 2.0);
    assert!(
        run(&[shot(a.with_z(1.6), b.with_z(1.6), 1.6, 12)], false).is_empty(),
        "empty home seat"
    );
    assert_eq!(
        run(&[shot(a.with_z(moved), b.with_z(moved), 1.6, 13)], false),
        [2]
    );
    let blast = Stimulus::new(
        2,
        Vec3::new(30., 0.8, 10.),
        Vec3::new(30., 0.8, 10.),
        2.,
        1.7,
        0,
        [0.; 2],
        7.,
    );
    assert_eq!(
        run(&[blast, blast], false),
        [3, 4],
        "one notification per death"
    );
    assert!(pose(15.)[..5].iter().all(|p| p.sphere[3] == 0.));
    assert!(pose(15.)[5].sphere[3] > 0.);
    run(&[], true);
    assert!(pose(16.).iter().all(|p| p.sphere[3] > 0.));
    let scare = |at: Vec3, t| Stimulus::new(0, at, at, 0., t, 0, [0.; 2], 4.);
    run(&[scare(Vec3::new(30., 0.9, -1.), 17.)], false);
    let away = pose(19.)[0].rows[2][3];
    let returning = pose(25.)[0].rows[2][3];
    assert!(returning > 0. && returning < away);
    run(&[scare(Vec3::new(30., 0.9, returning + 1.), 25.)], false);
    assert!(
        (pose(25.)[0].rows[2][3] - returning).abs() < 1e-5,
        "retrigger while walking home"
    );
    let unrelated: Vec<_> = (0..8)
        .map(|i| scare(Vec3::new(100. + i as f32 * 10., 1., 0.), 25.))
        .collect();
    run(&unrelated, false);
    assert!(
        (pose(25.)[0].rows[2][3] - returning).abs() < 1e-5,
        "unrelated gunfire does not evict a runner"
    );
}
