//! Unskinned pedestrians share a handful of meshes per fitted appearance. A
//! low-frequency snapshot carries both ends of the next walking interval.
use glam::{Affine3A, Quat, Vec3};
use render::{humans::Instance, scene::Material, Scene};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Default)]
pub(crate) struct Sample {
    pub pos: Vec3,
    pub next: Vec3,
    pub yaw: f32,
    pub next_yaw: f32,
    pub look: usize,
    pub active: bool,
    pub fitted: bool,
}

#[derive(Default)]
pub(crate) struct Distant {
    residents: Vec<Sample>,
    phases: Vec<f32>,
    poses: Vec<[usize; 5]>,
    sizes: Vec<Vec3>,
    first: usize,
    capacity: usize,
    duration: f32,
    age: f32,
    dt: f32,
    fresh: bool,
}

impl Distant {
    pub fn attach(&mut self, scene: &mut Scene, capacity: usize, looks: &[Instance], sizes: Vec<Vec3>, bodies: usize, skin: Option<[f32; 3]>) {
        self.capacity = capacity;
        self.first = scene.instances.len();
        self.poses.clear();
        self.sizes = sizes;
        if capacity == 0 { return; }
        let mut base_materials: Vec<[u32; 5]> = Vec::new();
        let mut palette = std::collections::BTreeMap::new();
        for (k, person) in looks.iter().enumerate() {
            let skin = skin.map(Vec3::from).unwrap_or_else(|| person.skin_color());
            let hair = person.hair_and_brows(skin).0;
            let colors = [person.shirt_color(), person.trouser_color(), person.shoe_color(), skin.into(), hair.into()];
            let materials = std::array::from_fn(|channel| {
                let color: [f32; 3] = colors[channel];
                *palette.entry((channel, color.map(f32::to_bits))).or_insert_with(|| scene.add_material(Material {
                    tint: color.map(|v| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }),
                    layer: -1, roughness: 0.85, flags: 0, scale: 1., metal: 0.,
                }))
            });
            let meshes = if k < bodies {
                base_materials.push(materials);
                [-1., -0.5, 0., 0.5, 1.].map(|s| scene.add_mesh(&person.street_silhouette(materials, s)))
            } else {
                let base = k % bodies;
                let dyes: [(u32,u32); 5] = std::array::from_fn(|i| (base_materials[base][i], materials[i]));
                self.poses[base].map(|mesh| scene.mesh_with_materials(mesh, &dyes))
            };
            self.poses.push(meshes);
        }
        // The renderer sizes its shadow list before the first draw. An empty
        // generic slot is budgeted as the largest mesh in the whole city.
        let reserve = *self.poses.iter().flatten().max_by_key(|&&mesh| scene.meshes[mesh].cluster_count).unwrap();
        for _ in 0..capacity { scene.add_instance(reserve, Affine3A::from_translation(Vec3::NEG_Y * 500.)); }
        scene.dynamic_instances.push(self.first..self.first + capacity);
    }

    pub fn clear(&mut self) {
        self.residents.clear();
        self.phases.clear();
        self.age = 0.;
    }

    pub fn replace(&mut self, residents: Vec<Sample>, duration: f32) {
        let fraction = self.fraction(1.);
        self.phases.resize_with(residents.len(), Default::default);
        for (i, phase) in self.phases.iter_mut().enumerate() {
            if let Some(last) = self.residents.get(i) {
                *phase = (*phase + last.pos.distance(last.next) * fraction / 1.4).fract();
            } else {
                *phase = ((i * 37) % 101) as f32 / 101.;
            }
        }
        self.residents = residents;
        self.duration = duration;
        self.age = 0.;
        self.fresh = true;
    }

    pub fn gait(&self, resident: usize) -> Option<(f32, f32)> {
        let i = resident.checked_sub(1)?;
        let sample = self.residents.get(i)?;
        let distance = sample.pos.distance(sample.next);
        Some(((self.phases[i] + distance * self.fraction(1.) / 1.4).fract(), distance / self.duration.max(0.001)))
    }

    pub fn resume_gait(&mut self, resident: usize, phase: f32) {
        if let Some((shown, _)) = self.gait(resident) {
            let base = &mut self.phases[resident - 1];
            *base = (*base + phase - shown).rem_euclid(1.);
        }
    }

    pub fn set_fitted(&mut self, resident: usize, fitted: bool) {
        if let Some(sample) = resident.checked_sub(1).and_then(|i| self.residents.get_mut(i)) { sample.fitted = fitted; }
    }

    pub fn visible_pose(&self, resident: usize, alpha: f32) -> Option<(Vec3, usize)> {
        self.at(resident.checked_sub(1)?, self.fraction(alpha)).map(|(pos, _, look, _)| (pos, look))
    }

    pub fn tick(&mut self, dt: f32) {
        self.dt = dt;
        if !self.fresh { self.age += dt; }
        self.fresh = false;
    }

    fn fraction(&self, alpha: f32) -> f32 {
        ((self.age - (1. - alpha) * self.dt) / self.duration.max(0.001)).clamp(0., 1.)
    }

    fn at(&self, index: usize, fraction: f32) -> Option<(Vec3, f32, usize, usize)> {
        let s = self.residents.get(index)?;
        if !s.active || s.fitted { return None; }
        let travel = s.pos.distance(s.next);
        let phase = self.phases[index] + travel * fraction / 1.4;
        let swing = if travel > 0.005 { (phase * TAU).sin() } else { 0. };
        let pose = (2. + 2. * swing).round() as usize;
        let turn = (s.next_yaw - s.yaw + PI).rem_euclid(TAU) - PI;
        Some((s.pos.lerp(s.next, fraction), s.yaw + turn * fraction, s.look, pose))
    }

    pub fn draw(&self, gpu: &render::Gpu, renderer: &render::Renderer, scene: &mut Scene, alpha: f32) -> Result<(), String> {
        if self.capacity == 0 { return Ok(()); }
        self.place(scene, alpha);
        renderer.update_instances(gpu, self.first, &scene.instances[self.first..self.first + self.capacity])
    }

    fn place(&self, scene: &mut Scene, alpha: f32) {
        let fraction = self.fraction(alpha);
        for i in 0..self.capacity {
            if let Some((pos, yaw, look, pose)) = self.at(i, fraction) {
                let transform = Affine3A::from_scale_rotation_translation(self.sizes[look], Quat::from_rotation_y(yaw), pos);
                scene.update_instance(self.first + i, self.poses[look][pose], transform);
            } else {
                scene.instances[self.first + i].cluster_count = 0;
            }
        }
    }
}
