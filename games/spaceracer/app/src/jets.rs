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
    boost_arc_start: f32, boost_arc_sustain: f32, boost_arc_decay: f32,
    boost_arc_spread: f32, boost_arc_height: f32,
    boost_width: f32, boost_radiance: f32, engine_glare_pixels: f32,
    warp_world_pixels: f32, warp_view_pixels: f32,
    emp_spark_fade: f32, sentry_spark_life: f32,
    damage_smoke_energy: f32,
    respawn_light_power: f32,
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
            c.boost_arc_start,c.boost_arc_sustain,c.boost_arc_decay,
            c.boost_arc_spread,c.boost_arc_height,c.boost_width,c.boost_radiance,c.engine_glare_pixels,
            c.warp_world_pixels,c.warp_view_pixels,c.emp_spark_fade,c.sentry_spark_life,
            c.damage_smoke_energy,
            c.respawn_light_power,
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
            || c.warp_view_pixels>64. || c.warp_world_pixels>64. || c.sentry_spark_life>0.25 || c.emp_spark_fade<=0.
            || c.boost_arc_start>40. || c.boost_arc_sustain>20. || c.boost_arc_spread>20. || c.boost_arc_height>12.
            || !(1.0..=3.).contains(&c.boost_width) || !(1.0..=4.).contains(&c.boost_radiance)
            || !(0.0..=48.).contains(&c.engine_glare_pixels)
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
    boost_start: Option<f32>,
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
struct Impact {target:usize,local:Vec3,normal:Vec3,birth:f32,seed:u32}
pub struct Jets {
    pub config: Config,
    engines: Vec<Engine>,
    sprites: Vec<Sprite>,
    heat: Vec<render::exhaust::Sprite>,
    pub lights: Vec<LocalLight>,
    time: f32,
    strike: u32,
    seen_shots: [u64; 6],
    impacts: VecDeque<Impact>,
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
            seen_shots: [0; 6],
            impacts: VecDeque::with_capacity(24),
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
            if !boosting {e.boost_start=None;} else if e.boost_start.is_none(){e.boost_start=Some(time);}
            let onset=e.boost_start.map_or(0.,|t| (-(time-t).max(0.)*c.boost_arc_decay).exp());
            let boost = if boosting { 1. } else { 0. };
            let color =
                Vec3::new(0.25, 0.7, 1.).lerp(Vec3::from_array(super::visuals::COLORS[i]), 0.45);
            let base = Affine3A::from_mat3_translation(pose.basis, pose.origin);
            if !p.crashed && p.recovery>0. {
                let alpha=((time*120.)-(race.tick.saturating_sub(1) as f32)).clamp(0.,1.);
                let progress=super::respawn::progress(race,i,alpha);
                let pulse=(std::f32::consts::PI*progress.powf(0.55)).sin().max(0.);
                self.lights.push(LocalLight {position:base.transform_point3(Vec3::new(0.,1.1,0.)),direction:pose.basis.y_axis,
                    intensity_cd:Vec3::new(0.3,0.8,1.)*c.respawn_light_power*pulse,range_m:15.,inner_cos:1.,outer_cos:-1.,source_radius_m:1.,falloff:2.});
            }
            if !p.crashed && p.recovery<=0. && p.energy < c.damage_smoke_energy as f64 {
                let amount=smooth(0.,c.damage_smoke_energy,c.damage_smoke_energy-p.energy as f32);
                for k in 0..12 {
                    let side=if k%2==0 {-1.}else{1.};
                    let age=(time*0.85+(k/2) as f32/6.+side*0.137).rem_euclid(1.);
                    let envelope=smooth(0.,0.12,age)*(1.-smooth(0.5,1.,age));
                    let drift=(k as f32*7.19+age*3.).sin()*age*0.8;
                    let at=base.transform_point3(Vec3::new(side*2.45+drift,0.9+age*if p.speed>40.{2.5}else{5.},-1.9-age*(p.speed as f32*0.07).min(12.)));
                    let size=1.2+age*3.2;
                    let mut smoke=glow(at,[size,size],Vec3::new(0.18,0.17,0.16),10);
                    smoke.additive=false;smoke.opacity=envelope*amount*0.42;
                    smoke.params=[k as f32,time,0.,0.];self.sprites.push(smoke);
                }
            }

