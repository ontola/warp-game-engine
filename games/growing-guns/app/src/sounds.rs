//! Growing Guns' sound: procedural patches
//! (`games/growing-guns/sounds/*.luau`, hot-reloaded) and the Godot
//! project's footstep, hurt and death recordings, through the engine's
//! synth mixer. Combat has bounded overlapping layers, authored close-range
//! levels and room sends from the same voice; other cues retain their original mix.
//! The trigger table is sfx.gd's call sites; see the patch headers.
use audio::sfx::Listener;
use audio::synth::{self, CompiledPatch, Ear, Falloff, Handle, Node, Patch, Place, Play, SoundSender, Unit};
use growing_guns::{Event, ExplosionKind, Special, StrikeKind, World, EYE};
use glam::Vec3;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// `name` in the executable's folder (a packaged game).
pub fn beside_exe(name: &str) -> Option<PathBuf> { std::env::current_exe().ok()?.parent().map(|d| d.join(name)) }

/// The Godot project's sound folder (`--sounds DIR`, `GG_SOUNDS`, `audio`
/// beside the executable, or a sibling checkout `../growing-guns/assets/audio`):
/// footsteps, hurts, deaths (and the crowd's samples in `crowd` inside it).
pub fn find_dir(arg: Option<&str>) -> Option<PathBuf> {
    let candidates = [arg.map(PathBuf::from), std::env::var("GG_SOUNDS").ok().map(PathBuf::from), beside_exe("audio")];
    candidates.into_iter().flatten().chain(godot_checkout("assets/audio")).find(|p| p.join("Step-1.wav").exists())
}

/// `rel` inside a Godot checkout called `growing-guns` beside any folder
/// above the working directory, the executable or the source tree this
/// binary was built from (C:/dev/growing-guns next to C:/dev/warp-gg, as on
/// the Mancave, wherever Splitscreen Club starts the game from).
pub fn godot_checkout(rel: &str) -> Vec<PathBuf> {
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok().and_then(|e| e.parent().map(PathBuf::from)), Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")))];
    let mut out: Vec<PathBuf> = Vec::new();
    for start in starts.into_iter().flatten() {
        for dir in start.ancestors() {
            let p = dir.join("growing-guns").join(rel);
            if !out.contains(&p) { out.push(p); }
        }
    }
    out
}

/// The patch folder: `--sfx DIR`, `GG_SFX`, beside the working directory, or
/// the source tree this binary was built from.
pub fn find_patches(arg: Option<&str>) -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    if arg.is_none() { return crate::web::served("sounds"); }
    let built = concat!(env!("CARGO_MANIFEST_DIR"), "/../sounds").to_string();
    let candidates = [arg.map(PathBuf::from), std::env::var("GG_SFX").ok().map(PathBuf::from), beside_exe("sounds"), Some("games/growing-guns/sounds".into()), Some("sounds".into()), Some(built.into())];
    candidates.into_iter().flatten().find(|p| p.join("shot.luau").exists())
}

