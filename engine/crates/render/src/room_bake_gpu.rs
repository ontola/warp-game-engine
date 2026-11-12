//! The room bake ([`crate::room_bake`]) on the GPU: the sky/sun transport
//! (`bake`) and the lamps' bounce (`bake_lights`), the costly parts, as
//! compute passes (shaders/room_bake.wgsl) over the same BVH, ray sets and
//! per-probe rotations, with the same neighbour smoothing. Results agree
//! with the CPU bake to float rounding (`gpu_matches_cpu` below); the CPU
//! bake stays for hosts without a GPU and for baked files.
//!
//! ```ignore
//! let placed = room_bake::Placement::new(&grid, &occluders);
//! let probes = room_bake_gpu::bake(&gpu, &grid, &occluders, &placed, sun, &Settings::default())?;
//! ```
use crate::irradiance_volume::{Grid, Probe};
use crate::room_bake::{Occluders, Placement, Settings};
use crate::Gpu;
use glam::Vec3;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    origin: [f32; 4],
    dims: [u32; 4],
    sun: [f32; 4],
    counts: [u32; 4],
    mode: [u32; 4],
}

/// One bake's GPU state: the geometry, rays and probes uploaded, the
/// pipelines, two probe buffers in turn.
struct Context<'a> {
    gpu: &'a Gpu,
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 6],
    geo: wgpu::Buffer,
    dirs: wgpu::Buffer,
    pos: wgpu::Buffer,
    probes: [wgpu::Buffer; 2],
    seen: wgpu::Buffer,
    lights: wgpu::Buffer,
    masks: wgpu::Buffer,
    base: Uniform,
    count: u32,
}

const SKY_DIRECT: usize = 0;
const NEIGHBOURS: usize = 1;
const BOUNCE_SKY: usize = 2;
const BOUNCE_LAMP: usize = 3;
const SMOOTH: usize = 4;
const PREPARE: usize = 5;

fn storage(gpu: &Gpu, label: &str, contents: &[u8]) -> wgpu::Buffer {
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST })
}

