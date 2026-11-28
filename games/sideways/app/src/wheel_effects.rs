//! Read-only adapter from fixed-tick wheel contacts to shared visual effects.
//! Emission is sampled once per physics tick, never once per rendered view.
use glam::{Quat, Vec3};
use render::wheel_particles::{Birth, Kind};
use render::tracks::Stamp;
use sideways::{race::{Race, WEATHERS}, surfaces::Surface, track_marks, wheel_fx};

const CARS: usize = 4;
const WHEELS: usize = 16;
const HZ: f64 = 120.0;
const BIRTH_CAPACITY: usize = 4096;
const MARK_CAPACITY: usize = 32768;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tick { round: u32, frame: u64, noise: i32, detail: i32 }
impl Tick {
    fn follows(self, old: Self) -> bool {
        self.round == old.round && self.noise == old.noise && self.detail == old.detail
            && old.frame.checked_add(1) == Some(self.frame)
    }
}
#[derive(Clone, Copy)]
struct Pose { position: Vec3, rotation: Quat, velocity: Vec3, angular: Vec3, penalty: f64, ghost: f64 }
impl Pose {
    fn discontinuous(self, old: Self) -> bool {
        let travel = (self.position - old.position).length();
        let speed = self.velocity.length().max(old.velocity.length());
        let turn = self.rotation.angle_between(old.rotation).abs();
        !travel.is_finite() || !turn.is_finite() || self.penalty > old.penalty + 0.01
            || (self.ghost > 0.0 && old.ghost <= 0.0)
            || travel > 2.0 + speed / HZ as f32 * 2.0
            || turn > 0.5 + self.angular.length().max(old.angular.length()) / HZ as f32 * 2.0
    }
}
struct Input { pose: Pose, contacts: Vec<wheel_fx::Contact>, water: [Option<f32>; 4], stale: usize }

pub struct WheelEffects {
    enabled: bool,
    seed: u64,
    last: Option<Tick>,
    previous: [Option<Pose>; CARS],
    particles: wheel_fx::Emitter,
    tracks: track_marks::Emitter,
    births: Vec<Birth>,
    marks: Vec<Stamp>,
    contacts: Vec<wheel_fx::Contact>,
    reset_pending: bool,
    emitted: [u64; 8],
    by_surface: [u64; 8],
    marks_created: u64,
    births_expired: u64,
    births_dropped: u64,
    marks_dropped: u64,
    stale_contacts: u64,
    discontinuities: u64,
    ignored_cars: usize,
    births_uploaded: u64,
    marks_uploaded: u64,
    track_stats: Option<render::tracks::Stats>,
}

impl WheelEffects {
    pub fn new(seed: u64) -> Self {
        Self::with_enabled(seed, std::env::var("SIDEWAYS_WHEEL_EFFECTS").as_deref() != Ok("0"))
    }
    fn with_enabled(seed: u64, enabled: bool) -> Self {
        Self { enabled, seed, last: None, previous: [None; CARS],
            particles: wheel_fx::Emitter::new(seed), tracks: track_marks::Emitter::new(),
            births: Vec::with_capacity(BIRTH_CAPACITY), marks: Vec::new(), contacts: Vec::with_capacity(WHEELS),
            reset_pending: true, emitted: [0; 8], by_surface: [0; 8], marks_created: 0,
            births_expired: 0, births_dropped: 0, marks_dropped: 0, stale_contacts: 0,
            discontinuities: 0, ignored_cars: 0, births_uploaded: 0, marks_uploaded: 0, track_stats: None }
    }
    /// Keep the constructor's A/B setting across lifecycle resets.
    pub fn reset(&mut self, seed: u64) { *self = Self::with_enabled(seed, self.enabled); }

