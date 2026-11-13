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

/// The bake's compute pipelines, compiled once ([`Kernels::new`]) for any
/// number of bakes on one device (the `*_with` functions).
pub struct Kernels {
    layout: wgpu::BindGroupLayout,
    pipelines: [wgpu::ComputePipeline; 14],
}

impl Kernels {
    pub fn new(gpu: &Gpu) -> Self {
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
        Self { layout, pipelines: ["sky_direct", "neighbours", "bounce_sky", "bounce_lamp", "smooth_light", "prepare", "place", "visibility_map", "planes", "reach", "denoise", "sky_flag", "sky_refine", "denoise_lamp"].map(pipeline) }
    }
}

/// One bake's GPU state: the geometry, rays and probes uploaded, two probe
/// buffers in turn.
struct Context<'a> {
    gpu: &'a Gpu,
    kernels: &'a Kernels,
    geo: wgpu::Buffer,
    dirs: wgpu::Buffer,
    pos: wgpu::Buffer,
    probes: [wgpu::Buffer; 2],
    seen: wgpu::Buffer,
    lights: wgpu::Buffer,
    masks: wgpu::Buffer,
    base: Uniform,
    /// Where the second direct-sky estimate's rays start in `dirs`, and how many.
    refine: (u32, u32),
    /// Where the refined bounce rays start in `dirs`, and how many (0: none).
    refined: (u32, u32),
    count: u32,
}

const SKY_DIRECT: usize = 0;
const NEIGHBOURS: usize = 1;
const BOUNCE_SKY: usize = 2;
const BOUNCE_LAMP: usize = 3;
const SMOOTH: usize = 4;
const PREPARE: usize = 5;
const PLACE: usize = 6;
const VISIBILITY: usize = 7;
const PLANES: usize = 8;
const REACH: usize = 9;
const DENOISE: usize = 10;
const SKY_FLAG: usize = 11;
const SKY_REFINE: usize = 12;
const DENOISE_LAMP: usize = 13;
// As the CPU bake (even hops: the flag ends in bit 30).
const SKY_HOPS: u32 = crate::room_bake::SKY_HOPS as u32;
const SKY_REFINE_FACTOR: usize = crate::room_bake::SKY_REFINE_FACTOR;
const _: () = assert!(SKY_HOPS % 2 == 0);
/// Iterations of the variance-guided filter after each sky bounce
/// (room_bake.wgsl `denoise`), as the CPU bake.
const DENOISE_PASSES: usize = crate::room_bake::DENOISE_PASSES;
/// Lamps baked in one pass (room_bake.wgsl LAMPS_PER_PASS).
const LAMPS_PER_PASS: usize = 8;

fn storage(gpu: &Gpu, label: &str, contents: &[u8]) -> wgpu::Buffer {
    gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST })
}

