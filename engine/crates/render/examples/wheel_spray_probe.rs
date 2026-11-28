//! Wheel spray look check: one driven wheel spinning up or drifting on each
//! loose surface, filmed from a chase camera. Emission is the engine's
//! `physics::vehicle::wheel_fx`, the same code games use.
//! `cargo run --release -p render --example wheel_spray_probe -- --out FRESH_DIR`
use glam::{Affine3A, Vec3};
use physics::vehicle::{surfaces::Surface, wheel_fx};
use render::cluster::{ClusterMesh, Part, Vertex};
use render::scene::{Material, TWO_SIDED};
use render::wheel_particles::{Birth, Kind};
use render::{Camera, Gpu, Lighting, Renderer, Scene};
use std::path::PathBuf;

const W: u32 = 960;
const H: u32 = 540;
const HZ: f64 = 120.0;

fn quad(scene: &mut Scene, c: Vec3, half: Vec3, up: Vec3, material: u32) {
    let a = half.cross(up);
    let p = [c - half - a, c + half - a, c + half + a, c - half + a];
    let uv = [[0., 0.], [8., 0.], [8., 8.], [0., 8.]];
    let vertices: Vec<_> = p.iter().zip(uv).map(|(p, uv)| Vertex { pos: p.to_array(), normal: up.to_array(), uv }).collect();
    let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &[0, 2, 1, 0, 3, 2], material }]));
    scene.add_instance(mesh, Affine3A::IDENTITY);
}

fn boxy(scene: &mut Scene, c: Vec3, s: Vec3, material: u32) {
    for (n, u) in [(Vec3::X, Vec3::Y), (Vec3::NEG_X, Vec3::Y), (Vec3::Y, Vec3::Z), (Vec3::NEG_Y, Vec3::Z), (Vec3::Z, Vec3::Y), (Vec3::NEG_Z, Vec3::Y)] {
        let v = n.cross(u);
        let (hn, hu, hv) = (n * s.dot(n.abs()) * 0.5, u * s.dot(u.abs()) * 0.5, v * s.dot(v.abs()) * 0.5);
        let p = [c + hn - hu - hv, c + hn + hu - hv, c + hn + hu + hv, c + hn - hu + hv];
        let vertices: Vec<_> = p.iter().map(|p| Vertex { pos: p.to_array(), normal: n.to_array(), uv: [0., 0.] }).collect();
        let mesh = scene.add_mesh(&ClusterMesh::build(&[Part { vertices: &vertices, indices: &[0, 1, 2, 0, 2, 3, 0, 2, 1, 0, 3, 2], material }]));
        scene.add_instance(mesh, Affine3A::IDENTITY);
    }
}

fn kind(k: wheel_fx::Kind) -> Kind {
    match k {
        wheel_fx::Kind::Stone => Kind::Stone,
        wheel_fx::Kind::SoilClod => Kind::SoilClod,
        wheel_fx::Kind::GrassBlade => Kind::GrassBlade,
        wheel_fx::Kind::MudClod => Kind::MudClod,
        wheel_fx::Kind::Dust => Kind::Dust,
        wheel_fx::Kind::TyreSmoke => Kind::TyreSmoke,
        wheel_fx::Kind::WaterDrop => Kind::WaterDrop,
        wheel_fx::Kind::Sand => Kind::Sand,
    }
}

/// Ground texture: mottled noise in the surface's colours.
fn ground(scene: &mut Scene, name: &str, a: [u8; 3], b: [u8; 3]) -> i32 {
    let img = image::RgbaImage::from_fn(128, 128, |x, y| {
        let h = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(2654435761) >> 24;
        let t = h as f32 / 255.0;
        let m = |i: usize| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t) as u8;
        image::Rgba([m(0), m(1), m(2), 255])
    });
    scene.add_image(name, img) as i32
}

