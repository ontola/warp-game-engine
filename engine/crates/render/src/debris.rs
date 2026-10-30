//! GPU debris: thousands of tumbling shards simulated and drawn entirely on
//! the GPU, colliding with the world through an occupancy grid (one bit per
//! voxel). Shards are ordinary scene instances in a reserved range, so they
//! light, cull and get motion vectors like everything else; a compute pass
//! integrates them and writes their transforms before any view or shadow
//! pass. Nothing is read back: debris is cosmetic, gameplay never sees it.
//!
//! The game owns a [`Debris`]: it keeps the occupancy grid in step with the
//! world (`occupancy_mut().fill_box`/`clear_box` as pieces break), spawns
//! bursts, advances simulated time per tick and flushes once per frame.
//!
//! ```ignore
//! let occ = Occupancy::new(world_min, world_max, 0.5);
//! let mut debris = Debris::reserve(&mut scene, 4096, occ, false);
//! let renderer = Renderer::new(&gpu, &scene, &light);
//! debris.burst(&scene, &BurstSpec { center, radius: 3.0, count: 300, meshes: &rock_meshes, ..BurstSpec::rubble() });
//! debris.advance(1.0 / 60.0);                  // per simulation tick
//! debris.flush(&gpu, &renderer)?;              // per frame
//! ```
use crate::scene::{Scene, NO_SHADOW};
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use std::sync::Mutex;
use wgpu::util::DeviceExt;

/// Fixed simulation step of the GPU integrator.
pub const STEP: f32 = 1.0 / 60.0;
/// Most steps one flush runs (longer gaps skip ahead).
pub const MAX_STEPS: u32 = 240;
const MAX_SPAWNS: usize = 8192;

/// A world-aligned grid of solid/empty voxels, one bit each, x fastest.
#[derive(Clone, Debug)]
pub struct Occupancy {
    pub origin: Vec3,
    pub voxel: f32,
    pub dims: [u32; 3],
    pub bits: Vec<u32>,
    /// Changed word range since the last upload.
    dirty: Option<(usize, usize)>,
}

impl Occupancy {
    /// Empty grid covering `min..max` with cubic voxels.
    pub fn new(min: Vec3, max: Vec3, voxel: f32) -> Self {
        let voxel = voxel.max(0.05);
        let d = ((max - min) / voxel).ceil().max(Vec3::ONE).as_uvec3();
        let words = ((d.x as u64 * d.y as u64 * d.z as u64).div_ceil(32)) as usize;
        Occupancy { origin: min, voxel, dims: d.to_array(), bits: vec![0; words.max(1)], dirty: Some((0, words.max(1))) }
    }

    pub fn words(&self) -> usize { self.bits.len() }

    fn index(&self, v: [i64; 3]) -> Option<usize> {
        let d = self.dims.map(i64::from);
        (v.iter().zip(d).all(|(&c, n)| c >= 0 && c < n)).then(|| (v[0] + v[1] * d[0] + v[2] * d[0] * d[1]) as usize)
    }

    fn voxel_of(&self, p: Vec3) -> [i64; 3] { ((p - self.origin) / self.voxel).floor().as_i64vec3().to_array() }

    /// Solid at a point; outside the grid is empty (free flight).
    pub fn solid(&self, p: Vec3) -> bool {
        self.index(self.voxel_of(p)).is_some_and(|i| self.bits[i >> 5] >> (i & 31) & 1 == 1)
    }

    fn touch(&mut self, word: usize) {
        self.dirty = Some(self.dirty.map_or((word, word + 1), |(a, b)| (a.min(word), b.max(word + 1))));
    }

