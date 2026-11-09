//! Animated crowds: thousands of spectators as ordinary scene instances (a
//! body and an arms instance each), posed on the GPU every frame: cheer
//! waves, hops, excitement, panic, and fleeing from blasts. After Growing
//! Guns' colosseum crowd (`crowd.gdshader`). Spectators can be killed
//! (their instances empty out immediately). Confirmed contacts arrive asynchronously for secondary effects.
//!
//! ```ignore
//! let mut crowd = Crowd::reserve(&mut scene, 15_000);               // before Renderer::new
//! crowd.set_seats(&scene, &seats, Vec2::ZERO);                    // any time (a new level)
//! crowd.mood(excitement, panic);
//! crowd.flee(blast, radius, time);
//! crowd.kill(i);
//! crowd.flush(&gpu, &renderer, time)?;                           // per frame
//! ```
use crate::scene::{Scene, NO_SHADOW};
use crate::Gpu;
use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use std::sync::Mutex;
use std::collections::VecDeque;
#[path="crowd_reactions.rs"]
mod reactions;
pub use reactions::CrowdHits;
use reactions::{Stimulus, Receipt, Reactions};
use wgpu::util::DeviceExt;

/// After the hold, scattered spectators walk back for this long.
pub const FLEE_RETURN: f32 = 4.5;
/// Scattered spectators stay away this long after the last blast.
pub const FLEE_HOLD: f32 = 6.0;

/// One spectator. `body` and `arms` are scene meshes (unit height, feet at
/// y = 0, facing +Z, arms at x = +-0.21).
#[derive(Clone, Copy, Debug)]
pub struct Seat {
    pub pos: Vec3,
    pub yaw: f32,
    /// Height in metres.
    pub scale: f32,
    /// Shoulder width and depth relative to the height (1 the mesh as built).
    pub width: f32,
    /// 0..1: animation phase and how far it runs when scared.
    pub phase: f32,
    /// 0..1: cheer tempo.
    pub tempo: f32,
    /// Hop amplitude (0 sits still).
    pub hop: f32,
    pub body: usize,
    pub arms: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuSeat { pos_yaw: [f32; 4], params: [f32; 4], meshes: [u32; 4], shape: [f32; 4] }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform { mood: [f32; 4], ring: [f32; 4], counts: [u32; 4] }

/// Where a scene's crowd lives (set by [`Crowd::reserve`]).
#[derive(Clone, Copy, Debug)]
pub struct CrowdSpec { pub first: u32, pub capacity: u32 }

/// Game-side handle.
pub struct Crowd {
    pub first: usize,
    capacity: usize,
    count: usize,
    seats: Vec<GpuSeat>,
    seats_dirty: bool,
    human_identities: Option<Vec<crate::humans::Instance>>,
    positions: Vec<Vec3>,
    alive: Vec<u32>,
    dirty: Option<(usize, usize)>,
    excitement: f32,
    panic: f32,
    arm_pivot: f32,
    ring: Vec2,
    stimuli: Vec<Stimulus>,
    receipts: VecDeque<Receipt>,
    hits:Vec<CrowdHits>,
    reset_reactions: bool,
}

impl Crowd {
    /// Reserve two instances per seat for up to `capacity` spectators
    /// (shadowless, empty until `set_seats`). Call before `Renderer::new`.
    pub fn reserve(scene: &mut Scene, capacity: usize) -> Self {
        assert!(scene.crowd.is_none(), "one crowd per scene");
        assert!(!scene.meshes.is_empty(), "add the crowd meshes before reserving");
        let first = scene.instances.len();
        for _ in 0..capacity * 2 {
            scene.add_instance(0, glam::Affine3A::IDENTITY);
            let i = scene.instances.last_mut().unwrap();
            i.cluster_count = 0;
            i.sphere = [0.0; 4];
            i.flags |= NO_SHADOW;
        }
        scene.crowd = Some(CrowdSpec { first: first as u32, capacity: capacity as u32 });
        Crowd { first, capacity, count: 0, seats: Vec::new(), seats_dirty: true, human_identities: None, positions: Vec::new(), alive: vec![0; capacity], dirty: Some((0, capacity)),
            excitement: 0.0, panic: 0.0, arm_pivot: 0.0, ring: Vec2::ZERO, stimuli: Vec::new(), receipts: VecDeque::new(), hits:Vec::new(), reset_reactions:true }
    }

