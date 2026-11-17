//! GPU particles: tens of thousands of sparks, embers and dust motes born,
//! simulated and drawn entirely on the GPU. The game only sends emitters
//! (where, how many, how they fly and look); a compute pass writes the
//! particles into a ring, integrates them at a fixed step (gravity, drag
//! towards the wind, a divergence-free swirl, a bouncing floor) and lists
//! the live ones for an indirect draw. Nothing per particle crosses the bus
//! and nothing is read back: particles are cosmetic, gameplay never sees them.
//!
//! Drawn per view in HDR after temporal reconstruction, like `fx` and
//! `sprites`: camera-facing quads (velocity streaks for sparks), softened
//! and clipped against the opaque depth, premultiplied so one particle can
//! add light (`additive` 1) or cover (`additive` 0). Unsorted: keep covering
//! particles faint. They write no depth and cast no shadow.
//!
//! Opt-in while it is new: `GpuParticles::new` gives a pool that ignores
//! emitters unless `KERN_GPU_PARTICLES=1` (or `GpuParticles::forced`).
//! The minimal forward renderer does not draw them.
//!
//! Storage buffers: five in the compute stage (the optional ground heightfield in
//! its own bind group), two in the vertex stage.
//!
//! ```ignore
//! let mut sparks = GpuParticles::new(65536);
//! sparks.set_floor(0.0);
//! sparks.emit(time, &Emitter { position, count: 4000, ..Emitter::sparks() });   // on the event
//! sparks.flush(&gpu, &renderer, time)?;                                         // per frame
//! ```
use crate::{Camera, Gpu, Lighting};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::sync::Mutex;

/// Fixed simulation step (seconds).
pub const STEP: f32 = 1.0 / 120.0;
/// Most steps one frame runs (1 s, for hitches and headless captures that
/// render now and then); longer gaps age particles without moving them.
pub const MAX_STEPS: u32 = 120;
/// Emitters one frame uploads; more in one frame: the newest win.
pub const MAX_EMITTERS: usize = 1024;
pub const MAX_CAPACITY: u32 = 1 << 20;

/// How one particle is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A round soft puff.
    Soft = 0,
    /// A streak along its velocity (set `stretch`), hot along its axis.
    Spark = 1,
    /// A small hard dot that flickers (with `glow`).
    Ember = 2,
    /// A round head with a fading trail of where it was `stretch` seconds ago.
    Trail = 3,
}

/// A burst of particles from one place. Metres, seconds, linear colour.
#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    pub position: Vec3,
    /// Births start uniformly within this ball.
    pub radius: f32,
    pub direction: Vec3,
    /// Half angle of the launch cone in radians (PI: every direction).
    pub spread: f32,
    /// Added to every launch (a moving source).
    pub velocity: Vec3,
    /// Launch speed range (m/s).
    pub speed: [f32; 2],
    pub count: u32,
    /// Births spread evenly over this many seconds (0: all at once).
    pub duration: f32,
    pub life: [f32; 2],
    /// Width at birth and at death (metres), linear over life.
    pub size: [f32; 2],
    /// Colour and alpha at birth and at death (0..1), linear over life.
    pub color: [Vec4; 2],
    /// Radiance in nits for colour 1: a glowing particle. 0: lit by sun and sky.
    pub glow: f32,
    /// 1 adds its light, 0 covers what is behind it.
    pub additive: f32,
    /// Downwards m/s² (negative rises).
    pub gravity: f32,
    /// Per second, towards the wind.
    pub drag: f32,
    /// Strength of the swirl (m/s²).
    pub turbulence: f32,
    /// Restitution on the floor; negative: the particle dies there.
    pub bounce: f32,
    /// Streak length per m/s of speed, in widths (0: round); for
    /// `Shape::Trail` the trail's length in seconds of travel.
    pub stretch: f32,
    pub shape: Shape,
}

