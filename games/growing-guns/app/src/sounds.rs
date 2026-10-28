//! Growing Guns' sound, as Godot's `sfx.gd` plays it: procedural patches
//! (`games/growing-guns/sounds/*.luau`, hot-reloaded) and the Godot
//! project's footstep, hurt and death recordings, through the engine's
//! synth mixer with Godot's playback rules (gains, pitch, unit sizes, dry and
//! room-reverb copies, the big-tail hall, loudness culling, the master bus).
//! The trigger table is sfx.gd's call sites; see the patch headers.
use audio::sfx::Listener;
use audio::synth::{self, Ear, Falloff, Handle, Node, Patch, Place, Play, SoundSender, Unit};
use growing_guns::{Event, ExplosionKind, Special, StrikeKind, World, EYE};
use glam::Vec3;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// One listener per local player, at eye height (for the crowd and stadium).
pub fn ears(world: &World, slots: &[usize]) -> Vec<Listener> {
    slots.iter().map(|&s| {
        let p = &world.players[s];
        let (sy, cy) = p.yaw.sin_cos();
        Listener { pos: (p.pos + Vec3::Y * EYE).to_array(), right: [cy, 0.0, -sy] }
    }).collect()
}

fn synth_ears(world: &World, slots: &[usize]) -> Vec<Ear> {
    slots.iter().map(|&s| {
        let p = &world.players[s];
        let (sy, cy) = p.yaw.sin_cos();
        Ear { pos: (p.pos + Vec3::Y * EYE).to_array(), right: [cy, 0.0, -sy], forward: [-sy, 0.0, -cy] }
    }).collect()
}

/// The Godot project's sound folder (`--sounds DIR`, `GG_SOUNDS`, or a
/// sibling checkout `../growing-guns/assets/audio`): footsteps, hurts, deaths.
pub fn find_dir(arg: Option<&str>) -> Option<PathBuf> {
    let candidates: [Option<String>; 3] = [arg.map(str::to_string), std::env::var("GG_SOUNDS").ok(), Some("../growing-guns/assets/audio".to_string())];
    candidates.into_iter().flatten().map(PathBuf::from).find(|p| p.join("Step-1.wav").exists())
}

/// The patch folder: `--sfx DIR`, `GG_SFX`, beside the working directory, or
/// the source tree this binary was built from.
pub fn find_patches(arg: Option<&str>) -> Option<PathBuf> {
    let built = concat!(env!("CARGO_MANIFEST_DIR"), "/../sounds").to_string();
    let candidates = [arg.map(str::to_string), std::env::var("GG_SFX").ok(), Some("games/growing-guns/sounds".into()), Some("sounds".into()), Some(built)];
    candidates.into_iter().flatten().map(PathBuf::from).find(|p| p.join("shot.luau").exists())
}

/// Patches by sound name, reloaded when their file changes.
#[derive(Default)]
pub struct Bank {
    dir: Option<PathBuf>,
    patches: HashMap<String, Arc<Patch>>,
    files: HashMap<PathBuf, SystemTime>,
    warned: std::collections::HashSet<String>,
    since: f32,
}

impl Bank {
    pub fn load(dir: Option<PathBuf>) -> Self {
        let mut b = Bank { dir, ..Default::default() };
        for line in b.scan() { eprintln!("sfx: {line}"); }
        if let Some(d) = &b.dir { println!("sfx: {} sounds from {}", b.patches.len(), d.display()); }
        b
    }

    /// Load new or changed files; returns what happened.
    pub fn scan(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(dir) = &self.dir else { return out };
        let Ok(rd) = std::fs::read_dir(dir) else { return out };
        for e in rd.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("luau") { continue; }
            let Ok(t) = e.metadata().and_then(|m| m.modified()) else { continue };
            if self.files.get(&path) == Some(&t) { continue; }
            let fresh = self.files.insert(path.clone(), t).is_some();
            match synth::lua::load_file(&path) {
                Ok(ps) => for p in ps {
                    if fresh { out.push(format!("reloaded {}", p.name)); }
                    self.warned.remove(&p.name);
                    self.patches.insert(p.name.clone(), Arc::new(p));
                },
                Err(e) => out.push(format!("{e} (keeping the last good version)")),
            }
        }
        out
    }

    /// Check for edits every half second.
    pub fn poll(&mut self, dt: f32) -> Vec<String> {
        self.since += dt;
        if self.since < 0.5 { return Vec::new(); }
        self.since = 0.0;
        self.scan()
    }

    pub fn get(&mut self, name: &str) -> Option<Arc<Patch>> {
        let p = self.patches.get(name).cloned();
        if p.is_none() && self.warned.insert(name.to_string()) { eprintln!("sfx: no patch {name}"); }
        p
    }

}