    /// Observe only after a real 120 Hz simulation step. Duplicate observations
    /// (pause, F12, split views) preserve both age and pending upload records.
    pub fn step(&mut self, race: &Race) {
        let tick = Tick { round: race.round, frame: race.frame, noise: race.stage.noise_seed, detail: race.stage.detail_seed };
        if self.last == Some(tick) { return; }
        self.ignored_cars = race.players.len().saturating_sub(CARS);
        if !self.enabled { self.last = Some(tick); return; }
        let inputs: Vec<_> = race.players.iter().take(CARS).enumerate().map(|(slot, player)| {
            let car = &player.car;
            let body = &race.phys.world.bodies[car.body];
            let t = body.position().translation;
            let r = body.position().rotation;
            let v = body.linvel(); let w = body.angvel(); let com = body.center_of_mass();
            let pose = Pose { position: Vec3::new(t.x,t.y,t.z), rotation: Quat::from_xyzw(r.x,r.y,r.z,r.w),
                velocity: Vec3::new(v.x,v.y,v.z), angular: Vec3::new(w.x,w.y,w.z), penalty: player.penalty, ghost: player.ghost };
            let com = Vec3::new(com.x,com.y,com.z);
            let mut input = Input { pose, contacts: Vec::with_capacity(4), water: [None; 4], stale: 0 };
            for (wi, wheel) in car.wheels.iter().take(4).enumerate() {
                if !wheel.contact { continue; }
                let point = Vec3::new(wheel.point.x,wheel.point.y,wheel.point.z);
                let anchor = Vec3::new(wheel.anchor.x,wheel.anchor.y,wheel.anchor.z);
                let hub = pose.position + pose.rotation * anchor;
                // Contacts are computed before the final physics step, so allow
                // one tick's point motion. Teleport() leaves old contacts set.
                let movement = (pose.velocity.length() + pose.angular.length() * anchor.length()) / HZ as f32;
                let reach = car.spec.travel as f32 + 0.25 + wheel.radius as f32 + movement + 0.15;
                if !point.is_finite() || !hub.is_finite() || point.distance(hub) > reach {
                    input.stale += 1; continue;
                }
                let mut c = wheel_fx::Contact::from_wheel((slot*4+wi) as u8, wheel,
                    point_velocity(pose.velocity, pose.angular, com, point).to_array());
                let level = if race.stage.rivers.is_empty() { f64::NEG_INFINITY }
                    else { race.stage.water_level(point.x as f64, point.z as f64) };
                let dry = if c.surface == Surface::Water && !(level > point.y as f64 + 0.08) {
                    race.stage.surface_near(gdcompat::math::v3(point.x,point.y,point.z), wheel.road_hint).0
                } else { c.surface };
                c.surface = contact_surface(dry, point.y, level);
                if c.surface == Surface::Water && level.is_finite() { input.water[wi] = Some(level as f32); }
                input.contacts.push(c);
            }
            input
        }).collect();
        let wetness = WEATHERS.iter().find(|(name,_)| *name == race.weather).map_or(0.0, |(_,w)| *w as f32);
        self.observe(tick, &inputs, wetness);
    }

