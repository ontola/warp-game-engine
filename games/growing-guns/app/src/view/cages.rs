//! Round-start cages from Godot's `spawn_cage.gd`: iron bars and a hanging
//! chain, split trapdoors, then a slow winch back into the sky. These are
//! presentation only; the simulation owns freezing, descent and release.
use glam::{Affine3A, Quat, Vec3};
use growing_guns::World;
use render::primitives::{self, Primitive};
use render::scene::{Material, NO_SHADOW};
use render::Scene;

const WIDTH: f32 = 2.6;
const HEIGHT: f32 = 3.4;
const FLOOR_OFFSET: f32 = 1.1;
const WINCH_DELAY: f32 = 0.34 + 0.8;
const WINCH_SECONDS: f32 = 7.0;
const WINCH_RISE: f32 = 140.0;
const PARTS: usize = 3; // frame/chain and the two independently hinged floors

#[derive(Clone, Copy)]
struct Cage { origin: Vec3, opened: Option<f32> }

pub(super) struct Cages {
    first: usize,
    frame: usize,
    floor: usize,
    cages: Vec<Option<Cage>>,
}

impl Cages {
    pub fn reserve(scene: &mut Scene, players: usize) -> Self {
        let iron = scene.add_material(Material {
            tint: super::srgb_to_linear([0.13, 0.13, 0.15]), layer: -1,
            roughness: 0.55, flags: 0, scale: 1.0, metal: 0.5,
        });
        let frame = scene.add_mesh(&frame_mesh(iron));
        let floor = scene.add_mesh(&primitives::cube(iron).mesh());
        let first = scene.instances.len();
        for _ in 0..players {
            for mesh in [frame, floor, floor] {
                scene.add_instance(mesh, Affine3A::IDENTITY);
                let i = scene.instances.last_mut().unwrap();
                i.flags |= NO_SHADOW;
                i.cluster_count = 0;
                i.sphere = [0.0; 4];
            }
        }
        Self { first, frame, floor, cages: vec![None; players] }
    }

    pub fn reset(&mut self, scene: &mut Scene, world: &World) {
        self.cages.fill(None);
        for k in 0..self.cages.len() { self.hide(scene, k); }
        self.sync(world);
    }

    /// Observe every simulation tick so release is captured even if a frame
    /// spans several ticks. Keep the empty cage at its final hold position;
    /// it must not follow the falling player after the floor opens.
    pub fn sync(&mut self, world: &World) {
        let now = world.tick as f32 * growing_guns::DT;
        let closed = world.cage_left().is_some();
        for (k, cage) in self.cages.iter_mut().enumerate() {
            let Some(p) = world.players.get(k) else { *cage = None; continue };
            if closed {
                *cage = (p.frozen && p.alive && !p.ghost).then_some(Cage {
                    origin: p.pos - Vec3::Y * FLOOR_OFFSET, opened: None,
                });
            } else if let Some(c) = cage {
                let opened = *c.opened.get_or_insert(now);
                if now - opened >= WINCH_DELAY + WINCH_SECONDS { *cage = None; }
            }
        }
    }

    pub fn pose(&self, scene: &mut Scene, world: &World, prev: &[crate::view::Pose], alpha: f32, clock: f32) {
        for (k, cage) in self.cages.iter().enumerate() {
            let Some(cage) = cage else { self.hide(scene, k); continue };
            let age = cage.opened.map_or(0.0, |t| (clock - t).max(0.0));
            if age >= WINCH_DELAY + WINCH_SECONDS { self.hide(scene, k); continue; }
            let origin = if cage.opened.is_none() {
                // Match the camera/player interpolation during the descent.
                crate::view::pose(world, k, prev, alpha).pos - Vec3::Y * FLOOR_OFFSET
            } else {
                let u = ((age - WINCH_DELAY) / WINCH_SECONDS).clamp(0.0, 1.0);
                cage.origin + Vec3::Y * (WINCH_RISE * u * u)
            };
            scene.update_instance(self.first + k * PARTS, self.frame, Affine3A::from_translation(origin));
            for (part, side, duration) in [(1, -1.0, 0.3), (2, 1.0, 0.34)] {
                let angle = side * 1.9 * (age / duration).clamp(0.0, 1.0).powi(3);
                scene.update_instance(self.first + k * PARTS + part, self.floor, floor_transform(origin, side, angle));
            }
        }
    }

    fn hide(&self, scene: &mut Scene, player: usize) {
        for i in &mut scene.instances[self.first + player * PARTS..self.first + (player + 1) * PARTS] {
            i.cluster_count = 0;
            i.sphere = [0.0; 4];
        }
    }
}