/// A recording as a patch: a clip played once at rate 1 (pitch is the voice's).
fn clip_patch(name: &str, clip: &audio::sfx::Clip) -> Patch {
    Patch { name: name.into(), nodes: vec![Node { unit: Unit::Clip { name: name.into(), samples: clip.0.clone(), looping: false }, inputs: vec![] }], out: 0, ..Default::default() }
}

/// How sfx.gd plays a stream (`_play_stream`).
#[derive(Clone, Copy)]
struct G {
    /// None: 2D (not positioned).
    at: Option<Vec3>,
    db: f32,
    pitch: f32,
    /// Godot attenuation unit size (m).
    unit: f32,
    /// A plain 3D player on Master (no room reverb, no fade).
    dry: bool,
    spl: Option<f32>,
    /// The big-tail hall send (3D only, -24 dB below the voice).
    tail: bool,
    /// 2D at +20 log10(unit), at most +18 dB (a self shot's "centre").
    center: bool,
    pan: f32,
    delay: bool,
}

impl G {
    fn at(at: Vec3, db: f32, unit: f32) -> G { G { at: Some(at), db, pitch: 1.0, unit, dry: false, spl: None, tail: true, center: false, pan: 0.0, delay: true } }
    fn ui(db: f32) -> G { G { at: None, db, pitch: 1.0, unit: 12.0, dry: false, spl: None, tail: false, center: false, pan: 0.0, delay: false } }
    fn dry(mut self) -> G { self.dry = true; self }
    fn spl(mut self, s: f32) -> G { self.spl = Some(s); self }
    fn pitch(mut self, p: f32) -> G { self.pitch = p; self }
    fn no_tail(mut self) -> G { self.tail = false; self }
}

const TAIL_SEND: f32 = 0.063; // -24 dB
/// Godot RaytracedReverb wet level (dry 1.0).
const ROOM_SEND: f32 = 0.35;

enum Later {
    CardFlip,
    NextRound(Vec3),
    Casing(Vec3, f32),
    Zip { at: Vec3, sf: f32, sc: f32, prox: f32 },
}

/// Start the speakers with the synth mixer (Godot's sound) and the simple
/// effects mixer (the crowd), summed. `layout` overrides the device's own
/// channel count (stereo, 5.1, 7.1). Returns the speaker (keep it alive) and
/// both senders.
pub fn speakers(layout: Option<&str>) -> Result<(app::sound::Speaker, SoundSender, audio::sfx::SfxSender), String> {
    let mut made = None;
    let named = layout.and_then(synth::Layout::named);
    let speaker = app::sound::open(|rate, channels| {
        let (tx, mut mix) = synth::mixer(named.clone().unwrap_or_else(|| synth::Layout::for_channels(channels)), 160, rate);
        // Finished voices are freed here, not on the audio thread.
        if let Some(trash) = mix.take_trash() { std::thread::Builder::new().name("sfx-trash".into()).spawn(move || for g in trash { drop(g) }).ok(); }
        let (crowd, cmix) = audio::sfx::sfx(32, rate);
        made = Some((tx, crowd));
        Box::new(audio::Sum::new(vec![Box::new(mix), Box::new(cmix)]))
    })?;
    let (tx, crowd) = made.expect("built in open");
    Ok((speaker, tx, crowd))
}

/// The same pair offline at 48 kHz stereo, for scripted runs.
pub fn offline() -> (Box<dyn audio::Source>, SoundSender, audio::sfx::SfxSender) {
    let (tx, mix) = synth::mixer(synth::Layout::stereo(), 160, audio::SAMPLE_RATE);
    let (crowd, cmix) = audio::sfx::sfx(32, audio::SAMPLE_RATE);
    (Box::new(audio::Sum::new(vec![Box::new(mix), Box::new(cmix)])), tx, crowd)
}

pub struct Sounds {
    tx: SoundSender,
    pub bank: Bank,
    steps: Vec<Arc<Patch>>,
    hurts: Vec<Arc<Patch>>,
    deaths: Vec<Arc<Patch>>,
    rng: u32,
    time: f32,
    later: Vec<(f32, Later)>,
    walked: Vec<f32>,
    last_zip: f32,
    lava_at: Vec<f32>,
    explosions: Vec<f32>,
    reloads: HashMap<usize, Handle>,
    strikes: Vec<(StrikeKind, Vec3, Vec<Handle>)>,
    descents: HashMap<usize, Handle>,
    pickups: usize,
    incoming: Option<Vec3>,
    offers: Vec<(bool, f64)>,
    shot_tick: Vec<u64>,
    ears: Vec<Vec3>,
}