    fn observe(&mut self, tick: Tick, inputs: &[Input], wetness: f32) {
        if self.last == Some(tick) { return; }
        let continuous = self.last.is_some_and(|old| tick.follows(old));
        if !continuous {
            self.particles.reset(hash(self.seed ^ (tick.round as u64).rotate_left(17)));
            self.tracks.reset(); self.births.clear(); self.marks.clear(); self.contacts.clear();
            self.previous = [None; CARS]; self.reset_pending = true;
        }
        self.last = Some(tick);
        self.expire_births();
        self.contacts.clear();
        let mut water = [None; WHEELS];
        for slot in 0..CARS {
            let input = inputs.get(slot);
            let cut = input.is_none_or(|p| self.previous[slot].is_none_or(|old| p.pose.discontinuous(old)));
            if continuous && cut {
                for wi in 0..4 { self.particles.reset_wheel((slot*4+wi) as u8); self.tracks.reset_wheel((slot*4+wi) as u8); }
                if input.is_some() && self.previous[slot].is_some() { self.discontinuities += 1; }
            }
            self.previous[slot] = input.map(|i| i.pose);
            if let Some(input) = input {
                self.stale_contacts += input.stale as u64;
                if continuous && !cut {
                    self.contacts.extend(input.contacts.iter().copied());
                    water[slot*4..slot*4+4].copy_from_slice(&input.water);
                }
            }
        }
        if !continuous { return; }
        let births = self.particles.tick(tick.frame, &self.contacts, wetness);
        for spawn in births {
            let kind = kind(spawn.kind);
            self.emitted[kind_index(spawn.kind)] += 1;
            self.by_surface[spawn.surface as usize] += 1;
            let mut position = Vec3::from_array(spawn.position);
            if spawn.kind == wheel_fx::Kind::WaterDrop {
                if let Some(level) = water[spawn.wheel_id as usize] { position.y = position.y.max(level + spawn.radius); }
            }
            self.births.push(Birth { position, velocity: Vec3::from_array(spawn.velocity), normal: Vec3::from_array(spawn.normal),
                radius: spawn.radius, lifetime: spawn.lifetime, wetness: spawn.wetness, kind,
                seed: hash(self.seed ^ (spawn.wheel_id as u64).rotate_left(32) ^ spawn.serial) as u32,
                birth_time: tick.frame as f64 / HZ });
        }
        self.births_dropped += keep_latest(&mut self.births, BIRTH_CAPACITY) as u64;
        for mark in self.tracks.tick(tick.frame, &self.contacts, wetness) {
            self.marks.push(Stamp { start: mark.start, end: mark.end, start_normal: mark.start_normal, end_normal: mark.end_normal,
                width: mark.width, compaction: mark.compaction, rubber: mark.rubber });
            self.marks_created += 1;
        }
        self.marks_dropped += keep_latest(&mut self.marks, MARK_CAPACITY) as u64;
    }

    fn expire_births(&mut self) {
        let time = self.simulation_time(); let before = self.births.len();
        self.births.retain(|b| time - b.birth_time < b.lifetime as f64);
        self.births_expired += (before-self.births.len()) as u64;
    }
    pub fn simulation_time(&self) -> f64 { self.last.map_or(0.0, |tick| tick.frame as f64 / HZ) }
    pub fn births(&self) -> &[Birth] { &self.births }
    pub fn marks(&self) -> &[Stamp] { &self.marks }
    pub fn clear_uploads(&mut self) { self.births.clear(); self.marks.clear(); }
    pub fn take_reset(&mut self) -> bool { std::mem::take(&mut self.reset_pending) }

    /// Upload once for all views. Acknowledged chunks are removed only after
    /// success, so retrying a later error never duplicates particle births.
    pub fn upload(&mut self, gpu: &render::Gpu, renderer: &mut render::Renderer) -> Result<(), String> {
        if self.reset_pending {
            renderer.reset_wheel_particles(); renderer.clear_tracks(gpu);
            self.reset_pending = false;
        }
        self.expire_births();
        while !self.marks.is_empty() {
            let count = self.marks.len().min(render::tracks::MAX_STAMPS_PER_UPDATE);
            self.track_stats = Some(renderer.update_tracks(gpu, &self.marks[..count])?);
            self.marks.drain(..count); self.marks_uploaded += count as u64;
        }
        renderer.update_wheel_particles(gpu, self.simulation_time(), &self.births)?;
        self.births_uploaded += self.births.len() as u64; self.births.clear();
        Ok(())
    }

