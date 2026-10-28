//! Growing Guns' sounds from the Godot project's WAVs (`assets/audio`) and,
//! for impacts and explosions, synthesised like Godot's `sfx.gd`
//! (`synth.rs`), played through `audio::sfx` and placed for the local players.
use audio::sfx::{spatial, Clip, Listener, SfxSender};
use growing_guns::{Event, World, EYE};
use std::path::Path;

pub struct Sounds {
    tx: SfxSender,
    gun: Vec<Clip>,
    jump: Vec<Clip>,
    hurt: Vec<Clip>,
    death: Vec<Clip>,
    step: Vec<Clip>,
    card: Vec<Clip>,
    pick: u32,
    walked: Vec<f32>,
    bank: crate::synth::Bank,
}

fn load(dir: &Path, names: impl Iterator<Item = String>) -> Vec<Clip> {
    names.filter_map(|n| audio::sfx::load_wav(&dir.join(&n)).map_err(|e| eprintln!("sound: {e}")).ok()).collect()
}

/// The Godot project's sound folder: `--sounds DIR`, `GG_SOUNDS`, or a
/// sibling checkout `../growing-guns/assets/audio`.
/// One listener per local player, at eye height.
pub fn ears(world: &World, slots: &[usize]) -> Vec<Listener> {
    slots.iter().map(|&s| {
        let p = &world.players[s];
        let (sy, cy) = p.yaw.sin_cos();
        Listener { pos: (p.pos + glam::Vec3::Y * EYE).to_array(), right: [cy, 0.0, -sy] }
    }).collect()
}

pub fn find_dir(arg: Option<&str>) -> Option<std::path::PathBuf> {
    let candidates: [Option<String>; 3] = [arg.map(str::to_string), std::env::var("GG_SOUNDS").ok(), Some("../growing-guns/assets/audio".to_string())];
    candidates.into_iter().flatten().map(std::path::PathBuf::from).find(|p| p.join("jump1.wav").exists())
}

impl Sounds {
    pub fn load(dir: &Path, tx: SfxSender) -> Self {
        let s = Sounds {
            tx,
            gun: load(dir, (1..=5).map(|i| format!("Gun - var{i}.wav"))),
            jump: load(dir, (1..=4).map(|i| format!("jump{i}.wav"))),
            hurt: load(dir, (1..=12).map(|i| format!("hurt{i}.wav"))),
            death: load(dir, (1..=4).map(|i| format!("death{i}.wav"))),
            step: load(dir, (1..=13).map(|i| format!("Step-{i}.wav"))),
            card: load(dir, std::iter::once("Card Pick.wav".to_string())),
            pick: 0,
            walked: Vec::new(),
            bank: Default::default(),
        };
        println!("sounds: {} clips from {}", s.gun.len() + s.jump.len() + s.hurt.len() + s.death.len() + s.step.len(), dir.display());
        s
    }

    fn pick<'a>(&mut self, set: &'a [Clip]) -> Option<&'a Clip> {
        self.pick = self.pick.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        set.get((self.pick >> 16) as usize % set.len().max(1))
    }

    fn next(&mut self) -> u32 { self.pick = self.pick.wrapping_mul(1_103_515_245).wrapping_add(12_345); self.pick >> 16 }

    fn jitter(&self) -> f32 { 0.94 + ((self.pick >> 8) % 1000) as f32 * 0.00012 }

    fn play(&mut self, set: usize, at: [f32; 3], ears: &[Listener], unit: f32, db: f32) {
        let set = match set { 0 => &self.gun, 1 => &self.jump, 2 => &self.hurt, 3 => &self.death, 5 => &self.card, _ => &self.step }.clone();
        let Some(clip) = self.pick(&set).cloned() else { return };
        let play = spatial(at, ears, unit).db(db).pitch(self.jitter());
        self.tx.play(&clip, play);
    }

    /// Sounds for one tick of events; `slots` are the local players (ears).
    pub fn tick(&mut self, world: &World, slots: &[usize], dt: f32) {
        let ears = ears(world, slots);
        for e in &world.events {
            match *e {
                Event::Shot { from, .. } => self.play(0, from.to_array(), &ears, 25.0, -4.0),
                Event::Jump { player } => self.play(1, world.players[player].pos.to_array(), &ears, 8.0, -2.0),
                Event::Hit { victim, .. } => self.play(2, world.players[victim].pos.to_array(), &ears, 12.0, 0.0),
                Event::Kill { victim, .. } => self.play(3, world.players[victim].pos.to_array(), &ears, 16.0, 2.0),
                // Only the picker hears their card (it plays at their own ear).
                Event::Picked { player, .. } => if let Some(k) = slots.iter().position(|&s| s == player) { self.play(5, ears[k].pos, &ears, 1000.0, -2.0) },
                Event::Impact { pos, damage, .. } => {
                    let ratio = (damage - growing_guns::destruct::RIFLE_CHIP_DAMAGE_BASE) / growing_guns::destruct::RIFLE_CHIP_DAMAGE_PER_RATIO;
                    let v = self.next();
                    let clip = self.bank.impact(ratio, v);
                    let db = -14.0 + ((ratio.max(1.0)).log2() * 4.0).clamp(0.0, 9.0);
                    self.tx.play(&clip, spatial(pos.to_array(), &ears, 8.0).db(db).pitch(self.jitter()));
                }
                Event::Explosion { pos, radius, .. } => {
                    let v = self.next();
                    let (bang, rumble) = (self.bank.bang(radius, v), self.bank.rumble(radius, v));
                    self.tx.play(&bang, spatial(pos.to_array(), &ears, (radius * 0.8).clamp(4.0, 24.0)).db(2.0));
                    self.tx.play(&rumble, spatial(pos.to_array(), &ears, (radius * 4.5).clamp(20.0, 100.0)).db(0.0));
                }
                _ => {}
            }
        }
        // Footsteps every 2.6 m on the floor.
        self.walked.resize(world.players.len(), 0.0);
        for (i, p) in world.players.iter().enumerate() {
            if !p.alive || !p.on_floor { continue; }
            self.walked[i] += (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt() * dt;
            if self.walked[i] > 2.6 {
                self.walked[i] = 0.0;
                self.play(4, p.pos.to_array(), &ears, 5.0, -10.0);
            }
        }
    }
}
