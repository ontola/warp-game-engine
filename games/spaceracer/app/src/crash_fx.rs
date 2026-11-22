//! Cosmetic rigid pieces from the actual ship. Luau chooses launch/lifetime;
//! bounded native sweeps and interpolation never feed back into the race.
use glam::{Affine3A, Quat, Vec3};
use render::{
    Scene,
    fx::Effects,
    slots::{SlotId, Slots},
};
use script::{
    Host, Limits,
    mlua::{self, LuaSerdeExt},
};
use serde::Deserialize;
use spaceracer::{environment::Obstacle, race::Race};
use std::path::Path;
#[derive(Deserialize)]
pub struct Config {
    pub camera: spaceracer::chase::Tuning,
    pub blur_start_speed: f32,
    pub blur_full_speed: f32,
    pub blur_strength: f32,
    pub boost_blur: f32,
    pub shutter_seconds: f32,
    pub max_blur_pixels: f32,
    pub baked_explosion_smoke: bool,
    pub crash_radius: f32,
    pub explosion_light_scale: f32,
    debris_life: f32,
    max_pieces: usize,
    gravity: f32,
    drag: f32,
    restitution: f32,
    inherited_velocity: f32,
    max_inherited_speed: f32,
    flame_seconds: f32,
    flame_interval: f32,
}
#[derive(Deserialize)]
struct Launch {
    velocity: [f32; 3],
    axis: [f32; 3],
    spin: f32,
    life: f32,
    burn: bool,
}
struct Piece {
    mesh: usize,
    slot: SlotId,
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    rotation: Quat,
    previous_rotation: Quat,
    scale: Vec3,
    axis: Vec3,
    spin: f32,
    life: f32,
    burn: bool,
}
struct Wreck {
    owner: usize,
    birth: f32,
    tick: u32,
    flame_at: f32,
    pieces: Vec<Piece>,
    obstacles: Vec<Obstacle>,
    floor_position: Vec3,
    floor_normal: Vec3,
    floor_radius: f32,
}
pub struct Crashes {
    pub config: Config,
    host: Host,
    wrecks: Vec<Wreck>,
}
impl Crashes {
    pub fn new(root: &Path) -> Result<Self, String> {
        let mut host = Host::new(Limits::default(), |_, _| Ok(()))?;
        host.add_source(
            "effects.luau",
            &vfs::read_to_string(root.join("../rules/effects.luau"))
                .map_err(|e| e.to_string())?,
        )?;
        let table = host.modules()[0]
            .exports
            .as_ref()
            .ok_or("effects module")?
            .clone();
        let config: Config = host
            .lua()
            .from_value(table.get("config").map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let c = &config;
        if !c.camera.valid() {return Err("invalid chase camera configuration".into());}
        if [
            c.blur_start_speed,
            c.blur_full_speed,
            c.blur_strength,
            c.boost_blur,
            c.shutter_seconds,
            c.max_blur_pixels,
            c.crash_radius,
            c.explosion_light_scale,
            c.debris_life,
            c.gravity,
            c.drag,
            c.restitution,
            c.inherited_velocity,
            c.max_inherited_speed,
            c.flame_seconds,
            c.flame_interval,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.)
            || c.blur_full_speed <= c.blur_start_speed
            || c.max_pieces > 48
            || !(0.5..=8.).contains(&c.debris_life)
            || c.gravity > 100.
            || c.restitution > 1.
            || c.max_inherited_speed > 100.
            || c.flame_interval < 0.04
        {
            return Err("invalid effects configuration".into());
        }
        Ok(Self {
            config,
            host,
            wrecks: Vec::new(),
        })
    }
    pub fn active_pieces(&self) -> usize {
        self.wrecks.iter().map(|w| w.pieces.len()).sum()
    }
    pub fn spawn(
        &mut self,
        race: &Race,
        owner: usize,
        time: f32,
        parts: &[(usize, SlotId, Affine3A)],
        scene: &mut Scene,
        slots: &mut Slots,
    ) -> Result<(), String> {
        // One bounded set per pilot; a repeated crash recycles its old fragments.
        for w in self.wrecks.iter().filter(|w| w.owner == owner) {
            for p in &w.pieces {
                slots.set(
                    scene,
                    p.slot,
                    p.mesh,
                    Affine3A::from_scale(Vec3::splat(0.0001)),
                );
            }
        }
        self.wrecks.retain(|w| w.owner != owner);
        let driver = &race.drivers[owner];
        let at = race.pose(owner).origin;
        let incoming = (driver.motion.crash_velocity * self.config.inherited_velocity)
            .clamp_length_max(self.config.max_inherited_speed);
        let func: mlua::Function = self.host.modules()[0]
            .exports
            .as_ref()
            .unwrap()
            .get("shard")
            .map_err(|e| e.to_string())?;
        let mut pieces = Vec::new();
        for (i, &(mesh, slot, xf)) in parts.iter().take(self.config.max_pieces).enumerate() {
            let lua = self.host.lua();
            let context = lua
                .to_value(&serde_json::json!({
                    "index": i, "seed": driver.motion.crash_id * 131 + owner as u64 * 43,
                }))
                .map_err(|e| e.to_string())?;
            let value = func.call(context).map_err(|e| e.to_string())?;
            let launch: Launch = lua.from_value(value).map_err(|e| e.to_string())?;
            let velocity = Vec3::from(launch.velocity);
            let axis = Vec3::from(launch.axis).try_normalize().unwrap_or(Vec3::Y);
            if !velocity.is_finite()
                || velocity.length() > 100.
                || !axis.is_finite()
                || !launch.spin.is_finite()
                || launch.spin.abs() > 30.
                || !(0.2..=10.).contains(&launch.life)
            {
                return Err("invalid Luau fragment launch".into());
            }
            let (scale, rotation, position) = xf.to_scale_rotation_translation();
            pieces.push(Piece {
                mesh,
                slot,
                position,
                previous: position,
                velocity: incoming + velocity,
                rotation,
                previous_rotation: rotation,
                scale,
                axis,
                spin: launch.spin,
                life: launch.life,
                burn: launch.burn,
            });
        }
        let mut obstacles: Vec<_> = race
            .obstacles
            .iter()
            .copied()
            .filter(|b| {
                ((b.min + b.max) * 0.5 - at).length() < (b.max - b.min).length() * 0.5 + 220.
            })
            .collect();
        obstacles.sort_by(|a, b| {
            ((a.min + a.max) * 0.5 - at)
                .length_squared()
                .total_cmp(&((b.min + b.max) * 0.5 - at).length_squared())
        });
        obstacles.truncate(64);
        let node = race.world.track.sample(race.pilots[owner].distance);
        let normal = node.geometry.basis().y_axis;
        self.wrecks.push(Wreck {
            owner,
            birth: time,
            tick: 0,
            flame_at: time,
            pieces,
            obstacles,
            floor_position: node.geometry.p,
            floor_normal: normal,
            floor_radius: node.geometry.width as f32,
        });
        Ok(())
    }
    pub fn update(
        &mut self,
        race: &Race,
        time: f32,
        scene: &mut Scene,
        slots: &mut Slots,
        fx: &mut Effects,
    ) {
        const DT: f32 = 1. / 120.;
        let c = &self.config;
        for wreck in &mut self.wrecks {
            let age = (time - wreck.birth).max(0.);
            let desired = (age / DT).floor() as u32;
            // Physics state is evaluated on a fixed cosmetic clock then blended.
            while wreck.tick < desired.min(1200) {
                for p in &mut wreck.pieces {
                    p.previous = p.position;
                    p.previous_rotation = p.rotation;
                    p.velocity = (p.velocity - Vec3::Y * c.gravity * DT) * (-c.drag * DT).exp();
                    let mut end = p.position + p.velocity * DT;
                    if let Some((_, hit)) = wreck
                        .obstacles
                        .iter()
                        .filter_map(|o| o.trace(p.position, end))
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                    {
                        if p.velocity.dot(hit.normal) < 0. {
                            end = hit.position + hit.normal * 0.1;
                            bounce(&mut p.velocity, hit.normal, c.restitution);
                        }
                    }
                    let height = race.environment.height(end.x, end.z) + 0.35;
                    if end.y < height {
                        end.y = height;
                        bounce(&mut p.velocity, Vec3::Y, c.restitution);
                    }
                    // Local ribbon contact, bounded to the crash's road width.
                    let rel = end - wreck.floor_position;
                    let d = rel.dot(wreck.floor_normal);
                    let before = (p.position - wreck.floor_position).dot(wreck.floor_normal);
                    if before >= 0.1
                        && d < 0.35
                        && (rel - wreck.floor_normal * d).length() < wreck.floor_radius
                    {
                        end += wreck.floor_normal * (0.35 - d);
                        bounce(&mut p.velocity, wreck.floor_normal, c.restitution);
                    }
                    p.position = end;
                    p.rotation = Quat::from_axis_angle(p.axis, p.spin * DT) * p.rotation;
                }
                wreck.tick += 1;
            }
            let alpha = (age / DT).fract();
            let emit = time > wreck.flame_at + c.flame_interval && age < c.flame_seconds;
            if emit {
                wreck.flame_at = time;
            }
            for p in &wreck.pieces {
                let at = p.previous.lerp(p.position, alpha);
                let fade = ((p.life - age) / 0.5).clamp(0., 1.);
                slots.set(
                    scene,
                    p.slot,
                    p.mesh,
                    Affine3A::from_scale_rotation_translation(
                        p.scale * fade.max(0.0001),
                        p.previous_rotation.slerp(p.rotation, alpha),
                        at,
                    ),
                );
                if emit && p.burn && fade > 0. {
                    fx.burning_fragment(time, at, 0.7);
                }
            }
        }
        self.wrecks
            .retain(|w| time - w.birth < w.pieces.iter().map(|p| p.life).fold(0., f32::max) + 0.02);
    }
}
fn bounce(v: &mut Vec3, normal: Vec3, restitution: f32) {
    let n = v.dot(normal);
    if n < 0. {
        *v = (*v - normal * n) * 0.78 - normal * n * restitution;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragments_pause_survive_respawn_and_expire_without_render_leaks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
        let mut crashes = Crashes::new(&root).unwrap();
        let mut race = Race::new(31, "normal", "city", 1, 3);
        let mut scene = Scene::default();
        let mat = scene.add_material(render::scene::Material {
            tint: [0.6, 0.1, 0.1],
            layer: -1,
            roughness: 0.8,
            flags: 0,
            scale: 1.,
            metal: 0.,
        });
        let mesh = scene.add_mesh(&render::primitives::cube(mat).mesh());
        let mut slots = Slots::reserve(&mut scene, 1);
        let xf = Affine3A::from_translation(race.pose(0).origin + Vec3::Y * 15.);
        let slot = slots.add(&mut scene, mesh, xf).unwrap();
        let mut fx = Effects::default();
        crashes
            .spawn(&race, 0, 0., &[(mesh, slot, xf)], &mut scene, &mut slots)
            .unwrap();
        crashes.update(&race, 0.25, &mut scene, &mut slots, &mut fx);
        let before = crashes.wrecks[0].pieces[0].position;
        let rows = scene.instances[slots.instance(slot)].rows;
        for _ in 0..10 {
            crashes.update(&race, 0.25, &mut scene, &mut slots, &mut fx);
        }
        assert_eq!(
            before, crashes.wrecks[0].pieces[0].position,
            "paused physics"
        );
        assert_eq!(
            rows,
            scene.instances[slots.instance(slot)].rows,
            "paused interpolation"
        );
        race.pilots[0].crashed = false;
        crashes.update(&race, 1., &mut scene, &mut slots, &mut fx);
        assert_eq!(crashes.active_pieces(), 1, "respawn retains detached wreck");
        assert_ne!(before, crashes.wrecks[0].pieces[0].position);
        crashes.update(&race, 11., &mut scene, &mut slots, &mut fx);
        assert_eq!(
            crashes.active_pieces(),
            0,
            "expiry even across a long presentation gap"
        );
        let rows = scene.instances[slots.instance(slot)].rows;
        assert!(
            rows.iter()
                .all(|r| Vec3::new(r[0], r[1], r[2]).length() < 0.001),
            "expired geometry hidden"
        );
        crashes
            .spawn(&race, 0, 12., &[(mesh, slot, xf)], &mut scene, &mut slots)
            .unwrap();
        crashes
            .spawn(&race, 0, 12.1, &[(mesh, slot, xf)], &mut scene, &mut slots)
            .unwrap();
        assert_eq!(
            crashes.active_pieces(),
            1,
            "repeated crash replaces owned slots"
        );
    }
    #[test]
    fn impact_loses_energy_and_points_outward() {
        let mut v = Vec3::new(40., -20., 0.);
        let energy = v.length_squared();
        bounce(&mut v, Vec3::Y, 0.32);
        assert!(v.y > 0. && v.length_squared() < energy);
        let before = v;
        bounce(&mut v, Vec3::Y, 0.32);
        assert_eq!(v, before);
    }
}