fn floor_transform(origin: Vec3, side: f32, angle: f32) -> Affine3A {
    let rotation = Quat::from_rotation_z(angle);
    let hinge = origin + Vec3::X * (side * WIDTH * 0.5);
    let center = hinge + rotation * (Vec3::X * (-side * WIDTH * 0.25));
    Affine3A::from_scale_rotation_translation(Vec3::new(WIDTH * 0.5, 0.08, WIDTH), rotation, center)
}

/// Bake the fixed pieces into one shared mesh, rather than dozens of dynamic
/// instances per player. Only the floor halves need separate transforms.
fn frame_mesh(material:u32)->render::cluster::ClusterMesh {
    // A 90 m chain shares this mesh with 7 cm bars. Generic coarse LODs
    // erase the bars relative to that large bounding box. The authored mesh
    // is already below 1,000 triangles; retain it on PC and minimum/VR alike.
    let source=frame(material);
    let (lo,hi)=source.vertices.iter().fold((Vec3::splat(f32::INFINITY),Vec3::splat(f32::NEG_INFINITY)),|(lo,hi),v|{
        let p=Vec3::from(v.pos);(lo.min(p),hi.max(p))});
    let center=(lo+hi)*0.5;
    let radius=source.vertices.iter().map(|v|Vec3::from(v.pos).distance(center)).fold(0.0f32,f32::max);
    render::cluster::ClusterMesh::build_fixed(&[source.part()],center.extend(radius+0.001).to_array())
}

fn frame(material: u32) -> Primitive {
    let mut out = Primitive { material, ..Default::default() };
    let cube = primitives::cube(material);
    let bar = primitives::cylinder(material, 0.035, 0.035, 6);
    let half = WIDTH * 0.5;
    for x in [-half, half] {
        for z in [-half, half] {
            append(&mut out, &cube, Vec3::new(0.1, HEIGHT, 0.1), Vec3::new(x, HEIGHT * 0.5, z));
        }
    }
    for y in [0.0, HEIGHT] {
        for side in [-half, half] {
            append(&mut out, &cube, Vec3::new(WIDTH + 0.2, 0.09, 0.14), Vec3::new(0.0, y, side));
            append(&mut out, &cube, Vec3::new(0.14, 0.09, WIDTH + 0.2), Vec3::new(side, y, 0.0));
        }
    }
    let bars = (WIDTH / 0.42) as usize;
    for k in 0..bars {
        let off = (k as f32 + 0.5) / bars as f32 * WIDTH - half;
        for side in [-half, half] {
            append(&mut out, &bar, Vec3::new(1.0, HEIGHT, 1.0), Vec3::new(off, HEIGHT * 0.5, side));
            append(&mut out, &bar, Vec3::new(1.0, HEIGHT, 1.0), Vec3::new(side, HEIGHT * 0.5, off));
        }
    }
    append(&mut out, &cube, Vec3::new(WIDTH + 0.1, 0.07, WIDTH + 0.1), Vec3::new(0.0, HEIGHT + 0.05, 0.0));
    let chain = primitives::cylinder(material, 0.05, 0.05, 6);
    append(&mut out, &chain, Vec3::new(1.0, 90.0, 1.0), Vec3::new(0.0, HEIGHT + 45.0, 0.0));
    out
}

