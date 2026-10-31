//! Game-owned exhaust presentation. No particle simulation or lights feed back
//! into the deterministic race. All views share world-space history, and each
//! view turns its own plume/flare quads. No crossed sheets or per-frame meshes.
use glam::{Affine3A, Vec3};
use render::{
    Gpu, Renderer,
    headlights::LocalLight,
    sprites::{Sprite, SpriteImage},
};
use script::{Host, Limits, mlua::LuaSerdeExt};
use serde::Deserialize;
use spaceracer::{handling::Input, race::Race, race_rules::Pose};
use std::{collections::VecDeque, path::Path};

const IMAGE: SpriteImage = SpriteImage(0x53524a4554);
const UV: [f32; 4] = [0., 0., 1., 1.];

#[derive(Deserialize)]
pub struct Config {
    nozzle: [f32; 3],
    reverse: [f32; 3],
    pub socket: [f32; 3],
    idle_length: f32,
    throttle_length: f32,
    acceleration_length: f32,
    boost_length: f32,
    width: f32,
    response: f32,
    brake_response: f32,
    radiance: f32,
    light_power: f32,
    boost_light_power: f32,
    trail_life: f32,
    trail_interval: f32,
    heat_pixels: f32,
    boost_heat_pixels: f32,
}
impl Config {
    fn load(root: &Path) -> Result<Self, String> {
        let mut host = Host::new(Limits::default(), |_, _| Ok(()))?;
        host.add_source(
            "jets.luau",
            &std::fs::read_to_string(root.join("../rules/jets.luau")).map_err(|e| e.to_string())?,
        )?;
        let value = host.modules()[0]
            .exports
            .as_ref()
            .ok_or("jets configuration missing")?
            .clone();
        let c: Self = host
            .lua()
            .from_value(script::mlua::Value::Table(value))
            .map_err(|e| e.to_string())?;
        let values = [
            c.idle_length,
            c.throttle_length,
            c.acceleration_length,
            c.boost_length,
            c.width,
            c.response,
            c.brake_response,
            c.radiance,
            c.light_power,
            c.boost_light_power,
            c.trail_life,
            c.trail_interval,
            c.heat_pixels,
            c.boost_heat_pixels,
        ];
        if values.iter().any(|v| !v.is_finite() || *v < 0.)
            || c.width <= 0.
            || !(0.02..=0.8).contains(&c.trail_life)
            || !(0.008..=0.1).contains(&c.trail_interval)
            || c.heat_pixels + c.boost_heat_pixels > 6.
            || c.nozzle
                .iter()
                .chain(&c.reverse)
                .chain(&c.socket)
                .any(|v| !v.is_finite())
        {
            return Err("invalid jet configuration".into());
        }
        Ok(c)
    }
}
#[derive(Default)]
struct Engine {
    power: f32,
    brake: f32,
    travel: f32,
    history: [VecDeque<(f32, Vec3)>; 2],
}
impl Engine {
    fn record(
        &mut self,
        side: usize,
        at: Vec3,
        time: f32,
        life: f32,
        interval: f32,
        enabled: bool,
    ) {
        let h = &mut self.history[side];
        if h.back()
            .is_some_and(|&(t, p)| time < t || at.distance(p) > 120.)
        {
            h.clear();
        }
        while h.front().is_some_and(|&(t, _)| time - t > life) {
            h.pop_front();
        }
        if enabled && h.back().is_none_or(|&(t, _)| time - t >= interval) {
            h.push_back((time, at));
        }
        // Bounded even if edited to the longest lifetime / smallest interval.
        while h.len() > 48 {
            h.pop_front();
        }
    }
}
pub struct Jets {
    pub config: Config,
    engines: Vec<Engine>,
    sprites: Vec<Sprite>,
    heat: Vec<render::exhaust::Sprite>,
    pub lights: Vec<LocalLight>,
    time: f32,
    strike: u32,
    uploaded: bool,
}
fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(747796405).wrapping_add(2891336453);
    x = ((x >> ((x >> 28) + 4)) ^ x).wrapping_mul(277803737);
    ((x >> 22) ^ x) as f32 / u32::MAX as f32
}
fn shaft(a: Vec3, b: Vec3, width: f32, color: Vec3, effect: u32, params: [f32; 4]) -> Sprite {
    let d = b - a;
    let mut s = Sprite::shaft(
        (a + b) * 0.5,
        [width, d.length().max(0.001)],
        -d.normalize_or_zero(),
        IMAGE,
        UV,
        color,
    );
    s.effect = effect;
    s.params = params;
    s.depth_bias = 0.;
    s
}
fn glow(at: Vec3, size: [f32; 2], color: Vec3, effect: u32) -> Sprite {
    let mut s = Sprite::billboard(at, size, IMAGE, UV, color);
    s.effect = effect;
    s.additive = true;
    s.alpha_cut = 0.;
    s.depth_bias = 0.;
    s
}
impl Jets {
    pub fn new(root: &Path) -> Result<Self, String> {
        Ok(Self {
            config: Config::load(root)?,
            engines: (0..6).map(|_| Engine::default()).collect(),
            sprites: Vec::with_capacity(1200),
            heat: Vec::with_capacity(24),
            lights: Vec::with_capacity(24),
            time: -1.,
            strike: 0,
            uploaded: false,
        })
    }
    pub fn update(&mut self, race: &Race, poses: &[Pose], time: f32) {
        let dt = (time - self.time).clamp(0., 0.1);
        let first = self.time < 0.;
        if time > self.time {
            self.strike = self.strike.wrapping_add(1);
        }
        self.time = time;
        self.sprites.clear();
        self.heat.clear();
        self.lights.clear();
        let c = &self.config;
        // Exposure-relative emission maintains an identifiable coloured envelope
        // in the day biomes, without overexposing the city.
        let radiance = 2_f32.powf(race.environment.exposure) * c.radiance;
        for (i, (p, pose)) in race.pilots.iter().zip(poses).enumerate() {
            let input = race.drivers[i].input.drive;
            let e = &mut self.engines[i];
            let alive = !p.crashed && p.recovery <= 0. && p.emp_time <= 0.;
            let target = if alive { input.throttle as f32 } else { 0. };
            if first || !alive {
                e.power = target;
            } else {
                e.power += (target - e.power) * (1. - (-dt * c.response).exp());
            }
            let brake = if alive { input.brake as f32 } else { 0. };
            if first || !alive {
                e.brake = brake;
            } else {
                e.brake += (brake - e.brake) * (1. - (-dt * c.brake_response).exp());
            }
            let boosting = (p.boost > 0. || p.on_pad)
                && !p.airborne
                && input.brake < 0.05
                && alive
                && e.power > 0.015;
            if !alive {
                for h in &mut e.history {
                    h.clear();
                }
            }
            let boost = if boosting { 1. } else { 0. };
            let color =
                Vec3::new(0.25, 0.7, 1.).lerp(Vec3::from_array(super::visuals::COLORS[i]), 0.45);
            let base = Affine3A::from_mat3_translation(pose.basis, pose.origin);
            let drive = if race.countdown <= 0. {
                smooth(0., 125., p.acceleration as f32)
            } else {
                0.
            };
            // Integrate rather than multiply absolute time by the changing speed.
            // The exhaust overtakes the camera, travelling faster than the racer.
            e.travel = (e.travel + dt * (p.speed as f32 + 220. + e.power * 160. + boost * 220.))
                .rem_euclid(18.);
            for side in 0..2 {
                let sign = if side == 0 { -1. } else { 1. };
                let at =
                    base.transform_point3(Vec3::new(c.nozzle[0] * sign, c.nozzle[1], c.nozzle[2]));
                let phase = time + i as f32 * 0.713 + side as f32 * 0.379;
                let surge = (0.5 + 0.5 * (phase * 23.7 + (phase * 9.1).sin() * 2.4).sin()).powi(4)
                    * 0.65
                    + (0.5 + 0.5 * (phase * 41.3).sin()).powi(8) * 0.35;
                let length = (c.idle_length
                    + e.power * c.throttle_length
                    + drive * e.power * c.acceleration_length
                    + boost * c.boost_length)
                    * (1. + boost * (surge - 0.3) * 0.35);
                let axis = -pose.basis.z_axis;
                if e.power > 0.015 {
                    let light = color * radiance * e.power;
                    self.sprites.push(shaft(
                        at,
                        at + axis * length,
                        c.width * (1. + boost * (0.2 + surge * 0.2)),
                        light,
                        1,
                        [phase, boost, 0., 0.],
                    ));
                    let core = at + axis * 0.08;
                    self.sprites
                        .push(glow(core, [2.4, 2.4], light * (2. + boost * 1.5), 2));
                    let mut flare = glow(
                        core,
                        [9. + boost * 3., 2.7 + boost * 0.9],
                        light * (0.6 + boost * 0.7),
                        3,
                    );
                    flare.params = [core.x, core.y, core.z, 0.];
                    self.sprites.push(flare);
                    self.lights.push(LocalLight {
                        position: at + axis * 0.5,
                        direction: axis,
                        intensity_cd: color
                            * e.power
                            * (c.light_power + boost * c.boost_light_power * (0.7 + surge * 0.3)),
                        range_m: 19. + boost * 9.,
                        inner_cos: 1.,
                        outer_cos: -1.,
                        source_radius_m: 0.7,
                        falloff: 2.,
                    });
                    self.heat.push(render::exhaust::Sprite {
                        position_radius: [at.x, at.y, at.z, 1.1],
                        axis_length: [axis.x, axis.y, axis.z, 4.],
                        colour_alpha: [0., 0., 0., e.power * 0.65],
                        params: [
                            1.,
                            i as f32 + side as f32 * 0.3,
                            time,
                            c.heat_pixels + boost * c.boost_heat_pixels,
                        ],
                    });
                    for k in 0..6 {
                        let distance = (e.travel + k as f32 * 3.).rem_euclid(18.);
                        let age = distance / 18.;
                        let jitter = pose.basis.x_axis * (phase * 11. + k as f32 * 2.4).sin()
                            + pose.basis.y_axis * (phase * 9. + k as f32 * 1.7).cos();
                        let pos = at + axis * distance + jitter * age * 0.45;
                        let mut w = shaft(
                            pos,
                            pos + axis * (1.2 + boost * 0.8),
                            0.24 + age * 0.24,
                            light * 0.5,
                            4,
                            [k as f32 * 2.4, 0., 0., 0.],
                        );
                        w.opacity = (1. - age).powi(2) * smooth(0., 1., distance);
                        self.sprites.push(w);
                    }
                    if boosting {
                        for branch in 0..5u32 {
                            let seed = self
                                .strike
                                .wrapping_mul(151)
                                .wrapping_add(i as u32 * 83 + side as u32 * 19 + branch * 71);
                            if hash(seed) < 0.60 {
                                continue;
                            }
                            let angle = hash(seed + 1) * std::f32::consts::TAU;
                            let reach = 3. + hash(seed + 2) * 6.;
                            let radial =
                                pose.basis.x_axis * angle.cos() + pose.basis.y_axis * angle.sin();
                            let mut a = at;
                            for segment in 1..=5 {
                                let t = segment as f32 / 5.;
                                let kink = (hash(seed + segment * 23) - 0.5)
                                    * (t * std::f32::consts::PI).sin();
                                let b = at + axis * (reach * t) + radial * (t * 0.8 + kink * 0.8);
                                self.sprites.push(shaft(
                                    a,
                                    b,
                                    0.10 * (1. - t * 0.6),
                                    light * 2.,
                                    5,
                                    [0.; 4],
                                ));
                                a = b;
                            }
                        }
                    }
                }
                if e.brake > 0.015 {
                    let at = base.transform_point3(Vec3::new(
                        c.reverse[0] * sign,
                        c.reverse[1],
                        c.reverse[2],
                    ));
                    let light = Vec3::new(1., 0.18, 0.025) * radiance * e.brake;
                    self.sprites.push(shaft(
                        at,
                        at - axis * (0.2 + e.brake * 2.8),
                        0.8,
                        light,
                        1,
                        [phase, 0., 1., 0.],
                    ));
                    self.sprites
                        .push(glow(at - axis * 0.06, [1.5, 1.5], light, 2));
                }
                e.record(side, at, time, c.trail_life, c.trail_interval, boosting);
                let h = &e.history[side];
                let mut tail = None;
                for &(t, point) in h.iter().chain(boosting.then_some(&(time, at))) {
                    if let Some((old_t, old_p)) = tail {
                        if point.distance_squared(old_p) > 0.0001 {
                            self.sprites.push(shaft(
                                point,
                                old_p,
                                0.65,
                                color * radiance * 0.38,
                                6,
                                [
                                    (time - t) / c.trail_life,
                                    (time - old_t) / c.trail_life,
                                    0.,
                                    0.,
                                ],
                            ));
                        }
                    }
                    tail = Some((t, point));
                }
            }
        }
    }
    pub fn flush(&mut self, gpu: &Gpu, renderer: &Renderer) -> Result<(), String> {
        if !self.uploaded {
            render::sprites::set_game_sprite(gpu, include_str!("../shaders/jets.wgsl"))?;
            renderer.set_sprite_image(
                gpu,
                IMAGE,
                &render::picture::Picture::solid(1, 1, [255; 4]),
                render::hud::ImageFilter::Linear,
            );
            self.uploaded = true;
        }
        renderer.set_sprites(gpu, &self.sprites)?;
        renderer.set_exhaust(gpu, &self.heat)
    }
}