impl Emitter {
    /// White-hot streaks that fly fast, fall and skitter over the floor.
    pub fn sparks() -> Self {
        Emitter { position: Vec3::ZERO, radius: 0.2, direction: Vec3::Y, spread: 1.4, velocity: Vec3::ZERO,
            speed: [6.0, 22.0], count: 800, duration: 0.0, life: [0.5, 1.6], size: [0.035, 0.015],
            color: [Vec4::new(1.0, 0.85, 0.55, 1.0), Vec4::new(1.0, 0.25, 0.04, 0.0)], glow: 40_000.0, additive: 1.0,
            gravity: 9.8, drag: 0.9, turbulence: 0.0, bounce: 0.35, stretch: 0.07, shape: Shape::Spark }
    }
    /// Slow glowing motes that ride the hot air up and swirl, and glow on
    /// where they land.
    pub fn embers() -> Self {
        Emitter { position: Vec3::ZERO, radius: 1.0, direction: Vec3::Y, spread: 1.0, velocity: Vec3::ZERO,
            speed: [0.5, 3.0], count: 400, duration: 0.6, life: [1.5, 3.5], size: [0.045, 0.02],
            color: [Vec4::new(1.0, 0.55, 0.15, 1.0), Vec4::new(0.8, 0.12, 0.02, 0.0)], glow: 12_000.0, additive: 1.0,
            gravity: -1.0, drag: 1.2, turbulence: 3.0, bounce: 0.05, stretch: 0.0, shape: Shape::Ember }
    }
    /// Burning chunks thrown out of a blast: they arc, fall, bounce and roll,
    /// trailing fire, and glow on where they come to rest as they cool.
    pub fn burning() -> Self {
        Emitter { position: Vec3::ZERO, radius: 0.4, direction: Vec3::Y, spread: 1.2, velocity: Vec3::ZERO,
            speed: [3.0, 14.0], count: 300, duration: 0.0, life: [2.0, 5.0], size: [0.07, 0.035],
            color: [Vec4::new(1.0, 0.55, 0.12, 1.0), Vec4::new(0.6, 0.08, 0.01, 0.0)], glow: 30_000.0, additive: 1.0,
            gravity: 9.8, drag: 0.35, turbulence: 0.6, bounce: 0.35, stretch: 0.04, shape: Shape::Trail }
    }
    /// Lit grit and dust thrown out low that hangs and settles.
    pub fn dust() -> Self {
        Emitter { position: Vec3::ZERO, radius: 0.8, direction: Vec3::Y, spread: 1.3, velocity: Vec3::ZERO,
            speed: [1.0, 6.0], count: 600, duration: 0.0, life: [1.5, 3.0], size: [0.05, 0.12],
            color: [Vec4::new(0.45, 0.4, 0.34, 0.5), Vec4::new(0.45, 0.42, 0.38, 0.0)], glow: 0.0, additive: 0.0,
            gravity: 2.0, drag: 2.5, turbulence: 2.0, bounce: 0.0, stretch: 0.0, shape: Shape::Soft }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct Command {
    origin_radius: [f32; 4],
    dir_cos: [f32; 4],
    vel_duration: [f32; 4],
    speed_life: [f32; 4],
    color0: [f32; 4],
    color1: [f32; 4],
    size_glow: [f32; 4],
    phys: [f32; 4],
    extra: [f32; 4],
    slots: [u32; 4],
}

/// One particle on the GPU (64 bytes; layout in gpu_particles_sim.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Particle {
    pub pos_age: [f32; 4],
    pub vel_life: [f32; 4],
    pub look: [u32; 4],
    pub phys: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct SimUniform { a: [f32; 4], b: [f32; 4], c: [u32; 4], g: [f32; 4], gd: [u32; 4] }

/// Where particles land besides the floor: the top of each column of a
/// grid in x and z. A particle that runs into a column from below its top
/// bounces off its side.
#[derive(Clone, Debug, PartialEq)]
pub struct Ground {
    /// The grid's corner (x, z).
    pub origin: [f32; 2],
    pub cell: f32,
    /// Cells along x and z; heights are x fastest.
    pub dims: [u32; 2],
    pub heights: Vec<f32>,
}

impl Ground {
    /// The top of every column of solid voxels (far below where empty).
    pub fn from_occupancy(o: &crate::debris::Occupancy) -> Self {
        let size = Vec3::from(o.dims.map(|d| d as f32)) * o.voxel;
        Self::from_occupancy_area(o, glam::Vec2::new(o.origin.x, o.origin.z), glam::Vec2::new(o.origin.x + size.x, o.origin.z + size.z))
    }

    /// The same for the columns over `min..max` (x, z) only.
    pub fn from_occupancy_area(o: &crate::debris::Occupancy, min: glam::Vec2, max: glam::Vec2) -> Self {
        let lo = ((min - glam::Vec2::new(o.origin.x, o.origin.z)) / o.voxel).floor().max(glam::Vec2::ZERO);
        let origin = glam::Vec2::new(o.origin.x, o.origin.z) + lo * o.voxel;
        let dims = ((max - origin) / o.voxel).ceil().max(glam::Vec2::ONE).as_uvec2();
        let mut g = Ground { origin: origin.to_array(), cell: o.voxel, dims: dims.to_array(), heights: vec![-1e9; (dims.x * dims.y) as usize] };
        g.refresh(o, min, max);
        g
    }

    /// Recompute the columns over `min..max` (after pieces broke there).
    /// The ground must be on the occupancy's grid (`from_occupancy*`).
    pub fn refresh(&mut self, o: &crate::debris::Occupancy, min: glam::Vec2, max: glam::Vec2) {
        let [dx, dy, dz] = o.dims;
        let off = ((glam::Vec2::from(self.origin) - glam::Vec2::new(o.origin.x, o.origin.z)) / o.voxel).round().as_ivec2();
        let a = ((min - glam::Vec2::from(self.origin)) / self.cell).floor().as_ivec2().max(glam::IVec2::ZERO);
        let b = ((max - glam::Vec2::from(self.origin)) / self.cell).ceil().as_ivec2().min(glam::UVec2::from(self.dims).as_ivec2());
        for gz in a.y..b.y {
            for gx in a.x..b.x {
                let (x, z) = (gx + off.x, gz + off.y);
                let mut top = -1e9;
                if x >= 0 && z >= 0 && (x as u32) < dx && (z as u32) < dz {
                    for y in (0..dy).rev() {
                        let i = (x as u32 + y * dx + z as u32 * dx * dy) as usize;
                        if o.bits[i >> 5] >> (i & 31) & 1 == 1 { top = o.origin.y + (y + 1) as f32 * o.voxel; break; }
                    }
                }
                self.heights[(gx as u32 + gz as u32 * self.dims[0]) as usize] = top;
            }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewUniform {
    output_vp: [[f32; 4]; 4], depth_vp: [[f32; 4]; 4], camera: [f32; 4], right: [f32; 4], up: [f32; 4],
    sun_dir: [f32; 4], sun: [f32; 4], ambient: [f32; 4], size: [f32; 4],
}

/// What one frame asks of the renderer (`GpuParticles::flush`).
#[derive(Clone, Debug, Default)]
pub struct Update {
    pub capacity: u32,
    /// Emitter births this frame, already given their ring slots.
    commands: Vec<Command>,
    steps: u32,
    /// Seconds to add to every age without moving (skipped steps).
    skip: f32,
    /// Seconds from the last simulated step to the frame's time.
    frac: f32,
    sim_time: f32,
    floor: f32,
    friction: f32,
    wind: Vec3,
    min_pixels: f32,
    /// A new ground (Some(None): none any more).
    ground: Option<Option<Ground>>,
    reset: bool,
    /// Particles may be alive (simulate and draw).
    active: bool,
}

/// The game's particle pool: emitters in, one flush per frame.
pub struct GpuParticles {
    capacity: u32,
    enabled: bool,
    cursor: u32,
    seed: u32,
    pending: Vec<Command>,
    /// The time of the last simulated step, once started.
    sim_time: Option<f32>,
    /// When the last particle dies.
    last_death: f32,
    floor: f32,
    friction: f32,
    wind: Vec3,
    ground: Option<Option<Ground>>,
    /// Narrowest a particle is drawn, in pixels.
    pub min_pixels: f32,
    reset: bool,
    /// Births emitted (diagnostics).
    pub born: u64,
}

impl GpuParticles {
    /// Whether `KERN_GPU_PARTICLES=1` turns GPU particles on.
    pub fn switched_on() -> bool { crate::switches::GPU_PARTICLES.on() }

    /// A pool of `capacity` particles (the ring: the oldest give way),
    /// live only with `KERN_GPU_PARTICLES=1` while the system is opt-in.
    pub fn new(capacity: u32) -> Self { Self::forced(capacity, Self::switched_on()) }

    /// A pool that is on or off regardless of the switch (tests, tools).
    pub fn forced(capacity: u32, enabled: bool) -> Self {
        GpuParticles { capacity: capacity.clamp(64, MAX_CAPACITY), enabled, cursor: 0, seed: 0x2545_f491, pending: Vec::new(), sim_time: None,
            last_death: f32::NEG_INFINITY, floor: -1e9, friction: 3.0, wind: Vec3::ZERO, ground: None, min_pixels: 1.0, reset: true, born: 0 }
    }

    pub fn enabled(&self) -> bool { self.enabled }
    pub fn capacity(&self) -> u32 { self.capacity }

    /// Particles bounce on (or die at) a horizontal floor at height `y`;
    /// `friction` per second slows what slides on it.
    pub fn set_floor(&mut self, y: f32) { self.floor = y; }
    pub fn set_friction(&mut self, friction: f32) { self.friction = friction.max(0.0); }
    /// Particles with drag drift towards this velocity.
    pub fn set_wind(&mut self, wind: Vec3) { self.wind = wind; }
    /// Columns to land on and bounce off (uploaded with the next flush).
    pub fn set_ground(&mut self, ground: Option<Ground>) {
        if !self.enabled { return; }
        self.ground = Some(ground.filter(|g| g.cell > 0.0 && g.dims[0] > 0 && g.dims[1] > 0 && g.heights.len() == (g.dims[0] * g.dims[1]) as usize));
    }

    /// Whether any particle may still be alive at `time`.
    pub fn busy(&self, time: f32) -> bool { !self.pending.is_empty() || time < self.last_death }

    /// Emit `e` at game time `time` (seconds, the same clock as `flush`).
    pub fn emit(&mut self, time: f32, e: &Emitter) {
        if !self.enabled || e.count == 0 || !e.position.is_finite() || !time.is_finite() { return; }
        let count = e.count.min(self.capacity);
        let first = self.cursor;
        self.cursor = (self.cursor + count) % self.capacity;
        self.seed = self.seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let life = [e.life[0].max(0.001), e.life[1].max(e.life[0]).max(0.001)];
        self.last_death = self.last_death.max(time + e.duration.max(0.0) + life[1] + STEP);
        self.born += count as u64;
        let dir = e.direction.normalize_or(Vec3::Y);
        self.pending.push(Command {
            origin_radius: e.position.extend(e.radius.max(0.0)).to_array(),
            dir_cos: dir.extend(e.spread.clamp(0.0, std::f32::consts::PI).cos()).to_array(),
            vel_duration: e.velocity.extend(e.duration.max(0.0)).to_array(),
            speed_life: [e.speed[0], e.speed[1], life[0], life[1]],
            color0: e.color[0].to_array(), color1: e.color[1].to_array(),
            size_glow: [e.size[0].max(0.0), e.size[1].max(0.0), e.glow.max(0.0), e.additive.clamp(0.0, 1.0)],
            phys: [e.gravity, e.drag.max(0.0), e.bounce, e.turbulence.max(0.0)],
            // Age at the first step is filled in at flush, from the birth time.
            extra: [e.stretch.max(0.0), e.shape as u32 as f32, time, 0.0],
            slots: [first, count, self.seed, 0],
        });
        if self.pending.len() > MAX_EMITTERS { self.pending.remove(0); }
    }

    /// Forget every particle (a new round, a replay seek).
    pub fn clear(&mut self) {
        self.pending.clear();
        self.sim_time = None;
        self.last_death = f32::NEG_INFINITY;
        self.reset = true;
    }

    /// This frame's update for game time `time` (moves the clock).
    pub fn prepare(&mut self, time: f32) -> Update {
        let start = *self.sim_time.get_or_insert(time);
        let (start, steps, skip) = if time < start {
            // The clock went back: start over at the new time.
            self.clear();
            self.sim_time = Some(time);
            (time, 0, 0.0)
        } else {
            let due = ((time - start) / STEP).floor() as u32;
            let steps = due.min(MAX_STEPS);
            let skip = (due - steps) as f32 * STEP;
            (start, steps, skip)
        };
        let after = start + skip + steps as f32 * STEP;
        self.sim_time = Some(after);
        let mut commands = std::mem::take(&mut self.pending);
        let mut prefix = 0;
        for c in &mut commands {
            // Its age at the first step this frame (skipped time included in skip).
            c.extra[2] = start - c.extra[2];
            c.slots[3] = prefix;
            prefix += c.slots[1];
        }
        Update { capacity: self.capacity, commands, steps, skip, frac: (time - after).max(0.0), sim_time: start,
            floor: self.floor, friction: self.friction, wind: self.wind, min_pixels: self.min_pixels, ground: self.ground.take(),
            reset: std::mem::take(&mut self.reset), active: self.enabled && time < self.last_death }
    }

    /// Send this frame's emitters and clock (once per frame).
    pub fn flush(&mut self, gpu: &Gpu, renderer: &crate::Renderer, time: f32) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        let update = self.prepare(time);
        renderer.update_gpu_particles(gpu, update)
    }
}

pub fn sim_source() -> &'static str { include_str!("shaders/gpu_particles_sim.wgsl") }
pub fn draw_source() -> &'static str { include_str!("shaders/gpu_particles_draw.wgsl") }

struct Target { size: (u32, u32), output: Option<wgpu::Texture>, uniform: wgpu::Buffer }

/// Renderer side: buffers, pipelines and this frame's work.
pub(crate) struct ParticleRenderer {
    capacity: u32,
    spawn: wgpu::ComputePipeline,
    simulate: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
    sim_bind: wgpu::BindGroup,
    ground_layout: wgpu::BindGroupLayout,
    ground_bind: wgpu::BindGroup,
    heights: wgpu::Buffer,
    /// Origin x, z, cell, on; cells along x and z.
    ground: ([f32; 4], [u32; 4]),
    draw_layout: wgpu::BindGroupLayout,
    sim_uniform: wgpu::Buffer,
    particles: wgpu::Buffer,
    commands: wgpu::Buffer,
    alive: wgpu::Buffer,
    args: wgpu::Buffer,
    pending: Option<Update>,
    /// The last update simulated something that may still be drawn.
    active: bool,
    frac: f32,
    min_pixels: f32,
    targets: Vec<Option<Target>>,
}

impl ParticleRenderer {
    pub fn new(gpu: &Gpu, capacity: u32) -> Self {
        let capacity = capacity.clamp(64, MAX_CAPACITY);
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let buf = |ty| wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None };
        let st = |read_only| buf(wgpu::BufferBindingType::Storage { read_only });
        let c = wgpu::ShaderStages::COMPUTE;
        let sim_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("gpu particles sim"), entries: &[
            entry(0, c, buf(wgpu::BufferBindingType::Uniform)), entry(1, c, st(false)), entry(2, c, st(true)), entry(3, c, st(false)), entry(4, c, st(false))] });
        let ground_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("gpu particles ground"), entries: &[entry(0, c, st(true))] });
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let draw_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("gpu particles draw"), entries: &[
            entry(0, vf, buf(wgpu::BufferBindingType::Uniform)), entry(1, wgpu::ShaderStages::VERTEX, st(true)), entry(2, wgpu::ShaderStages::VERTEX, st(true)),
            entry(3, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false })] });
        let sim_module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gpu particles sim"), source: wgpu::ShaderSource::Wgsl(sim_source().into()) });
        let draw_module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gpu particles draw"), source: wgpu::ShaderSource::Wgsl(draw_source().into()) });
        let sim_pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gpu particles sim"), bind_group_layouts: &[Some(&sim_layout), Some(&ground_layout)], immediate_size: 0 });
        let draw_pl = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gpu particles draw"), bind_group_layouts: &[Some(&draw_layout)], immediate_size: 0 });
        let pipe = |entry: &str| gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&sim_pl),
            module: &sim_module, entry_point: Some(entry), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let (spawn, simulate) = (pipe("spawn"), pipe("simulate"));
        let draw = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor { label: Some("gpu particles"), layout: Some(&draw_pl),
            vertex: wgpu::VertexState { module: &draw_module, entry_point: Some("particle_vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() }, depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &draw_module, entry_point: Some("particle_fs"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba16Float, blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: gpu.pipeline_cache() });
        let buffer = |label, size: u64, usage| gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16), usage, mapped_at_creation: false });
        let rw = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let particles = buffer("gpu particles", capacity as u64 * std::mem::size_of::<Particle>() as u64, rw | wgpu::BufferUsages::COPY_SRC);
        let commands = buffer("gpu particle emitters", (MAX_EMITTERS * std::mem::size_of::<Command>()) as u64, rw);
        let alive = buffer("gpu particles alive", capacity as u64 * 4, rw);
        let args = buffer("gpu particles draw args", 16, rw | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC);
        let sim_uniform = buffer("gpu particles sim", std::mem::size_of::<SimUniform>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        gpu.queue.write_buffer(&args, 0, bytemuck::cast_slice(&[6u32, 0, 0, 0]));
        let sim_bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("gpu particles sim"), layout: &sim_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: sim_uniform.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: particles.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: commands.as_entire_binding() }, wgpu::BindGroupEntry { binding: 3, resource: alive.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: args.as_entire_binding() }] });
        let heights = buffer("gpu particles ground", 16, rw);
        let ground_bind = Self::ground_bind(gpu, &ground_layout, &heights);
        ParticleRenderer { capacity, spawn, simulate, draw, sim_bind, ground_layout, ground_bind, heights, ground: ([0.0; 4], [0; 4]), draw_layout, sim_uniform, particles, commands, alive, args, pending: None,
            active: false, frac: 0.0, min_pixels: 1.0, targets: Vec::new() }
    }

    pub fn capacity(&self) -> u32 { self.capacity }

    fn ground_bind(gpu: &Gpu, layout: &wgpu::BindGroupLayout, heights: &wgpu::Buffer) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("gpu particles ground"), layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: heights.as_entire_binding() }] })
    }

    fn set_ground(&mut self, gpu: &Gpu, ground: Option<&Ground>) {
        let Some(g) = ground else { self.ground.0[3] = 0.0; return };
        let bytes = (g.heights.len() * 4) as u64;
        if self.heights.size() < bytes {
            self.heights = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particles ground"), size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            self.ground_bind = Self::ground_bind(gpu, &self.ground_layout, &self.heights);
        }
        gpu.queue.write_buffer(&self.heights, 0, bytemuck::cast_slice(&g.heights));
        self.ground = ([g.origin[0], g.origin[1], g.cell, 1.0], [g.dims[0], g.dims[1], 0, 0]);
    }

    /// Queue a frame's update; several before one frame merge (births kept, steps summed).
    pub fn update(&mut self, gpu: &Gpu, mut u: Update) {
        if let Some(ground) = u.ground.take() { self.set_ground(gpu, ground.as_ref()); }
        if let Some(old) = self.pending.take() {
            let mut commands = old.commands;
            let base = commands.last().map_or(0, |c| c.slots[3] + c.slots[1]);
            for c in &mut u.commands { c.slots[3] += base; }
            commands.append(&mut u.commands);
            u.commands = commands;
            u.steps += old.steps;
            u.skip += old.skip;
            u.sim_time = old.sim_time;
            u.reset |= old.reset;
        }
        if u.steps > MAX_STEPS { u.skip += (u.steps - MAX_STEPS) as f32 * STEP; u.steps = MAX_STEPS; }
        if u.commands.len() > MAX_EMITTERS {
            let drop = u.commands.len() - MAX_EMITTERS;
            u.commands.drain(..drop);
            let base = u.commands[0].slots[3];
            for c in &mut u.commands { c.slots[3] -= base; }
        }
        self.pending = Some(u);
    }

    /// Record spawn and simulation. Returns whether it ran.
    pub fn encode(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, timestamp: Option<(&wgpu::QuerySet, u32)>) -> bool {
        let Some(u) = self.pending.take() else { return false };
        if u.reset { enc.clear_buffer(&self.particles, 0, None); }
        self.frac = u.frac;
        self.min_pixels = u.min_pixels;
        // Clear the live count even when idle, so a stale list is never drawn.
        enc.clear_buffer(&self.args, 4, Some(4));
        self.active = u.active;
        if !u.active && u.commands.is_empty() { return false; }
        let births: u32 = u.commands.last().map_or(0, |c| c.slots[3] + c.slots[1]);
        let sim = SimUniform { a: [STEP, u.floor, u.friction, u.skip], b: [u.wind.x, u.wind.y, u.wind.z, u.sim_time],
            c: [u.steps, self.capacity, u.commands.len() as u32, births], g: self.ground.0, gd: self.ground.1 };
        gpu.queue.write_buffer(&self.sim_uniform, 0, bytemuck::bytes_of(&sim));
        if !u.commands.is_empty() { gpu.queue.write_buffer(&self.commands, 0, bytemuck::cast_slice(&u.commands)); }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gpu particles"), timestamp_writes: timestamp.map(|(query_set, start)|
            wgpu::ComputePassTimestampWrites { query_set, beginning_of_pass_write_index: Some(start), end_of_pass_write_index: Some(start + 1) }) });
        pass.set_bind_group(0, &self.sim_bind, &[]);
        pass.set_bind_group(1, &self.ground_bind, &[]);
        if births > 0 {
            pass.set_pipeline(&self.spawn);
            pass.dispatch_workgroups(births.div_ceil(64), 1, 1);
        }
        pass.set_pipeline(&self.simulate);
        pass.dispatch_workgroups(self.capacity.div_ceil(64), 1, 1);
        true
    }

    /// Draw the live particles over `target` for view `index`. With
    /// `reuse_output` the target itself is drawn into (a disposable
    /// post-temporal output); otherwise into a copy.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder, index: usize, target: &wgpu::Texture, reuse_output: bool,
        depth: &wgpu::TextureView, camera: Camera, output_vp: Mat4, depth_vp: Mat4, light: &Lighting) -> Result<Option<wgpu::Texture>, String> {
        if !self.active { return Ok(None); }
        if target.format() != wgpu::TextureFormat::Rgba16Float { return Err("gpu particles require HDR input".into()); }
        let size = (target.width(), target.height());
        if self.targets.len() <= index { self.targets.resize_with(index + 1, || None); }
        if self.targets[index].as_ref().is_none_or(|t| t.size != size) {
            self.targets[index] = Some(Target { size, output: None, uniform: gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particles view"),
                size: std::mem::size_of::<ViewUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }) });
        }
        let t = self.targets[index].as_mut().unwrap();
        let reuse = reuse_output && target.usage().contains(wgpu::TextureUsages::RENDER_ATTACHMENT);
        if !reuse && t.output.is_none() {
            t.output = Some(gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("gpu particle output, never temporal history"), size: target.size(),
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: target.format(),
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT, view_formats: &[] }));
        }
        let forward = (camera.target - camera.pos).normalize_or(Vec3::NEG_Z);
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(forward);
        let per_metre = self.min_pixels * 2.0 * (camera.fov_y * 0.5).tan() / size.1.max(1) as f32;
        let u = ViewUniform { output_vp: output_vp.to_cols_array_2d(), depth_vp: depth_vp.to_cols_array_2d(), camera: camera.pos.extend(camera.near).to_array(),
            right: right.extend(self.frac).to_array(), up: up.extend(per_metre).to_array(), sun_dir: light.sun_dir.extend(0.0).to_array(),
            sun: (light.sun_color * light.sun_lux.max(0.0)).extend(0.0).to_array(), ambient: (Vec3::new(0.78, 0.86, 1.0) * light.sky_lux.max(0.0)).extend(0.0).to_array(),
            size: [size.0 as f32, size.1 as f32, 1.0 / size.0 as f32, 1.0 / size.1 as f32] };
        gpu.queue.write_buffer(&t.uniform, 0, bytemuck::bytes_of(&u));
        let output = if reuse { target } else {
            let o = t.output.as_ref().unwrap();
            enc.copy_texture_to_texture(target.as_image_copy(), o.as_image_copy(), target.size());
            o
        };
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("gpu particles view"), layout: &self.draw_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: t.uniform.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: self.particles.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: self.alive.as_entire_binding() }, wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(depth) }] });
        let view = output.create_view(&Default::default());
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("gpu particles"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view,
            depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        pass.set_pipeline(&self.draw);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw_indirect(&self.args, 0);
        drop(pass);
        Ok(Some(output.clone()))
    }

    /// Explicit validation only: every particle and the live count.
    pub fn read_back(&self, gpu: &Gpu) -> Result<(Vec<Particle>, u32), String> {
        let bytes = self.capacity as u64 * std::mem::size_of::<Particle>() as u64;
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu particles readback"), size: bytes + 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.particles, 0, &read, 0, bytes);
        enc.copy_buffer_to_buffer(&self.args, 0, &read, bytes, 16);
        gpu.queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        read.slice(..).map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("{e:?}"))?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| format!("{e:?}"))?;
        let view = read.slice(..).get_mapped_range().map_err(|e| format!("{e:?}"))?;
        let particles = bytemuck::pod_collect_to_vec(&view[..bytes as usize]);
        let args: [u32; 4] = bytemuck::pod_read_unaligned(&view[bytes as usize..bytes as usize + 16]);
        Ok((particles, args[1]))
    }
}