    /// Set every voxel whose centre lies inside a box rotated about +Y.
    pub fn fill_box(&mut self, center: Vec3, half: Vec3, yaw: f32, solid: bool) {
        let rot = Quat::from_rotation_y(yaw);
        let inv = rot.inverse();
        let ext = Vec3::new(
            (rot * Vec3::new(half.x, 0.0, 0.0)).abs().x + (rot * Vec3::new(0.0, 0.0, half.z)).abs().x,
            half.y,
            (rot * Vec3::new(half.x, 0.0, 0.0)).abs().z + (rot * Vec3::new(0.0, 0.0, half.z)).abs().z);
        let lo = self.voxel_of(center - ext);
        let hi = self.voxel_of(center + ext);
        let d = self.dims.map(i64::from);
        for z in lo[2].max(0)..=hi[2].min(d[2] - 1) {
            for y in lo[1].max(0)..=hi[1].min(d[1] - 1) {
                for x in lo[0].max(0)..=hi[0].min(d[0] - 1) {
                    let c = self.origin + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * self.voxel;
                    let l = inv * (c - center);
                    if l.x.abs() > half.x || l.y.abs() > half.y || l.z.abs() > half.z { continue; }
                    let i = (x + y * d[0] + z * d[0] * d[1]) as usize;
                    let (w, b) = (i >> 5, 1u32 << (i & 31));
                    let old = self.bits[w];
                    self.bits[w] = if solid { old | b } else { old & !b };
                    if self.bits[w] != old { self.touch(w); }
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.bits.iter_mut().for_each(|w| *w = 0);
        self.dirty = Some((0, self.bits.len()));
    }

    /// Solid voxel count (diagnostics, tests).
    pub fn count(&self) -> u64 { self.bits.iter().map(|w| w.count_ones() as u64).sum() }
}

/// One shard on the GPU (80 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Shard {
    /// Position, age in seconds.
    pub pos_age: [f32; 4],
    /// Velocity, lifetime in seconds (0: dead).
    pub vel_life: [f32; 4],
    /// Full size per axis at birth, tumble angle.
    pub size_angle: [f32; 4],
    /// Tumble axis (unit), tumble rate in radians per second.
    pub axis_spin: [f32; 4],
    /// First cluster, cluster count, mesh bounding radius (f32 bits), pool slot.
    pub mesh: [u32; 4],
}

/// How a burst leaves its centre (Godot `gpu_debris.gdshader` `start()`).
#[derive(Clone, Copy, Debug)]
pub struct BurstSpec<'a> {
    pub center: Vec3,
    /// Shards start within this radius and the cloud scales with it.
    pub radius: f32,
    pub count: u32,
    /// Scene meshes to pick from per shard (rock variants); unit sized.
    pub meshes: &'a [usize],
    /// Longest edge of a typical shard in metres.
    pub size: f32,
    /// Launch speed in m/s.
    pub speed: f32,
    pub life: f32,
}

impl<'a> BurstSpec<'a> {
    /// A blast's rubble: shards from fist to boulder, a few seconds.
    pub fn rubble(center: Vec3, radius: f32, count: u32, meshes: &'a [usize]) -> Self {
        BurstSpec { center, radius, count, meshes, size: (0.3 + radius * 0.09).clamp(0.35, 1.0), speed: (5.0 + radius * 1.1).clamp(7.0, 17.0), life: 2.6 }
    }
    /// Flecks off a bullet hit.
    pub fn chips(center: Vec3, radius: f32, count: u32, size: f32, meshes: &'a [usize]) -> Self {
        BurstSpec { center, radius, count, meshes, size, speed: (3.0 + radius * 1.5).clamp(3.5, 8.0), life: 1.4 }
    }
}

/// Simulation constants (defaults from the Godot shader).
#[derive(Clone, Copy, Debug)]
pub struct Tuning { pub gravity: f32, pub bounce: f32, pub friction: f32, pub kill_y: f32 }
impl Default for Tuning { fn default() -> Self { Tuning { gravity: 21.0, bounce: 0.34, friction: 0.76, kill_y: -60.0 } } }

/// Where a scene's debris pool lives (set by [`Debris::reserve`]).
#[derive(Clone, Copy, Debug)]
pub struct DebrisSpec { pub first: u32, pub capacity: u32, pub occupancy_words: u32 }

/// Game-side handle: allocation, spawning, occupancy and timing.
pub struct Debris {
    pub first: usize,
    pub capacity: usize,
    pub tuning: Tuning,
    occupancy: Occupancy,
    meta_dirty: bool,
    pending: Vec<Shard>,
    cursor: usize,
    /// Simulated time each slot's shard dies at.
    dies: Vec<f64>,
    time: f64,
    unsimulated: f64,
    reset: bool,
    rng: u64,
    /// Shards spawned since creation (diagnostics).
    pub spawned: u64,
}

impl Debris {
    /// Reserve `capacity` instances for shards. Call before `Renderer::new`.
    /// Shards cast no shadows unless `shadows`.
    pub fn reserve(scene: &mut Scene, capacity: usize, occupancy: Occupancy, shadows: bool) -> Self {
        assert!(!scene.meshes.is_empty(), "add the shard meshes before reserving debris");
        assert!(scene.debris.is_none(), "one debris pool per scene");
        let first = scene.instances.len();
        for _ in 0..capacity {
            scene.add_instance(0, glam::Affine3A::IDENTITY);
            let i = scene.instances.last_mut().unwrap();
            i.cluster_count = 0;
            i.sphere = [0.0; 4];
            if !shadows { i.flags |= NO_SHADOW; }
        }
        scene.debris = Some(DebrisSpec { first: first as u32, capacity: capacity as u32, occupancy_words: occupancy.words() as u32 });
        Debris { first, capacity, tuning: Tuning::default(), occupancy, meta_dirty: true, pending: Vec::new(), cursor: 0, dies: vec![0.0; capacity],
            time: 0.0, unsimulated: 0.0, reset: false, rng: 0x9E37_79B9_7F4A_7C15, spawned: 0 }
    }