/// Godot's source animation replaces selected YXZ Euler components; multiplying
/// a new rotation on the right incorrectly rolls the already swept/canted fins.
pub fn control_transform(name: &str, local: Affine3A, input: Input, brake: f32) -> Affine3A {
    if !["WingControl", "Rudder", "Airbrake"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return local;
    }
    let (scale, rot, pos) = local.to_scale_rotation_translation();
    let (mut y, mut x, mut z) = rot.to_euler(glam::EulerRot::YXZ);
    if name.starts_with("WingControl") {
        let side = if name.ends_with('L') { -1. } else { 1. };
        x = (-input.trim as f32 * 0.55 - input.strafe as f32 * side * 0.5 + brake * 0.6)
            .clamp(-0.85, 0.95);
        z = -input.strafe as f32 * 0.16;
    } else if name.starts_with("Rudder") {
        y = (input.steer.signum() * input.steer.abs().powf(0.72) * 0.7) as f32;
    } else {
        x = brake * 1.15;
    }
    Affine3A::from_scale_rotation_translation(
        scale,
        glam::Quat::from_euler(glam::EulerRot::YXZ, y, x, z),
        pos,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhaust_reacts_before_launch_freezes_and_shuts_off_with_emp() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
        let mut jets = Jets::new(&root).unwrap();
        let mut race = Race::new(31, "normal", "city", 1, 3);
        race.drivers[0].input.drive.throttle = 1.;
        let poses: Vec<_> = (0..race.pilots.len()).map(|i| race.pose(i)).collect();
        jets.update(&race, &poses, 0.);
        assert_eq!(race.pilots[0].speed, 0.);
        assert_eq!(jets.lights.len(), 2); // nozzle light before countdown ends
        assert!(!jets.heat.is_empty());
        race.countdown = 0.;
        race.pilots[0].boost = 1.;
        jets.update(&race, &poses, 0.1);
        let sprites = jets.sprites.clone();
        let heat = jets.heat.clone();
        let strike = jets.strike;
        jets.update(&race, &poses, 0.1);
        assert_eq!(sprites, jets.sprites);
        assert_eq!(heat, jets.heat);
        assert_eq!(strike, jets.strike);
        race.pilots[0].emp_time = 2.;
        jets.update(&race, &poses, 0.2);
        assert!(jets.sprites.is_empty() && jets.lights.is_empty() && jets.heat.is_empty());
        race.pilots[0].emp_time = 0.;
        race.pilots[0].boost = 0.;
        race.drivers[0].input.drive.throttle = 0.;
        race.drivers[0].input.drive.brake = 1.;
        jets.update(&race, &poses, 0.3);
        assert!(
            jets.sprites
                .iter()
                .any(|s| s.effect == 1 && s.params[2] == 1.)
        );
        race.pilots[0].crashed = true;
        jets.update(&race, &poses, 0.4);
        assert!(jets.sprites.is_empty());
    }
    #[test]
    fn trail_freezes_and_clears_on_teleport() {
        let mut e = Engine::default();
        e.record(0, Vec3::ZERO, 1., 0.38, 0.016, true);
        e.record(0, Vec3::Z, 1., 0.38, 0.016, true);
        assert_eq!(e.history[0].len(), 1);
        e.record(0, Vec3::Z * 10., 1.1, 0.38, 0.016, true);
        assert_eq!(e.history[0].len(), 2);
        e.record(0, Vec3::Z * 400., 1.2, 0.38, 0.016, true);
        assert_eq!(e.history[0].len(), 1);
        e.record(0, Vec3::Z * 400., 1.6, 0.38, 0.016, false);
        assert!(e.history[0].is_empty());
    }
    #[test]
    fn control_animation_preserves_authored_sweep_cant_and_hinges() {
        let pos = Vec3::new(2.6, 0.64, -2.94);
        let original = Affine3A::from_rotation_translation(glam::Quat::from_rotation_z(0.3), pos);
        let input = Input {
            steer: 1.,
            ..Default::default()
        };
        let out = control_transform("RudderR", original, input, 0.);
        let (y, x, z) = out
            .to_scale_rotation_translation()
            .1
            .to_euler(glam::EulerRot::YXZ);
        assert!((y - 0.7).abs() < 1e-5 && x.abs() < 1e-5 && (z - 0.3).abs() < 1e-5);
        assert_eq!(Vec3::from(out.translation), pos);
        let wing = Affine3A::from_rotation_translation(glam::Quat::from_rotation_y(0.4), pos);
        let out = control_transform(
            "WingControlR",
            wing,
            Input {
                trim: -1.,
                strafe: -1.,
                ..Default::default()
            },
            1.,
        );
        let (y, x, _) = out
            .to_scale_rotation_translation()
            .1
            .to_euler(glam::EulerRot::YXZ);
        assert!((x - 0.95).abs() < 1e-5 && (y - 0.4).abs() < 1e-5);
    }
}