            if p.emp_time>0. && !p.crashed {
                let fade=smooth(0.,c.emp_spark_fade,p.emp_time as f32);
                for k in 0..7u32 {
                    let seed=self.strike.wrapping_mul(619).wrapping_add(i as u32*83+k*37);
                    if hash(seed)<0.22 {continue;}
                    let side=if k%2==0 {-1.}else{1.};
                    let anchors=[Vec3::new(-2.45,0.65,-3.9),Vec3::new(2.45,0.65,-3.9),Vec3::new(-3.15,1.1,-1.),Vec3::new(3.15,1.1,-1.),Vec3::new(-4.4,0.3,0.),Vec3::new(4.4,0.3,0.),Vec3::new(0.,1.5,2.)];
                    let start=anchors[k as usize]+Vec3::Y*hash(seed+1)*0.15;
                    let end=start+Vec3::new(side*(0.4+hash(seed+4)),0.2+hash(seed+5)*0.45,hash(seed+6)-0.5);
                    let mut a=base.transform_point3(start);
                    for j in 1..=3u32 {
                        let t=j as f32/3.;let kink=if j==3 {0.}else{(hash(seed+j*19)-0.5)*0.65};
                        let b=base.transform_point3(start.lerp(end,t)+Vec3::Y*kink);
                        let mut arc=shaft(a,b,0.07,Vec3::new(0.45,0.72,1.)*radiance*fade*2.2,5,[0.;4]);arc.min_pixels=2.;
                        self.sprites.push(arc);a=b;
                    }
                    self.sprites.push(glow(base.transform_point3(start),[0.6,0.6],Vec3::new(0.5,0.8,1.)*radiance*fade,2));
                }
            }
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
                    let plasma = light * (1. + boost * (c.boost_radiance - 1.));
                    self.sprites.push(shaft(
                        at,
                        at + axis * length,
                        c.width * (1. + boost * (c.boost_width - 1. + surge * 0.25)),
                        plasma,
                        1,
                        [phase, boost, 0., 0.],
                    ));
                    let core = at + axis * 0.08;
                    let nozzle_size=2.4+boost*1.5;
                    self.sprites.push(glow(core, [nozzle_size;2], plasma * (2. + boost), 2));
                    // A bounded optical halo, not a giant world-space exhaust.
                    // Screen-facing sizing is per viewport; source-depth gating
                    // in effect 13 prevents enlarged halos leaking through walls.
                    let glare_size=3.5+boost*2.;
                    let mut glare=glow(core,[glare_size;2],plasma*1.8,13);
                    glare.facing=render::sprites::Facing::Screen;
                    glare.min_pixels=c.engine_glare_pixels*(1.+boost*0.45);
                    glare.params=[core.x,core.y,core.z,boost];
                    self.sprites.push(glare);
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
                        // Two possible road strikes plus one short corona fork per
                        // nozzle. Replace each presentation: no faded bolt history.
                        for branch in 0..3u32 {
                            let seed=self.strike.wrapping_mul(151)
                                .wrapping_add(i as u32*83+side as u32*19+branch*71);
                            if branch>0 && hash(seed)<0.45 {continue;}
                            let reach=(c.boost_arc_sustain+(c.boost_arc_start-c.boost_arc_sustain)*onset)
                                *(0.18+hash(seed+2).powi(2)*1.05);
                            let spread=c.boost_arc_spread*(0.45+onset*0.55);
                            let road=race.world.track.sample(p.distance-reach as f64);
                            let road_x=(p.x+sign as f64*(2.+hash(seed+3)*spread) as f64)
                                .clamp(-road.geometry.width*0.88,road.geometry.width*0.88);
                            let grounded=branch<2 && road.supported(road_x,0.5);
                            if branch<2 && !grounded {continue;}
                            let end=if grounded {road.point(road_x,0.12)} else {
                                at+axis*reach*0.35+pose.basis.x_axis*sign*(1.+hash(seed+7)*3.)
                                    +pose.basis.y_axis*(hash(seed+8)-0.3)*3.
                            };
                            let up=if grounded {road.frame(road_x).y_axis}else{pose.basis.y_axis};
                            let sideways=pose.basis.x_axis;
                            let crown=(at+end)*0.5+up*c.boost_arc_height*(0.3+onset*0.7)*(0.4+hash(seed+4))
                                +sideways*sign*(hash(seed+5)-0.25)*3.;
                            let mut a=at;
                            for segment in 1..=8u32 {
                                let t=segment as f32/8.;let envelope=(t*std::f32::consts::PI).sin();
                                // A high arch descends to a real deck contact. Fine
                                // asymmetric kinks ride the curve, not parallel rays.
                                let b=at*(1.-t).powi(2)+crown*(2.*t*(1.-t))+end*t*t
                                    +(sideways*(hash(seed+segment*23)-0.5)
                                        +up*(hash(seed+segment*47)-0.35))*envelope*(0.6+onset*1.8);
                                let mut arc=shaft(a,b,(0.075+onset*0.045)*(1.-t*0.45),
                                    light*(1.7+hash(seed+6)),5,[0.;4]);
                                arc.min_pixels=1.25;
                                self.sprites.push(arc);a=b;
                            }
                            if grounded {
                                let f=road.frame(road_x);
                                let mut contact=glow(end,[2.+onset*3.;2],light*(0.25+onset*0.35),12);
                                contact.facing=render::sprites::Facing::Fixed {right:f.x_axis,up:f.z_axis};
                                self.sprites.push(contact);
                                if side==0 && branch==0 {self.lights.push(LocalLight {position:end+f.y_axis*0.45,direction:f.y_axis,intensity_cd:color*(180.+onset*650.),range_m:7.,inner_cos:1.,outer_cos:-1.,source_radius_m:0.4,falloff:2.});}
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
        for blast in &race.blasts {
            if blast.kind==spaceracer::race::Weapon::Emp && blast.age<0.18 {
                let fade=(1.-blast.age as f32/0.18).powi(2);
                self.sprites.push(glow(blast.position+Vec3::Y*2.,[7.,7.],Vec3::new(0.4,0.8,1.)*radiance*fade*3.,2));
            }
        }
        // World lenses are shared by all cameras; view lenses are explicitly
        // restricted to the owner. Both refract scene colour before HUD composition.
        for (i, pose) in poses.iter().enumerate() {
            let power = race.drivers[i].warp_fx;
            if power > 0.01 && !race.pilots[i].crashed {
                let at = pose.origin + pose.basis * Vec3::new(0., 0.8, -1.);
                self.heat.push(render::exhaust::Sprite {
                    position_radius: at.extend(11.5).to_array(),
                    axis_length: [0., 0., 1., 7.],
                    colour_alpha: [0., 0., 0., power],
                    params: [3., 0., time, c.warp_world_pixels],
                });
                if i < 4 {
                    self.heat.push(render::exhaust::Sprite {
                        position_radius: at.extend(1.).to_array(),
                        axis_length: [0., 0., 0., 0.],
                        colour_alpha: [0., 0., 0., power],
                        params: [4., i as f32, time, c.warp_view_pixels],
                    });
                }
                for k in 0..7 {
                    let a = k as f32 * std::f32::consts::TAU / 7.;
                    let offset =
                        pose.basis.x_axis * a.cos() * 5. + pose.basis.y_axis * a.sin() * 3.;
                    let start = pose.origin + offset;
                    let end = start - pose.basis.z_axis * (14. + k as f32 * 1.3);
                    let mut wake = shaft(
                        start,
                        end,
                        0.32,
                        Vec3::new(0.45, 0.8, 1.) * radiance * power * 0.3,
                        8,
                        [time, k as f32, 0., 0.],
                    );
                    wake.opacity = power;
                    self.sprites.push(wake);
                }
            }
        }
        // Sentry shot IDs are consumed once per presented frame, not per tick.
        // Thus 30/60/120-Hz views all see a shot, but no tracer repeats next frame.
        for owner in 0..poses.len() {
            let Some(beam)=race.beams.iter().filter(|b|!b.rail && b.owner==owner && b.serial>self.seen_shots[owner]).max_by_key(|b|b.serial) else {continue;};
            self.seen_shots[owner]=beam.serial;
            let sim_pose=race.pose(owner);let pose=poses[owner];
            let from=pose.origin+pose.basis*(sim_pose.basis.transpose()*(beam.from-sim_pose.origin));
            let to=if let Some(target)=beam.target {
                let target_pose=poses[target];
                let toward=(target_pose.basis.transpose()*(from-target_pose.origin)).normalize_or_zero();
                // Strike the visible outer shell, not an invisible point in its centre.
                let radius=1./(toward/Vec3::new(3.2,1.0,4.4)).length().max(0.001);
                let local=toward*(radius+0.15)+Vec3::Y*0.45;
                self.impacts.push_back(Impact {target,local,normal:toward,birth:time,seed:beam.serial as u32});
                target_pose.origin+target_pose.basis*local
            } else {beam.to};
            let mut line=shaft(from,to,0.045,Vec3::new(1.,0.88,0.6)*radiance*2.,11,[0.;4]);
            line.min_pixels=2.;self.sprites.push(line);
            self.sprites.push(glow(from,[0.8,0.8],Vec3::new(1.,0.8,0.4)*radiance*5.,9));
            self.lights.push(LocalLight {position:from,direction:Vec3::Z,intensity_cd:Vec3::new(1.,0.7,0.3)*900.,range_m:7.,inner_cos:1.,outer_cos:-1.,source_radius_m:0.1,falloff:2.});
        }
        while self.impacts.len()>24 || self.impacts.front().is_some_and(|s|time-s.birth>c.sentry_spark_life || time<s.birth) {self.impacts.pop_front();}
        for impact in &self.impacts {
            if race.pilots[impact.target].crashed {continue;}
            let pose=poses[impact.target];let age=(time-impact.birth).max(0.);let fade=1.-age/c.sentry_spark_life;
            let at=pose.origin+pose.basis*impact.local;let outward=pose.basis*impact.normal;
            self.sprites.push(glow(at,[0.75,0.75],Vec3::new(1.,0.65,0.24)*radiance*fade*3.,2));
            for k in 0..5u32 {
                let seed=impact.seed.wrapping_mul(73).wrapping_add(k*17);
                let direction=(outward+pose.basis*Vec3::new(hash(seed)-0.5,hash(seed+1),hash(seed+2)-0.5)).normalize_or_zero();
                let start=at+direction*age*18.;
                let mut spark=shaft(start,start+direction*(0.25+age*4.),0.055,Vec3::new(1.,0.7,0.3)*radiance*fade*2.,11,[0.;4]);
                spark.min_pixels=1.2;self.sprites.push(spark);
            }
        }
        // Missile ignition, plume and continuous camera-faded smoke share the existing
        // sprite pass; no new lights/meshes per frame and no particle cloud dots.
        let alpha = (time * 120. - race.tick.saturating_sub(1) as f32).clamp(0., 1.);
        for p in &race.projectiles {
            if p.kind == spaceracer::race::Weapon::GlideBomb {
                let power = 1.
                    - smooth(
                        race.combat.bomb_burn - 0.15,
                        race.combat.bomb_burn,
                        p.time as f32,
                    );
                if power > 0.001 {
                    let axis = -p.velocity.normalize_or_zero();
                    let at = p.previous.lerp(p.position, alpha) + axis * 2.05;
                    let light = Vec3::new(1., 0.5, 0.16) * radiance * power;
                    self.sprites.push(shaft(
                        at,
                        at + axis * 4.5,
                        0.65,
                        light,
                        1,
                        [time, 0., 0., 0.],
                    ));
                    self.sprites.push(glow(at, [1.2, 1.2], light * 2., 2));
                }
                continue;
            }
            let power = smooth(0.18, 0.6, p.time as f32);
            let axis = -p.velocity.try_normalize().unwrap_or(Vec3::Z);
            let at = p.previous.lerp(p.position, alpha) + axis * 1.75;
            let light = Vec3::new(0.5, 0.75, 1.) * radiance * power * 2.;
            self.sprites.push(shaft(
                at,
                at + axis * (3. + power * 5.),
                0.65,
                light,
                1,
                [time, 0.2, 0., 0.],
            ));
            self.sprites.push(glow(at, [1.4, 1.4], light * 2., 2));
            let mut flare = glow(at, [6., 1.4], light, 3);
            flare.params = [at.x, at.y, at.z, 0.];
            self.sprites.push(flare);
            if power > 0.01 {
                self.lights.push(LocalLight {
                    position: at + axis * 0.4,
                    direction: axis,
                    intensity_cd: Vec3::new(0.5, 0.75, 1.) * power * c.boost_light_power,
                    range_m: 24.,
                    inner_cos: 1.,
                    outer_cos: -1.,
                    source_radius_m: 0.3,
                    falloff: 2.,
                });
            }
            let mut next = at;
            for (k, point) in p.trail.iter().rev().enumerate() {
                if next.distance_squared(*point) > 0.01 {
                    let age = k as f32 / 48.;
                    let mut smoke = shaft(
                        next,
                        *point,
                        0.7 + age * 7.,
                        Vec3::splat(2_f32.powf(race.environment.exposure) * 0.035),
                        7,
                        [age, (k + 1) as f32 / 48., time, 0.],
                    );
                    smoke.additive = false;
                    smoke.opacity = 0.22 * power;
                    self.sprites.push(smoke);
                }
                next = *point;
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
    fn newest_sentry_shot_only_and_impacts_outlive_the_single_frame_tracer() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR"));let mut jets=Jets::new(root).unwrap();
        let mut race=Race::new(35,"normal","city",1,3);
        for serial in 1..=4 {race.beams.push(spaceracer::race::Beam {owner:0,target:Some(1),serial,from:race.pose(0).origin,to:race.pose(1).origin,age:0.,rail:false});}
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        jets.update(&race,&poses,1.);
        assert_eq!(jets.sprites.iter().filter(|s|s.effect==9).count(),1);
        assert_eq!(jets.sprites.iter().filter(|s|s.effect==11 && s.min_pixels==2.).count(),1);
        assert_eq!(jets.impacts.len(),1);
        jets.update(&race,&poses,1.+1./120.);
        assert!(!jets.sprites.iter().any(|s|s.effect==9 || (s.effect==11 && s.min_pixels==2.)));
        assert_eq!(jets.sprites.iter().filter(|s|s.effect==11).count(),5);
        jets.update(&race,&poses,1.2);assert!(jets.impacts.is_empty());
    }
    #[test]
    fn boost_arcs_move_arch_and_end_at_road_contacts_with_bounded_glare() {
        let mut jets=Jets::new(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let mut race=Race::new(35,"normal","city",1,3);
        race.countdown=0.;race.drivers[0].input.drive.throttle=1.;
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        jets.update(&race,&poses,0.);
        let normal=jets.sprites.iter().find(|s|s.effect==1).unwrap().clone();
        race.pilots[0].boost=3.;jets.update(&race,&poses,0.01);
        let boosted=jets.sprites.iter().find(|s|s.effect==1).unwrap();
        assert!(boosted.size[0]>normal.size[0]*1.7 && boosted.radiance.length()>normal.radiance.length()*1.5);
        let glare=jets.sprites.iter().find(|s|s.effect==13).unwrap();
        assert!(matches!(glare.facing,render::sprites::Facing::Screen));
        assert!(glare.min_pixels>=30. && glare.min_pixels<48. && glare.depth_bias==0.);
        let mut previous=Vec::new();let mut spans=Vec::new();
        for frame in 0..13 {
            jets.update(&race,&poses,0.02+frame as f32/120.);
            let arcs:Vec<_>=jets.sprites.iter().filter(|s|s.effect==5).collect();
            assert!(!arcs.is_empty() && arcs.len()<=48);
            assert!(arcs.iter().any(|s|(s.center-poses[0].origin).dot(poses[0].basis.y_axis)>1.));
            let contacts:Vec<_>=jets.sprites.iter().filter(|s|s.effect==12).map(|s|s.center).collect();
            assert!(!contacts.is_empty());
            for contact in &contacts {
                // The last segment really terminates at the deck glow.
                assert!(arcs.iter().any(|s|if let render::sprites::Facing::Axis{up}=s.facing {
                    (s.center-up*s.size[1]*0.5).distance(*contact)<0.001
                }else{false}));
                spans.push(contact.distance(poses[0].origin));
            }
            assert_ne!(contacts,previous,"each presentation replaces its bolt geometry");
            previous=contacts;
        }
        let min=spans.iter().copied().fold(f32::INFINITY,f32::min);
        let max=spans.iter().copied().fold(0.,f32::max);
        assert!(max>min*1.8,"strikes must vary widely in reach");
    }

    #[test]
    fn boost_ground_contacts_decay_and_emp_sparks_fade_out() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR"));let mut jets=Jets::new(root).unwrap();
        let mut race=Race::new(35,"normal","city",1,3);race.countdown=0.;race.pilots[0].boost=3.;race.drivers[0].input.drive.throttle=1.;
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        jets.update(&race,&poses,1.);
        let start=jets.sprites.iter().find(|s|s.effect==12).unwrap().size[0];
        jets.update(&race,&poses,2.);
        let sustain=jets.sprites.iter().find(|s|s.effect==12).unwrap().size[0];
        assert!(start>sustain*2.);
        race.pilots[0].emp_time=2.;jets.update(&race,&poses,2.1);
        let bright=jets.sprites.iter().filter(|s|s.effect==5).map(|s|s.radiance.length()).fold(0.,f32::max);
        assert!(bright>0.);assert!(!jets.sprites.iter().any(|s|s.effect==12));
        race.pilots[0].emp_time=0.1;jets.update(&race,&poses,2.2);
        let dim=jets.sprites.iter().filter(|s|s.effect==5).map(|s|s.radiance.length()).fold(0.,f32::max);
        assert!(dim<bright*0.2);
        race.pilots[0].emp_time=0.;race.pilots[0].boost=0.;jets.update(&race,&poses,2.3);
        assert!(!jets.sprites.iter().any(|s|s.effect==5));
    }
    #[test]
    fn wear_smoke_has_threshold_freezes_and_clears_during_reconstruction() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut jets=Jets::new(root).unwrap();let mut race=Race::new(31,"normal","city",1,3);
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        race.pilots[0].energy=22.;jets.update(&race,&poses,1.);
        assert!(!jets.sprites.iter().any(|s|s.effect==10));
        race.pilots[0].energy=12.;jets.update(&race,&poses,1.1);
        assert_eq!(jets.sprites.iter().filter(|s|s.effect==10).count(),12);
        let sprites=jets.sprites.clone();jets.update(&race,&poses,1.1);assert_eq!(sprites,jets.sprites);
        race.reset(0);jets.update(&race,&poses,1.2);
        assert!(!jets.sprites.iter().any(|s|s.effect==10));
        race.pilots[0].recovery=0.;race.pilots[0].energy=80.;jets.update(&race,&poses,1.3);
        assert!(!jets.sprites.iter().any(|s|s.effect==10));
    }
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
        assert!(jets.sprites.iter().any(|s|s.effect==5));
        assert!(jets.lights.is_empty() && jets.heat.is_empty());
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
    fn minigun_events_render_once_and_warp_has_separate_world_and_view_lenses() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut jets = Jets::new(&root).unwrap();
        let mut race = Race::new(35, "normal", "city", 1, 3);
        race.beams.push(spaceracer::race::Beam {
            owner: 0,
            target: None,
            serial: 1,
            from: Vec3::ZERO,
            to: Vec3::Z * 60.,
            age: 0.1,
            rail: false,
        });
        race.drivers[0].warp_fx = 0.8;
        let poses: Vec<_> = (0..6).map(|i| race.pose(i)).collect();
        jets.update(&race, &poses, 1.);
        assert_eq!(jets.sprites.iter().filter(|s| s.effect == 9).count(), 1);
        assert!(jets.heat.iter().any(|s| s.params[0] == 3.));
        assert!(
            jets.heat
                .iter()
                .any(|s| s.params[0] == 4. && s.params[1] == 0.)
        );
        jets.update(&race, &poses, 1.001);
        assert!(!jets.sprites.iter().any(|s| s.effect == 9 || s.effect == 5));
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
