//! Does TAA leave a trail behind a moving object? A bright cube crosses a
//! dark ground (`--speed` pixels a frame) under a still camera, rendered
//! with and without TAA. Prints, per frame, the pixels behind the cube that
//! TAA leaves brighter than the no-TAA frame (a ghost) and the pixels on the
//! cube it leaves darker (lag). Writes the last frames side by side.
//!
//!   cargo run --release -p render --example taa_ghost [OUT_DIR]
use glam::{Affine3A, Vec3};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{GpuInstance, Heightfield, Material};
use render::{Camera, Gpu, Lighting, Renderer, Scene, ViewRect};
use std::path::PathBuf;

const W: u32 = 640;
const H: u32 = 360;

fn scene() -> Scene {
    let mut scene = Scene::default();
    scene.sky = Some((16, 8, vec![[0.4, 0.5, 0.7, 1.]; 128]));
    let half = 200.0;
    let ground = scene.add_material(Material { tint: [0.05, 0.05, 0.05], layer: -1, roughness: 1., flags: 0, scale: 1., metal: 0. });
    let vertices = [[-half, 0., half], [half, 0., half], [-half, 0., -half], [half, 0., -half]]
        .map(|pos| Vertex { pos, normal: Vec3::Y.to_array(), uv: [0.; 2] });
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &[0, 1, 2, 1, 3, 2], material: ground }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
    scene.heightfield = Heightfield { origin: [-half, -half], cell: half / 50., w: 101, h: 101, heights: vec![0.0; 10201] };
    let bright = scene.add_material(Material { tint: [0.9, 0.9, 0.9], layer: -1, roughness: 1., flags: 0, scale: 1., metal: 0. });
    let cube = render::primitives::cube(bright);
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &cube.vertices, indices: &cube.indices, material: bright }]));
    scene.add_instance(mesh, Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)) * Affine3A::from_scale(Vec3::splat(2.0)));
    scene
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let speed: f32 = args.windows(2).find(|w| w[0] == "--speed").and_then(|w| w[1].parse().ok()).unwrap_or(8.0);
    let out = args.iter().skip(1).find(|a| !a.starts_with("--") && args.windows(2).all(|w| !(w[1] == **a && w[0].starts_with("--"))))
        .map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/screenshots/taa-ghost"));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::new()?;
    let cam = Camera { pos: Vec3::new(0.0, 12.0, 30.0), target: Vec3::new(0.0, 1.0, 0.0), fov_y: 50f32.to_radians(), near: 0.05 };
    let view = ViewRect { cam, x: 0, y: 0, w: W, h: H };
    // Metres per pixel at the cube: the vertical extent at its distance.
    let metres_per_px = 2.0 * (cam.pos - cam.target).length() * (25f32.to_radians()).tan() / H as f32;
    let frames = 24;
    let mut runs = Vec::new();
    for taa in [0.0, 1.0] {
        let s = scene();
        let cube_index = s.instances.len() - 1;
        let base: GpuInstance = s.instances[cube_index];
        let renderer = Renderer::new(&gpu, &s, &Lighting::default());
        let light = Lighting { taa, sharpen: 0.0, ..Default::default() };
        let mut images = Vec::new();
        for f in 0..frames {
            let x = (f as f32 - frames as f32 / 2.0) * speed * metres_per_px;
            let mut moved = base;
            for row in 0..3 { moved.rows[row][3] += [x, 0.0, 0.0][row]; }
            moved.sphere[0] += x;
            renderer.update_instances(&gpu, cube_index, &[moved])?;
            images.push(renderer.try_render_views(&gpu, &[view], &light, W, H, &[])?.0);
        }
        runs.push(images);
    }
    let lum = |p: &[u8]| (p[0] as f32 + p[1] as f32 + p[2] as f32) / (3.0 * 255.0);
    let (mut ghost_total, mut lag_total) = (0usize, 0usize);
    for f in 8..frames {
        let (a, b) = (&runs[0][f], &runs[1][f]);
        let (mut ghost, mut lag) = (0, 0);
        for (pa, pb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let (la, lb) = (lum(pa), lum(pb));
            if lb > la + 0.15 { ghost += 1; }
            if lb < la - 0.15 { lag += 1; }
        }
        ghost_total += ghost;
        lag_total += lag;
        println!("frame {f}: ghost {ghost} px, lag {lag} px");
    }
    println!("speed {speed} px/frame: ghost {:.1} px/frame, lag {:.1} px/frame (cube ~{:.0} px wide)",
        ghost_total as f32 / (frames - 8) as f32, lag_total as f32 / (frames - 8) as f32, 2.0 / metres_per_px);
    let last = frames - 1;
    let pair: Vec<u8> = (0..H as usize).flat_map(|y| {
        let row = |img: &Vec<u8>| img[y * W as usize * 4..(y + 1) * W as usize * 4].to_vec();
        [row(&runs[0][last]), row(&runs[1][last])].concat()
    }).collect();
    render::write_png(&out.join(format!("ghost_{speed}.png")), W * 2, H, &pair)
}
