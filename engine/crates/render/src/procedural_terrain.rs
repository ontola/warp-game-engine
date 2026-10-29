//! Procedural terrain: no heightmap, no voxels, no stored mesh. The ground
//! is a function (`terrain_field`), and every frame the cameras pick a
//! quadtree of patches (CDLOD) whose cells project to a few pixels. Each
//! patch is one 128-triangle cluster in a reserved pool; a compute pass
//! samples the function into a patch when it first appears, then the
//! normal cluster pipeline culls, rasters, shades and shadows it.
//!
//! ```ignore
//! let pool = procedural_terrain::reserve(&mut scene, 16384, terrain_material, seed, Style::default());
//! let renderer = Renderer::new(&gpu, &scene, &light);
//! let mut terrain = ProceduralTerrain::new(pool, Settings::default());
//! terrain.update(&gpu, &renderer, &[TerrainView::new(&camera, height_px)])?; // per frame
//! ```
use crate::cluster::{GpuCluster, Vertex};
use crate::scene::{GeometryCategory, GpuInstance, Scene, INSTANCE_CATEGORY_SHIFT};
use crate::{terrain_field, Camera, Gpu, Renderer};
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// Cells per patch side. 8x8 cells = 128 triangles = one cluster.
pub const CELLS: i32 = 8;
const VERTS: u32 = 81;
/// Instance flag: geometry rewritten at runtime (a dynamic shadow caster).
pub const PROCEDURAL: u32 = 128;

const SHADER: &str = include_str!("shaders/procedural_terrain.wgsl");

/// The reserved pool, made by [`reserve`] before the renderer.
#[derive(Clone, Copy, Debug)]
pub struct TerrainSpec {
    pub first_instance: u32,
    pub first_cluster: u32,
    pub first_vertex: u32,
    pub capacity: u32,
    pub seed: u32,
    pub style: terrain_field::Style,
}

/// Reserve `capacity` patch slots that draw with `material`.
pub fn reserve(scene: &mut Scene, capacity: u32, material: u32, seed: u32, style: terrain_field::Style) -> TerrainSpec {
    assert!(scene.procedural_terrain.is_none(), "one procedural terrain per scene");
    let first_vertex = scene.vertices.len() as u32;
    let first_tri = scene.tris.len() as u32;
    let first_cluster = scene.clusters.len() as u32;
    let first_instance = scene.instances.len() as u32;
    scene.vertices.resize(scene.vertices.len() + (capacity * VERTS) as usize, Vertex::zeroed());
    // One shared topology; diagonals alternate so slopes do not all lean one way.
    let at = |i: i32, j: i32| (j * (CELLS + 1) + i) as u32;
    for j in 0..CELLS {
        for i in 0..CELLS {
            let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
            let pack = |x: u32, y: u32, z: u32| x | y << 8 | z << 16;
            if (i + j) & 1 == 0 {
                scene.tris.push(pack(a, c, b));
                scene.tris.push(pack(b, c, d));
            } else {
                scene.tris.push(pack(a, c, d));
                scene.tris.push(pack(a, d, b));
            }
        }
    }
    let hidden = [0.0, -1.0e7, 0.0, 0.0];
    for slot in 0..capacity {
        let first_vert = scene.cluster_verts.len() as u32;
        scene.cluster_verts.extend((0..VERTS).map(|k| first_vertex + slot * VERTS + k));
        scene.clusters.push(GpuCluster {
            cull: hidden, parent: hidden, own: hidden,
            parent_error: f32::MAX, own_error: 0.0,
            first_tri, tri_count: (CELLS * CELLS * 2) as u32, first_vert, material, depth: 0, _pad: 0,
        });
        scene.instances.push(GpuInstance {
            rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            sphere: hidden,
            first_cluster: first_cluster + slot,
            cluster_count: 0,
            scale: 1.0,
            flags: PROCEDURAL | (GeometryCategory::Terrain as u32) << INSTANCE_CATEGORY_SHIFT,
        });
    }
    let spec = TerrainSpec { first_instance, first_cluster, first_vertex, capacity, seed, style };
    scene.procedural_terrain = Some(spec);
    spec
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Largest projected length of one cell, in pixels.
    pub cell_pixels: f32,
    /// Largest projected geometric error, in pixels.
    pub error_pixels: f32,
    /// Patches farther than this from every camera are not drawn (metres).
    pub view_distance: f32,
    /// Cell size of the finest patches (metres, a power of two).
    pub finest: f32,
    /// Level of the root patches (cell = finest * 2^level).
    pub root_level: u32,
    /// Cell target multiplier for patches outside every view.
    pub offscreen_factor: f32,
}