    pub fn occupancy(&self) -> &Occupancy { &self.occupancy }
    /// Edit the collision grid (uploaded at the next flush).
    pub fn occupancy_mut(&mut self) -> &mut Occupancy { &mut self.occupancy }
    /// Replace the grid (a new level). It must not need more words than the first.
    pub fn set_occupancy(&mut self, occ: Occupancy) {
        assert!(occ.words() <= self.occupancy.words().max(occ.words()), "occupancy grew");
        self.occupancy = occ;
        self.occupancy.dirty = Some((0, self.occupancy.words()));
        self.meta_dirty = true;
    }

    /// A replacement renderer has empty collision and shard buffers. Resend
    /// the full grid and keep spawns not yet flushed. Existing live shards
    /// cannot survive a rebuild: their current state lives only on the GPU.
    /// Call after rebuilding the renderer, before the next `flush`.
    pub fn invalidate_renderer(&mut self) {
        self.occupancy.dirty = Some((0, self.occupancy.words()));
        self.meta_dirty = true;
        let queued: Vec<_> = self.pending.iter().map(|s| {
            let slot = s.mesh[3] as usize;
            (slot, self.dies[slot])
        }).collect();
        self.dies.fill(0.0);
        for (slot, dies) in queued { self.dies[slot] = dies; }
        // The new GPU pool is already empty. A pending reset would run after
        // the spawn dispatch and kill the queued shards along with old ones.
        self.reset = false;
    }

    /// Shards alive now.
    pub fn live(&self) -> usize { self.dies.iter().filter(|&&t| t > self.time).count() }

    /// Kill every shard (a new level).
    pub fn clear(&mut self) {
        self.pending.clear();
        self.dies.iter_mut().for_each(|t| *t = 0.0);
        self.reset = true;
    }

    /// Simulated time passes (call per game tick).
    pub fn advance(&mut self, dt: f32) { self.time += dt as f64; self.unsimulated += dt as f64; }