impl Sounds {
    /// `samples`: the Godot project's audio folder (footsteps, hurts, deaths).
    pub fn new(tx: SoundSender, patches: Option<PathBuf>, samples: Option<&Path>) -> Self {
        tx.master(synth::Master::godot());
        tx.cull(Some(synth::Cull::default()));
        tx.hall(synth::Hall::default());
        // Godot RaytracedReverb: damping .55; room size ~0.18 in the arena.
        tx.room(0.8, 0.55, 1.0);
        let load = |names: &mut dyn Iterator<Item = String>| -> Vec<Arc<Patch>> {
            let Some(dir) = samples else { return Vec::new() };
            names.filter_map(|n| audio::sfx::load_wav(&dir.join(&n)).map_err(|e| eprintln!("sound: {e}")).ok().map(|c| Arc::new(clip_patch(&n, &c)))).collect()
        };
        let steps = load(&mut (1..=13).map(|i| format!("Step-{i}.wav")));
        let hurts = load(&mut (1..=12).map(|i| format!("hurt{i}.wav")));
        let deaths = load(&mut (1..=4).map(|i| format!("death{i}.wav")));
        if samples.is_some() { println!("sounds: {} recordings", steps.len() + hurts.len() + deaths.len()); }
        Sounds { tx, bank: Bank::load(patches), steps, hurts, deaths, rng: 0x1234_5678, time: 0.0, later: Vec::new(), walked: Vec::new(), last_zip: -1.0,
            lava_at: Vec::new(), explosions: Vec::new(), reloads: HashMap::new(), strikes: Vec::new(), descents: HashMap::new(), pickups: 0, incoming: None,
            offers: Vec::new(), shot_tick: Vec::new(), ears: Vec::new() }
    }


    /// How far the loudest recent sound is above "loud" (Godot's crowd duck: HDR max - 112 dB).
    pub fn duck_db(&self) -> f32 { (self.tx.loudest() - 112.0).max(0.0) }

    fn next(&mut self) -> u32 { self.rng ^= self.rng << 13; self.rng ^= self.rng >> 17; self.rng ^= self.rng << 5; self.rng }
    fn u(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * (self.next() >> 8) as f32 / (1u32 << 24) as f32 }
    fn pick(&mut self, set: &[Arc<Patch>]) -> Option<Arc<Patch>> { if set.is_empty() { None } else { let k = self.next() as usize % set.len(); Some(set[k].clone()) } }

    fn play(&mut self, world: &World, patch: &Arc<Patch>, params: &[(&str, f32)], g: G) -> Handle {
        let seed = self.next();
        let p = match g.at {
            None => {
                let db = if g.center { (g.db + 20.0 * g.unit.max(1e-3).log10()).min(18.0) } else { g.db };
                Play { place: Place::Pan(g.pan), pitch: g.pitch, spl: g.spl, seed, ..Default::default() }.db(db)
            }
            Some(at) => {
                let falloff = if g.dry { Falloff::godot(g.unit) } else {
                    // Raytraced players fade out linearly to where they are 30 dB down.
                    Falloff::godot(g.unit).fade(g.unit / 10f32.powf((-30.0 - g.db) / 20.0))
                };
                let occlusion = if g.dry { 0.0 } else { occlusion(world, &self.ears, at) };
                Play { place: Place::At(at.to_array()), falloff, pitch: g.pitch, spl: g.spl, seed, sound_delay: g.delay, occlusion,
                    reverb: if g.dry { 0.0 } else { ROOM_SEND }, tail: if g.tail { TAIL_SEND } else { 0.0 }, ..Default::default() }.db(g.db)
            }
        };
        self.tx.play(patch, params, p)
    }

    fn named(&mut self, world: &World, name: &str, params: &[(&str, f32)], g: G) -> Option<Handle> {
        let p = self.bank.get(name)?;
        Some(self.play(world, &p, params, g))
    }