impl Default for Settings {
    fn default() -> Self { Settings { cell_pixels: 24.0, error_pixels: 1.5, view_distance: 20000.0, finest: 1.0 / 16.0, root_level: 13, offscreen_factor: 8.0 } }
}

/// One camera as the LOD sees it.
#[derive(Clone, Copy, Debug)]
pub struct TerrainView {
    pub pos: Vec3,
    /// Pixels per radian at the screen centre: (height / 2) / tan(fov / 2).
    pub pixels_per_radian: f32,
    /// Unit view direction.
    pub forward: Vec3,
    /// Cosine of the half angle of a cone around `forward` that holds the
    /// whole view (its corners included).
    pub cos_half_cone: f32,
}

impl TerrainView {
    pub fn new(camera: &Camera, width_px: u32, height_px: u32) -> Self {
        let tan_y = (camera.fov_y * 0.5).tan();
        let tan_x = tan_y * width_px as f32 / height_px.max(1) as f32;
        let corner = (tan_x * tan_x + tan_y * tan_y).sqrt();
        TerrainView { pos: camera.pos, pixels_per_radian: height_px as f32 * 0.5 / tan_y,
            forward: (camera.target - camera.pos).normalize_or(Vec3::NEG_Z), cos_half_cone: 1.0 / (1.0 + corner * corner).sqrt() }
    }