    fn f(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Spawn one shard; the oldest is recycled when the pool is full.
    pub fn spawn(&mut self, scene: &Scene, mesh: usize, pos: Vec3, vel: Vec3, size: Vec3, spin: f32, life: f32) {
        if self.capacity == 0 { return; }
        // Prefer a dead slot near the cursor, else take the cursor's.
        let mut slot = self.cursor;
        for k in 0..self.capacity.min(64) {
            let s = (self.cursor + k) % self.capacity;
            if self.dies[s] <= self.time { slot = s; break; }
        }
        self.cursor = (slot + 1) % self.capacity;
        self.dies[slot] = self.time + life as f64;
        let m = &scene.meshes[mesh];
        let axis = Vec3::new(self.f() - 0.5, self.f() - 0.5, self.f() - 0.5).normalize_or(Vec3::Y);
        let angle = self.f() * std::f32::consts::TAU;
        self.pending.retain(|s| s.mesh[3] != slot as u32);
        self.pending.push(Shard {
            pos_age: pos.extend(0.0).to_array(),
            vel_life: vel.extend(life).to_array(),
            size_angle: size.extend(angle).to_array(),
            axis_spin: axis.extend(spin).to_array(),
            mesh: [m.first_cluster, m.cluster_count, (m.sphere[3] + Vec3::from_slice(&m.sphere[..3]).length()).to_bits(), slot as u32],
        });
        self.spawned += 1;
    }

    /// A cloud of shards (Godot's burst): outward and upward from the
    /// centre, pulled back out of solid voxels, heavy-tailed sizes (mostly
    /// rubble, some boulders), heavier pieces slower, squashed per axis.
    pub fn burst(&mut self, scene: &Scene, b: &BurstSpec) {
        self.burst_with_lift(scene, b, false);
    }

    /// Near-vertical launch for a lifting field; normal gravity still applies.
    /// Spatial scatter is independent of velocity, so the cloud does not fan out.
    pub fn burst_lift(&mut self, scene: &Scene, b: &BurstSpec) {
        self.burst_with_lift(scene, b, true);
    }

    fn burst_with_lift(&mut self, scene: &Scene, b: &BurstSpec, lift: bool) {
        if b.meshes.is_empty() { return; }
        for _ in 0..b.count {
            let r: [f32; 11] = std::array::from_fn(|_| self.f());
            let dir = (Vec3::new(r[0], r[1], r[2]) * 2.0 - 1.0 + Vec3::Y * 0.45).normalize_or(Vec3::Y);
            let mut pos = b.center + dir * b.radius * (0.2 + 0.5 * r[3]);
            if lift { pos.y = b.center.y + 0.2 + dir.y.abs() * b.radius * 0.35; }
            for _ in 0..4 {
                if !self.occupancy.solid(pos) { break; }
                pos = pos.lerp(b.center, 0.4);
            }
            let mut s = b.size * (0.55 + 1.15 * r[6] * r[6]);
            if r[7] < 0.14 { s *= 1.6 + 0.6 * r[8]; }
            let mass = (s / (b.size * 2.2)).clamp(0.0, 1.0);
            let mut vel = dir * b.speed * (0.3 + 1.15 * r[4]) * (1.15 - 0.6 * mass);
            vel.y += b.speed * (0.05 + 0.45 * r[5]);
            if lift {
                vel = Vec3::new((r[0] - 0.5) * 0.8, b.speed * (0.85 + 0.30 * r[4]), (r[2] - 0.5) * 0.8);
            }
            let prop = Vec3::new(0.5 + 0.5 * self.f(), 0.32 + 0.6 * self.f(), 0.45 + 0.55 * self.f());
            let mesh = b.meshes[(r[10] * b.meshes.len() as f32) as usize % b.meshes.len()];
            let life = b.life * (0.85 + 0.3 * self.f());
            self.spawn(scene, mesh, pos, vel, prop * s, (r[9] - 0.5) * 18.0, life);
        }
    }

    /// Upload spawns and grid changes and schedule this frame's simulation.
    pub fn flush(&mut self, gpu: &Gpu, renderer: &crate::Renderer) -> Result<(), String> {
        let steps = ((self.unsimulated / STEP as f64).floor() as u32).min(MAX_STEPS);
        self.unsimulated -= steps as f64 * STEP as f64;
        if self.unsimulated > STEP as f64 { self.unsimulated = 0.0; }
        let occ = self.occupancy.dirty.take().map(|(a, b)| (a, &self.occupancy.bits[a..b]));
        let meta = std::mem::take(&mut self.meta_dirty).then_some(&self.occupancy);
        // More than MAX_SPAWNS in one frame: the newest win (pool order).
        let spawns = std::mem::take(&mut self.pending);
        let spawns = &spawns[spawns.len().saturating_sub(MAX_SPAWNS)..];
        // Keep dispatching a moment past the last death so dead shards' instances are cleared.
        let active = self.dies.iter().any(|&t| t + 0.25 > self.time);
        renderer.update_debris(gpu, spawns, occ, meta, steps, std::mem::take(&mut self.reset), active, &self.tuning)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
    sim: [f32; 4],
    grid: [f32; 4],
    dims: [u32; 4],
    counts: [u32; 4],
    extra: [f32; 4],
}

/// Renderer side: buffers, pipelines and the pending dispatch.
pub(crate) struct DebrisRenderer {
    spec: DebrisSpec,
    spawn: wgpu::ComputePipeline,
    simulate: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    spawns: wgpu::Buffer,
    occupancy: wgpu::Buffer,
    state: Mutex<PendingState>,
}

#[derive(Default)]
struct PendingState { uniform: Option<Uniform>, spawn_batches: Vec<Vec<Shard>>, steps: u32, reset: bool, active: bool }

impl DebrisRenderer {
    pub fn new(gpu: &Gpu, spec: DebrisSpec, instances: &wgpu::Buffer) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("debris"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let buffer = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let st = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("debris"), entries: &[
            buffer(0, wgpu::BufferBindingType::Uniform), buffer(1, st(false)), buffer(2, st(true)), buffer(3, st(true)), buffer(4, st(false))] });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("debris"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipe = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry), layout: Some(&pipeline_layout), module: &module, entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let (spawn, simulate) = (pipe("spawn"), pipe("simulate"));
        let storage = |label, size: u64, usage| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false });
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let shards = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("debris shards"),
            contents: bytemuck::cast_slice(&vec![Shard::default(); spec.capacity.max(1) as usize]), usage: rw });
        let spawns = storage("debris spawns", (MAX_SPAWNS * std::mem::size_of::<Shard>()) as u64, rw);
        let occupancy = storage("debris occupancy", spec.occupancy_words.max(1) as u64 * 4, rw);
        let uniform = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("debris uniform"),
            contents: bytemuck::bytes_of(&Uniform::zeroed()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let entries = [uniform.as_entire_binding(), shards.as_entire_binding(), spawns.as_entire_binding(), occupancy.as_entire_binding(), instances.as_entire_binding()];
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("debris"), layout: &layout,
            entries: &entries.iter().enumerate().map(|(i, r)| wgpu::BindGroupEntry { binding: i as u32, resource: r.clone() }).collect::<Vec<_>>() });
        DebrisRenderer { spec, spawn, simulate, bind, uniform, spawns, occupancy, state: Mutex::new(PendingState::default()) }
    }

    pub fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        let (a, b) = (self.spec.first as usize, (self.spec.first + self.spec.capacity) as usize);
        first < b && first.saturating_add(count) > a
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update(&self, gpu: &Gpu, spawns: &[Shard], occ: Option<(usize, &[u32])>, meta: Option<&Occupancy>, steps: u32, reset: bool, active: bool, tuning: &Tuning) -> Result<(), String> {
        if let Some((first, words)) = occ {
            if first + words.len() > self.spec.occupancy_words.max(1) as usize { return Err("debris occupancy larger than reserved".into()); }
            gpu.queue.write_buffer(&self.occupancy, first as u64 * 4, bytemuck::cast_slice(words));
        }
        let mut st = self.state.lock().unwrap();
        let mut u = st.uniform.unwrap_or_else(Uniform::zeroed);
        if let Some(o) = meta {
            u.grid = o.origin.extend(o.voxel).to_array();
            u.dims = [o.dims[0], o.dims[1], o.dims[2], o.words() as u32];
        }
        u.sim = [STEP, tuning.gravity, tuning.bounce, tuning.friction];
        u.extra = [tuning.kill_y, 0.5, 0.0, 0.0];
        u.counts[1] = self.spec.capacity;
        u.counts[2] = self.spec.first;
        st.uniform = Some(u);
        if !spawns.is_empty() { st.spawn_batches.push(spawns.to_vec()); }
        st.steps += steps;
        st.reset |= reset;
        st.active = active;
        Ok(())
    }

    pub fn read_instances(&self, gpu: &Gpu, instances: &wgpu::Buffer) -> Result<Vec<crate::scene::GpuInstance>, String> {
        let size = std::mem::size_of::<crate::scene::GpuInstance>() as u64;
        let bytes = self.spec.capacity as u64 * size;
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("debris readback"), size: bytes.max(4), usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(instances, self.spec.first as u64 * size, &read, 0, bytes);
        gpu.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("{e:?}"))?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("{e:?}"))?;
        let view = read.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?;
        Ok(bytemuck::pod_collect_to_vec(&view[..bytes as usize]))
    }

    /// Record the spawn and simulation dispatches. Returns whether it ran.
    pub fn encode(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, timestamp: Option<(&wgpu::QuerySet, u32)>) -> bool {
        let mut st = self.state.lock().unwrap();
        let Some(mut u) = st.uniform else { return false };
        let batches = std::mem::take(&mut st.spawn_batches);
        let steps = std::mem::take(&mut st.steps);
        let reset = std::mem::take(&mut st.reset);
        if batches.is_empty() && !reset && !st.active { return false; }
        // Spawns from several flushes before one frame share one upload.
        let merged: Vec<Shard> = batches.into_iter().flatten().collect();
        let merged = &merged[merged.len().saturating_sub(MAX_SPAWNS)..];
        u.counts[0] = merged.len() as u32;
        u.counts[3] = steps;
        u.extra[2] = if reset { 1.0 } else { 0.0 };
        gpu.queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        if !merged.is_empty() { gpu.queue.write_buffer(&self.spawns, 0, bytemuck::cast_slice(merged)); }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("debris"), timestamp_writes: timestamp.map(|(query_set, start)|
            wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(start), end_of_pass_write_index: Some(start + 1) }) });
        pass.set_bind_group(0, &self.bind, &[]);
        if !merged.is_empty() {
            pass.set_pipeline(&self.spawn);
            pass.dispatch_workgroups((merged.len() as u32).div_ceil(64), 1, 1);
        }
        pass.set_pipeline(&self.simulate);
        pass.dispatch_workgroups(self.spec.capacity.div_ceil(64), 1, 1);
        true
    }
}