    /// Sounds for one tick of events; `slots` are the local players (ears).
    pub fn tick(&mut self, world: &World, slots: &[usize], dt: f32) {
        for line in self.bank.poll(dt) { eprintln!("sfx: {line}"); }
        self.time += dt;
        self.ears = synth_ears(world, slots).iter().map(|e| Vec3::from(e.pos)).collect();
        self.tx.ears(synth_ears(world, slots));
        let local = |p: usize| slots.contains(&p) && !world.players.get(p).is_some_and(|q| q.bot);
        // Split screen: a local shooter's own shot pans toward their half.
        let pan_of = |p: usize| -> f32 {
            let Some(k) = slots.iter().position(|&s| s == p) else { return 0.0 };
            if slots.len() < 2 { 0.0 } else if k % 2 == 0 { -0.8 } else { 0.8 }
        };
        self.shot_tick.resize(world.players.len(), u64::MAX);
        let killed: Vec<(usize, usize)> = world.events.iter().filter_map(|e| match *e { Event::Kill { victim, attacker: Some(a), .. } => Some((victim, a)), _ => None }).collect();
        for e in &world.events {
            match *e {
                Event::Shot { player, from, dir } => {
                    // One shot sound per shooter per tick (Godot: per physics frame).
                    if self.shot_tick[player] == world.tick { continue; }
                    self.shot_tick[player] = world.tick;
                    self.shot(world, player, from, dir, local(player), pan_of(player), slots);
                }
                Event::Impact { pos, damage, .. } => {
                    let ratio = (damage - growing_guns::destruct::RIFLE_CHIP_DAMAGE_BASE) / growing_guns::destruct::RIFLE_CHIP_DAMAGE_PER_RATIO;
                    let d = ratio.clamp(0.5, 6.0);
                    let dmg_db = (d.max(1.0).log2() * 4.0).clamp(0.0, 9.0);
                    self.named(world, "impact", &[("dmg_ratio", d)], G::at(pos, -16.0 + dmg_db, 8.0).dry().no_tail().spl(90.0 + dmg_db));
                }
                Event::Explosion { pos, radius, kind, .. } => self.explosion(world, pos, radius, kind),
                Event::Special { player, special } => match special {
                    Special::Grenade | Special::ClusterGrenade => {
                        let at = world.players[player].eye();
                        self.named(world, "grenade_launch", &[], G::at(at, -6.0, 12.0).spl(110.0));
                    }
                    _ => {}
                },
                Event::Slash { player, from, .. } => {
                    let w = &world.players[player].weapon;
                    let damage = w.melee_damage() as f32 * w.damage_mult;
                    self.named(world, "melee", &[("pitch_ratio", (50.0 / damage.max(1.0)).clamp(0.5, 1.2))], G::at(from, -6.0, 12.0).spl(90.0));
                }
                Event::Jump { player } => {
                    let at = world.players[player].pos;
                    for (db, lo, hi) in [(-7.5, 1.02, 1.12), (-10.5, 0.55, 0.65)] {
                        let (j, p) = (self.u(-1.5, 1.5), self.u(lo, hi));
                        if let Some(s) = self.pick(&self.steps.clone()) { self.play(world, &s, &[], G::at(at, db + j, 12.0).pitch(p).spl(75.0)); }
                    }
                }
                Event::Landed { player, speed } => {
                    if speed < 2.0 || world.players[player].ghost { continue; }
                    let t = ((speed - 2.0) / 23.0).clamp(0.0, 1.0);
                    let (db, pitch, spl) = (lerp(-22.0, 5.0, t), lerp(1.05, 0.55, t), lerp(75.0, 95.0, t));
                    let at = world.players[player].pos;
                    if let Some(s) = self.pick(&self.steps.clone()) { self.play(world, &s, &[], G::at(at, db, 12.0).pitch(pitch).spl(spl)); }
                    if t > 0.25 && let Some(s) = self.pick(&self.steps.clone()) { self.play(world, &s, &[], G::at(at, db - 4.0, 12.0).pitch(pitch * 0.5).spl(spl)); }
                    if let Some(h) = self.descents.remove(&player) { self.tx.release(&h); self.tx.stop(&h); }
                }
                Event::Dash { player } => { let at = world.players[player].pos; self.named(world, "dash", &[], G::at(at, -24.0, 12.0).spl(75.0)); }
                Event::Hit { victim, attacker, damage, head, pos } => {
                    // Hurt: the victim's own is 2D.
                    if let Some(s) = self.pick(&self.hurts.clone()) {
                        if local(victim) { self.play(world, &s, &[], G::ui(-25.0).spl(100.0)); }
                        else { let j = self.u(-1.5, 1.5); self.play(world, &s, &[], G::at(world.players[victim].pos, -17.5 + j, 12.0).spl(80.0)); }
                    }
                    if local(victim) {
                        let i = (damage as f32 / 25.0).clamp(0.04, 2.0);
                        self.named(world, "hit_received", &[("intensity", i)], G::ui(-12.0 + 20.0 * i.clamp(0.05, 2.0).log10()).spl(130.0));
                    }
                    if local(attacker) && attacker != victim && !killed.contains(&(victim, attacker)) {
                        let r = if damage == 0 { 1.0 } else { (damage as f32 / 22.0).clamp(0.5, 5.0) };
                        let kind = if head { 1.0 } else { 0.0 };
                        self.named(world, "hitmarker", &[("kind", kind), ("ratio", r)], G::ui(-5.0 + (2.0 * r.log2()).clamp(0.0, 4.0)).spl(130.0));
                    }
                    let _ = pos;
                }
                Event::Kill { victim, attacker, .. } => {
                    if let Some(s) = self.pick(&self.deaths.clone()) {
                        if local(victim) { self.play(world, &s, &[], G::ui(-12.0).spl(110.0)); }
                        else { let j = self.u(-1.5, 1.5); self.play(world, &s, &[], G::at(world.players[victim].pos, -8.0 + j, 12.0).spl(85.0)); }
                    }
                    if let Some(a) = attacker && local(a) && a != victim { self.named(world, "kill_confirm", &[], G::ui(-6.0).spl(130.0)); }
                    if let Some(h) = self.reloads.remove(&victim) { self.tx.stop(&h); }
                }
                Event::Lava { player, fall } => {
                    self.lava_at.resize(world.players.len(), -10.0);
                    // Contact sizzles at most every 0.9 s (Godot).
                    if !fall && self.time - self.lava_at[player] < 0.9 { continue; }
                    if !fall { self.lava_at[player] = self.time; }
                    let j = self.u(-1.0, 1.0);
                    let pitch = self.u(0.96, 1.04);
                    let (db, spl) = if fall { (-26.0 + j, 72.0) } else { (-23.0 + j, 95.0) };
                    self.named(world, "lava_sizzle", &[("fall", if fall { 1.0 } else { 0.0 })], G::at(world.players[player].pos, db, 18.0).pitch(pitch).no_tail().spl(spl));
                }
                Event::Reload { player, seconds } => {
                    if let Some(h) = self.reloads.remove(&player) { self.tx.stop(&h); }
                    let g = if local(player) { G::ui(-14.0) } else { G::at(world.players[player].eye(), -14.0, 12.0) };
                    if let Some(h) = self.named(world, "reload", &[("duration", seconds)], g.spl(70.0)) { self.reloads.insert(player, h); }
                }
                Event::Empty { player } => { let at = world.players[player].eye(); self.named(world, "empty_chamber", &[], G::at(at, -10.0, 5.0).dry().spl(55.0)); }
                Event::PickedUp { player, .. } => if local(player) { self.named(world, "pling", &[("pitch", 1.15)], G::ui(-10.0).spl(70.0)); },
                _ => {}
            }
        }
        self.state(world, slots, &local);
        self.flush_later(world);
        self.footsteps(world, dt);
    }

