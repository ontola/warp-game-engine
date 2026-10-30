//! The forest as a function, like `terrain_field`: where trees grow and what
//! the forest looks like from far away, evaluated the same way on the CPU
//! (placing tree instances, gameplay) and on the GPU (`shaders/forest_field.wgsl`,
//! line for line the same, after `terrain_field.wgsl`), so a distant forest
//! drawn into the terrain stands exactly where the trees are.
//!
//! Trees sit on a jittered 8 m grid. Slow noise (300 m) makes stands and
//! clearings; nothing grows on rock, scree, water or snow, on slopes steeper
//! than ~35 degrees, or within 250 m of the snow line. `density` (0..1) is
//! the share of forest ground that grows a tree.
//!
//! ```ignore
//! let trees = forest_field::trees_in_disc(Vec2::ZERO, 900.0, seed, &style, 0.6);
//! let far = forest_field::forest(x, z, seed, &style, 0.6); // colour, cover, crown height
//! ```
use crate::terrain_field::{self, Sample, Style};
use glam::{Vec2, Vec3};

pub const WGSL: &str = include_str!("shaders/forest_field.wgsl");

/// Metres between tree spots.
pub const CELL: f32 = 8.0;
/// Linear albedo of a conifer crown seen from afar (needles, shade between them).
pub const CROWN: [f32; 3] = [0.045, 0.085, 0.035];
/// Height of a full-size tree's crown top above its foot (metres).
pub const TREE_HEIGHT: f32 = 15.0;
/// The terrain detail cut the forest samples at: within a few centimetres of
/// cut 0 on the CPU, but defined on the GPU (terrain_field's fade divides by
/// the cut, and x / 0 is not portable WGSL: Metal gave other ground).
pub const CUT: f32 = 0.05;

/// The forest at a point, for drawing it where single trees are too small.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Forest {
    /// Share of the ground covered by tree spots that grow a tree, 0..1.
    pub density: f32,
    /// Linear crown albedo here (stands vary slightly).
    pub colour: Vec3,
    /// Height of the crowns above the ground (metres), 0 without forest.
    pub crown_height: f32,
}

/// One tree: foot position, rotation about Y, scale (x = z width, y height)
/// and which of the game's variants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tree {
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: Vec3,
    pub variant: u32,
}

/// A uniform 0..1 number for grid cell (i, j), channel k.
fn hash(i: i32, j: i32, k: u32, seed: u32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x8da6b343) ^ (j as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f) ^ k.wrapping_mul(0x165667b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b3c6d);
    h ^= h >> 12;
    (h & 0xffffff) as f32 / 16777216.0
}

/// Stands and clearings hundreds of metres across, -1..1.
fn stand(x: f32, z: f32, seed: u32) -> f32 {
    terrain_field::noised(Vec2::new(x, z) / 300.0, seed.wrapping_add(77)).x
}

/// The chance a tree spot in this stand grows a tree.
fn chance(stand: f32, density: f32) -> f32 {
    ((stand + 0.15) * 2.0).clamp(0.0, 1.0) * density
}

/// Whether ground can carry a tree.
fn grows(s: &Sample, style: &Style) -> bool {
    !(s.rock > 0.3 || s.scree > 0.3 || s.water > 0.01 || s.snow > 0.05 || s.up < 0.82 || s.pos.y > style.snow_line - 250.0)
}

/// The forest at (x, z).
pub fn forest(x: f32, z: f32, seed: u32, style: &Style, density: f32) -> Forest {
    let st = stand(x, z, seed);
    let s = terrain_field::eval(x, z, CUT, seed, style);
    let cover = if grows(&s, style) { chance(st, density) } else { 0.0 };
    let tone = (st * 0.5 + 0.5).clamp(0.0, 1.0);
    Forest {
        density: cover,
        colour: Vec3::from(CROWN) * (0.8 + 0.4 * tone),
        crown_height: if cover > 0.0 { TREE_HEIGHT * (0.85 + 0.3 * tone) } else { 0.0 },
    }
}

/// The tree of grid cell (i, j), if it grows one.
pub fn tree(i: i32, j: i32, seed: u32, style: &Style, density: f32) -> Option<Tree> {
    let (x, z) = ((i as f32 + hash(i, j, 1, seed)) * CELL, (j as f32 + hash(i, j, 2, seed)) * CELL);
    if hash(i, j, 3, seed) >= chance(stand(x, z, seed), density) { return None; }
    let s = terrain_field::eval(x, z, CUT, seed, style);
    if !grows(&s, style) { return None; }
    let width = 0.75 + 0.5 * hash(i, j, 4, seed);
    let height = width * (0.9 + 0.25 * hash(i, j, 5, seed));
    Some(Tree {
        pos: Vec3::new(x, s.pos.y - 0.2, z),
        yaw: hash(i, j, 6, seed) * std::f32::consts::TAU,
        scale: Vec3::new(width, height, width),
        variant: (hash(i, j, 7, seed) * 3.0) as u32 % 3,
    })
}