impl<'a> Context<'a> {
    fn new(gpu: &'a Gpu, grid: &Grid, occluders: &Occluders, placement: &Placement, sky_rays: usize, bounce_rays: usize,
        lights: &[crate::headlights::LocalLight]) -> Result<Self, String> {
        grid.validate()?;
        if placement.positions.len() != grid.count() || placement.inside.len() != grid.count() { return Err("probe placement is for another grid".into()); }
        let count = grid.count() as u32;
        // Geometry arena: nodes, then triangles.
        let mut geo: Vec<[f32; 4]> = Vec::with_capacity(occluders.nodes.len() * 2 + occluders.tris.len() * 3 + 1);
        for n in &occluders.nodes {
            geo.push([n.lo.x, n.lo.y, n.lo.z, f32::from_bits(n.first)]);
            geo.push([n.hi.x, n.hi.y, n.hi.z, f32::from_bits(n.count)]);
        }
        let tri_base = geo.len() as u32;
        for t in &occluders.tris { for p in t { geo.push([p.x, p.y, p.z, 0.0]); } }
        if geo.is_empty() { geo.push([0.0; 4]); }
        // Direction arena: sky rays, bounce rays, probe rotations.
        let mut dirs: Vec<[f32; 4]> = crate::room_bake::sphere(sky_rays).iter().map(|d| d.extend(0.0).to_array()).collect();
        let bounce_base = dirs.len() as u32;
        dirs.extend(crate::room_bake::sphere(bounce_rays).iter().map(|d| d.extend(0.0).to_array()));
        let rot_base = dirs.len() as u32;
        dirs.extend((0..grid.count()).map(|i| crate::room_bake::probe_rotation(i).to_array()));
        let pos: Vec<[f32; 4]> = placement.positions.iter().zip(&placement.inside).map(|(p, inside)| [p.x, p.y, p.z, if *inside { 0.0 } else { 1.0 }]).collect();
        let mut light_words: Vec<[f32; 4]> = lights.iter().flat_map(|l| [l.position.extend(l.range_m).to_array(), l.direction.extend(l.falloff).to_array(),
            l.intensity_cd.extend(l.source_radius_m).to_array(), [l.inner_cos, l.outer_cos, 0.0, 0.0]]).collect();
        if light_words.is_empty() { light_words.push([0.0; 4]); }
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("room bake"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/room_bake.wgsl").into()) });
        let entry = |binding, read_only: Option<bool>| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, count: None,
            ty: match read_only {
                None => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                Some(read_only) => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            } };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("room bake"), entries: &[
            entry(0, None), entry(1, Some(true)), entry(2, Some(true)), entry(3, Some(false)), entry(4, Some(true)),
            entry(5, Some(false)), entry(6, Some(false)), entry(7, Some(true)), entry(8, Some(false))] });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("room bake"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&pipeline_layout),
            module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let probes_bytes = vec![0u8; grid.count() * 6 * 16];
        // One word per probe and bounce ray (prepare).
        let mask_bytes = (grid.count() * bounce_rays * 4) as u64;
        if mask_bytes > u64::from(gpu.limits.max_storage_buffer_binding_size) { return Err(format!("room bake on the GPU needs {} MB per binding, the device allows {} MB", mask_bytes >> 20, gpu.limits.max_storage_buffer_binding_size >> 20)); }
        if lights.len() > 23 { return Err("room bake on the GPU takes at most 23 lights per bake".into()); }
        Ok(Self {
            gpu, pipelines: ["sky_direct", "neighbours", "bounce_sky", "bounce_lamp", "smooth_light", "prepare"].map(pipeline), layout,
            geo: storage(gpu, "room bake geometry", bytemuck::cast_slice(&geo)),
            dirs: storage(gpu, "room bake rays", bytemuck::cast_slice(&dirs)),
            pos: storage(gpu, "room bake probes", bytemuck::cast_slice(&pos)),
            probes: [storage(gpu, "room bake light a", &probes_bytes), storage(gpu, "room bake light b", &probes_bytes)],
            seen: storage(gpu, "room bake neighbours", &vec![0u8; grid.count() * 4]),
            lights: storage(gpu, "room bake lights", bytemuck::cast_slice(&light_words)),
            masks: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("room bake ray masks"), size: mask_bytes, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false }),
            base: Uniform { origin: grid.origin.extend(grid.cell).to_array(), dims: [grid.dims[0], grid.dims[1], grid.dims[2], count],
                sun: [0.0, 1.0, 0.0, 0.5], counts: [sky_rays as u32, bounce_rays as u32, tri_base, rot_base], mode: [0, lights.len() as u32, 0, bounce_base] },
            count,
        })
    }

    /// One pass reading probe buffer `from`, writing the other.
    fn pass(&self, enc: &mut wgpu::CommandEncoder, kernel: usize, from: usize, sun: Vec3, reflectance: f32, mode: [u32; 3]) {
        let mut u = self.base;
        u.sun = sun.extend(reflectance).to_array();
        u.mode = [mode[0], self.base.mode[1], mode[2], self.base.mode[3]];
        let uniform = self.gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("room bake pass"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
        fn e(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() } }
        let bind = self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("room bake"), layout: &self.layout, entries: &[
            e(0, &uniform), e(1, &self.geo), e(2, &self.dirs), e(3, &self.pos), e(4, &self.probes[from]), e(5, &self.probes[1 - from]),
            e(6, &self.seen), e(7, &self.lights), e(8, &self.masks)] });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("room bake"), timestamp_writes: None });
        pass.set_pipeline(&self.pipelines[kernel]);
        pass.set_bind_group(0, &bind, &[]);
        // Workgroup-per-probe kernels, or a thread per probe.
        let groups = if kernel == NEIGHBOURS || kernel == SMOOTH { self.count.div_ceil(64) } else { self.count };
        pass.dispatch_workgroups(groups.min(65535), groups.div_ceil(65535), 1);
    }

    /// Run `encode` and read probe buffer `which` back.
    fn run(&self, encode: impl FnOnce(&mut wgpu::CommandEncoder), which: usize) -> Result<Vec<Probe>, String> {
        let mut enc = self.gpu.device.create_command_encoder(&Default::default());
        encode(&mut enc);
        let size = self.probes[which].size();
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("room bake readback"), size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        enc.copy_buffer_to_buffer(&self.probes[which], 0, &readback, 0, size);
        let submission = self.gpu.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        self.gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(std::time::Duration::from_secs(120)) })
            .map_err(|e| format!("room bake GPU wait: {e:?}"))?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("room bake readback: {e:?}"))?;
        let words: Vec<[f32; 4]> = bytemuck::cast_slice(&readback.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?).to_vec();
        readback.unmap();
        Ok(words.chunks_exact(6).map(|axes| Probe { axes: std::array::from_fn(|a| Vec3::new(axes[a][0], axes[a][1], axes[a][2])) }).collect())
    }

    fn clear_probes(&self) {
        let zeros = vec![0u8; self.probes[0].size() as usize];
        for b in &self.probes { self.gpu.queue.write_buffer(b, 0, &zeros); }
    }
}

