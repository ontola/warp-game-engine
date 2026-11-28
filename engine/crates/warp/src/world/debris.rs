//! Wheel debris from the vehicle fleet (gravel, grass, sand, dust, tyre smoke)
//! uploaded once per frame to the renderer's shared wheel particle pool.
use super::World;
use physics::vehicle::wheel_fx::{self, Kind as K};
use render::wheel_particles::{Birth, Kind};

fn kind(k: K) -> Kind {
    match k {
        K::Stone => Kind::Stone,
        K::SoilClod => Kind::SoilClod,
        K::GrassBlade => Kind::GrassBlade,
        K::MudClod => Kind::MudClod,
        K::Dust => Kind::Dust,
        K::TyreSmoke => Kind::TyreSmoke,
        K::WaterDrop => Kind::WaterDrop,
        K::Sand => Kind::Sand,
    }
}

fn birth(time: f64, s: &wheel_fx::Spawn) -> Birth {
    let seed = (s.wheel_id as u64).rotate_left(32) ^ s.serial.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    Birth {
        position: glam::Vec3::from_array(s.position), velocity: glam::Vec3::from_array(s.velocity),
        normal: glam::Vec3::from_array(s.normal), radius: s.radius, lifetime: s.lifetime, wetness: s.wetness,
        kind: kind(s.kind), seed: (seed ^ seed >> 29) as u32, birth_time: time,
    }
}

impl World {
    pub(super) fn upload_wheel_debris(&mut self, gpu: &render::Gpu, renderer: &render::Renderer) -> Result<(), String> {
        let Some(fleet) = self.fleet.as_mut() else {
            if std::mem::take(&mut self.debris_live) { renderer.reset_wheel_particles(); }
            return Ok(());
        };
        let (reset, now, births) = fleet.take_wheel_debris();
        if reset { renderer.reset_wheel_particles(); }
        self.debris_live = true;
        let births: Vec<_> = births.iter().map(|(t, s)| birth(*t, s)).collect();
        // The pool keeps the newest; more than it holds would be overwritten anyway.
        let keep = births.len().saturating_sub(render::wheel_particles::MAX_PARTICLES);
        renderer.update_wheel_particles(gpu, now, &births[keep..])
    }
}