/// Every tree within `radius` of `centre` (x, z), row by row.
pub fn trees_in_disc(centre: Vec2, radius: f32, seed: u32, style: &Style, density: f32) -> Vec<Tree> {
    let (i0, i1) = (((centre.x - radius) / CELL).floor() as i32, ((centre.x + radius) / CELL).ceil() as i32);
    let (j0, j1) = (((centre.y - radius) / CELL).floor() as i32, ((centre.y + radius) / CELL).ceil() as i32);
    let mut trees = Vec::new();
    for j in j0..j1 {
        for i in i0..i1 {
            // Cheap distance test on the jittered spot before the terrain.
            let (x, z) = ((i as f32 + hash(i, j, 1, seed)) * CELL, (j as f32 + hash(i, j, 2, seed)) * CELL);
            if Vec2::new(x - centre.x, z - centre.y).length() > radius { continue; }
            if let Some(t) = tree(i, j, seed, style, density) { trees.push(t); }
        }
    }
    trees
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use wgpu::util::DeviceExt;

    #[test]
    fn trees_shun_rock_and_follow_stands_and_density() {
        let style = Style::named("alpine").unwrap();
        let trees = trees_in_disc(Vec2::ZERO, 600.0, 1, &style, 0.6);
        assert!(trees.len() > 200, "{} trees", trees.len());
        for t in &trees {
            let s = terrain_field::eval(t.pos.x, t.pos.z, CUT, 1, &style);
            assert!(grows(&s, &style) && (t.pos.y - (s.pos.y - 0.2)).abs() < 1e-3);
            assert!(t.variant < 3 && (0.75..=1.25).contains(&t.scale.x));
        }
        let fewer = trees_in_disc(Vec2::ZERO, 600.0, 1, &style, 0.2).len();
        assert!(fewer < trees.len() / 2, "density 0.2: {fewer} of {}", trees.len());
        // The far field agrees with the trees: cover where they grow.
        let covered = trees.iter().filter(|t| forest(t.pos.x, t.pos.z, 1, &style, 0.6).density > 0.0).count();
        assert!(covered * 10 >= trees.len() * 9, "{covered} of {} trees on forest cover", trees.len());
    }

    #[repr(C)]
    #[derive(Clone, Copy, Pod, Zeroable)]
    struct Out { forest: [f32; 4], crown: [f32; 4], tree: [f32; 4], shape: [f32; 4] }

    /// CPU and GPU evaluate the same function. terrain_eval differs from eval
    /// in the last bits, so a spot right on a threshold may flip; nearly all
    /// must agree, and agreeing ones exactly in place.
    #[test]
    fn gpu_matches_cpu() {
        let Ok(gpu) = crate::Gpu::new() else { return };
        let style = Style::named("alpine").unwrap();
        let (seed, density, n) = (3u32, 0.6f32, 64i32);
        let cells: Vec<[i32; 2]> = (0..n * n).map(|k| [k % n - n / 2, k / n - n / 2]).collect();
        let shader = format!("{}\n{}\n{}", terrain_field::WGSL, WGSL, r#"
struct Params { seed: u32, density: f32, pad0: u32, pad1: u32, style: TerrainStyle };
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> cells: array<vec2<i32>>;
@group(0) @binding(2) var<storage, read_write> out: array<array<vec4<f32>, 4>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= arrayLength(&cells) { return; }
    let c = cells[gid.x];
    let f = forest_at((f32(c.x) + 0.5) * FF_CELL, (f32(c.y) + 0.5) * FF_CELL, params.seed, params.style, params.density);
    let t = forest_tree(c.x, c.y, params.seed, params.style, params.density);
    out[gid.x] = array<vec4<f32>, 4>(vec4<f32>(f.density, f.colour), vec4<f32>(f.crown_height, 0.0, 0.0, 0.0),
        vec4<f32>(t.pos, t.yaw), vec4<f32>(t.scale.xy, f32(t.variant), f32(t.present)));
}"#);
        let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("forest parity"), source: wgpu::ShaderSource::Wgsl(shader.into()) });
        let pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let mut params = bytemuck::bytes_of(&[seed, density.to_bits(), 0, 0]).to_vec();
        params.extend_from_slice(bytemuck::bytes_of(&style));
        let params = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &params, usage: wgpu::BufferUsages::UNIFORM });
        let input = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: bytemuck::cast_slice(&cells), usage: wgpu::BufferUsages::STORAGE });
        let size = (cells.len() * std::mem::size_of::<Out>()) as u64;
        let output = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: output.as_entire_binding() }] });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((cells.len() as u32).div_ceil(64), 1, 1);
        }
        enc.copy_buffer_to_buffer(&output, 0, &read, 0, size);
        gpu.queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let gpu_out: Vec<Out> = bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()).to_vec();
        let (mut agree, mut trees) = (0, 0);
        for (c, g) in cells.iter().zip(&gpu_out) {
            let f = forest((c[0] as f32 + 0.5) * CELL, (c[1] as f32 + 0.5) * CELL, seed, &style, density);
            let cpu_tree = tree(c[0], c[1], seed, &style, density);
            let gpu_has = g.shape[3] > 0.5;
            if cpu_tree.is_some() == gpu_has && (f.density > 0.0) == (g.forest[0] > 0.0) { agree += 1; }
            if let (Some(t), true) = (cpu_tree, gpu_has) {
                trees += 1;
                assert!((Vec3::from_slice(&g.tree[..3]) - t.pos).abs().max_element() < 0.05, "tree {c:?}: cpu {:?} gpu {:?}", t.pos, &g.tree);
                assert_eq!(g.tree[3], t.yaw);
                assert_eq!([g.shape[0], g.shape[1], g.shape[2]], [t.scale.x, t.scale.y, t.variant as f32]);
            }
            if f.density > 0.0 && g.forest[0] > 0.0 {
                assert!((g.forest[0] - f.density).abs() < 1e-3, "density {c:?}: cpu {} gpu {}", f.density, g.forest[0]);
                assert!((Vec3::new(g.forest[1], g.forest[2], g.forest[3]) - f.colour).abs().max_element() < 1e-3);
                assert!((g.crown[0] - f.crown_height).abs() < 0.05);
            }
        }
        assert!(trees > 300, "{trees} trees compared");
        assert!(agree * 100 >= cells.len() * 99, "{agree} of {} cells agree", cells.len());
    }
}