fn check(settings: &Settings, sun: Option<Vec3>) -> Result<(), String> {
    let Settings { sky_rays, bounce_rays, reflectance, .. } = *settings;
    if sky_rays < 16 || bounce_rays < 16 || !reflectance.is_finite() || !(0.0..=1.0).contains(&reflectance)
        || sun.is_some_and(|s| !s.is_finite() || s.length_squared() < 0.5) {
        return Err("invalid room bake settings".into());
    }
    Ok(())
}

/// [`crate::room_bake::bake_placed`] on the GPU.
pub fn bake(gpu: &Gpu, grid: &Grid, occluders: &Occluders, placement: &Placement, sun: Vec3, settings: &Settings) -> Result<Vec<Probe>, String> {
    check(settings, Some(sun))?;
    let sun = sun.normalize();
    let cx = Context::new(gpu, grid, occluders, placement, settings.sky_rays, settings.bounce_rays, &[])?;
    let r = settings.reflectance;
    // Buffer 1 holds the light after each step: sky_direct writes it, a
    // bounce writes 0 and its smoothing 1 again; the last two smoothings
    // go 1 -> 0 -> 1.
    let mut probes = cx.run(|enc| {
        cx.pass(enc, PREPARE, 0, sun, r, [0, 0, 0]);
        cx.pass(enc, SKY_DIRECT, 0, sun, r, [0, 0, 0]);
        cx.pass(enc, NEIGHBOURS, 0, sun, r, [0, 0, 0]);
        for _ in 0..settings.bounces.max(1) {
            cx.pass(enc, BOUNCE_SKY, 1, sun, r, [0, 0, 0]);
            cx.pass(enc, SMOOTH, 0, sun, r, [0, 0, 0]);
        }
        cx.pass(enc, SMOOTH, 1, sun, r, [0, 0, 1]);
        cx.pass(enc, SMOOTH, 0, sun, r, [0, 0, 1]);
    }, 1)?;
    // Inside-wall probes light nothing (z = 2), as the CPU bake marks them.
    let valid = read_valid(&cx)?;
    for (p, ok) in probes.iter_mut().zip(valid) { if !ok { p.axes = [Vec3::new(0.0, 0.0, 2.0); 6]; } }
    Ok(probes)
}