    fn shot(&mut self, world: &World, player: usize, from: Vec3, dir: Vec3, is_self: bool, pan: f32, slots: &[usize]) {
        let p = &world.players[player];
        let w = &p.weapon;
        let last = p.ammo == 0;
        let d = w.damage_for_shot(last) / 22.0;
        let dmg_db = (d.max(0.5).log2() * 6.0).clamp(-4.0, 14.0);
        let silenced = w.silencer_stacks > 0;
        let mut pitch = (1.0 - d.max(1.0).log2() * 0.2).clamp(0.35, 1.15) * self.u(0.97, 1.03);
        let (mut self_base, mut world_base) = (-2.0 + self.u(-1.5, 1.5), 5.0 + self.u(-1.5, 1.5));
        if silenced { self_base -= 10.0; world_base -= 12.0; pitch *= 1.08; }
        let dry = if is_self { self_base } else { world_base } + dmg_db;
        let spl = 105.0 + 7.0 * d.max(1.0).log2();
        let params = [("damage", (w.damage() / 22.0).max(1.0)), ("accuracy", (1.0 - w.spread() / 0.06).clamp(0.0, 1.0)), ("velocity", w.bullet_speed_mult.clamp(0.5, 3.0))];
        if let Some(shot) = self.bank.get("shot") {
            let g = if is_self { G { center: true, pan, unit: 6.0, ..G::ui(dry) } } else { G::at(from, dry, 3.0).dry() };
            self.play(world, &shot, &params, g.pitch(pitch).spl(spl));
            // The wet copy always plays at the muzzle, through the room.
            self.play(world, &shot, &params, G { spl: None, ..G::at(from, dry - 10.0, 3.0).pitch(pitch) });
        }
        if d > 1.4 {
            let rr = (d * 1.5).clamp(2.0, 12.0).round();
            let g = if is_self { G { center: true, unit: (rr * 0.8).clamp(4.0, 12.0), ..G::ui(-12.0) } } else { G::at(from, -12.0, (rr * 0.8).clamp(4.0, 12.0)) };
            self.named(world, "explosion_rumble", &[("radius", rr)], g);
        }
        // The bolt click in the gap before the next shot (not on fast guns).
        let fi = w.fire_interval() as f32;
        if fi >= 0.18 { self.later.push((self.time + (fi * 0.45).min(0.13), Later::NextRound(from))); }
        // The casing lands about half a second later beside the shooter.
        let side = Vec3::new(p.yaw.cos(), 0.0, -p.yaw.sin());
        let floor = p.pos + side * 0.6;
        let speed_ratio = self.u(0.45, 0.8);
        let at = self.time + 0.42 + self.u(0.0, 0.08);
        self.later.push((at, Later::Casing(floor, speed_ratio)));
        if self.u(0.0, 1.0) < 0.6 { self.later.push((self.time + 0.6, Later::Casing(floor + side * 0.2, speed_ratio * 0.35))); }
        // Near misses: where the round passes closest to each local player.
        if !slots.contains(&player) {
            let speed = 165.0 * w.bullet_speed_mult;
            let sf = (speed / 165.0).clamp(0.2, 5.0);
            let sc = w.bullet_scale_for_shot(last).clamp(0.5, 4.0);
            for &s in slots {
                let ear = world.players[s].pos + Vec3::Y * EYE;
                let t = (ear - from).dot(dir).clamp(0.0, 400.0);
                let at = from + dir * t;
                let dist = at.distance(ear);
                if dist <= 12.0 {
                    let prox = (1.0 - dist / 12.0).clamp(0.0, 1.0).sqrt();
                    self.later.push((self.time + t / speed.max(1.0), Later::Zip { at, sf, sc, prox }));
                }
            }
        }
    }