impl<'a> Context<'a> {
    fn new(gpu: &'a Gpu, kernels: &'a Kernels, grid: &Grid, occluders: &Occluders, placement: &Placement, sky_rays: usize, bounce_rays: usize,
        lights: &[crate::headlights::LocalLight], refined_bounce: usize) -> Result<Self, String> {
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
        // Then the second direct-sky estimate's rays (sky_refine).
        let refine = (dirs.len() as u32, (sky_rays * SKY_REFINE_FACTOR) as u32);
        dirs.extend(crate::room_bake::sphere(sky_rays * SKY_REFINE_FACTOR).iter().map(|d| d.extend(0.0).to_array()));
        // Then the bounce rays of the probes near sky (0: none).
        let refined = (dirs.len() as u32, refined_bounce as u32);
        dirs.extend(crate::room_bake::sphere(refined_bounce).iter().map(|d| d.extend(0.0).to_array()));
        let pos: Vec<[f32; 4]> = placement.positions.iter().zip(&placement.inside).map(|(p, inside)| [p.x, p.y, p.z, if *inside { 0.0 } else { 1.0 }]).collect();
        let mut light_words: Vec<[f32; 4]> = lights.iter().flat_map(|l| [l.position.extend(l.range_m).to_array(), l.direction.extend(l.falloff).to_array(),
            l.intensity_cd.extend(l.source_radius_m).to_array(), [l.inner_cos, l.outer_cos, 0.0, 0.0]]).collect();
        if light_words.is_empty() { light_words.push([0.0; 4]); }
        // Room for every lamp of one pass (bounce_lamp): a probe set per slot.
        let slots = lights.len().clamp(1, LAMPS_PER_PASS);
        let probes_bytes = vec![0u8; grid.count() * 6 * 16 * slots];
        // One word per probe and bounce ray (prepare).
        let mask_bytes = (grid.count() * bounce_rays.max(refined_bounce) * 4) as u64;
        if mask_bytes > u64::from(gpu.limits.max_storage_buffer_binding_size) { return Err(format!("room bake on the GPU needs {} MB per binding, the device allows {} MB", mask_bytes >> 20, gpu.limits.max_storage_buffer_binding_size >> 20)); }
        if lights.len() > 23 { return Err("room bake on the GPU takes at most 23 lights per bake".into()); }
        Ok(Self {
            gpu, kernels,
            geo: storage(gpu, "room bake geometry", bytemuck::cast_slice(&geo)),
            dirs: storage(gpu, "room bake rays", bytemuck::cast_slice(&dirs)),
            pos: storage(gpu, "room bake probes", bytemuck::cast_slice(&pos)),
            probes: [storage(gpu, "room bake light a", &probes_bytes), storage(gpu, "room bake light b", &probes_bytes)],
            seen: storage(gpu, "room bake neighbours", &vec![0u8; grid.count() * 4]),
            lights: storage(gpu, "room bake lights", bytemuck::cast_slice(&light_words)),
            masks: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("room bake ray masks"), size: mask_bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
            base: Uniform { origin: grid.origin.extend(grid.cell).to_array(), dims: [grid.dims[0], grid.dims[1], grid.dims[2], count],
                sun: [0.0, 1.0, 0.0, 0.5], counts: [sky_rays as u32, bounce_rays as u32, tri_base, rot_base], mode: [0, lights.len() as u32, 0, bounce_base] },
            count, refine, refined,
        })
    }

    /// One pass reading probe buffer `from`, writing the other.
    fn pass(&self, enc: &mut wgpu::CommandEncoder, kernel: usize, from: usize, sun: Vec3, reflectance: f32, mode: [u32; 3]) {
        let mut u = self.base;
        u.sun = sun.extend(reflectance).to_array();
        u.mode = [mode[0], self.base.mode[1], mode[2], self.base.mode[3]];
        let uniform = self.gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("room bake pass"), contents: bytemuck::bytes_of(&u), usage: wgpu::BufferUsages::UNIFORM });
        fn e(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() } }
        let bind = self.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("room bake"), layout: &self.kernels.layout, entries: &[
            e(0, &uniform), e(1, &self.geo), e(2, &self.dirs), e(3, &self.pos), e(4, &self.probes[from]), e(5, &self.probes[1 - from]),
            e(6, &self.seen), e(7, &self.lights), e(8, &self.masks)] });
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("room bake"), timestamp_writes: None });
        pass.set_pipeline(&self.kernels.pipelines[kernel]);
        pass.set_bind_group(0, &bind, &[]);
        // Workgroup-per-probe kernels, or a thread per probe.
        let groups = if kernel == SMOOTH || kernel == DENOISE_LAMP { (self.count * mode[0].max(1)).div_ceil(64) }
            else if matches!(kernel, NEIGHBOURS | PLACE | PLANES | REACH | DENOISE | SKY_FLAG) { self.count.div_ceil(64) } else { self.count };
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
    bake_with(&Kernels::new(gpu), gpu, grid, occluders, placement, sun, settings)
}