fn read_valid(cx: &Context) -> Result<Vec<bool>, String> {
    let size = cx.pos.size();
    let readback = cx.gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("room bake valid"), size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = cx.gpu.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(&cx.pos, 0, &readback, 0, size);
    let submission = cx.gpu.queue.submit([enc.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    cx.gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(std::time::Duration::from_secs(60)) })
        .map_err(|e| format!("room bake GPU wait: {e:?}"))?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("room bake readback: {e:?}"))?;
    let words: Vec<[f32; 4]> = bytemuck::cast_slice(&readback.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?).to_vec();
    readback.unmap();
    Ok(words.iter().map(|p| p[3] > 0.5).collect())
}

/// [`crate::room_bake::bake_lights_placed`] on the GPU.
pub fn bake_lights(gpu: &Gpu, grid: &Grid, occluders: &Occluders, placement: &Placement, lights: &[crate::headlights::LocalLight], settings: &Settings) -> Result<Vec<Vec<Probe>>, String> {
    check(&Settings { sky_rays: 16, ..*settings }, None)?;
    let cx = Context::new(gpu, grid, occluders, placement, 16, settings.bounce_rays, lights)?;
    let r = settings.reflectance;
    let mut out = Vec::with_capacity(lights.len());
    // Valid probes are the placement's (not inside); seen once for all lights.
    let mut enc = cx.gpu.device.create_command_encoder(&Default::default());
    cx.pass(&mut enc, PREPARE, 0, Vec3::Y, r, [0, 0, 0]);
    cx.pass(&mut enc, NEIGHBOURS, 0, Vec3::Y, r, [0, 0, 0]);
    cx.gpu.queue.submit([enc.finish()]);
    for l in 0..lights.len() as u32 {
        // Buffer 0 holds the light (zero at first): a bounce writes 1, its
        // smoothing 0; the last two smoothings go 0 -> 1 -> 0.
        cx.clear_probes();
        out.push(cx.run(|enc| {
            for _ in 0..settings.bounces.max(1) {
                cx.pass(enc, BOUNCE_LAMP, 0, Vec3::Y, r, [l, 0, 0]);
                cx.pass(enc, SMOOTH, 1, Vec3::Y, r, [l, 0, 1]);
            }
            cx.pass(enc, SMOOTH, 0, Vec3::Y, r, [l, 0, 1]);
            cx.pass(enc, SMOOTH, 1, Vec3::Y, r, [l, 0, 1]);
        }, 0)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room_bake;
    use glam::{Affine3A, Quat};

    fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
        let (a, b) = (Vec3::from(min), Vec3::from(max));
        Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
    }

    /// A closed 4 m room with a door and a window, on open ground.
    fn room() -> Vec<Affine3A> {
        vec![
            bx([-50.0, -0.2, -50.0], [50.0, 0.0, 50.0]),
            bx([-2.2, 0.0, -2.2], [2.2, 3.0, -2.0]),
            bx([-2.2, 0.0, -2.0], [-2.0, 3.0, 2.0]),
            bx([2.0, 0.0, -2.0], [2.2, 1.0, 2.0]), bx([2.0, 2.2, -2.0], [2.2, 3.0, 2.0]),
            bx([2.0, 1.0, -2.0], [2.2, 2.2, -0.6]), bx([2.0, 1.0, 0.6], [2.2, 2.2, 2.0]),
            bx([-2.2, 0.0, 2.0], [-0.5, 3.0, 2.2]),
            bx([0.5, 0.0, 2.0], [2.2, 3.0, 2.2]),
            bx([-0.5, 2.1, 2.0], [0.5, 3.0, 2.2]),
            bx([-2.2, 3.0, -2.2], [2.2, 3.2, 2.2]),
            bx([-1.0, 0.0, -1.0], [0.0, 0.8, 0.2]),
        ]
    }

    fn compare(cpu: &[Probe], gpu: &[Probe]) -> (f32, f32) {
        // Largest and mean difference relative to the brightest value.
        let scale = cpu.iter().flat_map(|p| p.axes).map(|a| a.max_element()).fold(1e-6f32, f32::max);
        let (mut max, mut sum, mut n) = (0f32, 0f32, 0f32);
        for (c, g) in cpu.iter().zip(gpu) {
            for a in 0..6 { let d = (c.axes[a] - g.axes[a]).abs().max_element() / scale; max = max.max(d); sum += d; n += 1.0; }
        }
        (max, sum / n)
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_matches_cpu() {
        let gpu = Gpu::new().unwrap();
        let grid = Grid { origin: Vec3::new(-3.75, 0.25, -3.75), cell: 0.5, dims: [16, 7, 16] };
        let occ = Occluders::from_boxes(&room());
        let placed = room_bake::Placement::new(&grid, &occ);
        let settings = Settings { sky_rays: 512, bounce_rays: 128, bounces: 2, reflectance: 0.5 };
        let sun = Vec3::new(0.3, 0.8, 0.5);
        let t = std::time::Instant::now();
        let cpu = room_bake::bake_placed(&grid, &occ, &placed, sun, &settings).unwrap();
        let cpu_s = t.elapsed().as_secs_f32();
        let t = std::time::Instant::now();
        let on_gpu = bake(&gpu, &grid, &occ, &placed, sun, &settings).unwrap();
        let gpu_s = t.elapsed().as_secs_f32();
        let (max, mean) = compare(&cpu, &on_gpu);
        eprintln!("sky bake: cpu {cpu_s:.3} s, gpu {gpu_s:.3} s, max diff {max:.2e}, mean {mean:.2e} (of the brightest)");
        assert!(max < 1e-3 && mean < 1e-5, "sky bake differs: max {max}, mean {mean}");
        let lamp = crate::headlights::LocalLight { position: Vec3::new(0.0, 2.8, 0.0), direction: Vec3::NEG_Y, intensity_cd: Vec3::splat(800.0),
            range_m: 12.0, inner_cos: 0.2, outer_cos: 0.0, source_radius_m: 0.05, falloff: 2.0 };
        let cpu = room_bake::bake_lights_placed(&grid, &occ, &placed, &[lamp], &settings).unwrap();
        let on_gpu = bake_lights(&gpu, &grid, &occ, &placed, &[lamp], &settings).unwrap();
        let (max, mean) = compare(&cpu[0], &on_gpu[0]);
        eprintln!("lamp bake: max diff {max:.2e}, mean {mean:.2e}");
        assert!(max < 1e-3 && mean < 1e-5, "lamp bake differs: max {max}, mean {mean}");
    }
}

#[cfg(test)]
mod speed {
    use super::*;
    #[test]
    #[ignore = "needs a GPU; timing"]
    fn gpu_bake_speed() {
        let gpu = Gpu::new().unwrap();
        let boxes: Vec<glam::Affine3A> = (0..24).map(|k| glam::Affine3A::from_scale_rotation_translation(Vec3::new(8.0, 3.0, 0.2), glam::Quat::IDENTITY, Vec3::new(0.0, 1.5, -10.0 + k as f32))).collect();
        let occ = Occluders::from_boxes(&boxes);
        let grid = Grid { origin: Vec3::new(-6.0, 0.25, -14.0), cell: 0.5, dims: [25, 7, 80] };
        let placed = crate::room_bake::Placement::new(&grid, &occ);
        let settings = Settings::default();
        for k in 0..2 {
            let t = std::time::Instant::now();
            bake(&gpu, &grid, &occ, &placed, Vec3::new(0.3, 0.8, 0.5), &settings).unwrap();
            eprintln!("run {k}: {} probes, {} tris: gpu bake {:.3} s", grid.count(), occ.triangles(), t.elapsed().as_secs_f32());
        }
        let t = std::time::Instant::now();
        crate::room_bake::bake_placed(&grid, &occ, &placed, Vec3::new(0.3, 0.8, 0.5), &settings).unwrap();
        eprintln!("cpu bake {:.3} s", t.elapsed().as_secs_f32());
    }
}