    fn explosion(&mut self, world: &World, pos: Vec3, radius: f32, kind: ExplosionKind) {
        // Bullet blasts under 3.5 m are silent; at most 25 per half second.
        if radius < 3.5 { return; }
        self.explosions.retain(|t| self.time - *t < 0.5);
        if self.explosions.len() >= 25 { return; }
        self.explosions.push(self.time);
        let r = if kind == ExplosionKind::Ion { 38.0 } else { radius };
        let rb = r.clamp(2.0, 24.0).round();
        let (bang_unit, rumble_unit) = ((0.8 * r).clamp(4.0, 24.0), (4.5 * r).clamp(20.0, 100.0));
        let spl = 165.0 + 5.0 * (r / 6.0).max(1.0).log2();
        self.named(world, "explosion_bang", &[("radius", rb)], G::at(pos, 12.0, bang_unit).dry().spl(spl));
        self.named(world, "explosion_bang", &[("radius", rb)], G::at(pos, 10.0, bang_unit).spl(spl));
        self.named(world, "explosion_rumble", &[("radius", rb)], G::at(pos, 10.0, rumble_unit).spl(spl));
        if kind == ExplosionKind::Ion {
            let rb = r.clamp(8.0, 48.0).round();
            let unit = (1.6 * r).clamp(16.0, 72.0);
            let spl = 168.0 + 4.0 * (r / 6.0).max(1.0).log2();
            self.tx.lift(0.62);
            if let Some(p) = self.bank.get("ion_burst") {
                // Two copies and an extra hall send at -20 dB (relative -18 to the first copy).
                self.play(world, &p, &[("radius", rb)], G::at(pos, -2.0, unit).spl(spl));
                self.play(world, &p, &[("radius", rb)], G::at(pos, 0.0, unit).spl(spl));
            }
        }
    }

