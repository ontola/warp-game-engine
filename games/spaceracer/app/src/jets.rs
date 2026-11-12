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
    boost_arc_duration: f32, boost_arc_width: f32,
    sonic_speed: f32, sonic_reset_speed: f32, vapor_life: f32, vapor_opacity: f32,
    pub speed_kmh_per_unit: f32,
    boost_arc_spread: f32, boost_arc_height: f32,
    boost_width: f32, boost_radiance: f32, boost_release: f32, engine_glare_pixels: f32,
    warp_world_pixels: f32, warp_view_pixels: f32,
    emp_spark_fade: f32, sentry_spark_life: f32,
    damage_smoke_energy: f32,
    /// A weapon hit (energy lost at once, at least this much) sets a jet
    /// burning for hit_fire_seconds. Bumps and sentry fire stay below it.
    hit_fire_damage: f32, hit_fire_seconds: f32,
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
    exhaust_color: [f32;3],
    trail_radiance: f32,
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
            c.boost_arc_duration,c.boost_arc_width,
            c.sonic_speed,c.sonic_reset_speed,c.vapor_life,c.vapor_opacity,
            c.speed_kmh_per_unit,
            c.boost_arc_spread,c.boost_arc_height,c.boost_width,c.boost_radiance,c.boost_release,c.engine_glare_pixels,
            c.warp_world_pixels,c.warp_view_pixels,c.emp_spark_fade,c.sentry_spark_life,
            c.damage_smoke_energy,
            c.hit_fire_damage, c.hit_fire_seconds,
            c.respawn_light_power,
            c.idle_length,
            c.throttle_length,
            c.acceleration_length,
            c.boost_length,
            c.width,
            c.response,
            c.brake_response,
            c.radiance, c.trail_radiance,
            c.light_power,
            c.boost_light_power,
            c.trail_life,
            c.trail_interval,
            c.heat_pixels,
            c.boost_heat_pixels,
        ];
        if values.iter().any(|v| !v.is_finite() || *v < 0.)
            || c.hit_fire_damage<13. || c.hit_fire_seconds>3.
            || c.warp_view_pixels>64. || c.warp_world_pixels>64. || c.sentry_spark_life>0.25 || c.emp_spark_fade<=0.
            || c.boost_arc_start>40. || c.boost_arc_sustain>20. || c.boost_arc_spread>20. || c.boost_arc_height>12.
            || !(0.05..=1.).contains(&c.boost_arc_duration) || !(0.02..=0.5).contains(&c.boost_arc_width)
            || !(330.0..=520.).contains(&c.sonic_speed) || c.sonic_reset_speed>=c.sonic_speed
            || !(2.0..=3.6).contains(&c.speed_kmh_per_unit)
            || !(0.15..=0.8).contains(&c.vapor_life) || c.vapor_opacity>0.25
            || !(1.0..=3.).contains(&c.boost_width) || !(1.0..=4.).contains(&c.boost_radiance)
            || !(0.1..=1.5).contains(&c.boost_release)
            || !(0.0..=48.).contains(&c.engine_glare_pixels)
            || c.exhaust_color.iter().any(|v| !v.is_finite() || !(0.0..=1.).contains(v))
            || c.trail_radiance>0.25
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
    boost_until:f32,
    supersonic: bool,
    vapor_start: Option<f32>,
    power: f32,
    brake: f32,
    travel: f32,
    history: [VecDeque<(f32, Vec3, f32)>; 2],
}
impl Engine {
    fn record(
        &mut self,
        side: usize,
        at: Vec3,
        time: f32,
        life: f32,
        interval: f32,
        intensity: f32,
    ) {
        let h = &mut self.history[side];
        if h.back()
            .is_some_and(|&(t, p, _)| time < t || at.distance(p) > 120.)
        {
            h.clear();
        }
        while h.front().is_some_and(|&(t, _, _)| time - t > life) {
            h.pop_front();
        }
        if intensity>0.001 && h.back().is_none_or(|&(t, _, _)| time - t >= interval) {
            h.push_back((time, at,intensity));
        }
        // Bounded even if edited to the longest lifetime / smallest interval.
        while h.len() > 48 {
            h.pop_front();
        }
    }
}
struct Impact {target:usize,local:Vec3,normal:Vec3,birth:f32,seed:u32}
pub struct Jets {
    pub volume_plumes: bool,
    pub config: Config,
    engines: Vec<Engine>,
    sprites: Vec<Sprite>,
    heat: Vec<render::exhaust::Sprite>,
    pub lights: Vec<LocalLight>,
    time: f32,
    strike: u32,
    seen_shots: [u64; 6],
    impacts: VecDeque<Impact>,
    /// Per racer: energy last frame, seconds of hit fire left, burning side.
    hits: [(f64, f32, f32); 6],
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
/// Continuous surface patches across nose, cockpit, wings, nacelles and fins.
/// Each flash chooses a new patch and two nearby points, not a fixed hardpoint.
fn emp_patch(seed:u32)->(Vec3,Vec3,Vec3) {
    let side=if hash(seed.wrapping_add(41))<0.5 {-1.}else{1.};
    let (center,u,v,normal)=match (hash(seed.wrapping_add(43))*5.) as u32 {
        0=>(Vec3::new(0.,0.65,2.6),Vec3::X*0.85,Vec3::new(0.,-0.35,1.5),Vec3::Y),
        1=>(Vec3::new(0.,1.3,-0.2),Vec3::X*0.8,Vec3::new(0.,0.15,1.5),Vec3::Y),
        2=>(Vec3::new(side*3.1,0.25,-0.5),Vec3::new(side*1.4,0.,-0.4),Vec3::Z,Vec3::Y),
        3=>(Vec3::new(side*2.45,0.48,-2.7),Vec3::X*0.45,Vec3::Z*1.1,Vec3::new(side*0.4,0.9,0.).normalize()),
        _=>(Vec3::new(side*2.9,1.15,-2.1),Vec3::Y*0.65,Vec3::Z*0.7,Vec3::X*side),
    };
    let x=hash(seed.wrapping_add(47))*2.-1.;let y=hash(seed.wrapping_add(53))*2.-1.;
    let dx=(hash(seed.wrapping_add(59))*2.-1.)*0.7;
    let dy=(hash(seed.wrapping_add(61))*2.-1.)*0.7;
    let surface=center+normal*0.2;
    (surface+u*x+v*y,surface+u*(x+dx).clamp(-1.,1.)+v*(y+dy).clamp(-1.,1.),normal)
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

/// One bounded ray-integrated gas volume per engine, with octahedral axis packing.
fn plume(at: Vec3, axis: Vec3, length: f32, radius: f32, color: Vec3, time: f32) -> Sprite {
    let n=axis/axis.abs().element_sum();
    let mut oct=n.truncate();
    if n.z<0. {oct=glam::Vec2::new((1.-n.y.abs())*if n.x>=0. {1.}else{-1.},(1.-n.x.abs())*if n.y>=0. {1.}else{-1.});}
    let bound=((length*0.5).powi(2)+radius*radius).sqrt()*1.12;
    let mut s=glow(at+axis*length*0.5,[bound*2.;2],color,15);
    s.facing=render::sprites::Facing::Screen;s.depth_test=render::sprites::DepthTest::Shader;
    s.params=[oct.x,oct.y,length,time];s
}
impl Jets {
    pub fn new(root: &Path) -> Result<Self, String> {
        Ok(Self {
            config: Config::load(root)?,
            volume_plumes: true,
            engines: (0..6).map(|_| Engine::default()).collect(),
            sprites: Vec::with_capacity(1200),
            heat: Vec::with_capacity(24),
            lights: Vec::with_capacity(24),
            time: -1.,
            strike: 0,
            seen_shots: [0; 6],
            impacts: VecDeque::with_capacity(24),
            hits: [(100., 0., 1.); 6],
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
            let boost_thrust=alive && race.countdown<=0. && (p.boost>0. || p.on_pad) && input.brake<0.05;
            let target = if alive { (input.throttle*(1.-input.brake)) as f32 } else { 0. };
            let target=target.max(if boost_thrust {0.85}else{0.});
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
                && race.countdown<=0.
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
            if boosting {e.boost_until=time+c.boost_release;}
            if !alive || race.countdown>0. {e.boost_until=time;}
            let boost=smooth(0.,1.,(e.boost_until-time)/c.boost_release);
            let thrust=race.countdown<=0. && alive && target>0.015 && e.power>0.015;
            // Cosmetic Mach crossing: never changes boost/handling. Hysteresis
            // prevents repeated bursts while hovering around the sound barrier.
            if !alive || race.countdown>0. {e.supersonic=false;e.vapor_start=None;}
            else {
                if p.speed < c.sonic_reset_speed as f64 {e.supersonic=false;}
                if !e.supersonic && p.speed>=c.sonic_speed as f64 {
                    e.supersonic=true;e.vapor_start=Some(time);
                }
                if let Some(start)=e.vapor_start {
                    let age=(time-start)/c.vapor_life;
                    if age>=1. {e.vapor_start=None;}
                    else {
                        let envelope=smooth(0.,0.07,age)*(1.-smooth(0.3,1.,age));
                        // Thirty low-opacity billboards form a hollow turbulent
                        // cone, not a translucent solid mesh or full-screen fog.
                        for band in 0..3u32 {for sector in 0..10u32 {
                            let seed=self.strike.wrapping_mul(197).wrapping_add(i as u32*641+band*71+sector*31);
                            if hash(seed)<0.12 {continue;}
                            let t=(band as f32+0.5)/3.;
                            let angle=(sector as f32+hash(seed+1)*0.55)*std::f32::consts::TAU/10.+age*0.8;
                            let radius=(0.8+t*5.8)*(0.88+hash(seed+2)*0.24)*(1.+age*0.35);
                            let local=Vec3::new(angle.cos()*radius,angle.sin()*radius,2.-t*9.-age*3.+hash(seed+3));
                            let at=pose.origin+pose.basis*local;
                            let size=1.2+radius*0.55;
                            let mut cloud=glow(at,[size*(0.8+hash(seed+4)*0.5),size],Vec3::new(0.72,0.84,1.)*radiance*0.26,14);
                            cloud.facing=render::sprites::Facing::Screen;
                            cloud.additive=false;cloud.opacity=c.vapor_opacity*envelope*(0.6+hash(seed+5)*0.4);
                            cloud.params=[hash(seed+6)*20.,time,age,0.];
                            self.sprites.push(cloud);
                        }}
                    }
                }
            }
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
            // Charging on a recharge strip: a soft green-cyan glow under the hull
            // and motes drawn up from the strip into the craft.
            if p.charging && !p.airborne && !p.crashed {
                let pulse=0.75+0.25*(time*9.).sin();
                let energy=Vec3::new(0.3,1.,0.75);
                self.sprites.push(glow(base.transform_point3(Vec3::new(0.,0.3,-0.3)),[10.,8.],energy*radiance*2.4*pulse,2));
                for k in 0..14 {
                    let age=(time*1.8+k as f32/14.).rem_euclid(1.);
                    let x=(hash(i as u32*31+k)*2.-1.)*3.;
                    let z=(hash(i as u32*31+k+100)*2.-1.)*3.;
                    let at=base.transform_point3(Vec3::new(x*(1.-age*0.7),-0.2+age*1.6,z*(1.-age*0.7)));
                    let fade=smooth(0.,0.2,age)*(1.-smooth(0.7,1.,age));
                    self.sprites.push(glow(at,[0.9,0.9],energy*radiance*6.*fade,2));
                }
                self.lights.push(LocalLight{position:base.transform_point3(Vec3::new(0.,0.6,0.)),direction:pose.basis.y_axis,
                    intensity_cd:energy*c.respawn_light_power*0.4*pulse,range_m:9.,inner_cos:1.,outer_cos:-1.,source_radius_m:1.,falloff:2.});
            }
            // A rocket, glide bomb or railgun hit sets one jet burning briefly:
            // flickering flame at the nozzle under a short trail of dark smoke.
            if let Some(hit)=self.hits.get_mut(i) {
                if first || p.crashed || p.recovery>0. { hit.1=0.; }
                else if hit.0-p.energy>=c.hit_fire_damage as f64 { hit.1=c.hit_fire_seconds; hit.2=if hash(self.strike.wrapping_add(i as u32*13))<0.5 {-1.}else{1.}; }
                hit.0=p.energy;
                hit.1=(hit.1-dt).max(0.);
                if hit.1>0. {
                    let fade=smooth(0.,0.45,hit.1)*(1.-smooth(c.hit_fire_seconds-0.08,c.hit_fire_seconds,hit.1));
                    let side=hit.2;
                    let trail=(p.speed as f32*0.06).min(10.);
                    for k in 0..8 {
                        let age=(time*3.1+k as f32/8.).rem_euclid(1.);
                        let jitter=(k as f32*5.3+time*31.).sin()*0.25;
                        let at=base.transform_point3(Vec3::new(side*2.45+jitter,-0.1+age*1.2,-3.7-age*(2.+trail*0.35)));
                        let size=(2.2+age*2.)*(1.-age*0.45);
                        let flicker=0.75+0.25*(time*47.+k as f32*2.1).sin();
                        self.sprites.push(glow(at,[size,size*1.3],Vec3::new(1.,0.42,0.1)*radiance*4.*fade*flicker*(1.-age),2));
                    }
                    for k in 0..10 {
                        let age=(time*1.6+k as f32/10.).rem_euclid(1.);
                        let drift=(k as f32*7.19+age*3.).sin()*age;
                        let at=base.transform_point3(Vec3::new(side*2.45+drift,0.4+age*3.5,-4.2-age*(3.+trail)));
                        let size=1.4+age*4.;
                        let mut smoke=glow(at,[size,size],Vec3::new(0.22,0.2,0.19),10);
                        smoke.additive=false;smoke.opacity=fade*smooth(0.,0.15,age)*(1.-age)*0.8;
                        smoke.params=[k as f32+20.,time,0.,0.];self.sprites.push(smoke);
                    }
                    self.lights.push(LocalLight{position:base.transform_point3(Vec3::new(side*2.45,0.4,-4.)),direction:pose.basis.y_axis,
                        intensity_cd:Vec3::new(1.,0.45,0.12)*c.respawn_light_power*0.25*fade,range_m:10.,inner_cos:1.,outer_cos:-1.,source_radius_m:0.6,falloff:2.});
                }
            }

            if p.emp_time>0. && !p.crashed {
                let fade=smooth(0.,c.emp_spark_fade,p.emp_time as f32);
                for k in 0..7u32 {
                    let seed=self.strike.wrapping_mul(619).wrapping_add(i as u32*83+k*37);
                    if hash(seed)<0.22 {continue;}
                    let (start,end,normal)=emp_patch(seed);
                    let mut a=base.transform_point3(start);
                    for j in 1..=3u32 {
                        let t=j as f32/3.;let kink=if j==3 {0.}else{(hash(seed+j*19)-0.5)*0.65};
                        let b=base.transform_point3(start.lerp(end,t)+normal*(kink+0.25*(t*std::f32::consts::PI).sin()));
                        let mut arc=shaft(a,b,0.04+hash(seed+17)*0.04,Vec3::new(0.45,0.72,1.)*radiance*fade*2.2,5,[0.;4]);arc.min_pixels=2.;
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
                if e.power > 0.015 || (race.countdown>0. && alive) {
                    let engine_color=Vec3::from_array(c.exhaust_color);
                    let lamp_power=if race.countdown>0. {0.04+e.power*0.12}else{e.power};
                    let light = engine_color * radiance * lamp_power;
                    let plasma = light * (1. + boost * (c.boost_radiance - 1.));
                    if thrust && self.volume_plumes {self.sprites.push(plume(at,axis,length,
                        c.width*0.5*(1.+boost*(c.boost_width-1.+surge*0.15)),plasma,phase));}
                    let core = at + axis * 0.08;
                    let nozzle_size=1.1+boost*0.2;
                    let mut nozzle=glow(core,[nozzle_size;2],plasma*(2.+boost),2);
                    nozzle.facing=render::sprites::Facing::Screen;
                    nozzle.depth_test=render::sprites::DepthTest::Source;
                    self.sprites.push(nozzle);
                    // A bounded optical halo, not a giant world-space exhaust.
                    // Screen-facing sizing is per viewport; source-depth gating
                    // in effect 13 prevents enlarged halos leaking through walls.
                    if thrust {
                    let glare_size=1.4+boost*0.35;
                    let mut glare=glow(core,[glare_size;2],plasma*1.8,13);
                    glare.facing=render::sprites::Facing::Screen;
                    glare.depth_test=render::sprites::DepthTest::Source;
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
                    flare.depth_test=render::sprites::DepthTest::Source;
                    self.sprites.push(flare);
                    }
                    self.lights.push(LocalLight {
                        position: at + axis * 0.5,
                        direction: axis,
                        intensity_cd: engine_color
                            * lamp_power
                            * (c.light_power + boost * c.boost_light_power * (0.7 + surge * 0.3)),
                        range_m: 19. + boost * 9.,
                        inner_cos: 1.,
                        outer_cos: -1.,
                        source_radius_m: 0.7,
                        falloff: 2.,
                    });
                    if thrust {
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
                            light * 0.12,
                            4,
                            [k as f32 * 2.4, 0., 0., 0.],
                        );
                        w.opacity = (1. - age).powi(2) * smooth(0., 1., distance);
                        self.sprites.push(w);
                    }
                    if boosting && e.boost_start.is_some_and(|start| time-start<c.boost_arc_duration) {
                        // Two possible road strikes plus one short corona fork per
                        // nozzle. Replace each presentation: no faded bolt history.
                        for branch in 0..3u32 {
                            let seed=self.strike.wrapping_mul(151)
                                .wrapping_add(i as u32*83+side as u32*19+branch*71);
                            if branch>0 && hash(seed)<0.45 {continue;}
                            let reach=(c.boost_arc_sustain+(c.boost_arc_start-c.boost_arc_sustain)*onset)
                                *(0.55+hash(seed+2).powi(2)*0.65);
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
                                let mut arc=shaft(a,b,(0.035+onset*c.boost_arc_width)*(1.-t*0.45),
                                    light*(1.7+hash(seed+6)),5,[0.;4]);
                                arc.min_pixels=0.75+onset*1.75;
                                self.sprites.push(arc);a=b;
                            }
                            if grounded {
                                let f=road.frame(road_x);
                                let mut contact=glow(end,[0.65+onset;2],light*(0.18+onset*0.22),12);
                                contact.facing=render::sprites::Facing::Fixed {right:f.x_axis,up:f.z_axis};
                                self.sprites.push(contact);
                                if side==0 && branch==0 {self.lights.push(LocalLight {position:end+f.y_axis*0.45,direction:f.y_axis,intensity_cd:color*(100.+onset*340.),range_m:4.,inner_cos:1.,outer_cos:-1.,source_radius_m:0.4,falloff:2.});}
                            }
                        }
                    }
                }
                }
                if e.brake > 0.015 && race.countdown<=0. && alive {
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
                e.record(side, at, time, c.trail_life, c.trail_interval, boost);
                let h = &e.history[side];
                let mut tail = None;
                for &(t, point, strength) in h.iter().chain((boost>0.001).then_some(&(time, at,boost))) {
                    if let Some((old_t, old_p, old_strength)) = tail {
                        if point.distance_squared(old_p) > 0.0001 {
                            self.sprites.push(shaft(
                                point,
                                old_p,
                                0.65,
                                Vec3::from_array(c.exhaust_color) * radiance * c.trail_radiance,
                                6,
                                [
                                    (time - t) / c.trail_life,
                                    (time - old_t) / c.trail_life,
                                    strength,
                                    old_strength,
                                ],
                            ));
                        }
                    }
                    tail = Some((t, point,strength));
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
        for beam in race.beams.iter().filter(|b|b.rail) {
            let alpha=(time*120.-race.tick.saturating_sub(1) as f32).clamp(0.,1.);
            let age=(beam.age as f32-(1.-alpha)/120.).max(0.);let power=(1.-age/0.32).clamp(0.,1.).powi(2);
            let axis=(beam.to-beam.from).normalize_or_zero();
            let light=Vec3::new(0.25,0.7,1.)*radiance*power*8.;
            self.sprites.push(shaft(beam.from,beam.to,0.07+0.06*power,light*3.,11,[0.;4]));
            self.sprites.push(shaft(beam.from,beam.to,0.4+age*0.9,light,17,[0.;4]));
            let right=axis.any_orthonormal_vector();let up=axis.cross(right);
            for j in 0..5 {
                let radius=(0.7+age*7.)*(1.-j as f32*0.1);
                let mut ring=glow(beam.from.lerp(beam.to,(0.025+j as f32*0.12+age*0.8).min(0.98)),[radius*2.;2],light*0.3,16);
                ring.facing=render::sprites::Facing::Fixed{right,up};self.sprites.push(ring);
            }
            let contact=beam.target.is_some() || beam.from.distance(beam.to)<race.combat.rail_range-0.01;
            if contact && age<0.22 {
                let fade=(1.-age/0.22).powi(2);
                for k in 0..24u32 {
                    let seed=(beam.serial as u32).wrapping_mul(197).wrapping_add(beam.owner as u32*641+k*31);
                    let angle=hash(seed)*std::f32::consts::TAU;
                    let direction=(right*angle.cos()+up*angle.sin()-axis*(0.2+hash(seed+1)*0.65)).normalize();
                    let at=beam.to+direction*(0.2+age*(30.+hash(seed+2)*65.))-Vec3::Y*age*age*8.;
                    let mut spark=shaft(at,at-direction*(0.4+hash(seed+3)*1.5)*fade,0.035+hash(seed+4)*0.035,
                        Vec3::new(0.2,0.65,1.)*radiance*fade*18.,11,[0.;4]);
                    spark.min_pixels=1.;self.sprites.push(spark);
                }
            }
            for (at,scale) in [(beam.from,0.25+power*0.7),(beam.to,0.3+power*1.2)] {
                if at==beam.to && !contact {continue;}
                let mut flash=glow(at,[scale*8.;2],light,2);flash.facing=render::sprites::Facing::Screen;
                self.sprites.push(flash);
                self.lights.push(LocalLight {position:at,direction:axis,intensity_cd:Vec3::new(0.15,0.55,1.)*power*if at==beam.to {4500.}else{2500.},range_m:22.,inner_cos:1.,outer_cos:-1.,source_radius_m:0.2,falloff:2.});
            }
        }
        // Missile ignition, plume and continuous camera-faded smoke share the existing
        // sprite pass; no new lights/meshes per frame and no particle cloud dots.
        let alpha = (time * 120. - race.tick.saturating_sub(1) as f32).clamp(0., 1.);
        for p in &race.projectiles {
            if p.kind == spaceracer::race::Weapon::GlideBomb && p.lob.is_none() {
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
    fn boost_release_keeps_a_connected_fading_trail_then_expires() {
        let mut jets=Jets::new(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let mut race=Race::new(35,"normal","city",1,3);race.countdown=0.;
        race.pilots[0].boost=1.;race.drivers[0].input.drive.throttle=1.;
        let mut poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        jets.update(&race,&poses,1.);race.pilots[0].boost=0.;
        let mut last=1.;
        for n in 1..=5 {
            poses[0].origin+=Vec3::Z*10.;jets.update(&race,&poses,1.+n as f32*0.1);
            let strength=jets.engines[0].history[0].back().unwrap().2;
            assert!(strength>0. && strength<last);last=strength;
            assert!(jets.sprites.iter().any(|s|s.effect==6 && (s.params[2]-strength).abs()<0.001));
            let frozen=jets.sprites.clone();jets.update(&race,&poses,1.+n as f32*0.1);
            assert_eq!(jets.sprites,frozen,"pause must freeze the afterglow");
        }
        jets.update(&race,&poses,1.9);
        assert!(!jets.sprites.iter().any(|s|s.effect==6));
        race.pilots[0].boost=2.;jets.update(&race,&poses,2.);
        race.pilots[0].emp_time=2.;jets.update(&race,&poses,2.01);
        assert!(jets.engines[0].history.iter().all(VecDeque::is_empty));
        assert!(!jets.sprites.iter().any(|s|s.effect==6 || s.effect==15));
    }
    #[test]
    fn emp_flashes_sample_the_whole_hull_with_new_positions_each_frame() {
        let mut jets=Jets::new(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let mut race=Race::new(35,"normal","city",1,3);race.countdown=0.;race.pilots[0].emp_time=2.;
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();let pose=&poses[0];
        let mut positions=std::collections::HashSet::new();let mut lo=Vec3::splat(f32::INFINITY);let mut hi=-lo;
        let mut previous=Vec::new();
        for frame in 0..120 {
            let time=1.+frame as f32/120.;jets.update(&race,&poses,time);
            let starts:Vec<_>=jets.sprites.iter().filter(|s|s.effect==2).map(|s|s.center).collect();
            assert_ne!(starts,previous);previous=starts.clone();
            for at in starts {
                let local=pose.basis.transpose()*(at-pose.origin);lo=lo.min(local);hi=hi.max(local);
                assert!(positions.insert(at.to_array().map(f32::to_bits)),"reused EMP hardpoint");
            }
            let frozen=jets.sprites.clone();jets.update(&race,&poses,time);assert_eq!(jets.sprites,frozen);
        }
        assert!(lo.x< -4. && hi.x>4. && lo.z< -3.3 && hi.z>3.5 && hi.y>1.5,"insufficient coverage: {lo:?} {hi:?}");
    }
    #[test]
    #[ignore = "headless GPU diagnostic; run without a simultaneous game"]
    fn vapor_sprite_edges_gpu() {
        let gpu=render::Gpu::new().unwrap();
        render::sprites::set_game_sprite(&gpu,include_str!("../shaders/jets.wgsl")).unwrap();
        let mut scene=render::Scene::default();scene.graphics.minimal_forward=true;
        let material=scene.add_material(render::scene::Material{tint:[0.;3],layer:-1,roughness:1.,flags:render::scene::EMISSIVE,scale:1.,metal:0.});
        let mesh=scene.add_mesh(&render::primitives::quad(material).mesh());
        scene.add_instance(mesh,Affine3A::from_scale_rotation_translation(Vec3::new(20.,12.,1.),glam::Quat::IDENTITY,Vec3::new(0.,0.,-2.)));
        let light=render::Lighting{sun_lux:0.,sky_lux:0.,ev100:0.,..Default::default()};
        let renderer=render::Renderer::new(&gpu,&scene,&light);
        renderer.set_sprite_image(&gpu,IMAGE,&render::picture::Picture::solid(2,2,[255;4]),render::hud::ImageFilter::Nearest);
        let clouds:Vec<_>=(0..3).map(|i|{
            let mut cloud=glow(Vec3::new((i as f32-1.)*4.5,0.,0.),[4.;2],Vec3::splat(4.),14);
            cloud.facing=render::sprites::Facing::Screen;cloud.additive=false;cloud.opacity=0.5;
            cloud.params=[i as f32*7.23,1.,0.2,0.];cloud
        }).collect();
        renderer.set_sprites(&gpu,&clouds).unwrap();
        let camera=render::Camera{pos:Vec3::new(0.,0.,14.),target:Vec3::ZERO,fov_y:50_f32.to_radians(),near:0.1};
        let (pixels,_)=renderer.render(&gpu,&camera,&light,768,384);
        // The old cone produced 64-level neighbouring-pixel jumps on this
        // fixture. Soft billows must stay visible without those sharp seams.
        let mut peak=0u8;let mut gradient=0u8;
        for y in 1..384 {for x in 1..768 {
            let at=(y*768+x)*4;peak=peak.max(pixels[at]);
            gradient=gradient.max(pixels[at].abs_diff(pixels[at-4]))
                .max(pixels[at].abs_diff(pixels[at-768*4]));
        }}
        assert!(peak>40,"vapor disappeared: {peak}");
        assert!(gradient<20,"angular vapor seams: {gradient}");
        let path=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../build/vapor-sprite-probe.png");
        image::save_buffer(path,&pixels,768,384,image::ColorType::Rgba8).unwrap();
    }
    #[test]
    fn sonic_cone_crosses_once_fluctuates_freezes_and_dissipates_without_changing_boost() {
        let mut jets=Jets::new(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let mut race=Race::new(35,"normal","city",1,3);race.countdown=0.;
        race.pilots[0].speed=315.;race.pilots[0].boost=2.;
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        let vapor=|j:&Jets|j.sprites.iter().filter(|s|s.effect==14).cloned().collect::<Vec<_>>();
        jets.update(&race,&poses,0.);assert!(vapor(&jets).is_empty());
        race.pilots[0].speed=400.;jets.update(&race,&poses,0.1);assert!(vapor(&jets).is_empty(),"ordinary boost onset stays below the recalibrated barrier");
        assert!((jets.config.sonic_speed*jets.config.speed_kmh_per_unit-1234.8).abs()<0.1);
        race.pilots[0].speed=465.;jets.update(&race,&poses,1.);jets.update(&race,&poses,1.1);
        let first=vapor(&jets);assert!(!first.is_empty() && first.len()<=30);
        assert!(first.iter().all(|s|!s.additive && s.opacity<=0.25));
        jets.update(&race,&poses,1.1);assert_eq!(first,vapor(&jets));
        jets.update(&race,&poses,1.11);assert_ne!(first,vapor(&jets));
        jets.update(&race,&poses,1.8);assert!(vapor(&jets).is_empty());
        race.pilots[0].speed=450.;jets.update(&race,&poses,1.9);
        race.pilots[0].speed=460.;jets.update(&race,&poses,2.);assert!(vapor(&jets).is_empty());
        race.pilots[0].speed=315.;jets.update(&race,&poses,2.1);
        race.pilots[0].speed=465.;jets.update(&race,&poses,2.2);jets.update(&race,&poses,2.3);
        assert!(!vapor(&jets).is_empty());assert_eq!(race.pilots[0].boost,2.);
        race.pilots[0].crashed=true;jets.update(&race,&poses,2.4);assert!(vapor(&jets).is_empty());
    }
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
        let normal=jets.sprites.iter().find(|s|s.effect==15).unwrap().clone();
        race.pilots[0].boost=3.;jets.update(&race,&poses,0.01);
        let boosted=jets.sprites.iter().find(|s|s.effect==15).unwrap();
        assert!(boosted.size[0]>normal.size[0]*1.7 && boosted.radiance.length()>normal.radiance.length()*1.5);
        let glare=jets.sprites.iter().find(|s|s.effect==13).unwrap();
        assert!(matches!(glare.facing,render::sprites::Facing::Screen));
        assert!(glare.min_pixels>=18. && glare.min_pixels<24. && glare.depth_bias==0.);
        assert_eq!(glare.depth_test,render::sprites::DepthTest::Source);
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
        jets.update(&race,&poses,1.16);
        let sustain=jets.sprites.iter().find(|s|s.effect==12).unwrap().size[0];
        assert!(start>sustain*2.);
        jets.update(&race,&poses,1.33);
        assert!(!jets.sprites.iter().any(|s|s.effect==5 || s.effect==12),"lightning stops while the boost continues");
        assert!(jets.sprites.iter().any(|s|s.effect==15),"engine jet continues");
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
    fn a_weapon_hit_sets_a_jet_burning_for_a_moment() {
        let root=Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut jets=Jets::new(root).unwrap();let mut race=Race::new(31,"normal","city",1,3);
        let poses:Vec<_>=(0..6).map(|i|race.pose(i)).collect();
        jets.update(&race,&poses,1.);
        race.pilots[0].energy=88.;jets.update(&race,&poses,1.05);
        assert!(!jets.sprites.iter().any(|s|s.effect==10),"a bump does not burn");
        race.pilots[0].energy=38.;jets.update(&race,&poses,1.1);
        assert_eq!(jets.sprites.iter().filter(|s|s.effect==10).count(),10,"hit smoke");
        assert!(jets.sprites.iter().filter(|s|s.effect==2).any(|s|s.radiance.x>s.radiance.z*5.),"hit flame");
        for n in 1..=14 {jets.update(&race,&poses,1.1+n as f32*0.1);}
        assert!(!jets.sprites.iter().any(|s|s.effect==10),"the fire dies down after a second or so");
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
    fn countdown_has_only_lamps_then_thrust_volume_freezes_and_shuts_off_with_emp() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets");
        let mut jets = Jets::new(&root).unwrap();
        let mut race = Race::new(31, "normal", "city", 1, 3);
        race.drivers[0].input.drive.throttle = 1.;
        let poses: Vec<_> = (0..race.pilots.len()).map(|i| race.pose(i)).collect();
        jets.update(&race, &poses, 0.);
        assert_eq!(race.pilots[0].speed, 0.);
        assert_eq!(jets.lights.len(), 12); // small idle lamps on all six waiting ships
        assert!(jets.heat.is_empty());
        assert!(jets.sprites.iter().all(|s|s.effect==2));
        race.countdown = 0.;
        race.pilots[0].boost = 1.;
        jets.update(&race, &poses, 0.1);
        assert_eq!(jets.sprites.iter().filter(|s|s.effect==15).count(),2);
        assert!(jets.sprites.iter().filter(|s|s.effect==15).all(|s|s.depth_test==render::sprites::DepthTest::Shader));
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
        e.record(0, Vec3::ZERO, 1., 0.38, 0.016, 1.);
        e.record(0, Vec3::Z, 1., 0.38, 0.016, 1.);
        assert_eq!(e.history[0].len(), 1);
        e.record(0, Vec3::Z * 10., 1.1, 0.38, 0.016, 1.);
        assert_eq!(e.history[0].len(), 2);
        e.record(0, Vec3::Z * 400., 1.2, 0.38, 0.016, 1.);
        assert_eq!(e.history[0].len(), 1);
        e.record(0, Vec3::Z * 400., 1.6, 0.38, 0.016, 0.);
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
