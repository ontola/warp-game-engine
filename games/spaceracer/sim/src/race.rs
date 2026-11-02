//! Playable race orchestration. The existing handling/flight kernels remain the
//! authority for movement; this layer owns countdown, pilots, gates and recovery.
use crate::{
    generation::Generated,
    handling::{BOOST_SPEED, DT, Input, Pilot},
    motion::Motion,
    race_rules::{self, Bump, Checkpoints, Gate, Pose},
};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Controls {
    pub rail_aim: Option<crate::railgun::Aim>,
    pub drive: Input,
    pub fire: bool,
    pub reset: bool,
    pub left: bool,
    pub right: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weapon {
    Missile,
    Warp,
    Sentry,
    Emp,
    Railgun,
    GlideBomb,
}
impl Weapon {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "missile" => Self::Missile,
            "warp" => Self::Warp,
            "sentry" => Self::Sentry,
            "emp" => Self::Emp,
            "railgun" => Self::Railgun,
            "glidebomb" => Self::GlideBomb,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Missile => "CRUISE MISSILE",
            Self::Warp => "WARP DRIVE",
            Self::Sentry => "SENTRY",
            Self::Emp => "EMP",
            Self::Railgun => "RAILGUN",
            Self::GlideBomb => "GLIDE BOMB",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiStyle {
    pub pace:f64, pub caution:f64, pub line:f64, pub strafe:f64,
    pub reserve:f64, pub boost_chance:f64, pub boost_cooldown:f64, pub phase:f64,
}
impl AiStyle {
    fn valid(&self)->bool {
        (0.8..=1.2).contains(&self.pace) && (0.75..=1.3).contains(&self.caution)
            && (-0.4..=0.4).contains(&self.line) && (0.1..=0.6).contains(&self.strafe)
            && (25.0..=80.).contains(&self.reserve) && (0.1..=1.).contains(&self.boost_chance)
            && (1.0..=10.).contains(&self.boost_cooldown) && (0.0..=5.).contains(&self.phase)
    }
}
#[derive(Clone, Debug)]
pub struct Driver {
    pub motion: Motion,
    pub checkpoint: Checkpoints,
    pub bump: Bump,
    pub name: String,
    pub bot: bool,
    pub rank: usize,
    pub finish_time: Option<f64>,
    pub victory: Option<crate::victory::Victory>,
    pub crash_time: f64,
    pub reset_time: f64,
    pub safe_distance: f64,
    pub input: Controls,
    pub item: Option<Weapon>,
    pub active: Option<Weapon>,
    pub active_time: f64,
    pub active_age: f64,
    pub warp_fx: f32,
    pub sentry_target: Option<usize>,
    pub shot_time: f64,
    pub shot_count: u64,
    pub ai_rng: u64,
    pub ai_style: AiStyle,
    pub ai_line_goal: f64,
    pub ai_boost_next: f64,
    pub ai_seek_item: bool,
    pub ai_next: f64,
    pub ai_line: f64,
    pub ai_pace: f64,
    pub boost_ready: f64,
    pub last_jink: f64,
    pub rail_preview:Option<crate::railgun::Trace>,
    pub rail_preview_at:f64,
}
#[derive(Clone, Debug)]
pub struct Pickup {
    pub distance: f64,
    pub x: f64,
    pub cooldown: f64,
}
#[derive(Clone, Debug)]
pub struct Projectile {
    pub owner: usize,
    pub target: Option<usize>,
    pub kind: Weapon,
    pub position: Vec3,
    pub previous: Vec3,
    pub trail: std::collections::VecDeque<Vec3>,
    pub launch: Pose,
    pub launch_speed: f32,
    pub travel: f32,
    pub owner_clear: bool,
    pub owner_blast_clear: bool,
    pub locked: bool,
    pub terminal: bool,
    pub evaded: bool,
    pub x: f64,
    pub velocity: Vec3,
    pub distance: f64,
    pub time: f64,
}
#[derive(Clone, Debug)]
pub struct Blast {
    pub position: Vec3,
    pub radius: f32,
    pub age: f64,
    pub kind: Weapon,
}
#[derive(Clone, Debug)]
pub struct Beam {
    pub target: Option<usize>,
    pub owner: usize,
    pub serial: u64,
    pub from: Vec3,
    pub to: Vec3,
    pub age: f64,
    pub rail: bool,
}
pub struct Race {
    pub rules: crate::rules::Rules,
    pub balance: crate::rules::Balance,
    pub combat: crate::combat::Config,
    pub script_error: Option<String>,
    pub world: Generated,
    pub environment: crate::environment::Environment,
    pub obstacles: Vec<crate::environment::Obstacle>,
    pub pilots: Vec<Pilot>,
    pub drivers: Vec<Driver>,
    pub gates: Vec<Gate>,
    pub biome: String,
    pub laps: u32,
    pub clock: f64,
    pub countdown: f64,
    pub over: bool,
    pub tick: u64,
    pub pickups: Vec<Pickup>,
    pub projectiles: Vec<Projectile>,
    pub blasts: Vec<Blast>,
    pub beams: Vec<Beam>,
    pub destroyed: Option<(usize, usize, f64)>,
}
impl Race {
    pub fn new(seed: i64, difficulty: &str, biome: &str, players: usize, laps: u32) -> Self {
        Self::with_rules(
            seed,
            difficulty,
            biome,
            players,
            laps,
            crate::rules::Rules::new(None).expect("bundled rules"),
        )
        .expect("bundled track")
    }
    pub fn with_rules(
        seed: i64,
        difficulty: &str,
        biome: &str,
        players: usize,
        laps: u32,
        mut rules: crate::rules::Rules,
    ) -> Result<Self, String> {
        let seed = seed.clamp(1, 99999);
        let balance = rules.balance()?;
        let combat = crate::combat::Config::load(&mut rules)?;
        let mut environment = rules.environment(biome)?;
        let world = Generated::with_recipe(seed, difficulty, biome, rules.track(seed, difficulty)?);
        environment.ground = environment.ground.min(
            world
                .track
                .nodes
                .iter()
                .map(|n| n.geometry.p.y)
                .fold(f32::INFINITY, f32::min)
                - environment.hills
                - 12.,
        );
        let gates = world.checkpoints();
        let pilots: Vec<_> = (0..6)
            .map(|i| Pilot {
                slot: i,
                distance: -((i / 2) as f64) * 24.,
                x: if i % 2 == 0 { -9. } else { 9. },
                ..Default::default()
            })
            .collect();
        let mut drivers=Vec::with_capacity(6);
        for i in 0..6 {
            let style:AiStyle=rules.call("personality",serde_json::json!({"seed":seed,"slot":i}))?;
            if !style.valid() {return Err("invalid AI personality".into());}
            drivers.push(Driver {
                motion: Motion::default(),
                checkpoint: Checkpoints::default(),
                bump: Bump::default(),
                name: if i < players {
                    format!("PILOT {}", i + 1)
                } else {
                    format!("NOVA {}", i + 1)
                },
                bot: i >= players,
                rank: i + 1,
                finish_time: None,
                victory: None,
                crash_time: 0.,
                reset_time: 0.,
                safe_distance: 0.,
                input: Controls::default(),
                item: None,
                active: None,
                active_time: 0.,
                active_age: 0.,
                warp_fx: 0.,
                sentry_target: None,
                shot_time: 0.,
                shot_count: 0,
                ai_rng: seed as u64 * 9176 + i as u64 * 104729 + 1,
                ai_next: 0.,
                ai_line: style.line,
                ai_line_goal: style.line,
                ai_pace: style.pace,
                ai_boost_next: style.phase,
                ai_seek_item: false,
                ai_style: style,
                boost_ready: 4. + i as f64 * 0.7,
                last_jink: -99.,
                rail_preview:None,rail_preview_at:-1.,
            });
        }
        let mut pickups = vec![];
        for k in 1..=(world.track.length / balance.pickup_spacing) as usize {
            let distance = k as f64 * balance.pickup_spacing;
            let n = world.track.sample(distance);
            if !n.air_gap && n.geometry.shape_angle < 0.1 && !n.is_loop {
                for x in [-0.55, 0., 0.55] {
                    if n.supported(x * n.geometry.width, 6.) {
                        pickups.push(Pickup {
                            distance,
                            x: x * n.geometry.width,
                            cooldown: 0.,
                        });
                    }
                }
            }
        }
        Ok(Self {
            world,
            combat,
            environment,
            obstacles: vec![],
            pilots,
            drivers,
            gates,
            biome: biome.into(),
            laps: laps.clamp(1, 9),
            clock: 0.,
            countdown: balance.countdown,
            over: false,
            tick: 0,
            rules,
            balance,
            script_error: None,
            pickups,
            projectiles: vec![],
            blasts: vec![],
            beams: vec![],
            destroyed: None,
        })
    }
    pub fn reload(&mut self) {
        let changed = self.rules.host.poll();
        for line in self.rules.host.drain_log() {
            eprintln!("rules: {line}");
        }
        if !changed.is_empty() {
            match self.rules.balance() {
                Ok(b) => {
                    self.balance = b;
                    match crate::combat::Config::load(&mut self.rules) {
                        Ok(c) => self.combat = c,
                        Err(e) => {
                            self.script_error = Some(e);
                            return;
                        }
                    }
                    self.script_error = None;
                }
                Err(e) => self.script_error = Some(e),
            }
        }
    }
    pub fn pose(&self, i: usize) -> Pose {
        self.drivers[i].victory.as_ref().map_or_else(
            || {
                self.drivers[i]
                    .motion
                    .pose(&self.pilots[i], &self.world.track, self.clock)
            },
            |v| v.pose,
        )
    }
    fn random(&mut self, i: usize) -> f64 {
        let a = &mut self.drivers[i].ai_rng;
        *a = (*a * 48271) % 2147483647;
        *a as f64 / 2147483647.
    }
    pub fn target(&self, i: usize, cone: f32, range: f32) -> Option<usize> {
        let pose = self.pose(i);
        (0..self.pilots.len())
            .filter(|&j| j != i && !self.pilots[j].crashed && !self.pilots[j].finished && self.pilots[j].warp_time<=0.)
            .filter_map(|j| {
                let d = self.pose(j).origin - pose.origin;
                let len = d.length();
                (len < range && d.normalize_or_zero().dot(pose.basis.z_axis) > cone)
                    .then_some((j, len))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }
    pub fn ai(&mut self, i: usize) -> Controls {
        if self.clock >= self.drivers[i].ai_next {
            let args = serde_json::json!({"r1":self.random(i),"r2":self.random(i),"r3":self.random(i),"r4":self.random(i),"clock":self.clock,"style":self.drivers[i].ai_style});
            match self.rules.call::<serde_json::Value>("plan", args) {
                Ok(v) => {
                    self.drivers[i].ai_line_goal = v["line"].as_f64().unwrap_or(0.).clamp(-0.4,0.4);
                    self.drivers[i].ai_seek_item = v["seek_item"].as_bool().unwrap_or(false);
                    self.drivers[i].ai_pace = v["pace"].as_f64().unwrap_or(1.);
                    self.drivers[i].ai_next = v["next"].as_f64().unwrap_or(self.clock + 5.);
                }
                Err(e) => {
                    self.script_error = Some(e);
                    return Controls::default();
                }
            }
        }
        self.drivers[i].ai_line=crate::handling::toward(self.drivers[i].ai_line,self.drivers[i].ai_line_goal,0.14*DT);
        let mut boost_request=false;
        if self.clock>=self.drivers[i].ai_boost_next {
            let args=serde_json::json!({"r1":self.random(i),"r2":self.random(i),"clock":self.clock,"style":self.drivers[i].ai_style});
            match self.rules.call::<serde_json::Value>("boost_plan",args) {
                Ok(v)=>{
                    let next=v["next"].as_f64().unwrap_or(self.clock+1.);
                    self.drivers[i].ai_boost_next=next.clamp(self.clock+0.1,self.clock+5.);
                    boost_request=v["attempt"].as_bool().unwrap_or(false) && self.clock>self.drivers[i].boost_ready;
                }
                Err(e)=>{self.script_error=Some(e);return Controls::default();}
            }
        }
        let p = &self.pilots[i];
        let d = &self.drivers[i];
        let track = &self.world.track;
        let pickup=if d.ai_seek_item && d.item.is_none() {
            self.pickups.iter().filter_map(|item| {
                let ahead=(item.distance-p.distance).rem_euclid(track.length);
                (item.cooldown<=0. && (10.0..350.).contains(&ahead))
                    .then_some((ahead+(item.x-p.x).abs()*2.,serde_json::json!({"ahead":ahead,"x":item.x})))
            }).min_by(|a,b|a.0.total_cmp(&b.0)).map(|v|v.1)
        }else{None};
        let n = track.sample(p.distance);
        let next = track.sample(p.distance + 100.);
        let preview: Vec<_> = [0., 30., 65., 110., 165.]
            .into_iter()
            .map(|ahead| {
                let ahead = ahead * (p.speed / 390.).powi(2).max(1.);
                let v = track.sample(p.distance + ahead);
                serde_json::json!({"ahead":ahead,"curve":v.curve,"hairpin":v.feature=="hairpin"})
            })
            .collect();
        let mut args = serde_json::json!({"p":p,"node":{"curve":n.curve,"width":n.geometry.width,"zone":n.zone,"split_gap":n.split_gap,"route":n.preferred_route},"preview":preview,"hard":track.difficulty=="hard","boost_speed":BOOST_SPEED,"line":d.ai_line,"pace":d.ai_pace,"style":d.ai_style,"pickup":pickup,"next_curve":next.curve,"next_route":next.preferred_route,"boost_ready":boost_request,"missed":d.checkpoint.missed,"has_item":d.item.is_some(),"fire_tick":self.tick%120==i as u64*17,
            "near_jump":self.world.jump_distances.iter().any(|j|(j[0]-p.distance).rem_euclid(track.length)<500.),"at_jump":self.world.jump_distances.iter().any(|j|(j[0]-p.distance).rem_euclid(track.length)<180.)});
        if p.airborne {
            let (at, _, _) = track.project(d.motion.air.position, p.distance, track.length);
            let mut destination = at + 100.;
            for j in &self.world.jump_distances {
                let local = p.distance.rem_euclid(track.length);
                if local >= j[0] - 30. && local < j[1] {
                    destination = destination
                        .max((p.distance / track.length).floor() * track.length + j[1] + 40.);
                }
            }
            let aim = track.sample(destination);
            let frame = d.motion.air.frame;
            let target = aim.point(d.ai_line * 6., -0.5);
            let desired = (target - d.motion.air.position).normalize_or_zero() * 220.;
            let goal = desired + (desired - d.motion.air.velocity) * 0.8;
            args["air_trim"] = serde_json::json!(
                (-goal
                    .dot(frame.y_axis)
                    .atan2(goal.dot(frame.z_axis).max(20.))
                    * 1.6)
                    .clamp(-1., 1.)
            );
            args["air_steer"] = serde_json::json!(
                (-goal
                    .dot(frame.x_axis)
                    .atan2(goal.dot(frame.z_axis).max(20.))
                    * 3.)
                    .clamp(-1., 1.)
            );
            args["air_roll"] = serde_json::json!(
                (-aim
                    .geometry
                    .basis()
                    .y_axis
                    .dot(frame.x_axis)
                    .atan2(aim.geometry.basis().y_axis.dot(frame.y_axis))
                    * 1.37)
                    .clamp(-1., 1.)
            );
            args["air_reset"] = serde_json::json!(
                d.motion.air.time > 7. && target.distance(d.motion.air.position) > 100.
            );
            args["air_fire"] = serde_json::json!(
                d.item == Some(Weapon::GlideBomb) && self.bomb_target(i).is_some()
            );
        }
        match self.rules.call::<Controls>("ai", args) {
            Ok(mut c) => {
                if self.drivers[i].item==Some(Weapon::Railgun) {
                    c.fire=(self.tick+i as u64*3)%19==0 && self.rail_ready(i) && self.rail_trace(i,None,true).target.is_some();
                }
                c
            },
            Err(e) => {
                self.script_error = Some(e);
                Controls::default()
            }
        }
    }
    pub fn respawn_location(&self, i: usize) -> (f64, f64) {
        let p = &self.pilots[i]; let d = &self.drivers[i];
        let distance = if d.checkpoint.missed {
            d.checkpoint.return_distance(p, &self.gates, self.world.track.length).unwrap_or(d.safe_distance)
        } else { d.safe_distance };
        let distance = self.world.safe_respawn(distance).max(0.);
        let n = self.world.track.sample(distance);
        (distance, if n.split_gap > 0.01 { (n.split_gap + n.geometry.width)*0.5 } else {0.})
    }
    pub fn respawn_pose(&self, i: usize) -> Pose {
        let (distance,x) = self.respawn_location(i);
        let n = self.world.track.sample(distance);
        Pose {origin:n.point(x,1.3),basis:n.frame(x)}
    }
    pub fn reset(&mut self, i: usize) {
        let (distance,x) = self.respawn_location(i);
        let p = &mut self.pilots[i]; let d = &mut self.drivers[i];
        p.distance = distance; p.x = x; p.trim = 0.;
        p.speed = 90.;
        p.heading = 0.;
        p.slip = 0.;
        p.airborne = false;
        p.crashed = false;
        p.lift = 0.;
        p.lift_speed = 0.;
        p.unload = 0.;
        p.slide = 0.;
        p.boost = 0.;
        p.energy = if d.crash_time > 0. {
            65.
        } else {
            (p.energy - 15.).max(20.)
        };
        p.launch_cooldown = 1.;
        p.recovery = 0.65;
        d.crash_time = 0.;
        d.reset_time = 0.;
        let crash_id = d.motion.crash_id;
        d.motion = Motion::default();
        d.motion.crash_id = crash_id;
        d.checkpoint.missed = false;
    }
    pub fn crash(&mut self, i: usize) {
        if self.pilots[i].crashed || self.pilots[i].finished {
            return;
        }
        let pose = self.pose(i);
        self.drivers[i].motion.air.position = pose.origin;
        self.drivers[i].motion.air.frame = pose.basis;
        self.drivers[i]
            .motion
            .crash(&mut self.pilots[i], Vec3::ZERO);
    }
    pub fn step(&mut self, controls: &[Controls]) {
        if self.script_error.is_some() {
            return;
        }
        self.tick += 1;
        if self.over {
            for i in 0..self.pilots.len() {
                if self.pilots[i].finished {
                    self.step_victory(i);
                }
            }
            return;
        }
        let mut input: Vec<_> = (0..self.pilots.len())
            .map(|i| {
                if self.pilots[i].finished {
                    Controls::default() // Finished drivers have their own racing policy.
                } else if self.drivers[i].bot {
                    self.ai(i)
                } else {
                    controls.get(i).copied().unwrap_or_default()
                }
            })
            .collect();
        for (d, c) in self.drivers.iter_mut().zip(&input) {
            if (c.drive.steer - d.input.drive.steer).abs() > 0.65 && c.drive.steer.abs() > 0.7 {
                d.last_jink = self.clock;
            }
            d.input = *c;
        }
        if self.countdown > 0. {
            self.countdown = (self.countdown - DT).max(0.);
            return;
        }
        self.clock += DT;
        let before: Vec<_> = (0..self.pilots.len()).map(|i| self.pose(i)).collect();
        race_rules::slipstream(
            &mut self.pilots,
            &before,
            self.world.track.length,
            0.,
            false,
            DT,
        );
        for i in 0..self.pilots.len() {
            if self.pilots[i].finished {
                self.step_victory(i);
                continue;
            }
            if self.pilots[i].crashed {
                self.drivers[i].crash_time += DT;
                if self.drivers[i].crash_time >= self.balance.respawn_seconds {
                    self.reset(i);
                }
                continue;
            }
            if input[i].reset && (self.pilots[i].airborne || self.drivers[i].checkpoint.missed) {
                self.reset(i);
                continue;
            }
            if self.pilots[i].recovery > 0. {
                self.pilots[i].recovery = (self.pilots[i].recovery - DT).max(0.);
                continue;
            }
            let warping =
                self.drivers[i].active == Some(Weapon::Warp) && self.pilots[i].emp_time <= 0.;
            if warping {
                if let Err(e) = self.step_warp(i) {
                    self.script_error = Some(e);
                    return;
                }
                input[i] = Controls {
                    drive: Input {
                        throttle: 1.,
                        ..Default::default()
                    },
                    ..Default::default()
                };
            } else {
                self.drivers[i].warp_fx *= (-DT as f32 * 12.).exp();
            }
            let p = &mut self.pilots[i];
            let d = &mut self.drivers[i];
            p.warp_time = if d.active == Some(Weapon::Warp) {
                d.active_time
            } else {
                0.
            };
            p.emp_time = (p.emp_time - DT).max(0.);
            d.checkpoint.flash = (d.checkpoint.flash - DT).max(0.);
            d.bump.begin(p, input[i].left, input[i].right, DT, 0.);
            p.bump_velocity = d.bump.velocity();
            if warping {
                // Dedicated ribbon autopilot has already integrated this tick.
            } else if p.airborne {
                d.motion.step_air(
                    p,
                    &self.world.track,
                    input[i].drive,
                    DT,
                    &self.biome,
                    -180.,
                    |a, b| self.environment.trace(&self.obstacles, a, b),
                );
            } else {
                let n = self.world.track.sample(p.distance);
                let launch = p.ground_step(input[i].drive, n.surface(), DT);
                let n = self.world.track.sample(p.distance);
                p.constrain(n.surface());
                d.motion.landing_blend = (d.motion.landing_blend - DT * 4.).max(0.);
                let pose = d.motion.pose(p, &self.world.track, self.clock);
                let velocity = (pose.origin - before[i].origin) / DT as f32;
                d.motion.ground_acceleration = (velocity - d.motion.ground_velocity) / DT as f32;
                d.motion.ground_velocity = velocity;
                let rot = Quat::from_mat3(&(before[i].basis.inverse() * pose.basis)).normalize();
                let (axis, angle) = rot.to_axis_angle();
                d.motion.air.rates = (axis * angle / DT as f32).clamp_length_max(3.);
                if !p.finished && (launch || !n.supported(p.x, 0.)) {
                    d.motion.launch(p, pose);
                } else if n.rails && !n.air_gap {
                    d.safe_distance = p.distance;
                }
            }
            let pose = d.motion.pose(p, &self.world.track, self.clock);
            d.checkpoint.advance(
                p,
                &self.gates,
                self.world.track.length,
                Some(before[i].origin),
                pose.origin,
            );
            if d.checkpoint.index < self.gates.len()
                && p.distance
                    > d.checkpoint
                        .progress(p, &self.gates, self.world.track.length)
                        + 90.
            {
                d.checkpoint.missed = true;
            }
            if p.distance >= p.lap as f64 * self.world.track.length {
                if d.checkpoint.index >= self.gates.len() {
                    p.lap += 1;
                    d.checkpoint = Checkpoints::default();
                    if p.lap > self.laps && !p.finished {
                        p.finished = true;
                        d.finish_time = Some(self.clock);
                    }
                } else {
                    d.checkpoint.missed = true;
                    p.distance -= self.world.track.length;
                }
            }
            p.refill(DT, self.balance.refill_rate);
            if input[i].drive.boost && p.boost > 0. {
                d.boost_ready = self.clock + d.ai_style.boost_cooldown;
            }
            if p.energy <= 0. && !p.finished {
                self.crash(i);
            }
        }
        for i in 0..self.pilots.len() {
            if self.pilots[i].finished && self.drivers[i].victory.is_none() {
                self.begin_victory(i);
            }
        }
        self.contacts();
        self.weapons(&input);
        let mut order: Vec<_> = (0..self.pilots.len()).collect();
        order.sort_by(
            |&a, &b| match (self.drivers[a].finish_time, self.drivers[b].finish_time) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => self.drivers[b]
                    .checkpoint
                    .progress(&self.pilots[b], &self.gates, self.world.track.length)
                    .total_cmp(&self.drivers[a].checkpoint.progress(
                        &self.pilots[a],
                        &self.gates,
                        self.world.track.length,
                    )),
            },
        );
        for (rank, i) in order.into_iter().enumerate() {
            self.drivers[i].rank = rank + 1;
        }
        self.over =
            self.pilots.iter().all(|p| p.finished) || self.clock >= self.balance.race_timeout;
    }
    fn contacts(&mut self) {
        // Resolve packs against rails repeatedly. EMP disables engines, not hulls.
        for _ in 0..10 {
            let mut touching=false;
            for a in 0..self.pilots.len() {
                for b in a+1..self.pilots.len() {
                    if !crate::contacts::solid(&self.pilots[a]) || !crate::contacts::solid(&self.pilots[b]) {continue;}
                    if self.pilots[a].airborne || self.pilots[b].airborne {
                        let separation=crate::contacts::air(self.pose(a),self.pose(b));
                        if separation.length_squared()<0.000001 {continue;}
                        touching=true;
                        let normal=separation.normalize();
                        // Never push a grounded ship vertically through the road.
                        let both=self.pilots[a].airborne && self.pilots[b].airborne;
                        let velocity=|i:usize|if self.pilots[i].airborne {self.drivers[i].motion.air.velocity}else{self.drivers[i].motion.ground_velocity};
                        let closing=(velocity(a)-velocity(b)).dot(normal);
                        let impulse=(-closing).max(0.)*if both {0.55}else{1.1};
                        for (i,sign) in [(a,1.),(b,-1.)] {
                            if !self.pilots[i].airborne {continue;}
                            let air=&mut self.drivers[i].motion.air;
                            air.position+=(separation+normal*0.004)*sign*if both {0.5}else{1.};
                            air.velocity+=normal*impulse*sign;
                        }
                        continue;
                    }
                    let n=self.world.track.sample(self.pilots[a].distance);
                    let separation=crate::contacts::ground(&self.pilots[a],&self.pilots[b],self.world.track.length,&n);
                    if separation.length_squared()<0.000001 {continue;}
                    touching=true;
                    let normal=separation.normalize();
                    let correction=separation*0.5+normal*0.002;
                    self.pilots[a].x+=correction.x;self.pilots[b].x-=correction.x;
                    self.pilots[a].distance+=correction.y;self.pilots[b].distance-=correction.y;
                    let velocity=|p:&Pilot|glam::DVec2::new(p.slip,p.speed*p.heading.cos());
                    let closing=(velocity(&self.pilots[a])-velocity(&self.pilots[b])).dot(normal);
                    if closing<0. {
                        let impulse=-closing*0.55;
                        for (i,sign) in [(a,1.),(b,-1.)] {
                            let p=&mut self.pilots[i];
                            p.slip+=normal.x*impulse*sign;
                            let forward=glam::DVec2::new(p.heading.sin(),p.heading.cos());
                            p.speed=(p.speed+normal.dot(forward)*impulse*sign).max(0.);
                        }
                    }
                    for (attacker,victim) in [(a,b),(b,a)] {
                        if self.drivers[attacker].bump.time>0. && self.pilots[attacker].emp_time<=0.
                            && !self.drivers[attacker].bump.hits.contains(&victim) {
                            self.drivers[attacker].bump.hits.push(victim);
                            self.damage(attacker,victim,12.,self.drivers[attacker].bump.side*55.);
                        }
                    }
                }
            }
            for p in &mut self.pilots {
                if !p.airborne && !p.crashed {p.constrain(self.world.track.sample(p.distance).surface());}
            }
            if !touching {break;}
        }
    }
    pub(crate) fn damage(&mut self, owner: usize, target: usize, amount: f64, impulse: f64) {
        if self.pilots[target].crashed
            || self.pilots[target].finished
            || self.pilots[target].warp_time > 0.
        {
            return;
        }
        let p = &mut self.pilots[target];
        p.energy = (p.energy - amount).max(0.);
        p.flash = 0.15;
        p.slip += impulse;
        p.slide = p.slide.max(0.65);
        if p.energy <= 0. {
            self.crash(target);
            self.destroyed = Some((owner, target, self.clock));
        }
    }
    fn explode(&mut self, owner: usize, position: Vec3, kind: Weapon) {
        let radius = if kind == Weapon::GlideBomb {
            self.balance.bomb_radius
        } else {
            self.balance.missile_radius
        };
        self.blasts.push(Blast {
            position,
            radius,
            age: 0.,
            kind,
        });
        for j in 0..self.pilots.len() {
            if j == owner {
                continue;
            }
            let distance = self.pose(j).origin.distance(position);
            if distance < radius {
                let damage = (if kind == Weapon::GlideBomb {
                    self.balance.bomb_damage
                } else {
                    self.balance.missile_damage
                }) * (1. - distance as f64 / radius as f64 * 0.7);
                self.damage(owner, j, damage, if j % 2 == 0 { 48. } else { -48. });
            }
        }
    }
    pub(crate) fn weapons(&mut self, input: &[Controls]) {
        for b in &mut self.blasts {
            b.age += DT;
        }
        self.blasts.retain(|b| b.age < 1.2);
        for b in &mut self.beams {
            b.age += DT;
        }
        self.beams.retain(|b| b.age < if b.rail {0.32}else{0.15});
        for k in 0..self.pickups.len() {
            self.pickups[k].cooldown = (self.pickups[k].cooldown - DT).max(0.);
            if self.pickups[k].cooldown > 0. {
                continue;
            }
            for i in 0..self.pilots.len() {
                let p = &self.pilots[i];
                let d = &self.drivers[i];
                if p.crashed || p.airborne || p.finished || d.item.is_some() {
                    continue;
                }
                let along = (p.distance - self.pickups[k].distance + self.world.track.length * 0.5)
                    .rem_euclid(self.world.track.length)
                    - self.world.track.length * 0.5;
                if along.abs() < 9. && (p.x - self.pickups[k].x).abs() < 7. {
                    let rank = d.rank;
                    let roll = self.random(i);
                    match self
                        .rules
                        .call::<String>("pickup", serde_json::json!({"rank":rank,"roll":roll}))
                    {
                        Ok(name) => self.drivers[i].item = Weapon::parse(&name),
                        Err(e) => self.script_error = Some(e),
                    }
                    self.pickups[k].cooldown = self.balance.pickup_seconds;
                    break;
                }
            }
        }
        for i in 0..self.pilots.len() {
            if self.rail_ready(i) {
                if self.clock>=self.drivers[i].rail_preview_at {
                    let aim=input.get(i).and_then(|c|c.rail_aim);
                    let quick=self.rail_trace(i,aim,false);
                    self.drivers[i].rail_preview=Some(if quick.target.is_some(){self.rail_trace(i,aim,true)}else{quick});
                    self.drivers[i].rail_preview_at=self.clock+0.075;
                }
            }else{self.drivers[i].rail_preview=None;self.drivers[i].rail_preview_at=-1.;}
            if self.pilots[i].crashed || self.pilots[i].finished {
                continue;
            }
            if input.get(i).is_some_and(|c| c.fire) {
                if let Some(kind) = self.drivers[i].item {
                    #[derive(Deserialize, Default)]
                    #[serde(default)]
                    struct Activation {
                        effect: String,
                        duration: f64,
                        cone: f32,
                        range: f32,
                        damage: f64,
                        speed: f32,
                    }
                    let p = &self.pilots[i];
                    let activation=self.rules.call::<Activation>("activate",serde_json::json!({"item":kind,"active":self.drivers[i].active.is_some(),"emp":p.emp_time,"airborne":p.airborne,"crashed":p.crashed,"finished":p.finished,"speed":p.speed}));
                    match activation {
                        Err(e) => self.script_error = Some(e),
                        Ok(a) if a.effect != "none" => {
                            let valid = a.duration.is_finite()
                                && (0.0..=60.0).contains(&a.duration)
                                && (-1.0..=1.0).contains(&a.cone)
                                && (0.0..=2000.0).contains(&a.range)
                                && (0.0..=100.0).contains(&a.damage)
                                && (0.0..=1000.0).contains(&a.speed);
                            if !valid {
                                self.script_error = Some("invalid weapon activation values".into());
                                continue;
                            }
                            let pose = self.pose(i);
                            match a.effect.as_str() {
                                "emp" => {
                                    for j in 0..self.pilots.len() {
                                        if j != i && self.pose(j).origin.distance_squared(pose.origin) <= a.range * a.range {
                                            self.interrupt_warp(j);
                                            self.pilots[j].emp_time = a.duration;
                                            self.pilots[j].boost = 0.;
                                        }
                                    }
                                    self.projectiles.retain(|p| p.kind != Weapon::Missile || p.position.distance_squared(pose.origin) > a.range * a.range);
                                    self.blasts.push(Blast {
                                        position: pose.origin,
                                        radius: a.range,
                                        age: 0.,
                                        kind,
                                    });
                                }
                                "active" => {
                                    self.drivers[i].active = Some(kind);
                                    self.drivers[i].active_time = a.duration;
                                    self.drivers[i].active_age = 0.;
                                    self.drivers[i].shot_time =
                                        self.clock + self.combat.sentry_spinup;
                                    if kind == Weapon::Warp {
                                        self.pilots[i].warp_time = a.duration;
                                        self.pilots[i].boost = 0.;
                                    }
                                }
                                "ray" => {
                                    let shot=self.rail_trace(i,input.get(i).and_then(|c|c.rail_aim),true);
                                    let target=shot.target;let to=shot.to;
                                    self.drivers[i].shot_count+=1;
                                    self.beams.push(Beam {
                                        target,
                                        owner: i,
                                        serial: self.drivers[i].shot_count,
                                        from: shot.from,
                                        to,
                                        age: 0.,
                                        rail: true,
                                    });
                                    if self.beams.iter().filter(|b|b.rail).count()>12 {
                                        if let Some(old)=self.beams.iter().position(|b|b.rail) {self.beams.remove(old);}
                                    }
                                    if let Some(j) = target {
                                        self.damage(i, j, a.damage, 25.);
                                    }
                                }
                                "projectile" => self.projectiles.push(Projectile {
                                    owner: i,
                                    target: if kind == Weapon::GlideBomb {
                                        self.bomb_target(i).map(|v| v.0)
                                    } else {
                                        self.target(i, a.cone, a.range)
                                    },
                                    kind,
                                    position: pose.origin + pose.basis * Vec3::new(0., 1.15, -1.8),
                                    previous: pose.origin + pose.basis * Vec3::new(0., 1.15, -1.8),
                                    trail: std::collections::VecDeque::with_capacity(48),
                                    launch: pose,
                                    launch_speed: self.pilots[i].speed as f32,
                                    travel: 0.,
                                    owner_clear: false,
                                    owner_blast_clear: false,
                                    locked: false,
                                    terminal: false,
                                    evaded: false,
                                    x: self.pilots[i].x,
                                    velocity: if kind == Weapon::GlideBomb {
                                        self.bomb_launch(i).1
                                    } else {
                                        pose.basis.z_axis * a.speed
                                    },
                                    distance: self.pilots[i].distance - 1.8,
                                    time: 0.,
                                }),
                                _ => {
                                    self.script_error =
                                        Some(format!("unknown weapon effect: {}", a.effect));
                                    continue;
                                }
                            }
                            self.drivers[i].item = None;
                        }
                        _ => {}
                    }
                }
            }
            if self.drivers[i].active_time > 0. {
                if self.drivers[i].active == Some(Weapon::Sentry) {
                    self.step_sentry(i);
                } else if self.drivers[i].active != Some(Weapon::Warp) {
                    self.drivers[i].active_time -= DT;
                }
                if self.drivers[i].active_time <= 0. {
                    self.drivers[i].active = None;
                }
            }
        }
        let mut pending = std::mem::take(&mut self.projectiles);
        for mut p in pending.drain(..) {
            p.time += DT;
            p.previous = p.position;
            let expired = if p.kind == Weapon::Missile {
                match self.guide_missile(&mut p) {
                    Ok(expired) => expired,
                    Err(e) => {
                        self.script_error = Some(e);
                        return;
                    }
                }
            } else {
                if let Some(at) = self.step_bomb(&mut p) {
                    self.bomb_explode(&p, at);
                    continue;
                }
                if p.time > self.balance.projectile_timeout { continue; }
                false
            };
            if p.trail
                .back()
                .is_none_or(|v| v.distance_squared(p.position) >= 36.)
            {
                if p.trail.len() >= 48 {
                    p.trail.pop_front();
                }
                p.trail.push_back(p.position);
            }
            let before = p.previous;
            let hit = p.time > 0.38
                && (0..self.pilots.len()).any(|j| {
                    if j == p.owner || self.pilots[j].crashed || self.pilots[j].finished {
                        return false;
                    }
                    let d = p.position - before;
                    let point = self.pose(j).origin;
                    let t = ((point - before).dot(d) / d.length_squared().max(0.001)).clamp(0., 1.);
                    point.distance(before + d * t) < 9.
                });

            if (hit && p.kind == Weapon::Missile)
                || expired
                || p.time > self.balance.projectile_timeout
            {
                self.explode(p.owner, p.position, p.kind);
            } else {
                self.projectiles.push(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contact_race() -> Race {
        let mut r=Race::new(35,"normal","city",1,3);
        for p in &mut r.pilots {p.recovery=3.;p.distance=200.;p.x=0.;p.heading=0.;p.speed=200.;}
        r.pilots[0].recovery=0.;r.pilots[1].recovery=0.;r
    }
    #[test]
    fn emp_hulls_separate_and_transfer_rear_impact_momentum() {
        for emp in [0.,3.] {
            let mut r=contact_race();r.pilots[0].distance=207.;r.pilots[0].emp_time=emp;
            r.pilots[1].speed=300.;r.contacts();
            assert!(r.pilots[0].distance-r.pilots[1].distance>=8.9);
            assert!(r.pilots[0].speed>200. && r.pilots[1].speed<300.);
            assert_eq!(r.pilots[0].x,0.,"rear hits must not teleport cars sideways");
        }
    }
    #[test]
    fn rotated_emp_hulls_and_three_wide_pack_resolve() {
        let mut r=contact_race();r.pilots[0].heading=0.7;r.pilots[0].x=7.;r.pilots[1].emp_time=3.;r.contacts();
        let n=r.world.track.sample(r.pilots[0].distance);
        assert!(crate::contacts::ground(&r.pilots[0],&r.pilots[1],r.world.track.length,&n).length()<0.02);
        let mut r=contact_race();r.pilots[2].recovery=0.;
        for i in 0..3 {r.pilots[i].x=(i as f64-1.)*8.;r.pilots[i].emp_time=3.;}
        r.contacts();
        for (a,b) in [(0,1),(1,2)] {assert!((r.pilots[a].x-r.pilots[b].x).abs()>9.38);}
    }
    #[test]
    fn warp_and_recovery_ghosts_are_deliberate_emp_is_not() {
        for recovery in [false,true] {
            let mut r=contact_race();
            if recovery {r.pilots[0].recovery=1.;}else{r.pilots[0].warp_time=1.;}
            r.contacts();assert_eq!(r.pilots[0].x,r.pilots[1].x);
            assert_eq!(r.pilots[0].distance,r.pilots[1].distance);
        }
        let mut r=contact_race();let pose=r.pose(1);r.pilots[0].airborne=true;r.pilots[0].emp_time=3.;
        r.drivers[0].motion.air.position=pose.origin;r.drivers[0].motion.air.frame=pose.basis;
        r.contacts();assert!(crate::contacts::air(r.pose(0),r.pose(1)).length()<0.002);
    }
    fn missile_race() -> Race {
        let mut r = Race::new(80385, "hard", "city", 1, 3);
        r.countdown = 0.;
        for (i, d) in r.drivers.iter_mut().enumerate() {
            d.bot = false;
            d.rank = i + 1;
        }
        for (i, p) in r.pilots.iter_mut().enumerate() {
            p.distance = if i == 0 { 0. } else { 400. + i as f64 * 80. };
            p.x = 0.;
            p.speed = 200.;
        }
        r.drivers[0].item = Some(Weapon::Missile);
        r.weapons(&[Controls {
            fire: true,
            ..Default::default()
        }]);
        r
    }
    #[test]
    fn missile_follows_banked_track_and_locks_only_near_impact() {
        let mut r = missile_race();
        let mut p = r.projectiles.remove(0);
        r.pilots[1].distance = 2500.;
        r.drivers[1].rank = 1;
        let mut saw_lock = false;
        for _ in 0..1800 {
            p.time += DT;
            r.pilots[1].distance += 180. * DT;
            r.guide_missile(&mut p).unwrap();
            if p.terminal {
                saw_lock = p.locked;
                break;
            }
            if p.time > 1.05 {
                assert!(
                    p.position
                        .distance(r.world.track.sample(p.distance).point(p.x, 10.))
                        < 0.01
                );
            }
        }
        assert!(
            saw_lock,
            "must catch the moving leader rather than orbit/cut through corners"
        );
    }
    #[test]
    fn missile_retargets_leader_until_laser_lock_then_keeps_target() {
        let mut r = missile_race();
        let mut p = r.projectiles.remove(0);
        r.drivers[2].rank = 1;
        r.drivers[1].rank = 4;
        p.time = 1.2;
        r.guide_missile(&mut p).unwrap();
        assert_eq!(p.target, Some(2));
        r.pilots[2].distance = p.distance + 60.;
        r.guide_missile(&mut p).unwrap();
        assert!(p.locked);
        r.drivers[3].rank = 0;
        r.guide_missile(&mut p).unwrap();
        assert_eq!(p.target, Some(2));
    }
    #[test]
    fn cruise_hits_moving_leader_after_previous_winner_finished() {
        let mut r = missile_race();
        r.pilots[1].finished = true;
        r.drivers[2].rank = 2;
        for _ in 0..1200 {
            r.clock += DT;
            for p in r.pilots.iter_mut().skip(2) {
                p.distance += 180. * DT;
            }
            r.weapons(&[]);
            if r.projectiles.is_empty() {
                break;
            }
        }
        assert!(r.projectiles.is_empty());
        assert!(r.pilots[2].energy < 100.);
        assert_eq!(r.pilots[1].energy, 100.);
    }
    #[test]
    fn emp_uses_world_radius_including_altitude_and_spares_distant_missiles() {
        let mut r=missile_race();let center=r.pose(0).origin;
        for (i,offset) in [(1,Vec3::X*99.),(2,Vec3::X*101.),(3,Vec3::Y*101.),(4,Vec3::Y*99.)] {
            r.pilots[i].airborne=true;r.drivers[i].motion.air.position=center+offset;
        }
        r.projectiles[0].position=center+Vec3::Z*101.;
        r.drivers[0].item=Some(Weapon::Emp);
        r.weapons(&[Controls {fire:true,..Default::default()}]);
        assert_eq!(r.pilots[0].emp_time,0.);
        for i in [1,4] {assert_eq!(r.pilots[i].emp_time,3.);}
        for i in [2,3] {assert_eq!(r.pilots[i].emp_time,0.);}
        assert_eq!(r.projectiles.len(),1);
        assert_eq!(r.blasts.last().unwrap().radius,100.);
    }
    #[test]
    fn untargeted_missile_fuses_and_emp_clears_inflight_missile() {
        let mut r = missile_race();
        for p in r.pilots.iter_mut().skip(1) {
            p.finished = true;
        }
        for _ in 0..480 {
            r.clock += DT;
            r.weapons(&[]);
        }
        assert!(r.projectiles.is_empty());
        let mut r = missile_race();
        r.pilots[1].distance=40.;
        r.drivers[1].item = Some(Weapon::Emp);
        r.weapons(&[
            Controls::default(),
            Controls {
                fire: true,
                ..Default::default()
            },
        ]);
        assert!(r.projectiles.is_empty());
        assert!(r.pilots[0].emp_time > 0.);
    }
    #[test]
    fn scripted_weapon_activation_preserves_queued_items_and_ground_bombs() {
        let mut r = Race::new(35, "easy", "city", 1, 1);
        r.countdown = 0.;
        let fire = Controls {
            fire: true,
            ..Default::default()
        };
        r.drivers[0].item = Some(Weapon::GlideBomb);
        r.step(&[fire]);
        assert_eq!(r.drivers[0].item, Some(Weapon::GlideBomb));
        assert!(r.projectiles.is_empty());
        r.drivers[0].item = Some(Weapon::Sentry);
        r.step(&[fire]);
        assert_eq!(r.drivers[0].active, Some(Weapon::Sentry));
        assert!(r.drivers[0].item.is_none());
        r.drivers[0].item = Some(Weapon::Emp);
        r.step(&[fire]);
        assert_eq!(r.drivers[0].item, Some(Weapon::Emp));
        assert_eq!(r.pilots[1].emp_time, 0.);
        r.drivers[0].active = None;
        r.drivers[0].item = Some(Weapon::Missile);
        for p in r.pilots.iter_mut().skip(1) {
            p.finished = true;
        }
        r.step(&[fire]);
        assert_eq!(r.projectiles.len(), 1);
        assert!(r.projectiles[0].target.is_none());
    }
    #[test]
    fn countdown_and_replay() {
        let mut a = Race::new(35, "normal", "city", 1, 1);
        let mut b = Race::new(35, "normal", "city", 1, 1);
        for tick in 0..2400 {
            let c = Controls {
                drive: Input {
                    throttle: 1.,
                    steer: if tick % 500 > 350 { 0.1 } else { 0. },
                    ..Default::default()
                },
                ..Default::default()
            };
            a.step(&[c]);
            b.step(&[c]);
            assert_eq!(
                serde_json::to_string(&a.pilots).unwrap(),
                serde_json::to_string(&b.pilots).unwrap()
            );
            if tick < 359 {
                assert_eq!(a.pilots[0].speed, 0.);
            }
        }
    }
    #[test]
    fn respawn_camera_target_matches_reset_even_after_missed_checkpoint() {
        let mut r=Race::new(80385,"hard","city",1,3);
        for missed in [false,true] {
            r.drivers[0].safe_distance=3400.;
            r.drivers[0].checkpoint.missed=missed;
            r.pilots[0].trim=-1.; r.pilots[0].heading=1.; r.pilots[0].lift=12.;
            let target=r.respawn_pose(0);r.reset(0);let actual=r.pose(0);
            assert!(target.origin.distance(actual.origin)<0.001);
            assert!(target.basis.z_axis.distance(actual.basis.z_axis)<0.001);
        }
    }
    #[test]
    fn crash_respawns_and_emp_spares_owner() {
        let mut r = Race::new(35, "easy", "city", 2, 1);
        r.countdown = 0.;
        r.drivers[0].item = Some(Weapon::Emp);
        r.step(&[Controls {
            fire: true,
            ..Default::default()
        }]);
        assert_eq!(r.pilots[0].emp_time, 0.);
        assert!(r.pilots[1].emp_time > 2.);
        r.crash(0);
        for _ in 0..241 {
            r.step(&[]);
        }
        assert!(!r.pilots[0].crashed);
        assert!(r.pilots[0].energy > 0.);
    }
    #[test]
    fn ai_styles_and_boost_decisions_are_distinct_and_seeded() {
        fn trace() -> (Vec<Vec<u64>>, Vec<Vec<Controls>>, Vec<[bool; 2]>) {
            let mut r = Race::new(35, "hard", "city", 0, 3);
            // Identical straight-line snapshots isolate driver decisions from
            // different grid positions, collisions and track timing.
            let distance = (0..(r.world.track.length as usize)).step_by(10)
                .map(|v| v as f64).find(|&s| {
                    [0.,30.,65.,100.,110.,165.].iter().all(|a| r.world.track.sample(s+a).curve.abs()<0.002)
                        && !r.world.jump_distances.iter().any(|j| (j[0]-s).rem_euclid(r.world.track.length)<500.)
                }).expect("safe straight for boost decisions");
            for p in &mut r.pilots {
                p.distance=distance; p.lap=2; p.speed=250.; p.energy=100.; p.x=0.;
            }
            let mut boosts=vec![Vec::new();6];
            let mut controls=vec![Vec::new();6];
            let mut pickup_choices=vec![[false;2];6];
            for tick in 0..1440 {
                r.clock=tick as f64*DT;
                for i in 0..6 {
                    let c=r.ai(i);
                    if c.drive.boost {boosts[i].push(tick);}
                    controls[i].push(c);
                    pickup_choices[i][r.drivers[i].ai_seek_item as usize]=true;
                }
                assert!(r.script_error.is_none(),"{:?}",r.script_error);
            }
            (boosts,controls,pickup_choices)
        }
        let (boosts,controls,choices)=trace();
        let (again,again_controls,_)=trace();
        assert_eq!(boosts,again,"seeded decisions must replay exactly");
        assert_eq!(serde_json::to_string(&controls).unwrap(),serde_json::to_string(&again_controls).unwrap());
        assert!(boosts.iter().all(|v| !v.is_empty()),"every driver uses boost: {boosts:?}");
        let first:std::collections::BTreeSet<_>=boosts.iter().map(|v|v[0]).collect();
        assert!(first.len()>=4,"boost decisions should not synchronize: {boosts:?}");
        for i in 0..6 {
            for j in i+1..6 {
                assert_ne!(boosts[i],boosts[j]);
                assert!(controls[i].iter().zip(&controls[j]).any(|(a,b)| (a.drive.steer-b.drive.steer).abs()>0.02),"identical cornering lines for {i}/{j}");
            }
        }
        assert!(choices.iter().any(|c| c[0]&&c[1]),"drivers must sometimes pass up pickups");
    }

    #[test]
    fn race_ai_progress() {
        for seed in [31, 33, 35] {
            let mut r = Race::new(seed, "normal", "city", 0, 1);
            for _ in 0..120 * 100 {
                r.step(&[]);
                if r.over {
                    break;
                }
            }
            assert!(r.script_error.is_none(), "{:?}", r.script_error);
            let finish = r.pilots.iter().filter(|p| p.finished).count();
            println!("seed {seed} finishers {finish}");
            assert!(finish >= 4, "seed {seed}: {finish}");
        }
    }
}