    pub fn metadata(&self) -> serde_json::Value {
        let contacts: Vec<_> = self.contacts.iter().take(WHEELS).map(|c| serde_json::json!({
            "wheel": c.id, "surface": c.surface.name(), "point": c.point, "normal": c.normal,
            "point_velocity": c.inherited_velocity, "forward_speed": c.forward_speed,
            "lateral_speed": c.lateral_speed, "tread_speed": c.tread_speed, "load": c.load, "slide": c.slide
        })).collect();
        serde_json::json!({"enabled":self.enabled,"mode":"fixed_tick_shared_wheel_contacts","tick":self.last.map(|t|t.frame),
            "round":self.last.map(|t|t.round),"simulation_time":self.simulation_time(),"cars_limit":CARS,"wheel_limit":WHEELS,
            "birth_capacity":BIRTH_CAPACITY,"mark_queue_capacity":MARK_CAPACITY,"births_queued":self.births.len(),"marks_queued":self.marks.len(),
            "born_total":self.emitted.iter().sum::<u64>(),"born_by_kind":{"stone":self.emitted[0],"soil":self.emitted[1],"grass":self.emitted[2],
                "mud":self.emitted[3],"dust":self.emitted[4],"tyre_smoke":self.emitted[5],"water":self.emitted[6],"sand":self.emitted[7]},
            "born_by_surface":self.by_surface,"surface_order":Surface::ALL.map(Surface::name),"marks_created":self.marks_created,
            "births_uploaded":self.births_uploaded,"marks_uploaded":self.marks_uploaded,
            "track_atlas":self.track_stats.map(|s|serde_json::json!({"stamps":s.stamps,"rejected":s.rejected,"tiles_resident":s.tiles_resident,
                "tiles_evicted":s.tiles_evicted,"texels_written":s.texels_written,"last_upload_bytes":s.last_upload_bytes,"gpu_bytes":s.gpu_bytes,"cpu_tile_bytes":s.cpu_tile_bytes})),
            "births_expired_before_upload":self.births_expired,"birth_queue_dropped":self.births_dropped,"mark_queue_dropped":self.marks_dropped,
            "stale_contacts_rejected":self.stale_contacts,"car_discontinuities":self.discontinuities,"ignored_cars":self.ignored_cars,
            "reset_pending":self.reset_pending,"emitter_budget_dropped":self.particles.stats.budget_dropped,
            "emitter_invalid_contacts":self.particles.stats.invalid_contacts,"recent_contacts":contacts,"physics_unchanged":true,
            "wetness_source":"authoritative_race_weather_not_visual_sky","tyre_mark_width_m":0.24})
    }
}

fn point_velocity(linear: Vec3, angular: Vec3, com: Vec3, point: Vec3) -> Vec3 { linear + angular.cross(point-com) }
fn contact_surface(dry: Surface, y: f32, level: f64) -> Surface { if level.is_finite() && level > y as f64 + 0.08 { Surface::Water } else { dry } }
fn keep_latest<T>(items: &mut Vec<T>, capacity: usize) -> usize { let drop = items.len().saturating_sub(capacity); if drop>0 {items.drain(..drop);} drop }
fn hash(mut x: u64) -> u64 { x=x.wrapping_add(0x9e3779b97f4a7c15);x=(x^(x>>30)).wrapping_mul(0xbf58476d1ce4e5b9);x=(x^(x>>27)).wrapping_mul(0x94d049bb133111eb);x^(x>>31) }
fn kind_index(k: wheel_fx::Kind) -> usize { match k {wheel_fx::Kind::Stone=>0,wheel_fx::Kind::SoilClod=>1,wheel_fx::Kind::GrassBlade=>2,wheel_fx::Kind::MudClod=>3,wheel_fx::Kind::Dust=>4,wheel_fx::Kind::TyreSmoke=>5,wheel_fx::Kind::WaterDrop=>6,wheel_fx::Kind::Sand=>7} }
fn kind(k: wheel_fx::Kind) -> Kind { match k {wheel_fx::Kind::Stone=>Kind::Stone,wheel_fx::Kind::SoilClod=>Kind::SoilClod,wheel_fx::Kind::GrassBlade=>Kind::GrassBlade,wheel_fx::Kind::MudClod=>Kind::MudClod,wheel_fx::Kind::Dust=>Kind::Dust,wheel_fx::Kind::TyreSmoke=>Kind::TyreSmoke,wheel_fx::Kind::WaterDrop=>Kind::WaterDrop,wheel_fx::Kind::Sand=>Kind::Sand} }