    /// State the events do not carry: strikes, pickups, telegraphs, card
    /// offers, descents, reloads ending.
    fn state(&mut self, world: &World, slots: &[usize], local: &dyn Fn(usize) -> bool) {
        // Reloads end (or the reloader died).
        let done: Vec<usize> = self.reloads.keys().copied().filter(|&p| !world.players.get(p).is_some_and(|q| q.reloading && q.alive)).collect();
        for p in done { if let Some(h) = self.reloads.remove(&p) { self.tx.stop(&h); } }
        // Strikes: new ones start their cues; gone ones stop them.
        let mut keep = vec![false; self.strikes.len()];
        for s in &world.strikes {
            if let Some(k) = self.strikes.iter().position(|(kind, t, _)| *kind == s.kind && t.distance(s.target) < 0.01) {
                keep[k] = true;
                // The air strike's rocket falls from the sky to the target.
                if s.kind == StrikeKind::AirStrike {
                    let u = (s.age / growing_guns::AIR_STRIKE_TRAVEL).clamp(0.0, 1.0) as f32;
                    let sky = s.target + Vec3::new(0.0, 263.0, 93.0);
                    let at = sky.lerp(s.target, u * u);
                    if let Some(h) = self.strikes[k].2.first() { self.tx.move_to(h, Place::At(at.to_array()), 0.0); }
                }
                continue;
            }
            let mut handles = Vec::new();
            match s.kind {
                StrikeKind::AirStrike => {
                    let travel = growing_guns::AIR_STRIKE_TRAVEL as f32;
                    let sky = s.target + Vec3::new(0.0, 263.0, 93.0);
                    self.tx.lift(0.42);
                    if let Some(h) = self.named(world, "air_strike_inbound", &[("travel", travel), ("distant", 0.0)], G { delay: false, ..G::at(sky, -4.0, 58.0) }) { handles.push(h); }
                    // The telegraph at the target.
                    self.named(world, "mine_plant", &[], G::at(s.target, -10.0, 12.0).spl(90.0));
                }
                StrikeKind::IonCannon => {
                    let charge = growing_guns::ION_CHARGE as f32;
                    self.tx.lift(0.52);
                    // Attached at the target (not culled, not delayed), and a positional companion.
                    if let Some(h) = self.named(world, "ion_charge", &[("charge", charge)], G { delay: false, ..G::at(s.target, -14.0, 140.0) }) { handles.push(h); }
                    if let Some(h) = self.named(world, "ion_charge", &[("charge", charge)], G::at(s.target, -10.0, 140.0).spl(118.0)) { handles.push(h); }
                }
            }
            self.strikes.push((s.kind, s.target, handles));
            keep.push(true);
        }
        let mut k = 0;
        let tx = self.tx.clone();
        self.strikes.retain(|(_, _, hs)| { let r = keep[k]; k += 1; if !r { for h in hs { tx.release(h); } } r });
        // Pickups dropping in.
        if world.pickups.len() > self.pickups {
            for p in &world.pickups[self.pickups..] { let at = p.pos; self.named(world, "pickup_drop", &[], G::at(at, -11.0, 12.0).spl(75.0)); }
        }
        self.pickups = world.pickups.len();
        // Coop: an enemy about to rise.
        let incoming = if world.coop { world.waves.incoming.as_ref().map(|i| i.ground) } else { None };
        if let Some(g) = incoming && self.incoming.is_none_or(|o| o.distance(g) > 0.01) { self.named(world, "enemy_incoming", &[], G::at(g, -8.5, 12.0).spl(95.0)); }
        self.incoming = incoming;
        // Card offers for local players: the cards flip in, then countdown beeps.
        self.offers.resize(world.players.len(), (false, 0.0));
        let humans = world.players.iter().filter(|p| !p.bot).count();
        for &s in slots {
            let p = &world.players[s];
            let (had, age) = self.offers[s];
            match &p.offer {
                Some(cards) if local(s) => {
                    if !had {
                        // Godot: each card slides in, waits index x 0.15 s, then flips (0.1 s).
                        for i in 0..cards.len() { self.later.push((self.time + 0.4 + i as f32 * 0.19 + 0.1, Later::CardFlip)); }
                    }
                    if humans > 1 {
                        for th in [3.0, 2.0, 1.0] {
                            let (prev_left, left) = (growing_guns::CARD_PICK_TIMEOUT - age, growing_guns::CARD_PICK_TIMEOUT - p.offer_age);
                            if had && prev_left > th && left <= th {
                                let step = (3.0 - th).clamp(0.0, 2.0) as f32;
                                self.named(world, "pick_beep", &[("step", step)], G::ui(-8.0).spl(140.0));
                            }
                        }
                    }
                    self.offers[s] = (true, p.offer_age);
                }
                _ => self.offers[s] = (false, 0.0),
            }
        }
        // The sky drop at round start, for the local player.
        for &s in slots {
            let p = &world.players[s];
            match (p.launching.is_some() && local(s), self.descents.contains_key(&s)) {
                (true, false) => if let Some(h) = self.named(world, "rocket_descent", &[], G::ui(-4.0).spl(130.0)) { self.descents.insert(s, h); },
                (false, true) => if let Some(h) = self.descents.remove(&s) { self.tx.stop(&h); },
                _ => {}
            }
        }
    }