/// [`bake`] with kernels compiled once.
pub fn bake_with(kernels: &Kernels, gpu: &Gpu, grid: &Grid, occluders: &Occluders, placement: &Placement, sun: Vec3, settings: &Settings) -> Result<Vec<Probe>, String> {
    check(settings, Some(sun))?;
    let sun = sun.normalize();
    let cx = Context::new(gpu, kernels, grid, occluders, placement, settings.sky_rays, settings.bounce_rays, &[], settings.bounce_rays * crate::room_bake::BOUNCE_REFINE_FACTOR)?;
    let r = settings.reflectance;
    // `cur`: the probe buffer holding the light. sky_direct writes 1; the
    // second estimate (sky_refine) 0; each bounce and denoise pass flips it.
    let cur = std::cell::Cell::new(1usize);
    let step = |enc: &mut wgpu::CommandEncoder, kernel: usize, mode: [u32; 3]| { cx.pass(enc, kernel, cur.get(), sun, r, mode); cur.set(1 - cur.get()); };
    let mut probes = cx.run(|enc| {
        cx.pass(enc, SKY_DIRECT, 0, sun, r, [0, 0, 0]);
        cx.pass(enc, NEIGHBOURS, 0, sun, r, [0, 0, 0]);
        // The direct sky again with many more rays near the probes that saw
        // any, out to SKY_HOPS steps of open space.
        for k in 0..=SKY_HOPS { cx.pass(enc, SKY_FLAG, 1, sun, r, [0, 0, k]); }
        step(enc, SKY_REFINE, [cx.refine.0, 0, cx.refine.1]);
        // What each bounce ray sees, once; probes near sky trace
        // BOUNCE_REFINE_FACTOR times the rays (their own set).
        cx.pass(enc, PREPARE, 0, sun, r, [cx.refined.0, 0, cx.refined.1]);
        // Each bounce denoised by its estimates' variance before the next
        // gathers it.
        for _ in 0..settings.bounces.max(1) {
            step(enc, BOUNCE_SKY, [cx.refined.0, 0, cx.refined.1]);
            for _ in 0..DENOISE_PASSES { step(enc, DENOISE, [0, 0, 0]); }
        }
    }, (1 + 1 + settings.bounces.max(1) * (1 + DENOISE_PASSES)) % 2)?;
    debug_assert_eq!(cur.get(), (1 + 1 + settings.bounces.max(1) * (1 + DENOISE_PASSES)) % 2);
    // Inside-wall probes light nothing (z = 2), as the CPU bake marks them.
    let valid = read_valid(&cx)?;
    for (p, ok) in probes.iter_mut().zip(valid) { if !ok { p.axes = [Vec3::new(0.0, 0.0, 2.0); 6]; } }
    Ok(probes)
}

/// Submit `enc` and read `buffer` back as words.
fn read<T: bytemuck::Pod>(gpu: &Gpu, mut enc: wgpu::CommandEncoder, buffer: &wgpu::Buffer, bytes: u64) -> Result<Vec<T>, String> {
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("room bake readback"), size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    enc.copy_buffer_to_buffer(buffer, 0, &readback, 0, bytes);
    let submission = gpu.queue.submit([enc.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    readback.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    gpu.device.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(std::time::Duration::from_secs(120)) })
        .map_err(|e| format!("room bake GPU wait: {e:?}"))?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("room bake readback: {e:?}"))?;
    let words: Vec<T> = bytemuck::cast_slice(&readback.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?).to_vec();
    readback.unmap();
    Ok(words)
}

fn read_valid(cx: &Context) -> Result<Vec<bool>, String> {
    let words: Vec<[f32; 4]> = read(cx.gpu, cx.gpu.device.create_command_encoder(&Default::default()), &cx.pos, cx.pos.size())?;
    Ok(words.iter().map(|p| p[3] > 0.5).collect())
}

/// What every bake of one geometry state shares, on the GPU: the probes'
/// placement ([`Placement`]), their visibility maps, reach and separation
/// planes, as `room_bake`'s `Placement::new`, `bake_visibility_placed`,
/// `bake_reach_placed` and `separation_planes_placed` bake them.
pub struct Geometry {
    pub placement: Placement,
    pub visibility: Vec<[u32; crate::room_bake::VISIBILITY_SIDE * crate::room_bake::VISIBILITY_SIDE]>,
    pub reach: Vec<[u32; crate::room_bake::REACH_WORDS]>,
    pub planes: Vec<[f32; 4]>,
    /// Per probe the 26 neighbours it sees (bit order: irradiance_volume::neighbour_bit).
    pub neighbours: Vec<u32>,
}