    /// Seat a new crowd (a new level); everyone alive, nobody fleeing.
    /// Seats beyond the capacity are dropped.
    pub fn set_seats(&mut self, scene: &Scene, seats: &[Seat], ring_center: Vec2) {
        let seats = &seats[..seats.len().min(self.capacity)];
        self.seats = seats.iter().map(|s| {
            let (b, a) = (&scene.meshes[s.body], &scene.meshes[s.arms]);
            GpuSeat { pos_yaw: s.pos.extend(s.yaw).to_array(), params: [s.scale, s.phase, s.tempo, s.hop],
                meshes: [b.first_cluster, b.cluster_count, a.first_cluster, a.cluster_count], shape: [s.width, 0.0, 0.0, 0.0] }
        }).collect();
        self.human_identities = None;
        self.count = seats.len();
        self.positions = seats.iter().map(|s| s.pos).collect();
        self.alive = (0..self.capacity).map(|i| u32::from(i < self.count)).collect();
        self.dirty = Some((0, self.capacity));
        self.seats_dirty = true;
        self.ring = ring_center;
        self.reset_reactions = true;
        self.stimuli.clear();
        self.receipts.clear();
        self.hits.clear();
    }

    /// Use the procedural human renderer for these seats and retain their match reactions.
    pub fn set_humans(&mut self, seed: u64, seats: &[Seat]) {
        self.human_identities = Some(seats.iter().take(self.count).enumerate()
            .map(|(i, seat)| crate::humans::Instance::spectator(seed, i, seat)).collect());
        self.seats_dirty = true;
    }

    pub fn uses_humans(&self) -> bool { self.human_identities.is_some() }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }
    pub fn alive(&self) -> usize { self.alive.iter().filter(|&&a| a != 0).count() }
    pub fn position(&self, i: usize) -> Vec3 { self.positions[i] }
    pub fn body_scale(&self, i: usize) -> Vec3 {
        if let Some(person) = self.human(i) {
            return person.body_scale();
        }
        let seat = self.seats[i];
        seat.params[0] * Vec3::new(seat.shape[0], 1., seat.shape[0])
    }
    pub fn human(&self, i: usize) -> Option<&crate::humans::Instance> {
        self.human_identities.as_ref()?.get(i)
    }
    pub fn is_alive(&self, i: usize) -> bool { self.alive[i] != 0 }

    /// Body language: 0..1 each (the crowd sound's enthusiasm and fear).
    pub fn mood(&mut self, excitement: f32, panic: f32) { self.excitement = excitement.clamp(0.0, 1.0); self.panic = panic.clamp(0.0, 1.0); }

    /// Rotate both arms forward around this local shoulder height. Zero keeps
    /// the legacy translating pose. Arm meshes must share this pivot plane.
    pub fn arm_pivot(&mut self, height: f32) { self.arm_pivot = height.max(0.0); }

    pub fn kill(&mut self, i: usize) {
        if std::mem::replace(&mut self.alive[i], 0) != 0 {
            self.dirty = Some(self.dirty.map_or((i, i + 1), |(a, b)| (a.min(i), b.max(i + 1))));
        }
    }

    /// Everyone back in their seats (a new round).
    pub fn revive(&mut self) {
        for a in &mut self.alive[..self.count] { *a = 1; }
        self.dirty = Some((0, self.count.max(1).min(self.capacity)));
        self.reset_reactions = true;
        self.stimuli.clear();
        self.receipts.clear();
        self.hits.clear();
    }

    /// Kill everyone within `radius` of `at`; returns their positions.
    pub fn kill_near(&mut self, at: Vec3, radius: f32, limit: usize) -> Vec<Vec3> {
        let mut out = Vec::new();
        for i in 0..self.count {
            if out.len() >= limit { break; }
            if self.alive[i] != 0 && self.positions[i].distance_squared(at) <= radius * radius {
                self.kill(i);
                out.push(self.positions[i]);
            }
        }
        out
    }