/// Patches by sound name, reloaded when their file changes.
#[derive(Default)]
pub struct Bank {
    dir: Option<PathBuf>,
    patches: HashMap<String, Arc<CompiledPatch>>,
    files: HashMap<PathBuf, vfs::Stamp>,
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
        let Ok(rd) = vfs::list(dir) else { return out };
        for path in rd {
            if path.extension().and_then(|x| x.to_str()) != Some("luau") { continue; }
            let Some(t) = vfs::stamp(&path) else { continue };
            if self.files.get(&path) == Some(&t) { continue; }
            let fresh = self.files.insert(path.clone(), t).is_some();
            match synth::lua::load_file(&path).and_then(|ps| ps.iter().map(Patch::compile).collect::<Result<Vec<_>, _>>()) {
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

    /// Every loaded patch (live `play_sound`).
    pub fn patches(&self) -> impl Iterator<Item = &Arc<CompiledPatch>> { self.patches.values() }

    /// A loaded patch, without the missing-patch warning.
    pub fn find(&self, name: &str) -> Option<Arc<CompiledPatch>> { self.patches.get(name).cloned() }

    /// Change a parameter's default for every later play (live `set_param`);
    /// values a trigger passes still win. Editing the file resets it.
    pub fn set_default(&mut self, name: &str, param: &str, value: f32) -> Result<f32, String> {
        let p = self.patches.get_mut(name).ok_or_else(|| format!("no sound {name}"))?;
        let (next, old) = p.with_default(param, value)?;
        *p = Arc::new(next);
        Ok(old)
    }

    pub fn get(&mut self, name: &str) -> Option<Arc<CompiledPatch>> {
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
    /// Local weapon report: reserves its own voice budget, flat on its side.
    center: bool,
    /// Override the room send without running a duplicate synth voice.
    room: Option<f32>,
    pan: f32,
    delay: bool,
}

impl G {
    fn at(at: Vec3, db: f32, unit: f32) -> G { G { at: Some(at), db, pitch: 1.0, unit, dry: false, spl: None, tail: true, center: false, room: None, pan: 0.0, delay: true } }
    fn ui(db: f32) -> G { G { at: None, db, pitch: 1.0, unit: 12.0, dry: false, spl: None, tail: false, center: false, room: None, pan: 0.0, delay: false } }
    fn dry(mut self) -> G { self.dry = true; self }
    fn spl(mut self, s: f32) -> G { self.spl = Some(s); self }
    fn pitch(mut self, p: f32) -> G { self.pitch = p; self }
    fn room(mut self, send: f32) -> G { self.room = Some(send); self }
    fn no_tail(mut self) -> G { self.tail = false; self }
}

const TAIL_SEND: f32 = 0.063; // -24 dB
/// sfx.gd `air_strike_inbound_db`.
const AIR_STRIKE_DB: f32 = 2.0;
/// Godot RaytracedReverb wet level (dry 1.0).
const ROOM_SEND: f32 = 0.35;

enum Later {
    CardFlip,
    NextRound(usize, Vec3),
    Casing(usize, Vec3, f32),
    Zip { at: Vec3, sf: f32, sc: f32, prox: f32 },
}

/// Start the speakers with the synth mixer (Godot's sound) and the simple
/// effects mixer (the crowd), summed. `layout` overrides the device's own
/// channel count (stereo, 5.1, 7.1). Returns the speaker (keep it alive) and
/// both senders. `music` joins the sum at the device's rate.
pub fn speakers(layout: Option<&str>, music: Option<&crate::music::Shared>) -> Result<(app::sound::Speaker, SoundSender, audio::sfx::SfxSender), String> {
    let mut made = None;
    let named = layout.and_then(synth::Layout::named);
    let speaker = app::sound::open(|rate, channels| {
        #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
        let (tx, mut mix) = synth::mixer(named.clone().unwrap_or_else(|| synth::Layout::for_channels(channels)), 160, rate);
        // Finished voices are freed here, not on the audio thread.
        // A browser has one thread: the mixer frees them itself.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(trash) = mix.take_trash() { std::thread::Builder::new().name("sfx-trash".into()).spawn(move || for g in trash { drop(g) }).ok(); }
        let (crowd, cmix) = audio::sfx::sfx(32, rate);
        made = Some((tx, crowd));
        let mut sum: Vec<Box<dyn audio::Source>> = vec![Box::new(mix), Box::new(cmix)];
        if let Some(m) = music { crate::music::lock(m).set_sample_rate(rate as f32); sum.push(Box::new(crate::music::Player::new(m))); }
        Box::new(audio::Sum::limited(sum, -0.3))
    })?;
    let (tx, crowd) = made.expect("built in open");
    Ok((speaker, tx, crowd))
}

/// The same pair offline at 48 kHz stereo, for scripted runs.
pub fn offline(music: Option<&crate::music::Shared>) -> (Box<dyn audio::Source>, SoundSender, audio::sfx::SfxSender) {
    let (tx, mix) = synth::mixer(synth::Layout::stereo(), 160, audio::SAMPLE_RATE);
    let (crowd, cmix) = audio::sfx::sfx(32, audio::SAMPLE_RATE);
    let mut sum: Vec<Box<dyn audio::Source>> = vec![Box::new(mix), Box::new(cmix)];
    if let Some(m) = music { crate::music::lock(m).set_sample_rate(audio::SAMPLE_RATE); sum.push(Box::new(crate::music::Player::new(m))); }
    (Box::new(audio::Sum::limited(sum, -0.3)), tx, crowd)
}

pub struct Sounds {
    tx: SoundSender,
    pub bank: Bank,
    steps: Vec<Arc<CompiledPatch>>,
    hurts: Vec<Arc<CompiledPatch>>,
    deaths: Vec<Arc<CompiledPatch>>,
    rng: u32,
    time: f32,
    later: Vec<(f32, Later)>,
    walked: Vec<f32>,
    last_zip: f32,
    lava_at: Vec<f32>,
    explosions: Vec<f32>,
    layers: [Vec<f32>; 4],
    reloads: HashMap<usize, Handle>,
    strikes: Vec<(StrikeKind, Vec3, Vec<Handle>)>,
    descents: HashMap<usize, Handle>,
    pickups: usize,
    incoming: Option<Vec3>,
    offers: Vec<(bool, f64)>,
    shot_tick: Vec<u64>,
    ears: Vec<Vec3>,
    /// Per player: Some(stereo pan) for a local human. Everything that
    /// comes from them plays flat (Joep: no surround around yourself), on
    /// their half's side in split screen.
    own_pan: Vec<Option<f32>>,
    /// Ducked by shots and explosions (sfx.gd's `ProceduralMusic.duck`).
    pub music: Option<crate::music::Shared>,
}

impl Sounds {
    /// `samples`: the Godot project's audio folder (footsteps, hurts, deaths).
    pub fn new(tx: SoundSender, patches: Option<PathBuf>, samples: Option<&Path>) -> Self {
        tx.master(synth::Master { compressors: vec![synth::Compressor { threshold: -8.0, ratio: 3.0, release: 0.16 }], ceiling: -1.0 });
        tx.cull(Some(synth::Cull::default()));
        tx.hall(synth::Hall::default());
        // Godot RaytracedReverb: damping .55; room size ~0.18 in the arena.
        tx.room(0.8, 0.55, 1.0);
        let load = |names: &mut dyn Iterator<Item = String>| -> Vec<Arc<CompiledPatch>> {
            let Some(dir) = samples else { return Vec::new() };
            names.filter_map(|n| audio::sfx::load_wav(&dir.join(&n)).map_err(|e| eprintln!("sound: {e}")).ok().map(|c| Arc::new(clip_patch(&n, &c).compile().expect("clip patch")))).collect()
        };
        let steps = load(&mut (1..=13).map(|i| format!("Step-{i}.wav")));
        let hurts = load(&mut (1..=12).map(|i| format!("hurt{i}.wav")));
        let deaths = load(&mut (1..=4).map(|i| format!("death{i}.wav")));
        if samples.is_some() { println!("sounds: {} recordings", steps.len() + hurts.len() + deaths.len()); }
        Sounds { tx, bank: Bank::load(patches), steps, hurts, deaths, rng: 0x1234_5678, time: 0.0, later: Vec::new(), walked: Vec::new(), last_zip: -1.0,
            lava_at: Vec::new(), explosions: Vec::new(), layers: std::array::from_fn(|_| Vec::new()), reloads: HashMap::new(), strikes: Vec::new(), descents: HashMap::new(), pickups: 0, incoming: None,
            offers: Vec::new(), shot_tick: Vec::new(), own_pan: Vec::new(), ears: Vec::new(), music: None }
    }


    /// A discontinuity is silent: discard active voices and delayed events,
    /// while retaining decoded samples and compiled sound patches.
    pub fn reset_replay(&mut self) {
        self.tx.stop_all();
        self.rng = 0x1234_5678;
        self.time = 0.0;
        self.later.clear(); self.walked.clear(); self.last_zip = -1.0;
        self.lava_at.clear(); self.explosions.clear();
        for layers in &mut self.layers { layers.clear(); }
        self.reloads.clear();
        self.strikes.clear(); self.descents.clear(); self.pickups = 0;
        self.incoming = None; self.offers.clear(); self.shot_tick.clear();
        self.own_pan.clear(); self.ears.clear();
    }

    /// The synth mixer (live `play_sound`).
    pub fn sender(&self) -> &SoundSender { &self.tx }

    /// How far the loudest recent sound is above "loud" (Godot's crowd duck: HDR max - 112 dB).
    pub fn duck_db(&self) -> f32 { (self.tx.loudest() - 112.0).max(0.0) }

    fn duck_music(&self, amount: f32) { if let Some(m) = &self.music { crate::music::lock(m).duck(amount); } }

    fn next(&mut self) -> u32 { self.rng ^= self.rng << 13; self.rng ^= self.rng >> 17; self.rng ^= self.rng << 5; self.rng }
    fn u(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * (self.next() >> 8) as f32 / (1u32 << 24) as f32 }
    fn pick(&mut self, set: &[Arc<CompiledPatch>]) -> Option<Arc<CompiledPatch>> { if set.is_empty() { None } else { let k = self.next() as usize % set.len(); Some(set[k].clone()) } }

    fn play(&mut self, world: &World, patch: &Arc<CompiledPatch>, params: &[(&str, f32)], g: G) -> Handle {
        let seed = self.next();
        let combat = matches!(patch.name.as_str(), "shot" | "explosive_launch" | "explosion_bang" | "explosion_rumble");
        let p = match g.at {
            None => {
                let db = g.db; // Authored level: no artificial close-muzzle gain.
                // `tail` on a flat sound: it still goes through the room.
                let (room, tail) = if g.tail { (ROOM_SEND, TAIL_SEND) } else { (0.0, 0.0) };
                let reverb = g.room.unwrap_or(room);
                Play { place: Place::Pan(g.pan), pitch: g.pitch, spl: g.spl, seed, reverb, tail, ..Default::default() }.db(db)
            }
            Some(at) => {
                let mut falloff = if g.dry { Falloff::godot(g.unit) } else {
                    // Raytraced players fade out linearly to where they are 30 dB down.
                    Falloff::godot(g.unit).fade(g.unit / 10f32.powf((-30.0 - g.db) / 20.0))
                };
                if combat { falloff.near_max = 1.0; }
                let occlusion = if g.dry { 0.0 } else { occlusion(world, &self.ears, at) };
                Play { place: Place::At(at.to_array()), falloff, pitch: g.pitch, spl: g.spl, seed, sound_delay: g.delay, occlusion,
                    reverb: g.room.unwrap_or(if g.dry { 0.0 } else { ROOM_SEND }), tail: if g.tail { TAIL_SEND } else { 0.0 }, ..Default::default() }.db(g.db)
            }
        };
        self.tx.play(patch, params, p)
    }

    /// A sound from player `p` at `at`: flat on their side when they are a
    /// local human, at the level the 3D sound had at their ears; else 3D.
    fn from(&self, world: &World, p: usize, at: Vec3, db: f32, unit: f32) -> G {
        match self.own_pan.get(p).copied().flatten() {
            Some(pan) => {
                let d = world.players[p].eye().distance(at).max(0.5);
                G { pan, ..G::ui((db + 20.0 * (unit / d).log10()).min(18.0)) }
            }
            None => G::at(at, db, unit),
        }
    }

    /// `from` for a sound due later: a player gone since plays it in 3D.
    fn from_alive(&self, world: &World, p: usize, at: Vec3, db: f32, unit: f32) -> G {
        if world.players.get(p).is_some() { self.from(world, p, at, db, unit) } else { G::at(at, db, unit) }
    }

    /// A 2D sound for local player `p` (hurt, hit markers): on their side.
    fn ui_for(&self, p: usize, db: f32) -> G { G { pan: self.own_pan.get(p).copied().flatten().unwrap_or(0.0), ..G::ui(db) } }

    // Admit whole voices before allocating DSP state. Leave existing tails intact
    // and reserve separate budgets for reports and bass, below the 160-voice pool.
    fn named(&mut self, world: &World, name: &str, params: &[(&str, f32)], mut g: G) -> Option<Handle> {
        let p = self.bank.get(name)?;
        let family = match name { "shot" | "explosive_launch" => Some(if g.center { 0 } else { 1 }), "explosion_bang" => Some(2), "explosion_rumble" => Some(3), _ => None };
        if let Some(family) = family {
            let delay = g.at.filter(|_| g.delay).map_or(0.0, |at| self.ears.iter().map(|e| e.distance(at)).fold(1000.0, f32::min) / 343.0);
            let ends = &mut self.layers[family];
            ends.retain(|&end| end > self.time);
            if ends.len() >= [80, 32, 24, 12][family] { return None; }
            // Lower new overlapping layers smoothly in energy, without modulating
            // a playing waveform or asking the limiter to flatten every transient.
            g.db -= 10.0 * (1.0 + ends.len() as f32).log10();
            ends.push(self.time + p.max_length.max(0.1) / g.pitch.max(0.1) + delay + 0.1);
        }
        Some(self.play(world, &p, params, g))
    }

    /// Sounds for one tick of events; `slots` are the local players (ears).
    pub fn tick(&mut self, world: &World, slots: &[usize], dt: f32) {
        for line in self.bank.poll(dt) { eprintln!("sfx: {line}"); }
        self.time += dt;
        self.ears = synth_ears(world, slots).iter().map(|e| Vec3::from(e.pos)).collect();
        self.tx.ears(synth_ears(world, slots));
        let local = |p: usize| slots.contains(&p) && !world.players.get(p).is_some_and(|q| q.bot);
        // Split screen: a local player's own sounds pan toward their half.
        let pan_of = |p: usize| -> f32 {
            let Some(k) = slots.iter().position(|&s| s == p) else { return 0.0 };
            if slots.len() < 2 { 0.0 } else if k % 2 == 0 { -0.8 } else { 0.8 }
        };
        self.own_pan = (0..world.players.len()).map(|p| local(p).then(|| pan_of(p))).collect();
        self.shot_tick.resize(world.players.len(), u64::MAX);
        let killed: Vec<(usize, usize)> = world.events.iter().filter_map(|e| match *e { Event::Kill { victim, attacker: Some(a), .. } => Some((victim, a)), _ => None }).collect();
        for e in &world.events {
            match *e {
                Event::Shot { player, from, dir } => {
                    // One shot sound per shooter per tick (Godot: per physics frame).
                    if self.shot_tick[player] == world.tick { continue; }
                    self.shot_tick[player] = world.tick;
                    if world.players[player].howitzer.is_some() {
                        let g = self.from(world, player, from, -2.0, 30.0);
                        if world.players[player].siege_crossbow {
                            self.named(world,"melee", &[("pitch_ratio",0.65)],g.spl(100.0));
                        } else { self.named(world, "grenade_launch", &[], g.spl(118.0)); }
                    } else { self.shot(world, player, from, dir, local(player), pan_of(player), slots); }
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
                        let g = self.from(world, player, at, -6.0, 12.0);
                        self.named(world, "grenade_launch", &[], g.spl(110.0));
                    }
                    _ => {}
                },
                Event::Slash { player, from, .. } => {
                    let w = &world.players[player].weapon;
                    let damage = w.melee_damage() as f32 * w.damage_mult;
                    let g = self.from(world, player, from, -6.0, 12.0);
                    self.named(world, "melee", &[("pitch_ratio", (50.0 / damage.max(1.0)).clamp(0.5, 1.2))], g.spl(90.0));
                }
                Event::CrowdLaunched { player } => {
                    let g = self.from(world, player, world.players[player].pos, -9., 8.);
                    self.named(world, "crowd_launch", &[], g.no_tail().spl(86.));
                    if local(player) {
                        let wind = self.ui_for(player, -12.);
                        self.named(world, "flight_wind", &[], wind.no_tail().spl(82.));
                    }
                }
                Event::RifleCollected { player } => {
                    let g = self.from(world, player, world.players[player].pos, -7., 8.);
                    self.named(world, "rifle_collect", &[], g.no_tail().spl(85.));
                }
                Event::Jump { player } => {
                    let at = world.players[player].pos;
                    for (db, lo, hi) in [(-7.5, 1.02, 1.12), (-10.5, 0.55, 0.65)] {
                        let (j, p) = (self.u(-1.5, 1.5), self.u(lo, hi));
                        let g = self.from(world, player, at, db + j, 12.0);
                        if let Some(s) = self.pick(&self.steps.clone()) { self.play(world, &s, &[], g.pitch(p).spl(75.0)); }
                    }
                }
                Event::Landed { player, speed } => {
                    if speed < 2.0 || world.players[player].ghost { continue; }
                    let t = ((speed - 2.0) / 23.0).clamp(0.0, 1.0);
                    let (db, pitch, spl) = (lerp(-22.0, 5.0, t), lerp(1.05, 0.55, t), lerp(75.0, 95.0, t));
                    let at = world.players[player].pos;
                    let (g, g2) = (self.from(world, player, at, db, 12.0), self.from(world, player, at, db - 4.0, 12.0));
                    if let Some(s) = self.pick(&self.steps.clone()) { self.play(world, &s, &[], g.pitch(pitch).spl(spl)); }
                    if t > 0.25 && let Some(s) = self.pick(&self.steps.clone()) { self.play(world, &s, &[], g2.pitch(pitch * 0.5).spl(spl)); }
                    if let Some(h) = self.descents.remove(&player) { self.tx.release(&h); self.tx.stop(&h); }
                }
                Event::Dash { player } => { let g = self.from(world, player, world.players[player].pos, -24.0, 12.0); self.named(world, "dash", &[], g.spl(75.0)); }
                Event::Hit { victim, attacker, damage, head, pos } => {
                    // Hurt: the victim's own is 2D.
                    if let Some(s) = self.pick(&self.hurts.clone()) {
                        if local(victim) { let g = self.ui_for(victim, -25.0); self.play(world, &s, &[], g.spl(100.0)); }
                        else { let j = self.u(-1.5, 1.5); self.play(world, &s, &[], G::at(world.players[victim].pos, -17.5 + j, 12.0).spl(80.0)); }
                    }
                    if local(victim) {
                        let i = (damage as f32 / 25.0).clamp(0.04, 2.0);
                        let g = self.ui_for(victim, -12.0 + 20.0 * i.clamp(0.05, 2.0).log10());
                        self.named(world, "hit_received", &[], g.spl(130.0));
                    }
                    if local(attacker) && attacker != victim && !killed.contains(&(victim, attacker)) {
                        let r = if damage == 0 { 1.0 } else { (damage as f32 / 22.0).clamp(0.5, 5.0) };
                        let name = if head { "hitmarker_head" } else { "hitmarker" };
                        let g = self.ui_for(attacker, -5.0 + (2.0 * r.log2()).clamp(0.0, 4.0));
                        self.named(world, name, &[("dmg_ratio", r)], g.spl(130.0));
                    }
                    let _ = pos;
                }
                Event::Kill { victim, attacker, .. } => {
                    if let Some(s) = self.pick(&self.deaths.clone()) {
                        if local(victim) { let g = self.ui_for(victim, -12.0); self.play(world, &s, &[], g.spl(110.0)); }
                        else { let j = self.u(-1.5, 1.5); self.play(world, &s, &[], G::at(world.players[victim].pos, -8.0 + j, 12.0).spl(85.0)); }
                    }
                    if let Some(a) = attacker && local(a) && a != victim { let g = self.ui_for(a, -6.0); self.named(world, "kill_confirm", &[], g.spl(130.0)); }
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
                    let g = self.from(world, player, world.players[player].pos, db, 18.0);
                    self.named(world, "lava_sizzle", &[("fall_death", if fall { 1.0 } else { 0.0 })], g.pitch(pitch).no_tail().spl(spl));
                }
                Event::Reload { player, seconds } => {
                    if let Some(h) = self.reloads.remove(&player) { self.tx.stop(&h); }
                    let g = if local(player) { self.ui_for(player, -14.0) } else { G::at(world.players[player].eye(), -14.0, 12.0) };
                    if let Some(h) = self.named(world, "reload", &[("duration", seconds)], g.spl(70.0)) { self.reloads.insert(player, h); }
                }
                Event::MinePlanted { pos, .. } => { self.named(world, "mine_plant", &[], G::at(pos, -10.0, 12.0).spl(90.0)); }
                Event::Empty { player } => { let g = self.from(world, player, world.players[player].eye(), -10.0, 5.0); self.named(world, "empty_chamber", &[], g.dry().spl(55.0)); }
                Event::PickedUp { player, .. } => if local(player) { let g = self.ui_for(player, -10.0); self.named(world, "pling", &[("pitch_ratio", 1.15)], g.spl(70.0)); },
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
        let dmg_db = (d.max(0.5).log2() * 1.0).clamp(-4.0, 3.0);
        let silenced = w.silencer_stacks > 0;
        let mut pitch = (1.0 - d.max(1.0).log2() * 0.2).clamp(0.35, 1.15) * self.u(0.97, 1.03);
        let (mut self_base, mut world_base) = (-12.0 + self.u(-1.5, 1.5), -14.0 + self.u(-1.5, 1.5));
        if silenced { self_base -= 10.0; world_base -= 12.0; pitch *= 1.08; }
        let dry = if is_self { self_base } else { world_base } + dmg_db;
        self.duck_music(crate::music::shot_duck(w.damage_for_shot(last)));
        let params = [("damage", (w.damage() / 22.0).max(1.0)), ("accuracy", (1.0 - w.spread() / 0.06).clamp(0.0, 1.0)), ("velocity", w.bullet_speed_mult.clamp(0.5, 3.0))];
        let g = if is_self { G { center: true, pan, unit: 6.0, ..G::ui(dry) } } else { G::at(from, dry, 3.0).dry() };
        // A send from the same report replaces the independently generated wet
        // copy. Weapon feedback must survive a blast's HDR gate (spl=None).
        self.named(world, "shot", &params, g.pitch(pitch).room(0.10));
        if w.explosive_radius > 0.0 {
            let g = if is_self { G { center: true, pan, ..G::ui(-22.0) } }
                else { G::at(from, -24.0, 8.0).dry() };
            self.named(world, "explosive_launch", &[], g.no_tail().room(0.03));
        }
        if d > 1.4 && w.explosive_radius <= 0.0 {
            let rr = (d * 1.5).clamp(2.0, 12.0).round();
            let g = if is_self { G { center: true, pan, unit: (rr * 0.8).clamp(4.0, 12.0), ..G::ui(-18.0) } } else { G::at(from, -18.0, (rr * 0.8).clamp(4.0, 12.0)).dry().no_tail() };
            self.named(world, "explosion_rumble", &[("radius", rr)], g);
        }
        // The bolt click in the gap before the next shot (not on fast guns).
        let fi = w.fire_interval() as f32;
        if fi >= 0.18 { self.later.push((self.time + (fi * 0.45).min(0.13), Later::NextRound(player, from))); }
        // The casing lands about half a second later beside the shooter.
        let side = Vec3::new(p.yaw.cos(), 0.0, -p.yaw.sin());
        let floor = p.pos + side * 0.6;
        let speed_ratio = self.u(0.45, 0.8);
        let at = self.time + 0.42 + self.u(0.0, 0.08);
        self.later.push((at, Later::Casing(player, floor, speed_ratio)));
        if self.u(0.0, 1.0) < 0.6 { self.later.push((self.time + 0.6, Later::Casing(player, floor + side * 0.2, speed_ratio * 0.35))); }
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
        let spl = 118.0 + 4.0 * (r / 6.0).max(1.0).log2();
        self.duck_music(crate::music::explosion_duck(r));
        self.named(world, "explosion_bang", &[("radius", rb)], G::at(pos, 10.0, bang_unit).dry().room(0.08).spl(spl));
        self.named(world, "explosion_rumble", &[("radius", rb)], G::at(pos, 3.0 * ((r - 6.0) / 18.0).clamp(0.0, 1.0), rumble_unit).dry().no_tail().spl(spl));
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
                    if let Some(h) = self.strikes[k].2.first() {
                        self.tx.move_to(h, Place::At(at.to_array()), 0.0);
                        // air_strike.gd `_process`: the woosh swells on top of its
                        // own envelope, -4 dB + linear_to_db(lerp(0.62, 1, u²)).
                        self.tx.gain(h, 10f32.powf(AIR_STRIKE_DB / 20.0) * (0.62 + 0.38 * u * u));
                    }
                }
                continue;
            }
            let mut handles = Vec::new();
            match s.kind {
                StrikeKind::AirStrike => {
                    let travel = growing_guns::AIR_STRIKE_TRAVEL as f32;
                    let sky = s.target + Vec3::new(0.0, 263.0, 93.0);
                    self.tx.lift(0.42);
                    if let Some(h) = self.named(world, "air_strike_inbound", &[("travel_seconds", travel), ("distant", 0.0)], G { delay: false, ..G::at(sky, AIR_STRIKE_DB, 80.0) }) {
                        // The swell starts at 0.62 (air_strike.gd `_process`).
                        self.tx.gain(&h, 10f32.powf(AIR_STRIKE_DB / 20.0) * 0.62);
                        handles.push(h);
                    }
                    // The telegraph at the target.
                    self.named(world, "mine_plant", &[], G::at(s.target, -10.0, 12.0).spl(90.0));
                }
                StrikeKind::IonCannon => {
                    let charge = growing_guns::ION_CHARGE as f32;
                    self.tx.lift(0.52);
                    // Attached at the target (not culled, not delayed), and a positional companion.
                    if let Some(h) = self.named(world, "ion_cannon_charge", &[("charge_seconds", charge)], G { delay: false, ..G::at(s.target, -14.0, 140.0) }) { handles.push(h); }
                    if let Some(h) = self.named(world, "ion_cannon_charge", &[("charge_seconds", charge)], G::at(s.target, -10.0, 140.0).spl(118.0)) { handles.push(h); }
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
                            let (prev_left, left) = (world.card_pick_seconds - age, world.card_pick_seconds - p.offer_age);
                            if had && prev_left > th && left <= th {
                                self.named(world, "pick_countdown_beep", &[("seconds_left", th as f32)], G::ui(-8.0).spl(140.0));
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
                Later::NextRound(p, at) => { let g = self.from_alive(world, p, at, -16.0, 5.0); self.named(world, "next_round", &[], g.dry().spl(65.0)); }
                Later::Casing(p, at, ratio) => {
                    let db = -18.0 + lerp(-22.0, 2.0, ratio.clamp(0.05, 1.2));
                    let pitch = self.u(0.96, 1.04);
                    let g = self.from_alive(world, p, at, db, 6.0);
                    self.named(world, "casing_drop", &[], g.dry().no_tail().pitch(pitch).spl(75.0));
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
            let g = self.from(world, i, at, db, 12.0);
            if let Some(st) = self.pick(&self.steps.clone()) { self.play(world, &st, &[], g.pitch(pitch).spl(78.0)); }
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

#[cfg(test)]
mod audio_stress_tests {
    use super::*;
    #[test]
    fn stacked_uzi_bazooka_mix() { stress_mix(synth::Layout::stereo(), 48000.0, true); }

    #[test]
    fn stacked_uzi_bazooka_surround() { stress_mix(synth::Layout::named("5.1").unwrap(), 44100.0, false); }

    #[test]
    fn inbound_rocket_rumble_stays_bounded_when_stacked() {
        let world=World::new(17,&[false]);
        let (tx,mut mix)=synth::mixer(synth::Layout::stereo(),160,48000.0);
        let mut sounds=Sounds::new(tx,Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"),"/../sounds"))),None);
        sounds.tx.ears(vec![Ear {pos:[0.0;3],forward:[0.0,0.0,-1.0],right:[1.0,0.0,0.0]}]);
        sounds.ears=vec![Vec3::ZERO];
        for _ in 0..4 { sounds.named(&world,"air_strike_inbound",&[("travel_seconds",2.3),("distant",0.0)],G {delay:false,..G::at(Vec3::NEG_Z*3.0,AIR_STRIKE_DB,80.0)}); }
        let samples=mix.render_seconds(2.3);
        assert!(samples.iter().all(|s|s.is_finite() && s.abs()<=1.0));
        assert!(samples.iter().any(|s|s.abs()>0.05));
    }

    #[test]
    fn explosions_dominate_weapon_reports() {
        let mut world = World::new(17, &[false]);
        crate::scripts::attach_dir(&mut world, None).unwrap();
        for id in ["uzi", "uzi", "bazooka", "bazooka"] {
            world.give(0, world.cards.find(id).unwrap());
        }
        let mut levels = Vec::new();
        for blast in [false, true] {
            let (tx, mut mix) = synth::mixer(synth::Layout::stereo(), 160, 48000.0);
            let mut sounds = Sounds::new(tx, Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../sounds"))), None);
            sounds.tx.ears(vec![Ear { pos: [0.0;3], forward: [0.0,0.0,-1.0], right: [1.0,0.0,0.0] }]);
            sounds.ears = vec![Vec3::ZERO];
            if blast { sounds.explosion(&world, Vec3::new(0.0,0.0,-2.0), 24.0, ExplosionKind::Conventional); }
            else { sounds.shot(&world, 0, -Vec3::Z, -Vec3::Z, true, 0.0, &[0]); }
            let audio = mix.render_seconds(4.0);
            if blast {
                // The aftershock must remain audible after the transient has died.
                let tail = &audio[144_000..240_000]; // 1.5-2.5 s, 48 kHz stereo.
                let rms = (tail.iter().map(|v| v*v).sum::<f32>() / tail.len() as f32).sqrt();
                assert!(rms >= 0.08, "Large explosion lost its sustained rumble: {rms}");
            }
            let loudest = audio.chunks(4800).map(|c| (c.iter().map(|v| v*v).sum::<f32>() / c.len() as f32).sqrt()).fold(0.0f32, f32::max);
            levels.push(loudest);
            if let Ok(dir) = std::env::var("GG_AUDIO_REVIEW") {
                audio::analysis::write_wav(&Path::new(&dir).join(if blast { "explosion.wav" } else { "shot.wav" }), 48000, 2, &audio).unwrap();
            }
        }
        let advantage = 20.0 * (levels[1] / levels[0]).log10();
        println!("BALANCE shot={} blast={} explosion advantage={advantage:.2} dB", levels[0], levels[1]);
        assert!(advantage >= 4.0, "Explosion must clearly dominate the report");
    }

    fn stress_mix(layout: synth::Layout, rate: f32, capture: bool) {
        let mut world = World::new(17, &[false; 4]);
        crate::scripts::attach_dir(&mut world, None).unwrap();
        for player in 0..4 {
            for id in ["uzi", "uzi", "bazooka", "bazooka"] {
                let card = world.cards.find(id).unwrap();
                world.give(player, card);
            }
        }
        let channels = layout.channels();
        let (tx, mut mix) = synth::mixer(layout, 160, rate);
        let mut sounds = Sounds::new(tx, Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../sounds"))), None);
        sounds.tx.ears(vec![Ear { pos: [0.0;3], forward: [0.0,0.0,-1.0], right: [1.0,0.0,0.0] }]);
        sounds.ears = vec![Vec3::ZERO];
        let mut audio = Vec::new();
        let mut max_voices = 0;
        let start = web_time::Instant::now();
        // 12 close shots + blasts per second, then four shooters, then decay.
        for frame in 0..840 {
            sounds.time = frame as f32 / 60.0;
            if frame < 480 && frame % 5 == 0 {
                for player in 0..if frame < 240 { 1 } else { 4 } {
                    sounds.shot(&world, player, Vec3::new(0.0,0.0,-1.0), -Vec3::Z, true, 0.0, &[0,1,2,3]);
                    sounds.explosion(&world, Vec3::new(0.0,0.0,-2.0), 24.0, ExplosionKind::Conventional);
                }
            }
            sounds.flush_later(&world);
            audio.extend(mix.render_seconds(1.0/60.0));
            max_voices = max_voices.max(mix.active());
        }
        let peak = audio.iter().fold(0.0f32, |a,b| a.max(b.abs()));
        let tail = audio[audio.len() - rate as usize * channels..].iter().fold(0.0f32, |a,b| a.max(b.abs()));
        println!("STRESS peak={peak} tail={tail} max_voices={max_voices} stolen={} culled={} render_seconds={:.3}", mix.stolen, mix.culled, start.elapsed().as_secs_f32());
        if capture && let Ok(path) = std::env::var("GG_AUDIO_CAPTURE") { audio::analysis::write_wav(Path::new(&path), rate as u32, channels as u16, &audio).unwrap(); }
        assert!(audio.iter().all(|s| s.is_finite()));
        assert!(peak <= 0.892, "Sustained spam must leave headroom, peak={peak}");
        assert_eq!(mix.stolen, 0, "Do not hard-cut playing waveforms at the pool limit");
        assert!(max_voices < 130, "Keep slots for UI and character cues");
        assert!(tail < 0.001, "Bass must settle after firing stops");
        assert_eq!(mix.active(), 0);
    }
}