/// The renderer's pool, made on the first update.
#[derive(Default)]
pub(crate) struct ParticleSlot(pub Mutex<Option<ParticleRenderer>>);

impl ParticleSlot {
    pub fn update(&self, gpu: &Gpu, u: Update) -> Result<(), String> {
        let mut g = self.0.lock().unwrap();
        if g.is_none() && !u.active && u.commands.is_empty() { return Ok(()); }
        if g.as_ref().is_some_and(|r| r.capacity() != u.capacity.clamp(64, MAX_CAPACITY)) {
            return Err("gpu particle capacity changed; one pool per renderer".into());
        }
        g.get_or_insert_with(|| ParticleRenderer::new(gpu, u.capacity)).update(gpu, u);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage_buffers(source: &str) -> usize {
        let m = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap();
        m.global_variables.iter().filter(|(_, v)| matches!(v.space, naga::AddressSpace::Storage { .. })).count()
    }

    #[test]
    fn shaders_validate_and_fit_webgpu_minimum() {
        assert_eq!(std::mem::size_of::<Particle>(), 64);
        assert_eq!(std::mem::size_of::<Command>(), 160);
        assert_eq!(std::mem::size_of::<SimUniform>(), 80);
        assert_eq!(std::mem::size_of::<ViewUniform>(), 240);
        // The WebGPU spec guarantees 8 storage buffers per stage.
        assert!(storage_buffers(sim_source()) <= 5);
        assert!(storage_buffers(draw_source()) <= 2);
    }

    #[test]
    fn ground_follows_occupancy_columns() {
        let mut o = crate::debris::Occupancy::new(Vec3::new(-4.0, 0.0, -4.0), Vec3::new(4.0, 4.0, 4.0), 0.5);
        o.fill_box(Vec3::new(1.0, 1.0, 1.0), Vec3::new(0.5, 1.0, 0.5), 0.0, true);
        let mut g = Ground::from_occupancy_area(&o, glam::Vec2::new(-2.0, -2.0), glam::Vec2::new(2.0, 2.0));
        assert_eq!((g.origin, g.dims), ([-2.0, -2.0], [8, 8]));
        let at = |g: &Ground, x: f32, z: f32| g.heights[(((x - g.origin[0]) / g.cell) as u32 + ((z - g.origin[1]) / g.cell) as u32 * g.dims[0]) as usize];
        assert_eq!(at(&g, 1.1, 1.1), 2.0);
        assert!(at(&g, -1.0, -1.0) < -1e8);
        o.fill_box(Vec3::new(1.0, 1.0, 1.0), Vec3::new(0.5, 1.0, 0.5), 0.0, false);
        g.refresh(&o, glam::Vec2::new(0.0, 0.0), glam::Vec2::new(2.0, 2.0));
        assert!(at(&g, 1.1, 1.1) < -1e8);
        assert_eq!(g, Ground::from_occupancy_area(&o, glam::Vec2::new(-2.0, -2.0), glam::Vec2::new(2.0, 2.0)));
    }

    #[test]
    fn disabled_pool_ignores_emitters() {
        let mut p = GpuParticles::forced(1024, false);
        p.emit(0.0, &Emitter::sparks());
        assert!(!p.busy(0.0));
        assert_eq!(p.born, 0);
    }

    #[test]
    fn clock_steps_skips_and_ring_wraps() {
        let mut p = GpuParticles::forced(1000, true);
        let u = p.prepare(1.0);
        assert_eq!((u.steps, u.reset), (0, true));
        p.emit(1.0, &Emitter { count: 600, ..Emitter::sparks() });
        p.emit(1.01, &Emitter { count: 600, ..Emitter::sparks() });
        let u = p.prepare(1.0 + 2.5 * STEP);
        assert_eq!(u.steps, 2);
        assert!((u.frac - 0.5 * STEP).abs() < 1e-4);
        assert_eq!(u.commands[0].slots, [0, 600, u.commands[0].slots[2], 0]);
        assert_eq!(u.commands[1].slots[0], 600);
        assert_eq!(u.commands[1].slots[3], 600);
        assert!(u.commands[1].extra[2] < 0.0, "born after the first step: negative age");
        assert!(u.active);
        // A long stall ages without simulating more than MAX_STEPS.
        let u = p.prepare(3.0);
        assert_eq!(u.steps, MAX_STEPS);
        assert!(u.skip > 0.8);
        // Long after the last death nothing runs.
        assert!(!p.prepare(10.0).active);
        // The clock going back starts over.
        assert!(p.prepare(5.0).reset);
    }

    #[test]
    fn gpu_spawns_simulates_bounces_and_expires() {
        let gpu = Gpu::new().unwrap();
        let mut r = ParticleRenderer::new(&gpu, 4096);
        let mut p = GpuParticles::forced(4096, true);
        p.set_floor(0.0);
        // A 1 m high block under the embers' launch point x in [4, 6].
        let mut heights = vec![-1e9; 20 * 20];
        for z in 0..20 { for x in 14..16 { heights[x + z * 20] = 1.0; } }
        p.set_ground(Some(Ground { origin: [-10.0, -10.0], cell: 1.0, dims: [20, 20], heights }));
        let run = |p: &mut GpuParticles, r: &mut ParticleRenderer, t: f32| {
            let u = p.prepare(t);
            r.update(&gpu, u);
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            r.encode(&gpu, &mut enc, None);
            gpu.queue.submit([enc.finish()]);
            r.read_back(&gpu).unwrap()
        };
        run(&mut p, &mut r, 0.0);
        p.emit(0.0, &Emitter { position: Vec3::new(0.0, 1.0, 0.0), direction: Vec3::NEG_Y, spread: 1.2, speed: [2.0, 6.0], count: 3000, life: [2.0, 2.0], ..Emitter::sparks() });
        p.emit(0.0, &Emitter { position: Vec3::new(5.0, 1.0, 0.0), count: 500, duration: 0.5, life: [2.0, 2.0], bounce: -1.0, gravity: -1.2, turbulence: 5.0, ..Emitter::embers() });
        let (parts, live) = run(&mut p, &mut r, 0.05);
        assert!(live > 3000 && live < 3500, "sparks all born, embers partly ({live})");
        assert!(parts.iter().take(3000).all(|q| q.vel_life[3] == 2.0));
        for k in 1..10 { run(&mut p, &mut r, 0.05 + 0.1 * k as f32); }
        let (parts, live) = run(&mut p, &mut r, 1.05);
        assert!(live > 3000, "all embers born ({live})");
        assert!(parts.iter().filter(|q| q.vel_life[3] > 0.0).all(|q| q.pos_age[1] >= -1e-4), "nothing under the floor");
        let resting = parts[..3000].iter().filter(|q| q.pos_age[1].abs() < 1e-3).count();
        assert!(resting > 2000, "sparks land on the floor ({resting})");
        assert!(parts[3000..3500].iter().filter(|q| q.vel_life[3] > 0.0).all(|q| q.pos_age[1] > 0.0), "embers rise");
        assert!(parts[3000..3500].iter().filter(|q| q.vel_life[3] > 0.0 && (4.0..6.0).contains(&q.pos_age[0])).all(|q| q.pos_age[1] >= 1.0 - 1e-3), "none inside the block");
        assert!(parts[3000..3500].iter().any(|q| q.vel_life[3] > 0.0 && (4.0..6.0).contains(&q.pos_age[0])), "some over it");
        for k in 1..20 { run(&mut p, &mut r, 1.05 + 0.1 * k as f32); }
        let (_, live) = run(&mut p, &mut r, 3.1);
        assert_eq!(live, 0, "all expired");
        assert!(!p.busy(3.1));
    }
}