pub fn bake_geometry(gpu: &Gpu, grid: &Grid, occluders: &Occluders) -> Result<Geometry, String> {
    bake_geometry_with(&Kernels::new(gpu), gpu, grid, occluders)
}

/// [`bake_geometry`] with kernels compiled once.
pub fn bake_geometry_with(kernels: &Kernels, gpu: &Gpu, grid: &Grid, occluders: &Occluders) -> Result<Geometry, String> {
    grid.validate()?;
    const VIS: usize = crate::room_bake::VISIBILITY_SIDE * crate::room_bake::VISIBILITY_SIDE;
    const REACH_W: usize = crate::room_bake::REACH_WORDS;
    debug_assert!(VIS == 64 && REACH_W == 16);
    // The grid points to start from; the 256 placement directions as the
    // sky rays; a mask stride of a visibility map and a reach per probe.
    let start = Placement { positions: (0..grid.count()).map(|i| grid.position(i)).collect(), inside: vec![false; grid.count()] };
    let cx = Context::new(gpu, kernels, grid, occluders, &start, 256, VIS + REACH_W, &[], 0)?;
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    for kernel in [PLACE, VISIBILITY, PLANES, REACH, NEIGHBOURS] { cx.pass(&mut enc, kernel, 0, Vec3::Y, 0.5, [0, 0, 0]); }
    gpu.queue.submit([enc.finish()]);
    let pos: Vec<[f32; 4]> = read(gpu, gpu.device.create_command_encoder(&Default::default()), &cx.pos, cx.pos.size())?;
    let masks: Vec<u32> = read(gpu, gpu.device.create_command_encoder(&Default::default()), &cx.masks, cx.masks.size())?;
    let planes: Vec<[f32; 4]> = read(gpu, gpu.device.create_command_encoder(&Default::default()), &cx.probes[1], cx.probes[1].size())?;
    let neighbours: Vec<u32> = read(gpu, gpu.device.create_command_encoder(&Default::default()), &cx.seen, cx.seen.size())?;
    let stride = VIS + REACH_W;
    Ok(Geometry {
        placement: Placement { positions: pos.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect(), inside: pos.iter().map(|p| p[3] < 0.5).collect() },
        visibility: masks.chunks_exact(stride).map(|m| std::array::from_fn(|k| m[k])).collect(),
        reach: masks.chunks_exact(stride).map(|m| std::array::from_fn(|k| m[VIS + k])).collect(),
        planes: planes.chunks_exact(6).map(|p| p[0]).collect(),
        neighbours,
    })
}

/// [`crate::room_bake::bake_lights_placed`] on the GPU.
pub fn bake_lights(gpu: &Gpu, grid: &Grid, occluders: &Occluders, placement: &Placement, lights: &[crate::headlights::LocalLight], settings: &Settings) -> Result<Vec<Vec<Probe>>, String> {
    bake_lights_with(&Kernels::new(gpu), gpu, grid, occluders, placement, lights, settings)
}