    /// Whether a sphere can be inside the view cone.
    fn sees(&self, centre: Vec3, radius: f32) -> bool {
        let to = centre - self.pos;
        let d = to.length();
        if d <= radius { return true; }
        // Cone/sphere test: angle to the centre minus the sphere's angular size.
        let cos_a = to.dot(self.forward) / d;
        let sin_r = (radius / d).min(1.0);
        let cos_r = (1.0 - sin_r * sin_r).sqrt();
        let sin_c = (1.0 - self.cos_half_cone * self.cos_half_cone).max(0.0).sqrt();
        // cos(cone + r) <= cos_a
        cos_a >= self.cos_half_cone * cos_r - sin_c * sin_r
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Job {
    origin: [f32; 2],
    spacing: f32,
    mask: u32,
    slot: u32,
    _pad: [u32; 3],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub patches: usize,
    pub built: usize,
    pub freed: usize,
    /// Patches dropped because the pool was full.
    pub overflow: usize,
    pub finest_level: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Node { level: u32, ix: i32, iz: i32 }

impl Node {
    fn parent(self) -> Node { Node { level: self.level + 1, ix: self.ix >> 1, iz: self.iz >> 1 } }
    fn children(self) -> [Node; 4] {
        let (l, x, z) = (self.level - 1, self.ix * 2, self.iz * 2);
        [Node { level: l, ix: x, iz: z }, Node { level: l, ix: x + 1, iz: z }, Node { level: l, ix: x, iz: z + 1 }, Node { level: l, ix: x + 1, iz: z + 1 }]
    }
}

/// The CPU half: patch selection and slot bookkeeping.
pub struct ProceduralTerrain {
    pub spec: TerrainSpec,
    pub settings: Settings,
    slots: HashMap<(Node, u32), u32>,
    free: Vec<u32>,
}

impl ProceduralTerrain {
    pub fn new(spec: TerrainSpec, settings: Settings) -> Self {
        ProceduralTerrain { spec, settings, slots: HashMap::new(), free: (0..spec.capacity).rev().collect() }
    }

    fn cell(&self, level: u32) -> f32 { self.settings.finest * (1u32 << level) as f32 }
    fn size(&self, level: u32) -> f32 { self.cell(level) * CELLS as f32 }

    fn wants_split(&self, n: Node, views: &[TerrainView]) -> bool {
        if n.level == 0 { return false; }
        let size = self.size(n.level);
        let cell = self.cell(n.level);
        let (x0, z0) = (n.ix as f32 * size, n.iz as f32 * size);
        let sample = terrain_field::eval(x0 + size * 0.5, z0 + size * 0.5, cell * CELLS as f32, self.spec.seed, &self.spec.style);
        let centre_y = sample.pos.y;
        let error = sample.error(cell);
        let reach = size * 0.75 + 8.0;
        let centre = Vec3::new(x0 + size * 0.5, centre_y, z0 + size * 0.5);
        let radius = (size * size * 0.5 + reach * reach).sqrt();
        views.iter().any(|v| {
            let dx = (v.pos.x - centre.x).abs() - size * 0.5;
            let dz = (v.pos.z - centre.z).abs() - size * 0.5;
            let dy = (v.pos.y - centre_y).abs() - reach;
            let d = (dx.max(0.0).powi(2) + dz.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt().max(0.05);
            // Out of view, patches only cast shadows: much coarser is fine.
            let scale = if v.sees(centre, radius) { 1.0 } else { self.settings.offscreen_factor };
            let px = v.pixels_per_radian / d;
            cell * px > self.settings.cell_pixels * scale || error * px > self.settings.error_pixels * scale
        })
    }

    /// Leaves of the patch quadtree for these views, balanced so that
    /// neighbours (diagonals too) differ by at most one level.
    fn select(&self, views: &[TerrainView]) -> HashSet<Node> {
        let root = self.settings.root_level;
        let rs = self.size(root);
        let vd = self.settings.view_distance;
        let mut roots = HashSet::new();
        for v in views {
            let (a, b) = (((v.pos.x - vd) / rs).floor() as i32, ((v.pos.x + vd) / rs).floor() as i32);
            let (c, d) = (((v.pos.z - vd) / rs).floor() as i32, ((v.pos.z + vd) / rs).floor() as i32);
            for ix in a..=b {
                for iz in c..=d {
                    let cx = ((ix as f32 + 0.5) * rs - v.pos.x).abs() - rs * 0.5;
                    let cz = ((iz as f32 + 0.5) * rs - v.pos.z).abs() - rs * 0.5;
                    if cx.max(0.0).hypot(cz.max(0.0)) <= vd { roots.insert(Node { level: root, ix, iz }); }
                }
            }
        }
        let mut leaves = HashSet::new();
        let mut stack: Vec<Node> = roots.into_iter().collect();
        while let Some(n) = stack.pop() {
            if self.wants_split(n, views) { stack.extend(n.children()); } else { leaves.insert(n); }
        }
        balance(&mut leaves, root);
        leaves
    }

    /// Choose this frame's patches, build the new ones on the GPU and hide
    /// the ones no longer needed. Call once per frame before rendering.
    pub fn update(&mut self, gpu: &Gpu, renderer: &Renderer, views: &[TerrainView]) -> Result<Stats, String> {
        let leaves = self.select(views);
        let mut stats = Stats { patches: leaves.len(), finest_level: u32::MAX, ..Default::default() };
        let wanted: HashMap<(Node, u32), ()> = leaves.iter().map(|&n| ((n, mask(&leaves, n, self.settings.root_level)), ())).collect();
        let mut changed: Vec<(u32, GpuInstance)> = Vec::new();
        self.slots.retain(|key, slot| {
            if wanted.contains_key(key) { return true; }
            self.free.push(*slot);
            changed.push((*slot, hidden_instance(&self.spec, *slot)));
            stats.freed += 1;
            false
        });
        let mut jobs = Vec::new();
        // Deterministic order: nearest-first would need sorting anyway.
        let mut new: Vec<_> = wanted.keys().filter(|k| !self.slots.contains_key(k)).copied().collect();
        new.sort_by_key(|(n, m)| (n.level, n.ix, n.iz, *m));
        for key in new {
            let Some(slot) = self.free.pop() else { stats.overflow += 1; continue; };
            let (n, m) = key;
            let size = self.size(n.level);
            let cell = self.cell(n.level);
            let origin = [n.ix as f32 * size, n.iz as f32 * size];
            jobs.push(Job { origin, spacing: cell, mask: m, slot, _pad: [0; 3] });
            let centre_y = terrain_field::eval(origin[0] + size * 0.5, origin[1] + size * 0.5, cell * CELLS as f32, self.spec.seed, &self.spec.style).pos.y;
            let half = size * 0.5;
            let radius = (2.0 * half * half).sqrt() + size + 8.0 + terrain_field::MAX_PUSH;
            let mut inst = hidden_instance(&self.spec, slot);
            inst.sphere = [origin[0] + half, centre_y, origin[1] + half, radius];
            inst.cluster_count = 1;
            changed.push((slot, inst));
            self.slots.insert(key, slot);
            stats.built += 1;
        }
        for n in &leaves { stats.finest_level = stats.finest_level.min(n.level); }
        changed.sort_by_key(|c| c.0);
        // Coalesce neighbouring slots into one upload each.
        let mut i = 0;
        while i < changed.len() {
            let mut j = i + 1;
            while j < changed.len() && changed[j].0 == changed[j - 1].0 + 1 { j += 1; }
            let run: Vec<GpuInstance> = changed[i..j].iter().map(|c| c.1).collect();
            renderer.write_procedural_instances(gpu, (self.spec.first_instance + changed[i].0) as usize, &run)?;
            i = j;
        }
        renderer.queue_procedural_terrain(jobs)?;
        Ok(stats)
    }
}

fn hidden_instance(spec: &TerrainSpec, slot: u32) -> GpuInstance {
    GpuInstance {
        rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
        sphere: [0.0, -1.0e7, 0.0, 0.0],
        first_cluster: spec.first_cluster + slot,
        cluster_count: 0,
        scale: 1.0,
        flags: PROCEDURAL | (GeometryCategory::Terrain as u32) << INSTANCE_CATEGORY_SHIFT,
    }
}

/// The leaf covering node `n`'s area at `n.level` or coarser, if any.
fn covering(leaves: &HashSet<Node>, mut n: Node, root: u32) -> Option<Node> {
    loop {
        if leaves.contains(&n) { return Some(n); }
        if n.level >= root { return None; }
        n = n.parent();
    }
}

fn balance(leaves: &mut HashSet<Node>, root: u32) {
    let mut queue: Vec<Node> = leaves.iter().copied().collect();
    queue.sort_by_key(|n| (n.level, n.ix, n.iz));
    while let Some(n) = queue.pop() {
        if !leaves.contains(&n) { continue; }
        for dz in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dz == 0 { continue; }
                let m = Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz };
                if let Some(c) = covering(leaves, m, root) {
                    if c.level > n.level + 1 {
                        leaves.remove(&c);
                        for k in c.children() { leaves.insert(k); queue.push(k); }
                        queue.push(n);
                    }
                }
            }
        }
    }
}

/// Edge (bits 0..3: -x, +x, -z, +z) and corner (bits 4..7) flags for
/// neighbours one level coarser.
fn mask(leaves: &HashSet<Node>, n: Node, root: u32) -> u32 {
    let coarser = |dx: i32, dz: i32| {
        covering(leaves, Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz }, root).is_some_and(|c| c.level > n.level)
    };
    let mut m = 0;
    if coarser(-1, 0) { m |= 1; }
    if coarser(1, 0) { m |= 2; }
    if coarser(0, -1) { m |= 4; }
    if coarser(0, 1) { m |= 8; }
    for (bit, (cx, cz)) in [(-1, -1), (1, -1), (-1, 1), (1, 1)].into_iter().enumerate() {
        if coarser(cx, 0) || coarser(0, cz) || coarser(cx, cz) { m |= 16 << bit; }
    }
    m
}