#[cfg(test)]
mod tests {
    use super::*;
    fn tick(frame: u64) -> Tick { Tick { round: 1, frame, noise: 7, detail: 8 } }
    fn inputs(frame: u64, count: usize) -> Vec<Input> {
        (0..count).map(|slot| {
            let x = frame as f32 * 0.1;
            Input { pose: Pose { position: Vec3::new(x,0.4,slot as f32*5.0), rotation: Quat::IDENTITY,
                    velocity: Vec3::X*12.0, angular: Vec3::ZERO, penalty: 0.0, ghost: 0.0 },
                contacts: (0..4).map(|wi|wheel_fx::Contact { id:(slot*4+wi)as u8, grounded:true,
                    surface:Surface::Gravel, point:[x+wi as f32*0.2,0.0,slot as f32*5.0], normal:[0.0,1.0,0.0],
                    inherited_velocity:[12.0,0.0,0.0], forward:[1.0,0.0,0.0], right:[0.0,0.0,1.0],
                    forward_speed:12.0,lateral_speed:4.0,tread_speed:22.0,load:3000.0,slide:2.0 }).collect(), water:[None;4],stale:0 }
        }).collect()
    }
    #[test]
    fn pause_and_multiple_views_preserve_birth_ages_and_pending_marks() {
        let mut a=WheelEffects::with_enabled(3,true);let mut b=WheelEffects::with_enabled(3,true);
        for t in 0..240 {
            a.observe(tick(t),&inputs(t,4),0.0);b.observe(tick(t),&inputs(t,4),0.0);
            for _ in 0..4 { b.observe(tick(t),&inputs(t,4),0.0); }
        }
        assert_eq!(a.metadata(),b.metadata());assert!(!a.births.is_empty());assert!(!a.marks.is_empty());
        for (a,b) in a.births.iter().zip(&b.births) {assert_eq!(a.birth_time,b.birth_time);assert_eq!(a.position,b.position);assert_eq!(a.velocity,b.velocity);assert_eq!(a.seed,b.seed);}
        assert!(a.births.iter().any(|b|b.birth_time<a.simulation_time()-0.1));
        assert!(a.emitted[0]>0&&a.emitted[4]>0);assert_eq!(a.contacts.len(),16);
        let before=a.metadata();a.clear_uploads();assert_eq!(a.emitted,b.emitted);assert!(a.births.is_empty()&&a.marks.is_empty());
        assert!(before["marks_queued"].as_u64().unwrap()>0);
    }
    #[test]
    fn recovery_stale_contacts_and_stage_cuts_never_bridge() {
        let mut fx=WheelEffects::with_enabled(9,true);
        for t in 0..30 {fx.observe(tick(t),&inputs(t,1),0.0);}
        fx.clear_uploads();let mut recovered=inputs(30,1);
        recovered[0].pose.position.x+=100.0;recovered[0].pose.penalty=5.0;recovered[0].pose.ghost=2.0;
        // Deliberately retain the old contact location, as Car::teleport does.
        fx.observe(tick(30),&recovered,0.0);
        assert!(fx.contacts.is_empty()&&fx.births.is_empty()&&fx.marks.is_empty());assert_eq!(fx.discontinuities,1);
        let mut next=inputs(31,1);next[0].pose=recovered[0].pose;next[0].pose.position.x+=0.1;
        for c in &mut next[0].contacts {c.point[0]+=100.0;}
        fx.observe(tick(31),&next,0.0);assert!(fx.marks.is_empty());
        for c in &mut next[0].contacts {c.point[0]+=0.1;}next[0].pose.position.x+=0.1;
        fx.observe(tick(32),&next,0.0);assert!(!fx.marks.is_empty());
        assert!(fx.marks.iter().all(|s|Vec3::from(s.start).distance(Vec3::from(s.end))<0.11));
        fx.take_reset();fx.observe(Tick {round:2,..tick(33)},&inputs(33,1),0.0);
        assert!(fx.take_reset());assert!(fx.marks.is_empty()&&fx.births.is_empty());
        fx.observe(tick(100),&inputs(100,1),0.0);assert!(fx.take_reset());
    }
    #[test]
    fn per_contact_water_and_com_velocity_are_physical() {
        assert_eq!(contact_surface(Surface::Grass,0.0,0.5),Surface::Water);
        assert_eq!(contact_surface(Surface::Grass,0.6,0.5),Surface::Grass);
        assert_eq!(contact_surface(Surface::Gravel,0.0,f64::NEG_INFINITY),Surface::Gravel);
        // Rotation around world COM [10,2,0], not body origin [0,0,0].
        let velocity=point_velocity(Vec3::new(3.0,0.0,0.0),Vec3::new(0.0,2.0,0.0),Vec3::new(10.0,2.0,0.0),Vec3::new(11.0,2.0,0.0));
        assert_eq!(velocity,Vec3::new(3.0,0.0,-2.0));
        let mut fx=WheelEffects::with_enabled(1,true);
        for t in 0..120 {let mut i=inputs(t,1);i[0].contacts[0].surface=Surface::Water;i[0].water[0]=Some(0.5);fx.observe(tick(t),&i,0.0);}
        let water:Vec<_>=fx.births.iter().filter(|b|b.kind==Kind::WaterDrop).collect();assert!(!water.is_empty());
        assert!(water.iter().all(|b|b.position.y>=0.5+b.radius));assert!(fx.emitted[0]>0&&fx.emitted[6]>0);
    }
    #[test]
    fn bounded_offline_queue_records_drops_and_expiry() {
        let mut fx=WheelEffects::with_enabled(1,true);
        for t in 0..2300 {fx.observe(tick(t),&inputs(t,4),0.0);}
        assert_eq!(fx.marks.len(),MARK_CAPACITY);assert!(fx.marks_dropped>0&&fx.births_dropped>0);assert!(fx.births.len()<=BIRTH_CAPACITY);
        // One car alone no longer fills the queue, so old births age out.
        for t in 2300..2700 {fx.observe(tick(t),&inputs(t,1),0.0);}
        assert!(fx.births_expired>0);assert!(fx.births.iter().all(|b|fx.simulation_time()-b.birth_time<b.lifetime as f64));
        assert!(fx.emitted.iter().sum::<u64>()>0);
    }
    #[test]
    fn actual_four_car_race_digest_and_recovery_are_unchanged() {
        use kern::{Sim, Timings, digest_of};
        use sideways::{race::{Settings,Control},spec::Cars};
        let mut race=Race::new(1,Settings {stage_km:0.2,bots:4,rivals:0,..Settings::default()},Cars::load(None).unwrap());
        let mut fx=WheelEffects::with_enabled(4,true);let mut timings=Timings::default();fx.step(&race);
        let mut contacts=0;
        for t in 0..600 {
            if t==450 {race.step_with_inputs(&mut timings,&[Some(Control{throttle:0.8,recover:true,..Default::default()})]);}
            else {race.step(&mut timings);}
            let digest=digest_of(&race);fx.step(&race);fx.step(&race);assert_eq!(digest,digest_of(&race));
            contacts+=fx.contacts.len();assert!(fx.contacts.len()<=16);
        }
        assert!(contacts>0);assert!(fx.discontinuities>=1);assert!(fx.emitted.iter().sum::<u64>()>0);
        assert!(fx.marks.iter().all(|m|Vec3::from(m.start).distance(Vec3::from(m.end))<=2.01));
        let mut disabled=WheelEffects::with_enabled(4,false);disabled.step(&race);
        assert!(disabled.births.is_empty()&&disabled.marks.is_empty()&&disabled.contacts.is_empty());
    }
}