/// [`bake_lights`] with kernels compiled once.
pub fn bake_lights_with(kernels: &Kernels, gpu: &Gpu, grid: &Grid, occluders: &Occluders, placement: &Placement, lights: &[crate::headlights::LocalLight], settings: &Settings) -> Result<Vec<Vec<Probe>>, String> {
    check(&Settings { sky_rays: 16, ..*settings }, None)?;
    let cx = Context::new(gpu, kernels, grid, occluders, placement, 16, settings.bounce_rays, lights, 0)?;
    let r = settings.reflectance;
    let mut out = Vec::with_capacity(lights.len());
    // Valid probes are the placement's (not inside); seen once for all lights.
    let mut enc = cx.gpu.device.create_command_encoder(&Default::default());
    cx.pass(&mut enc, PREPARE, 0, Vec3::Y, r, [0, 0, 0]);
    cx.pass(&mut enc, NEIGHBOURS, 0, Vec3::Y, r, [0, 0, 0]);
    cx.gpu.queue.submit([enc.finish()]);
    // Up to LAMPS_PER_PASS lamps at once, one probe set (slot) each.
    for first in (0..lights.len()).step_by(LAMPS_PER_PASS) {
        let slots = (lights.len() - first).min(LAMPS_PER_PASS);
        let (first, n) = (first as u32, slots as u32);
        // Buffer 0 holds the light (zero at first): a bounce writes 1, its
        // DENOISE_PASSES (odd) filtering 0 again.
        cx.clear_probes();
        let all = cx.run(|enc| {
            for _ in 0..settings.bounces.max(1) {
                cx.pass(enc, BOUNCE_LAMP, 0, Vec3::new(n as f32, 0.0, 0.0), r, [first, 0, 0]);
                for k in 0..DENOISE_PASSES { cx.pass(enc, DENOISE_LAMP, 1 - k % 2, Vec3::Y, r, [n, 0, 0]); }
            }
        }, (1 + DENOISE_PASSES) % 2)?;
        out.extend(all.chunks_exact(grid.count()).take(slots).map(|set| set.to_vec()));
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
        // Three lamps in one pass (each its own slot), one of them red.
        let lamps = [lamp, crate::headlights::LocalLight { position: Vec3::new(1.5, 2.0, 1.5), intensity_cd: Vec3::new(500.0, 50.0, 20.0), inner_cos: 0.9, outer_cos: -0.5, ..lamp },
            crate::headlights::LocalLight { position: Vec3::new(-1.5, 0.5, -1.5), direction: Vec3::Y, ..lamp }];
        let cpu = room_bake::bake_lights_placed(&grid, &occ, &placed, &lamps, &settings).unwrap();
        let on_gpu = bake_lights(&gpu, &grid, &occ, &placed, &lamps, &settings).unwrap();
        assert_eq!(on_gpu.len(), 3);
        for l in 0..3 {
            let (max, mean) = compare(&cpu[l], &on_gpu[l]);
            eprintln!("lamp {l} bake: max diff {max:.2e}, mean {mean:.2e}");
            assert!(max < 1e-3 && mean < 1e-5, "lamp {l} bake differs: max {max}, mean {mean}");
        }
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

#[cfg(test)]
mod geometry_tests {
    use super::*;
    use crate::room_bake;

    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_geometry_matches_cpu() {
        let gpu = Gpu::new().unwrap();
        let grid = Grid { origin: Vec3::new(-3.75, 0.25, -3.75), cell: 0.5, dims: [16, 7, 16] };
        let boxes = [
            ([-50.0, -0.2, -50.0], [50.0, 0.0, 50.0]), ([-2.2, 0.0, -2.2], [2.2, 3.0, -2.0]), ([-2.2, 0.0, -2.0], [-2.0, 3.0, 2.0]),
            ([2.0, 0.0, -2.0], [2.2, 3.0, 2.0]), ([-2.2, 0.0, 2.0], [-0.5, 3.0, 2.2]), ([0.5, 0.0, 2.0], [2.2, 3.0, 2.2]),
            ([-0.5, 2.1, 2.0], [0.5, 3.0, 2.2]), ([-2.2, 3.0, -2.2], [2.2, 3.2, 2.2]), ([0.0, 0.0, -0.1], [0.12, 2.0, 1.3]),
        ].map(|(a, b): ([f32; 3], [f32; 3])| { let (a, b) = (Vec3::from(a), Vec3::from(b)); glam::Affine3A::from_scale_rotation_translation(b - a, glam::Quat::IDENTITY, (a + b) * 0.5) });
        let occ = Occluders::from_boxes(&boxes);
        let t = std::time::Instant::now();
        let cpu = room_bake::Placement::new(&grid, &occ);
        let vis = room_bake::bake_visibility_placed(&grid, &occ, &cpu);
        let reach = room_bake::bake_reach_placed(&grid, &occ, &cpu);
        let planes = room_bake::separation_planes_placed(&grid, &occ, &cpu);
        let cpu_s = t.elapsed().as_secs_f32();
        let t = std::time::Instant::now();
        let g = bake_geometry(&gpu, &grid, &occ).unwrap();
        eprintln!("geometry: cpu {cpu_s:.3} s, gpu {:.3} s", t.elapsed().as_secs_f32());
        let moved = cpu.positions.iter().zip(&g.placement.positions).map(|(a, b)| a.distance(*b)).fold(0f32, f32::max);
        assert!(moved < 1e-4, "positions differ by {moved}");
        assert_eq!(cpu.inside, g.placement.inside);
        let half = |w: u32| crate::hdr::half_to_f32(w as u16);
        let vis_diff = vis.iter().zip(&g.visibility).flat_map(|(a, b)| a.iter().zip(b))
            .map(|(a, b)| (half(*a & 0xffff) - half(*b & 0xffff)).abs().max((half(*a >> 16) - half(*b >> 16)).abs())).fold(0f32, f32::max);
        assert!(vis_diff < 2e-3, "visibility maps differ by {vis_diff}");
        let reach_diff: u32 = reach.iter().zip(&g.reach).flat_map(|(a, b)| a.iter().zip(b)).map(|(a, b)| (a ^ b).count_ones()).sum();
        let reach_total: u32 = reach.iter().flatten().map(|w| w.count_ones()).sum();
        assert!(reach_diff * 1000 <= reach_total, "reach differs in {reach_diff} of {reach_total} voxels");
        // A ray along a box edge meets two faces at the same distance; float
        // rounding may pick the other one. Rare, and either is that surface.
        let plane_diffs: Vec<usize> = (0..planes.len()).filter(|&i| (0..4).any(|k| (planes[i][k] - g.planes[i][k]).abs() > 1e-4)).collect();
        for &i in plane_diffs.iter().take(4) { eprintln!("plane {i}: cpu {:?} gpu {:?} at {}", planes[i], g.planes[i], cpu.positions[i]); }
        let plane_diff = plane_diffs.len();
        assert!(plane_diff * 100 <= planes.len(), "{plane_diff} of {} planes differ", planes.len());
        eprintln!("geometry agrees: positions {moved:.1e}, visibility {vis_diff:.1e}, reach {reach_diff}/{reach_total}, {plane_diff} planes picked the other of two equally near surfaces");
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    use crate::room_bake;
    use glam::{Affine3A, Quat};

    fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
        let (a, b) = (Vec3::from(min), Vec3::from(max));
        Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
    }

    /// Two 4 m rooms in a row on open ground: the first with a window the
    /// sun shines through onto its floor, a door between them; the second
    /// sees the sky only through the door and then the window (the case
    /// that showed as blotches before `sky_refine`).
    pub(super) fn two_rooms() -> Vec<Affine3A> {
        vec![
            bx([-50.0, -0.2, -50.0], [50.0, 0.0, 50.0]),
            // Outer walls: x -2.2..2.2, z -6.2..2.2, 3 m high, roof.
            bx([-2.2, 0.0, 2.0], [2.2, 3.0, 2.2]),
            bx([-2.2, 0.0, -6.2], [2.2, 3.0, -6.0]),
            bx([-2.2, 0.0, -6.0], [-2.0, 3.0, 2.0]),
            // East wall with a window in room 1 (z 0..1.2, y 1..2.2).
            bx([2.0, 0.0, -6.0], [2.2, 3.0, 0.0]), bx([2.0, 0.0, 1.2], [2.2, 3.0, 2.0]),
            bx([2.0, 0.0, 0.0], [2.2, 1.0, 1.2]), bx([2.0, 2.2, 0.0], [2.2, 3.0, 1.2]),
            bx([-2.2, 3.0, -6.2], [2.2, 3.2, 2.2]),
            // Between the rooms at z -2, a door x -0.5..0.5, 2.1 high.
            bx([-2.0, 0.0, -2.1], [-0.5, 3.0, -1.9]), bx([0.5, 0.0, -2.1], [2.0, 3.0, -1.9]), bx([-0.5, 2.1, -2.1], [0.5, 3.0, -1.9]),
        ]
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_bake_matches_reference_paths() {
        let gpu = Gpu::new().unwrap();
        let grid = Grid { origin: Vec3::new(-2.75, 0.25, -6.75), cell: 0.5, dims: [12, 7, 20] };
        let occ = Occluders::from_boxes(&two_rooms());
        let sun = Vec3::new(0.8, 0.45, 0.3).normalize();
        let settings = Settings::default();
        let placed = room_bake::Placement::new(&grid, &occ);
        let baked = bake(&gpu, &grid, &occ, &placed, sun, &settings).unwrap();
        for (name, zs) in [("room 1 (window)", -1.6f32..1.6), ("room 2 (door only)", -5.6..-2.4)] {
            let (mut b, mut r, mut dev, mut n) = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 0.0);
            for i in 0..grid.count() {
                let p = placed.positions[i];
                if placed.inside[i] || p.x.abs() > 1.6 || !zs.contains(&p.z) || !(0.4..2.6).contains(&p.y) { continue; }
                let reference = room_bake::reference_probe(&occ, p, sun, settings.reflectance, settings.bounces, 4000, i as u64);
                for a in 0..6 { b += baked[i].axes[a]; r += reference.axes[a]; dev += (baked[i].axes[a] - reference.axes[a]).abs(); }
                n += 6.0;
            }
            let (b, r, dev) = (b / n, r / n, dev / n);
            eprintln!("{name}: bake {b} reference {r}, mean per-probe deviation {dev} over {} probe axes", n as u32);
            for (ch, what) in [(0, "direct sky"), (1, "bounced sky"), (2, "bounced sun")] {
                assert!((b[ch] - r[ch]).abs() <= 0.15 * r[ch] + 2e-4, "{name} {what}: bake {} reference {}", b[ch], r[ch]);
            }
        }
    }
}

#[cfg(test)]
mod lamp_reference_tests {
    use super::*;
    use crate::room_bake;

    /// A lamp in the first of two rooms joined by a door: the second room's
    /// probes get its bounce only through the door, from the few surfaces
    /// near it the lamp lights. Bake (2 bounces) against brute-force paths.
    #[test]
    #[ignore = "needs a GPU"]
    fn gpu_lamp_bake_matches_reference_paths() {
        let gpu = Gpu::new().unwrap();
        let grid = Grid { origin: Vec3::new(-2.75, 0.25, -6.75), cell: 0.5, dims: [12, 7, 20] };
        let occ = Occluders::from_boxes(&super::reference_tests::two_rooms());
        let lamp = crate::headlights::LocalLight { position: Vec3::new(0.8, 2.6, 0.5), direction: Vec3::NEG_Y, intensity_cd: Vec3::splat(1500.0),
            range_m: 20.0, inner_cos: 0.3, outer_cos: -0.2, source_radius_m: 0.05, falloff: 2.0 };
        let settings = Settings { bounces: 2, ..Settings::default() };
        let placed = room_bake::Placement::new(&grid, &occ);
        let baked = bake_lights(&gpu, &grid, &occ, &placed, &[lamp], &settings).unwrap();
        let cpu = room_bake::bake_lights_placed(&grid, &occ, &placed, &[lamp], &settings).unwrap();
        for (name, zs) in [("room 1 (lamp)", -1.6f32..1.6), ("room 2 (through the door)", -5.6..-2.4)] {
            let (mut b, mut c, mut r, mut n) = (0.0, 0.0, 0.0, 0.0);
            for i in 0..grid.count() {
                let p = placed.positions[i];
                if placed.inside[i] || p.x.abs() > 1.6 || !zs.contains(&p.z) || !(0.4..2.6).contains(&p.y) || i % 2 != 0 { continue; }
                let reference = room_bake::reference_lamp_probe(&occ, p, &lamp, settings.reflectance, settings.bounces, 2000, i as u64);
                for a in 0..6 { b += baked[0][i].axes[a].element_sum(); c += cpu[0][i].axes[a].element_sum(); r += reference.axes[a].element_sum(); }
                n += 6.0;
            }
            let (b, c, r) = (b / n, c / n, r / n);
            eprintln!("{name}: gpu bake {b:.4} cpu bake {c:.4} reference {r:.4} over {} probe axes", n as u32);
            assert!((b - r).abs() <= 0.15 * r + 1e-3, "{name}: bake {b} reference {r}");
        }
    }
}