pub const SHADER: &str = include_str!("shaders/debris.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debris_wgsl_validates_and_layouts_match() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<Shard>(), 80);
        assert_eq!(std::mem::size_of::<Uniform>(), 80);
    }

    #[test]
    fn lifting_burst_launches_every_shard_nearly_vertically() {
        let mut scene=Scene::default();
        let cube=scene.add_mesh(&crate::primitives::cube(0).mesh());
        let occ=Occupancy::new(Vec3::splat(-20.0),Vec3::splat(20.0),0.5);
        let mut debris=Debris::reserve(&mut scene,512,occ,false);
        let meshes=[cube];
        let mut burst=BurstSpec::rubble(Vec3::ZERO,6.0,300,&meshes);
        burst.speed=26.0;burst.life=5.5;
        debris.burst_lift(&scene,&burst);
        assert_eq!(debris.pending.len(),300);
        for shard in &debris.pending {
            let v=shard.vel_life;
            assert!((22.1..=29.91).contains(&v[1]));
            assert!(v[0].abs()<=0.4 && v[2].abs()<=0.4);
            assert!(shard.pos_age[1]>0.0 && v[3]>4.6);
        }
    }

    /// Real GPU (any adapter; `KERN_ADAPTER=software` runs it on a CPU rasteriser):
    /// cargo test -p render --lib debris::tests::shards_fall_bounce_and_settle_on_the_gpu -- --ignored
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn shards_fall_bounce_and_settle_on_the_gpu() {
        use crate::{Camera, Lighting, Renderer, ViewRect};
        let gpu = Gpu::new().unwrap();
        let mut scene = Scene::default();
        let m = scene.add_material(crate::scene::Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&crate::primitives::cube(m).mesh());
        scene.add_instance(cube, glam::Affine3A::from_scale_rotation_translation(Vec3::new(40.0, 2.0, 40.0), Quat::IDENTITY, Vec3::new(0.0, -1.0, 0.0)));
        let mut occ = Occupancy::new(Vec3::new(-20.0, -4.0, -20.0), Vec3::new(20.0, 20.0, 20.0), 0.5);
        occ.fill_box(Vec3::new(0.0, -1.0, 0.0), Vec3::new(20.0, 1.0, 20.0), 0.0, true);
        let mut d = Debris::reserve(&mut scene, 512, occ, false);
        let light = Lighting::default();
        let r = Renderer::new(&gpu, &scene, &light);
        let target = Renderer::output_texture(&gpu, 64, 64);
        let view = [ViewRect { cam: Camera { pos: Vec3::new(0.0, 6.0, 14.0), target: Vec3::ZERO, fov_y: 1.0, near: 0.1 }, x: 0, y: 0, w: 64, h: 64 }];
        d.burst(&scene, &BurstSpec { life: 30.0, ..BurstSpec::rubble(Vec3::new(0.0, 1.5, 0.0), 2.0, 300, &[cube]) });
        let frame = |d: &mut Debris, ticks: u32| { for _ in 0..ticks { d.advance(STEP); } d.flush(&gpu, &r).unwrap(); r.try_render_into(&gpu, &view, &light, &target).unwrap(); };
        frame(&mut d, 6);
        let early = r.debris_debug_instances(&gpu).unwrap();
        let live: Vec<_> = early.iter().filter(|i| i.cluster_count > 0).collect();
        assert_eq!(live.len(), 300);
        let top = |i: &&crate::scene::GpuInstance| i.rows[1][3];
        assert!(live.iter().map(top).fold(f32::MIN, f32::max) > 2.0, "thrown upwards");
        for _ in 0..30 { frame(&mut d, 10); }
        let late = r.debris_debug_instances(&gpu).unwrap();
        let ys: Vec<f32> = late.iter().filter(|i| i.cluster_count > 0).map(|i| i.rows[1][3]).collect();
        assert_eq!(ys.len(), 300, "long-lived shards stay");
        let resting = ys.iter().filter(|&&y| (-0.3..1.2).contains(&y)).count();
        assert!(resting > 270, "settled on the floor: {resting} of 300, e.g. {:?}", &ys[..8]);
        assert!(ys.iter().all(|&y| y > -1.0), "none fell through the floor");
        // Dying: instances empty out.
        d.clear();
        frame(&mut d, 1);
        assert!(r.debris_debug_instances(&gpu).unwrap().iter().all(|i| i.cluster_count == 0));
        // Same integrator, now the Ion-style launch: measure actual GPU
        // instances during ascent, near the apex and on the way down.
        let meshes=[cube];
        let mut lift=BurstSpec::rubble(Vec3::new(0.0,1.0,0.0),2.0,300,&meshes);
        lift.speed=26.0;lift.life=30.0;
        d.burst_lift(&scene,&lift);
        frame(&mut d,1);
        let start=r.debris_debug_instances(&gpu).unwrap();
        for _ in 0..6 {frame(&mut d,10);}
        let raised=r.debris_debug_instances(&gpu).unwrap();
        for (a,b) in start.iter().zip(&raised).filter(|(a,_)|a.cluster_count>0) {
            assert!(b.rows[1][3]>a.rows[1][3]+8.0,"all shards rise");
            assert!((b.rows[0][3]-a.rows[0][3]).abs()<0.5 && (b.rows[2][3]-a.rows[2][3]).abs()<0.5);
        }
        for _ in 0..12 {frame(&mut d,10);}
        let fallen=r.debris_debug_instances(&gpu).unwrap();
        assert!(raised.iter().zip(&fallen).filter(|(a,_)|a.cluster_count>0).all(|(a,b)|b.rows[1][3]<a.rows[1][3]),"gravity returns the lifted cloud");
    }

    #[test]
    fn occupancy_fills_rotated_boxes_and_clears() {
        let mut o = Occupancy::new(Vec3::splat(-10.0), Vec3::splat(10.0), 0.5);
        o.fill_box(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 0.5), 0.0, true);
        assert_eq!(o.count(), 8 * 4 * 2);
        assert!(o.solid(Vec3::new(1.9, 0.9, 0.4)) && !o.solid(Vec3::new(2.1, 0.0, 0.0)));
        o.fill_box(Vec3::new(5.0, 0.0, 5.0), Vec3::new(2.0, 1.0, 0.5), std::f32::consts::FRAC_PI_2, true);
        assert!(o.solid(Vec3::new(5.1, 0.2, 6.8)) && !o.solid(Vec3::new(6.8, 0.2, 5.1)), "rotated a quarter turn");
        o.fill_box(Vec3::ZERO, Vec3::new(2.0, 1.0, 0.5), 0.0, false);
        assert!(!o.solid(Vec3::new(1.9, 0.9, 0.4)));
        assert!(!o.solid(Vec3::splat(50.0)), "outside is empty");
    }

    #[test]
    fn renderer_rebuild_resends_collision_and_keeps_only_unflushed_shards() {
        let mut scene = Scene::default();
        let cube = scene.add_mesh(&crate::primitives::cube(0).mesh());
        let mut occ = Occupancy::new(Vec3::splat(-2.0), Vec3::splat(2.0), 0.5);
        occ.fill_box(Vec3::new(0.0, -1.0, 0.0), Vec3::new(2.0, 1.0, 2.0), 0.0, true);
        let mut d = Debris::reserve(&mut scene, 4, occ, false);
        d.spawn(&scene, cube, Vec3::Y, Vec3::ZERO, Vec3::ONE, 0.0, 10.0);
        // The first shard and collision upload were consumed by the old
        // renderer. One later shard still waits for its first upload.
        d.pending.clear();
        d.occupancy.dirty = None;
        d.meta_dirty = false;
        d.advance(STEP);
        d.spawn(&scene, cube, Vec3::Y * 2.0, Vec3::NEG_Y, Vec3::ONE, 1.0, 2.0);
        let pending = d.pending[0];
        let slot = pending.mesh[3] as usize;
        let dies = d.dies[slot];
        let bits = d.occupancy.bits.clone();
        d.reset = true;

        d.invalidate_renderer();

        assert!(d.meta_dirty);
        assert_eq!(d.occupancy.dirty, Some((0, d.occupancy.words())));
        assert_eq!(d.occupancy.bits, bits, "resend the existing collision world");
        assert_eq!(d.live(), 1, "discarded GPU shards no longer occupy pool slots");
        assert_eq!(d.dies[slot], dies, "queued lifetime is not extended");
        assert_eq!(bytemuck::cast_slice::<Shard, u8>(&d.pending), bytemuck::bytes_of(&pending));
        assert!(!d.reset, "a reset after spawning would erase the queued shard");
    }

    #[test]
    fn bursts_recycle_the_oldest_and_start_outside_solids() {
        let mut scene = Scene::default();
        let m = scene.add_material(crate::scene::Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let rock = scene.add_mesh(&crate::primitives::cube(m).mesh());
        let mut occ = Occupancy::new(Vec3::splat(-20.0), Vec3::splat(20.0), 0.5);
        occ.fill_box(Vec3::new(0.0, -1.0, 0.0), Vec3::new(20.0, 1.0, 20.0), 0.0, true);
        let mut d = Debris::reserve(&mut scene, 100, occ, false);
        assert!(scene.instances[d.first..].iter().all(|i| i.cluster_count == 0 && i.flags & NO_SHADOW != 0));
        d.burst(&scene, &BurstSpec::rubble(Vec3::new(0.0, 0.2, 0.0), 3.0, 80, &[rock]));
        assert_eq!(d.live(), 80);
        assert!(d.pending.iter().all(|s| s.pos_age[1] > -0.01 || !d.occupancy.solid(Vec3::from_slice(&s.pos_age[..3]))));
        d.burst(&scene, &BurstSpec::rubble(Vec3::ZERO, 3.0, 80, &[rock]));
        assert_eq!(d.live(), 100, "pool full: oldest recycled");
        let slots: std::collections::HashSet<u32> = d.pending.iter().map(|s| s.mesh[3]).collect();
        assert_eq!(slots.len(), d.pending.len(), "one pending spawn per slot");
        d.advance(10.0);
        assert_eq!(d.live(), 0);
    }
}