struct Case { name: &'static str, surface: Surface, forward: f32, tread: f32, lateral: f32 }

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1)
    }
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let out = PathBuf::from(&args.windows(2).find(|a| a[0] == "--out").ok_or("--out DIR required")?[1]);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let cases = [
        Case { name: "gravel-spin", surface: Surface::Gravel, forward: 3.0, tread: 14.0, lateral: 0.0 },
        // Sliding towards the side camera: this outside wheel bulldozes gravel out.
        Case { name: "gravel-drift", surface: Surface::Gravel, forward: 12.0, tread: 13.0, lateral: -5.0 },
        Case { name: "grass-spin", surface: Surface::Grass, forward: 3.0, tread: 14.0, lateral: 0.0 },
        Case { name: "dirt-spin", surface: Surface::Dirt, forward: 3.0, tread: 14.0, lateral: 0.0 },
        Case { name: "sand-spin", surface: Surface::Sand, forward: 3.0, tread: 14.0, lateral: 0.0 },
    ];
    let gpu = Gpu::new()?;
    for case in &cases {
        let mut scene = Scene::default();
        scene.sky = Some((16, 8, vec![[0.55, 0.7, 1.0, 1.0]; 128]));
        let (a, b) = match case.surface {
            Surface::Grass => ([40, 70, 20], [70, 105, 35]),
            Surface::Dirt => ([70, 50, 32], [100, 75, 50]),
            Surface::Sand => ([150, 128, 92], [182, 158, 115]),
            _ => ([95, 88, 78], [140, 130, 115]),
        };
        let layer = ground(&mut scene, case.name, a, b);
        let g = scene.add_material(Material { tint: [1.0; 3], layer, roughness: 1., flags: TWO_SIDED, scale: 1., metal: 0. });
        quad(&mut scene, Vec3::new(0., 0., -10.), Vec3::new(30., 0., 0.), Vec3::Y, g);
        let dark = scene.add_material(Material { tint: [0.03; 3], layer: -1, roughness: 0.6, flags: TWO_SIDED, scale: 1., metal: 0. });
        let light = Lighting { ev100: 14.0, ..Default::default() };
        // Emit for 1.2 s while the wheel drives forward along -Z.
        let mut emitter = wheel_fx::Emitter::new(7);
        let mut births = Vec::new();
        let ticks = (1.2 * HZ) as u64;
        let mut pos = Vec3::ZERO;
        let dir = Vec3::new(case.lateral, 0.0, -case.forward);
        for tick in 1..=ticks {
            pos += dir / HZ as f32;
            let c = wheel_fx::Contact {
                id: 0, grounded: true, surface: case.surface, point: pos.to_array(), normal: [0., 1., 0.],
                inherited_velocity: dir.to_array(), forward: [0., 0., -1.], right: [1., 0., 0.],
                forward_speed: case.forward, lateral_speed: case.lateral, tread_speed: case.tread, load: 3000.0, slide: 1.0,
            };
            let t = tick as f64 / HZ;
            for s in emitter.tick(tick, &[c], 0.0) {
                births.push(Birth {
                    position: Vec3::from_array(s.position), velocity: Vec3::from_array(s.velocity), normal: Vec3::from_array(s.normal),
                    radius: s.radius, lifetime: s.lifetime, wetness: s.wetness, kind: kind(s.kind), seed: s.serial as u32 ^ 0x9e37_79b9, birth_time: t,
                });
            }
        }
        // Tyre and a hint of the car body at the final wheel position.
        boxy(&mut scene, pos + Vec3::new(0., 0.33, 0.), Vec3::new(0.21, 0.66, 0.66), dark);
        boxy(&mut scene, pos + Vec3::new(0.75, 0.75, -1.35), Vec3::new(1.7, 0.7, 4.0), dark);
        let renderer = Renderer::new(&gpu, &scene, &light);
        renderer.update_wheel_particles(&gpu, ticks as f64 / HZ, &births)?;
        for (view, cam_pos) in [("side", Vec3::new(-5.0, 1.1, 2.0)), ("chase", Vec3::new(-1.8, 1.5, 7.0))] {
            let camera = Camera { pos: pos + cam_pos, target: pos + Vec3::new(0.0, 0.6, 1.6), fov_y: 50f32.to_radians(), near: 0.05 };
            let mut img = Vec::new();
            for _ in 0..3 {
                img = renderer.render(&gpu, &camera, &light, W, H).0;
            }
            render::write_png(&out.join(format!("{}-{view}.png", case.name)), W, H, &img)?;
        }
        eprintln!("{}: {} births", case.name, births.len());
    }
    Ok(())
}