/// The GPU half, owned by the renderer.
pub(crate) struct TerrainRenderer {
    spec: TerrainSpec,
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    jobs: Mutex<(wgpu::Buffer, wgpu::BindGroup)>,
    pending: Mutex<Vec<Job>>,
    vertices: wgpu::Buffer,
    clusters: wgpu::Buffer,
}

impl TerrainRenderer {
    pub fn new(gpu: &Gpu, spec: TerrainSpec, vertices: &wgpu::Buffer, clusters: &wgpu::Buffer) -> Self {
        let source = format!("{}\n{}", terrain_field::WGSL, SHADER);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("procedural terrain"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None }, count: None };
        let st = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("procedural terrain"), entries: &[
            entry(0, wgpu::BufferBindingType::Uniform), entry(1, st(true)), entry(2, st(false)), entry(3, st(false))] });
        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("procedural terrain"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some("procedural terrain build"), layout: Some(&pipeline_layout),
            module: &module, entry_point: Some("build"), compilation_options: Default::default(), cache: gpu.pipeline_cache() });
        let params = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("procedural terrain params"), size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let jobs = Self::job_buffer(gpu, 1024);
        let bind = Self::bind(gpu, &layout, &params, &jobs, vertices, clusters);
        TerrainRenderer { spec, pipeline, layout, params, jobs: Mutex::new((jobs, bind)), pending: Mutex::new(Vec::new()),
            vertices: vertices.clone(), clusters: clusters.clone() }
    }

    fn job_buffer(gpu: &Gpu, count: usize) -> wgpu::Buffer {
        gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("procedural terrain jobs"), size: (count * std::mem::size_of::<Job>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
    }

    fn bind(gpu: &Gpu, layout: &wgpu::BindGroupLayout, params: &wgpu::Buffer, jobs: &wgpu::Buffer, vertices: &wgpu::Buffer, clusters: &wgpu::Buffer) -> wgpu::BindGroup {
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("procedural terrain"), layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: jobs.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: vertices.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: clusters.as_entire_binding() }] })
    }

    /// Validation readback: slot `slot`'s 81 vertices and its cluster words.
    pub fn debug_slot(&self, gpu: &Gpu, slot: u32, instances: &wgpu::Buffer) -> (Vec<f32>, Vec<u32>, Vec<f32>) {
        let read = |src: &wgpu::Buffer, offset: u64, bytes: u64| -> Vec<u8> {
            let dst = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(src, offset, &dst, 0, bytes);
            gpu.queue.submit([enc.finish()]);
            dst.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            let v = dst.slice(..).get_mapped_range().expect("mapped").to_vec();
            v
        };
        let v = read(&self.vertices, ((self.spec.first_vertex + slot * VERTS) * 32) as u64, (VERTS * 32) as u64);
        let c = read(&self.clusters, ((self.spec.first_cluster + slot) * 80) as u64, 80);
        let i = read(instances, ((self.spec.first_instance + slot) * 80) as u64, 80);
        (bytemuck::cast_slice(&v).to_vec(), bytemuck::cast_slice(&c).to_vec(), bytemuck::cast_slice(&i).to_vec())
    }

    pub fn contains_owner_range(&self, first: usize, count: usize) -> bool {
        let (a, b) = (self.spec.first_instance as usize, (self.spec.first_instance + self.spec.capacity) as usize);
        first < b && first.saturating_add(count) > a
    }

    pub fn queue(&self, jobs: Vec<Job>) {
        let mut pending = self.pending.lock().unwrap();
        // A slot rebuilt twice before a frame keeps only its last job.
        let mut seen = HashSet::new();
        for j in jobs.iter() { seen.insert(j.slot); }
        pending.retain(|j| !seen.contains(&j.slot));
        pending.extend(jobs);
    }

    /// Build queued patches. Runs before every view and shadow pass.
    pub fn encode(&self, gpu: &Gpu, enc: &mut wgpu::CommandEncoder) -> bool {
        let jobs = std::mem::take(&mut *self.pending.lock().unwrap());
        if jobs.is_empty() { return false; }
        let mut held = self.jobs.lock().unwrap();
        let bytes = (jobs.len() * std::mem::size_of::<Job>()) as u64;
        if held.0.size() < bytes {
            let buffer = Self::job_buffer(gpu, jobs.len().next_power_of_two());
            let bind = Self::bind(gpu, &self.layout, &self.params, &buffer, &self.vertices, &self.clusters);
            *held = (buffer, bind);
        }
        gpu.queue.write_buffer(&held.0, 0, bytemuck::cast_slice(&jobs));
        let params = [self.spec.first_vertex, self.spec.first_cluster, jobs.len() as u32, self.spec.seed];
        gpu.queue.write_buffer(&self.params, 0, bytemuck::cast_slice(&params));
        gpu.queue.write_buffer(&self.params, 16, bytemuck::bytes_of(&self.spec.style));
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("procedural terrain"), timestamp_writes: None });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &held.1, &[]);
        let n = jobs.len() as u32;
        pass.dispatch_workgroups(n.min(65535), n.div_ceil(65535), 1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain() -> ProceduralTerrain {
        ProceduralTerrain::new(TerrainSpec { first_instance: 0, first_cluster: 0, first_vertex: 0, capacity: 1 << 16, seed: 1, style: terrain_field::Style::default() }, Settings::default())
    }

    #[test]
    fn selection_is_balanced_and_detailed_near_the_camera() {
        let t = terrain();
        let y = terrain_field::height(10.0, 20.0, 1, &terrain_field::Style::default());
        let pos = Vec3::new(10.0, y + 1.7, 20.0);
        let camera = Camera { pos, target: pos + Vec3::new(1.0, -0.1, 0.3), fov_y: 1.0, near: 0.1 };
        let views = [TerrainView::new(&camera, 1920, 1080)];
        let leaves = t.select(&views);
        let finest = leaves.iter().map(|n| n.level).min().unwrap();
        assert!(finest <= 2, "fine patches near the camera: finest {finest}, camera {pos:?}");
        for &n in &leaves {
            for dz in -1..=1 { for dx in -1..=1 {
                if let Some(c) = covering(&leaves, Node { level: n.level, ix: n.ix + dx, iz: n.iz + dz }, t.settings.root_level) {
                    assert!(c.level <= n.level + 1, "unbalanced {n:?} next to {c:?}");
                }
            }}
        }
        let mut hist = std::collections::BTreeMap::new();
        for n in &leaves { *hist.entry(n.level).or_insert(0) += 1; }
        let mut raw = HashSet::new();
        let mut stack: Vec<Node> = Vec::new();
        let rs = t.size(t.settings.root_level);
        for ix in -3..=3 { for iz in -3..=3 { stack.push(Node { level: t.settings.root_level, ix, iz }); } }
        while let Some(n) = stack.pop() { if t.wants_split(n, &views) { stack.extend(n.children()); } else { raw.insert(n); } }
        let mut seen = 0; let mut far = Vec::new();
        for n in leaves.iter().filter(|n| n.level == 6) {
            let size = t.size(6);
            let c = Vec3::new((n.ix as f32 + 0.5) * size, views[0].pos.y, (n.iz as f32 + 0.5) * size);
            if views[0].sees(c, size) { seen += 1; }
            far.push((c - views[0].pos).length() as i32);
        }
        far.sort();
        eprintln!("cam {:?} level6 seen {seen} dist p10 {} p50 {} p90 {}", views[0].pos, far[far.len()/10], far[far.len()/2], far[far.len()*9/10]);
        eprintln!("{} patches {:?}, unbalanced {} root size {rs}", leaves.len(), hist, raw.len());
        assert!(leaves.len() < 20000, "{} patches", leaves.len());
    }
}