    pub fn flee(&mut self, at: Vec3, radius: f32, time: f32) {
        self.stimuli.push(Stimulus::new(
            0,
            at,
            at,
            0.,
            time,
            0,
            [self.excitement, self.panic],
            radius * 2.6 + 2.,
        ));
    }

    /// A collision-clipped segment. Nonzero ids follow one bullet across ticks.
    pub fn bullet_path(&mut self, from: Vec3, to: Vec3, id: u64, time: f32) {
        if from.distance_squared(to) > 1e-10 {
            self.stimuli.push(Stimulus::new(
                1,
                from,
                to,
                0.,
                time,
                id,
                [self.excitement, self.panic],
                2.8,
            ));
        }
    }

    pub fn explosion(&mut self, at: Vec3, radius: f32, time: f32) {
        self.stimuli.push(Stimulus::new(
            2,
            at,
            at,
            radius,
            time,
            0,
            [self.excitement, self.panic],
            radius * 2.6 + 2.,
        ));
    }

    /// GPU deaths already affect every view. Notifications drive secondary FX and telemetry.
    pub fn take_hits(&mut self) -> Result<Vec<CrowdHits>, String> {
        self.collect_hits()?;
        Ok(std::mem::take(&mut self.hits))
    }

    fn collect_hits(&mut self) -> Result<(), String> {
        while let Some(receipt) = self.receipts.front() {
            let Some(groups) = receipt.collect(&mut self.alive)? else {
                break;
            };
            self.hits.extend(groups);
            self.receipts.pop_front();
        }
        Ok(())
    }

    /// Replay reconstruction can resolve just the simulation ticks that had contacts.
    pub fn has_pending_events(&self) -> bool {
        !self.stimuli.is_empty() || !self.receipts.is_empty()
    }

    /// Send kills and the mood; schedules this frame's posing.
    pub fn flush(&mut self, gpu: &Gpu, renderer: &crate::Renderer, time: f32) -> Result<(), String> {
        self.collect_hits()?;
        if self.seats_dirty {
            renderer.set_crowd_humans(gpu, self.human_identities.as_deref(), self.first as u32)?;
        }
        let alive = self.dirty.take().map(|(a, b)| (a, &self.alive[a..b]));
        let seats = std::mem::take(&mut self.seats_dirty).then_some(bytemuck::cast_slice::<_, u8>(&self.seats));
        let u = Uniform {
            mood: [time, self.excitement, self.panic, self.arm_pivot],
            ring: [self.ring.x, self.ring.y, 0.0, 0.0],
            // All reserved seats run: unseated ones are dead and empty out.
            counts: [self.capacity as u32, self.first as u32, u32::from(self.human_identities.is_some()), self.count as u32],

        };
        renderer.update_crowd(gpu, bytemuck::bytes_of(&u), seats, alive)?;
        let crowd=renderer.crowd.as_ref().ok_or("missing scene crowd")?;
        self.receipts.extend(crowd.reactions.dispatch(gpu,self.reset_reactions,&self.stimuli,self.count));
        self.reset_reactions=false;
        self.stimuli.clear();
        Ok(())
    }
}

/// Renderer side.
pub(crate) struct CrowdRenderer {
    first: u32,
    count: u32,
    pipeline: wgpu::ComputePipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    alive: wgpu::Buffer,
    seats: wgpu::Buffer,
    pending: Mutex<bool>,
    reactions:Reactions,
}