fn append(out: &mut Primitive, part: &Primitive, scale: Vec3, position: Vec3) {
    let base = out.vertices.len() as u32;
    out.vertices.extend(part.vertices.iter().map(|&v| render::cluster::Vertex {
        pos: (Vec3::from(v.pos) * scale + position).to_array(),
        normal: (Vec3::from(v.normal) / scale).normalize().to_array(), ..v
    }));
    out.indices.extend(part.indices.iter().map(|&i| base + i));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translation(i: &render::scene::GpuInstance) -> Vec3 { Vec3::new(i.rows[0][3], i.rows[1][3], i.rows[2][3]) }

    #[test]
    fn cage_frame_lod_retains_the_thin_bars_despite_its_long_chain() {
        let source=frame(0);let generic=source.mesh();let fixed=frame_mesh(0);
        assert_eq!(fixed.root_triangles(),source.indices.len()/3);
        assert!(fixed.root_triangles()<1000,"cage budget grew unexpectedly");
        eprintln!("cage triangles: source {}, generic root {}, retained root {}",source.indices.len()/3,generic.root_triangles(),fixed.root_triangles());
        for v in &fixed.vertices {assert!(Vec3::from(v.pos).distance(Vec3::from_slice(&fixed.sphere[..3]))<=fixed.sphere[3]);}
        if let Ok(path)=std::env::var("WARP_CAGE_CAPTURE") {
            let gpu=render::Gpu::new().unwrap();let mut scene=Scene::default();scene.graphics.minimal_forward=true;
            scene.add_material(Material{tint:[0.2,0.2,0.24],layer:-1,roughness:0.5,flags:0,scale:1.,metal:0.5});
            let mesh=scene.add_mesh(&fixed);scene.add_instance(mesh,Affine3A::IDENTITY);
            let light=render::Lighting::default();let renderer=render::Renderer::new(&gpu,&scene,&light);
            let camera=render::Camera{pos:Vec3::new(6.,4.,10.),target:Vec3::new(0.,2.,0.),fov_y:0.65,near:0.05};
            let (image,_)=renderer.render(&gpu,&camera,&light,640,640);
            render::write_png(std::path::Path::new(&path),640,640,&image).unwrap();
        }
    }

    #[test]
    fn cage_follows_descent_opens_at_release_and_retracts_without_the_player() {
        let mut world = World::new_with(3, &[false, false], |w| w.modifier_chance = 0.0);
        let mut scene = Scene::default();
        let mut cages = Cages::reserve(&mut scene, 2);
        cages.reset(&mut scene, &world);
        let mut prev = crate::view::Pose::all(&world);
        for _ in 0..120 {
            prev = crate::view::Pose::all(&world);
            world.step(&[]);
            cages.sync(&world);
        }
        cages.pose(&mut scene, &world, &prev, 0.4, world.tick as f32 * growing_guns::DT);
        let origin = prev[0].pos.lerp(world.players[0].pos, 0.4) - Vec3::Y * FLOOR_OFFSET;
        assert!(translation(&scene.instances[cages.first]).abs_diff_eq(origin, 0.0001));
        assert!(scene.instances.iter().all(|i| i.cluster_count > 0 && i.flags & NO_SHADOW != 0));
        while world.cage_left().is_some() { world.step(&[]); cages.sync(&world); }
        let cage = cages.cages[0].unwrap();
        let released = cage.opened.expect("the existing simulation release opens the visual floor");
        world.players[0].pos -= Vec3::Y * 8.0;
        cages.pose(&mut scene, &world, &[], 1.0, released + 0.34);
        assert!(translation(&scene.instances[cages.first]).abs_diff_eq(cage.origin, 0.0001));
        for part in 1..PARTS {
            assert!(translation(&scene.instances[cages.first + part]).y < cage.origin.y - 0.5, "trapdoors swing down");
        }
        cages.pose(&mut scene, &world, &[], 1.0, released + WINCH_DELAY + WINCH_SECONDS * 0.5);
        assert!((translation(&scene.instances[cages.first]).y - cage.origin.y - 35.0).abs() < 0.001);
        cages.pose(&mut scene, &world, &[], 1.0, released + WINCH_DELAY + WINCH_SECONDS + 0.01);
        assert!(scene.instances.iter().all(|i| i.cluster_count == 0));
        world.release_cage();
        world.players[0].frozen = true; // Card-pick freezing is not a spawn cage.
        cages.reset(&mut scene, &world);
        cages.pose(&mut scene, &world, &[], 1.0, released);
        assert!(scene.instances.iter().all(|i| i.cluster_count == 0));
    }

    #[test]
    fn trapdoors_share_a_closed_floor_and_keep_their_hinges_when_opened() {
        for side in [-1.0, 1.0] {
            let closed = floor_transform(Vec3::ZERO, side, 0.0);
            assert!(closed.transform_point3(Vec3::X * (-side * 0.5)).abs_diff_eq(Vec3::ZERO, 0.0001));
            for angle in [0.0, side * 0.9, side * 1.9] {
                let xf = floor_transform(Vec3::ZERO, side, angle);
                assert!(xf.transform_point3(Vec3::X * (side * 0.5)).abs_diff_eq(Vec3::X * (side * WIDTH * 0.5), 0.0001));
            }
        }
    }

    #[test]
    fn remote_snapshots_release_cages_and_do_not_recage_frozen_card_pickers() {
        let mut host = World::new_with(3, &[false, false], |w| w.modifier_chance = 0.0);
        let mut remote = World::remote(host.arena.clone(), 2);
        let mut scene = Scene::default();
        let mut cages = Cages::reserve(&mut scene, 2);
        remote.apply(host.snapshot());
        cages.reset(&mut scene, &remote);
        assert!(cages.cages.iter().all(Option::is_some));
        host.release_cage();
        remote.apply(host.snapshot());
        cages.sync(&remote);
        assert!(cages.cages.iter().all(|c| c.is_some_and(|c| c.opened.is_some())));
        host.players[0].frozen = true;
        remote.apply(host.snapshot());
        cages.reset(&mut scene, &remote);
        assert!(cages.cages.iter().all(Option::is_none));
    }
}