    fn flush_later(&mut self, world: &World) {
        let now = self.time;
        let due: Vec<Later> = { let mut d = Vec::new(); self.later.retain_mut(|(t, l)| if *t <= now { d.push(std::mem::replace(l, Later::CardFlip)); false } else { true }); d };
        for l in due {
            match l {
                Later::CardFlip => { self.named(world, "card_flip", &[], G::ui(-10.0).spl(60.0)); }
                Later::NextRound(at) => { self.named(world, "next_round", &[], G::at(at, -16.0, 5.0).dry().spl(65.0)); }
                Later::Casing(at, ratio) => {
                    let db = -18.0 + lerp(-22.0, 2.0, ratio.clamp(0.05, 1.2));
                    let pitch = self.u(0.96, 1.04);
                    self.named(world, "casing_drop", &[], G::at(at, db, 6.0).dry().no_tail().pitch(pitch).spl(75.0));
                }
                Later::Zip { at, sf, sc, prox } => {
                    if now - self.last_zip < 0.032 { continue; }
                    self.last_zip = now;
                    let sn = ((sf - 0.2) / 4.8).clamp(0.0, 1.0);
                    let db = lerp(-36.0, -24.0, prox) + lerp(-10.0, 6.0, sn);
                    let mut g = G::at(at, db, 12.0).dry().no_tail().spl(lerp(95.0, 135.0, sn));
                    if sn > 0.0 { g.tail = true; self.tx.lift(lerp(0.3, 1.0, sn)); }
                    if let Some(p) = self.bank.get("bullet_zip") {
                        let seed = self.next();
                        let falloff = Falloff::godot(12.0);
                        let play = Play { place: Place::At(at.to_array()), falloff, spl: g.spl, seed, sound_delay: true,
                            tail: if g.tail { 10f32.powf(lerp(-30.0, -2.0, sn) / 20.0) } else { 0.0 }, ..Default::default() }.db(db);
                        self.tx.play(&p, &[("speed_factor", sf), ("scale_factor", sc)], play);
                    }
                }
            }
        }
    }

    /// Footsteps every 2.2 m on the floor (Godot `player.gd`).
    fn footsteps(&mut self, world: &World, dt: f32) {
        self.walked.resize(world.players.len(), 0.0);
        for i in 0..world.players.len() {
            let p = &world.players[i];
            if !p.alive || !p.on_floor || p.ghost { continue; }
            self.walked[i] += (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt() * dt;
            if self.walked[i] < 2.2 { continue; }
            self.walked[i] = 0.0;
            let a = p.weapon.body_scale_axes;
            let size = p.weapon.body_scale * (a.x * a.y * a.z).max(1e-3).cbrt();
            let s = size.clamp(0.4, 3.0);
            let db = -30.0 + self.u(-2.0, 2.0) + (6.0 * s.log2()).clamp(-6.0, 10.0);
            let pitch = (1.0 / s.sqrt()).clamp(0.5, 1.6) * self.u(0.92, 1.08);
            let at = p.pos;
            if let Some(st) = self.pick(&self.steps.clone()) { self.play(world, &st, &[], G::at(at, db, 12.0).pitch(pitch).spl(78.0)); }
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }

/// Godot's raytraced muffle: the share of four listener rays with a clear
/// line to the sound (here the ear and three points a metre and a half
/// around it), through the arena's blocks. 0 clear .. 1 occluded.
fn occlusion(world: &World, ears: &[Vec3], at: Vec3) -> f32 {
    let Some(ear) = ears.iter().copied().min_by(|a, b| a.distance(at).total_cmp(&b.distance(at))) else { return 0.0 };
    let clear = |from: Vec3| {
        let d = at - from;
        let len = d.length();
        len <= 1e-3 || growing_guns::shape::raycast(&world.arena.grid, from, d / len, (len - 0.3).max(0.0)).is_none()
    };
    let side = (at - ear).cross(Vec3::Y).normalize_or_zero() * 1.5;
    let seen = [ear, ear + Vec3::Y * 1.5, ear + side, ear - side].into_iter().filter(|&p| clear(p)).count();
    1.0 - seen as f32 / 4.0
}