impl CrowdRenderer {
    pub fn new(gpu: &Gpu, spec: &CrowdSpec, instances: &wgpu::Buffer) -> Self {
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("crowd"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("crowd"), layout: None, module: &module,
            entry_point: Some("animate"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let init = |label, bytes: &[u8], usage| gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: bytes, usage });
        let n = spec.capacity.max(1) as usize;
        let seats = init("crowd seats", bytemuck::cast_slice(&vec![GpuSeat::zeroed(); n]), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let alive = init("crowd alive", bytemuck::cast_slice(&vec![0u32; n]), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let uniform = init("crowd uniform", bytemuck::bytes_of(&Uniform::zeroed()), wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let reactions=Reactions::new(gpu,n,&uniform,&seats,&alive);
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("crowd"), layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: seats.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: alive.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: instances.as_entire_binding() },
            wgpu::BindGroupEntry {binding:4,resource:reactions.state.as_entire_binding()}] });
        CrowdRenderer { reactions, first: spec.first, count: spec.capacity, pipeline, bind, uniform, alive, seats, pending: Mutex::new(false) }
    }

    pub(crate) fn reaction_state(&self) -> &wgpu::Buffer { &self.reactions.state }

    pub(crate) fn pose_uniform(&self) -> &wgpu::Buffer { &self.uniform }

    pub fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        let (a, b) = (self.first as usize, (self.first + self.count * 2) as usize);
        first < b && first.saturating_add(count) > a
    }

    pub fn update(&self, gpu: &Gpu, uniform: &[u8], seats: Option<&[u8]>, alive: Option<(usize, &[u32])>) -> Result<(), String> {
        if let Some(s) = seats {
            if s.len() as u64 > self.seats.size() { return Err("more crowd seats than reserved".into()); }
            if !s.is_empty() { gpu.queue.write_buffer(&self.seats, 0, s); }
        }
        if let Some((first, a)) = alive {
            if first + a.len() > self.count as usize { return Err("crowd alive range out of bounds".into()); }
            gpu.queue.write_buffer(&self.alive, first as u64 * 4, bytemuck::cast_slice(a));
        }
        gpu.queue.write_buffer(&self.uniform, 0, uniform);
        *self.pending.lock().unwrap() = true;
        Ok(())
    }

    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, timestamp: Option<(&wgpu::QuerySet, u32)>) -> bool {
        if !std::mem::take(&mut *self.pending.lock().unwrap()) || self.count == 0 { return false; }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("crowd"), timestamp_writes: timestamp.map(|(query_set, start)|
            wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(start), end_of_pass_write_index: Some(start + 1) }) });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind, &[]);
        pass.dispatch_workgroups(self.count.div_ceil(64), 1, 1);
        true
    }
}

pub const SHADER: &str = concat!(include_str!("shaders/crowd_displacement.wgsl"),"\n",include_str!("shaders/crowd_pose.wgsl"),"\n",include_str!("shaders/crowd.wgsl"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crowd_wgsl_validates_and_layouts_match() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        assert_eq!(std::mem::size_of::<GpuSeat>(), 64);
        assert_eq!(std::mem::size_of::<Uniform>(), 48);
        let reaction = module.types.iter().find(|(_, t)| t.name.as_deref() == Some("Reaction")).unwrap().1;
        assert!(matches!(&reaction.inner, naga::TypeInner::Struct { span, .. } if *span == reactions::REACTION_BYTES));
    }

    #[test]
    fn seating_mask_resets_on_revive() {
        let mut scene = Scene::default();
        let m = scene.add_material(crate::scene::Material { tint: [0.5; 3], layer: -1, roughness: 0.8, flags: 0, scale: 1.0, metal: 0.0 });
        let cube = scene.add_mesh(&crate::primitives::cube(m).mesh());
        let seats: Vec<Seat> = (0..10).map(|i| Seat { pos: Vec3::new(i as f32, 0.0, 30.0), yaw: 0.0, scale: 1.7, width: 1.0, phase: 0.5, tempo: 0.5, hop: 0.2, body: cube, arms: cube }).collect();
        let mut c = Crowd::reserve(&mut scene, 12);
        assert_eq!(scene.instances.len(), 24);
        c.set_seats(&scene, &seats, Vec2::ZERO);
        let dead = c.kill_near(Vec3::new(4.0, 0.0, 30.0), 1.5, 10);
        assert_eq!(dead.len(), 3);
        assert_eq!(c.alive(), 7);
        c.revive();
        assert_eq!(c.alive(), 10);
    }
}

#[cfg(test)]
#[path="crowd_reactions_tests.rs"]
mod reaction_tests;
