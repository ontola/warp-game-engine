//! A descent (run.gd): five biomes, one big level each. Walk the level
//! along its path; each time the wizards get further in, monsters come at
//! them from ahead. Two clearings on the way are held by a larger pack: beat
//! it and a shrine of tomes rises there (and the fallen get back up). At the
//! end of the level is the boss's arena; beat the boss for the last shrine
//! and the portal on to the next biome. The first level starts with the
//! practice strip (practice.gd) for wizards who know nothing yet.

use crate::El;
use crate::enemy::{self, Spawn, kinds};
use crate::fx::{self, C, Ev};
use crate::level::Level;
use crate::relics;
use crate::rng::{Pcg, Rand};
use crate::wizard::{ELEMENT_ORDER, Wizard};
use crate::world::{Id, Later, World};
use glam::{Vec2, Vec3};
use std::collections::BTreeMap;

pub const PORTAL_R: f32 = 1.7;
/// Everyone stands in the portal's ring this long to go on.
pub const PORTAL_TIME: f64 = 1.2;
/// Metres along the path between the packs that come at you on the way.
pub const WAVE_GAP: f32 = 30.0;
pub const FIRST_WAVE: f32 = 8.0;
pub const FIRST_CALM: f32 = 22.0;
/// Stand in an altar's ring this long to take its tome.
pub const TAKE_TIME: f64 = 0.9;
pub const ALTAR_R: f32 = 1.4;
/// A monster left this far from every wizard no longer holds anything up.
pub const STRAY_DIST: f32 = 45.0;
pub const GATE_BACK: f32 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Fight,
    Cleared,
    Travel,
    Dead,
    Won,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncKind {
    Wave,
    /// A clearing's pack, and the shrine it holds.
    Guard(usize),
    Boss,
}

#[derive(Clone, Copy, Debug)]
pub struct Encounter {
    /// Metres along the path the front wizard must reach.
    pub at: f32,
    pub kind: EncKind,
}

/// A tome on a shrine.
#[derive(Clone, Debug)]
pub struct Altar {
    pub pos: Vec3,
    pub relic: usize,
    pub taken: bool,
    /// How far the ring has filled, and in whose colour.
    pub hold: f64,
    pub fill_c: C,
    /// "biome_k": each wizard takes one tome per shrine.
    pub shrine: String,
    /// Rising (0 to 1), or sinking away once taken (1 to 0).
    pub rise: f32,
}

/// What a practice ward waits for before it opens, and what it does then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Urns,
    Archer,
    Caster,
    Shrine,
}

/// A wall of mist across the way on (run.gd _wards).
#[derive(Clone, Debug)]
pub struct Ward {
    /// The shrine it waits for, -1 for a practice stop.
    pub k: i32,
    /// Where it crosses the path, and the way on there (unit): it stands
    /// across the path, turned with it.
    pub c: Vec2,
    pub n: Vec2,
    pub at: f32,
    /// How far it reaches to either side of `c`, along `across()` (lo < 0).
    pub lo: f32,
    pub hi: f32,
    /// Shut and holding the wizards back (ward_step).
    pub holds: bool,
    pub check: Option<Check>,
    /// Opening: 1 shut, fading to 0, then gone.
    pub open: f32,
    pub opening: bool,
    /// The boss arena's door behind you (Joep): it faces back the way you
    /// came, closes once everyone is in and holds you in, not out.
    pub back: bool,
}

impl Ward {
    /// Along the wall, left to right as you face the way on.
    pub fn across(&self) -> Vec2 {
        Vec2::new(-self.n.y, self.n.x)
    }

    /// How far `p` is past the wall (negative: this side), and whether it
    /// is within the wall's width.
    pub fn past(&self, p: Vec2) -> (f32, bool) {
        let d = p - self.c;
        let t = d.dot(self.across());
        (d.dot(self.n), t > self.lo - 1.0 && t < self.hi + 1.0)
    }

    /// The middle of the wall.
    pub fn mid(&self) -> Vec2 {
        self.c + self.across() * (self.lo + self.hi) * 0.5
    }
}

/// The practice strip on the first level (practice.gd).
#[derive(Clone, Debug, Default)]
pub struct Practice {
    pub start: f32,
    pub urns: Vec<usize>,
    pub archer: Option<Id>,
    pub caster: Option<Id>,
    pub shrine: Option<Vec3>,
    pub shrine_els: Vec<El>,
    pub fill: f64,
    hold: BTreeMap<usize, f64>,
    /// How long the current lesson's monster has lain beaten: its wall
    /// opens a little later even if nobody rolled or deflected, so the
    /// strip never holds a player who missed the hint.
    beaten_t: f64,
}

pub mod practice {
    pub const URNS_AT: f32 = 10.0;
    /// Seconds after a lesson's monster falls that its wall opens anyway.
    pub const GRACE: f64 = 3.0;
    pub const WARD_1: f32 = 14.0;
    pub const ARCHER_AT: f32 = 24.0;
    pub const WARD_2: f32 = 27.0;
    pub const CASTER_AT: f32 = 36.0;
    pub const WARD_3: f32 = 39.0;
    pub const SHRINE_AT: f32 = 44.0;
    pub const WARD_4: f32 = 47.0;
    /// Where the first pack waits, past the strip.
    pub const END: f32 = 56.0;
    pub const SHRINE_R: f32 = 1.4;
    pub const LEARN_TIME: f64 = 0.9;
}

#[derive(Clone, Debug)]
pub struct Run {
    pub biome_index: usize,
    pub state: State,
    pub spawn_queue: Vec<(usize, i32)>,
    incoming: BTreeMap<i32, i32>,
    tag: i32,
    spawn_t: f64,
    pub encounters: Vec<Encounter>,
    wave_at: f32,
    pending: Vec<usize>,
    guard_quiet: BTreeMap<usize, f64>,
    pub rng: Pcg,
    pub seed: u64,
    pub kills: u64,
    pub altars: Vec<Altar>,
    pub portal: Option<Vec3>,
    pub portal_t: f64,
    end_t: f64,
    quiet_t: f64,
    pub boss_fell: bool,
    pub boss: Option<Id>,
    pub wards: Vec<Ward>,
    /// The boss arena has shut behind the party: the whole of it is theirs
    /// to spread out in (wizard::leash).
    pub sealed: bool,
    pub practice: Option<Practice>,
    lead: f32,
    lead_t: f64,
    /// Unlocks earned but not yet given (wizard, up to, gate). A shrine's
    /// unlock waits for its wizard to walk through where its wall of mist
    /// stood (the gate: that wall, kept after it is gone); one with no
    /// wall waits for a calm moment (Joep: never mid-fight), `calm_t`
    /// seconds so far.
    learn: Vec<(Id, i32, Option<Ward>)>,
    calm_t: f64,
    /// This level's first shrine has taught.
    taught: bool,
    /// Shrines that have risen this level ("{biome}_{k}"), with or without
    /// tomes: one with none left to offer must still open its wall.
    risen: Vec<String>,
}

impl Default for Run {
    fn default() -> Run {
        Run {
            biome_index: 0,
            state: State::Fight,
            spawn_queue: Vec::new(),
            incoming: BTreeMap::new(),
            tag: -1,
            spawn_t: 0.0,
            encounters: Vec::new(),
            wave_at: -1.0,
            pending: Vec::new(),
            guard_quiet: BTreeMap::new(),
            rng: Pcg::new(1),
            seed: 1,
            kills: 0,
            altars: Vec::new(),
            risen: Vec::new(),
            portal: None,
            portal_t: 0.0,
            end_t: 0.0,
            quiet_t: 0.0,
            boss_fell: false,
            boss: None,
            wards: Vec::new(),
            sealed: false,
            practice: None,
            lead: 0.0,
            lead_t: 0.0,
            learn: Vec::new(),
            calm_t: 0.0,
            taught: false,
        }
    }
}

impl Run {
    /// How deep in the run this level is (0 is the first).
    pub fn depth(&self) -> usize {
        self.biome_index
    }
}

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

impl World {
    /// A new run on seed `s` (Run.start). The tomes found stay with you
    /// into the next run, even after dying.
    pub fn run_start(&mut self, s: u64) {
        self.run.seed = s;
        self.run.rng = Pcg::new(s);
        self.run.biome_index = 0;
        self.run.kills = 0;
        for id in self.wizards.clone() {
            self.wiz_mut(id).took.clear();
            self.revive_id(id, 1.0);
            let mut a = self.am(id);
            a.hp = a.max_hp;
        }
        self.enter_level();
    }

    pub fn enter_level(&mut self) {
        self.clear_level();
        self.run.state = State::Fight;
        let depth = self.run.depth();
        self.level = Level::build(
            self.run.biome_index,
            self.run.seed.wrapping_mul(31).wrapping_add(depth as u64),
        );
        // Each level teaches once at its first shrine and once at its boss.
        self.run.taught = false;
        for id in self.wizards.clone() {
            let wind = self.a(id).mods.wind as i32;
            self.am(id).winds = wind;
        }
        let sz = self.level.start;
        let starts = [
            Vec2::new(-1.5, -1.0),
            Vec2::new(1.5, -1.0),
            Vec2::new(-3.5, 0.5),
            Vec2::new(3.5, 0.5),
            Vec2::new(0.0, 1.5),
            Vec2::new(-5.0, 1.5),
        ];
        for (i, id) in self.wizards.clone().into_iter().enumerate() {
            if !self.alive(id) {
                self.revive_id(id, 0.4);
            }
            let at = sz + starts[i % starts.len()];
            {
                let mut a = self.am(id);
                a.pos = Vec3::new(at.x, 0.0, at.y);
                a.vel = Vec3::ZERO;
                a.push = Vec3::ZERO;
            }
            self.wiz_mut(id).aim_dir = Vec3::NEG_Z;
        }
        // Packs along the way, evenly spread between the clearings; the two
        // clearings' packs where they are; the boss at the arena's edge. The
        // first level starts quiet for a bit, to learn the controls.
        let mut guards = Vec::new();
        for k in 0..2 {
            guards.push(Encounter {
                at: self.level.clearing_at[k] - self.level.clearings[k].z * 0.6,
                kind: EncKind::Guard(k),
            });
        }
        let boss_at = self.level.clearing_at[2] - self.level.clearings[2].z * 0.8;
        let mut first = if self.run.biome_index == 0 {
            crate::knobs::first_calm() as f32
        } else {
            crate::knobs::first_wave() as f32
        };
        // Knob `practice` 0: nobody walks the strip, so nobody starts
        // without their first two elements either.
        if crate::knobs::practice() < 0.5 {
            for id in self.wizards.clone() {
                if self.wiz(id).unlocks < 0 {
                    self.unlock_to(id, 0);
                }
            }
        }
        if self.run.biome_index == 0 && self.practice_wanted() {
            self.practice_build();
            let start = self.run.practice.as_ref().map(|p| p.start).unwrap_or(0.0);
            first = first.max(start + practice::END);
        }
        let mut stops = vec![first];
        for g in &guards {
            if g.at > first + 12.0 {
                stops.push(g.at);
            }
        }
        stops.push(boss_at);
        self.run.encounters.push(Encounter {
            at: first,
            kind: EncKind::Wave,
        });
        for j in 0..stops.len() - 1 {
            let (a, b) = (stops[j], stops[j + 1]);
            let n = (((b - a) / crate::knobs::wave_gap() as f32).ceil() as i32 - 1).max(0);
            for m in 0..n {
                let t = (m as f32 + 1.0 + self.run.rng.range(-0.15, 0.15)) / (n as f32 + 1.0);
                self.run.encounters.push(Encounter {
                    at: a + (b - a) * t,
                    kind: EncKind::Wave,
                });
            }
        }
        self.run.encounters.extend(guards);
        self.run.encounters.push(Encounter {
            at: boss_at,
            kind: EncKind::Boss,
        });
        // Godot's sort_custom is not stable either; ties never happen here.
        self.run.encounters.sort_by(|a, b| a.at.total_cmp(&b.at));
        self.build_wards();
        let biome = self.run.biome_index;
        self.ev(Ev::Level { biome, depth });
    }

    /// For testing: put the wizards `run` metres along the level, as if the
    /// packs on the way were never met. Below 0: just short of the boss.
    pub fn skip_to(&mut self, mut run: f32) {
        if run < 0.0 {
            run = self.level.clearing_at[2] - self.level.clearings[2].z - 3.0;
        }
        self.run.encounters.retain(|e| e.at >= run);
        let ca = self.level.clearing_at.clone();
        self.run.wards.retain(|w| {
            let at = if w.k < 0 { w.at } else { ca[w.k as usize] };
            at >= run
        });
        let c = self.level.point_at(run);
        for (i, id) in self.wizards.clone().into_iter().enumerate() {
            let x = (i % 3) as f32 - 1.0;
            let z = (i / 3) as f32;
            self.am(id).pos = Vec3::new(c.x + x * 1.5, 0.0, c.y + z * 1.5);
        }
    }

    fn clear_level(&mut self) {
        for id in self
            .enemies
            .drain(..)
            .chain(self.corpses.drain(..))
            .chain(self.fx.drain(..))
            .chain(self.fields.drain(..))
        {
            let _ = self.ecs.despawn(id);
        }
        self.later.clear();
        let r = &mut self.run;
        r.altars.clear();
        r.risen.clear();
        r.portal = None;
        r.spawn_queue.clear();
        r.incoming.clear();
        r.encounters.clear();
        r.pending.clear();
        r.guard_quiet.clear();
        r.wards.clear();
        r.sealed = false;
        r.practice = None;
        r.wave_at = -1.0;
        // How far the front wizard got belongs to the level just left.
        r.lead = 0.0;
        r.lead_t = 0.0;
        r.boss = None;
        r.boss_fell = false;
    }

    fn queue_wave(&mut self, players: usize, scale: f64) {
        let mix = self.level.biome().enemies;
        let mut pool: Vec<&str> = Vec::new();
        for &(k, n) in mix {
            for _ in 0..n {
                pool.push(k);
            }
        }
        let depth = self.run.depth() as f64;
        let mut count = ((2.0 + 1.6 * players as f64)
            * (1.0 + 0.3 * depth)
            * self.settings.monster_count
            * crate::knobs::monster_count()
            * scale) as i64;
        // The first biome is where you learn to shoot and roll: fewer.
        if self.run.biome_index == 0 {
            count = 2.max((count as f64 * 0.6) as i64);
        }
        while count > 0 {
            // Some come in packs (hellhounds): the whole pack at once.
            let k = enemy::kind(pool[self.run.rng.pick(pool.len())]);
            if kinds()[k].solo && self.run.spawn_queue.iter().any(|q| q.0 == k) {
                continue;
            }
            for _ in 0..kinds()[k].pack {
                self.run.spawn_queue.push((k, self.run.tag));
                count -= 1;
            }
        }
    }

    /// A monster comes in at `at`, after a red circle shows where unless
    /// `telegraph` is off (Run.spawn_enemy).
    pub fn spawn_enemy(&mut self, k: usize, at: Vec3, telegraph: bool, tag: i32) {
        let players = self.wizards.len().max(1) as f64;
        let depth = self.run.depth() as f64;
        let hp_scale =
            (1.0 + 0.35 * (players - 1.0)) * (1.0 + 0.33 * depth) * self.settings.monster_hp * crate::knobs::monster_hp();
        let champ = (self.run.rng.f() as f64) < 0.06 + 0.03 * depth;
        // A boss must stay a fight however many tomes the party has stacked
        // (Joep: the Drowned King went down too easily).
        let hp_scale = if kinds()[k].boss { hp_scale * self.boss_toughness() } else { hp_scale };
        if telegraph {
            self.summon_circle(at);
            *self.run.incoming.entry(tag).or_insert(0) += 1;
            self.later(
                0.8,
                Later::Spawn {
                    kind: k,
                    at,
                    tag,
                    hp_scale,
                    champ,
                },
            );
            return;
        }
        self.spawned(k, at, tag, hp_scale, champ);
    }

    /// How much tougher a boss comes in than its level alone makes it: a
    /// third more in a party, a little less alone, and more for each tome
    /// the wizards carry on average.
    pub fn boss_toughness(&self) -> f64 {
        let (n, tomes) = self.wizards.iter().filter_map(|&id| self.ecs.get::<&crate::wizard::Wizard>(id).ok().map(|w| w.relics.len())).fold((0usize, 0usize), |(n, t), r| (n + 1, t + r));
        let each = if n == 0 { 0.0 } else { tomes as f64 / n as f64 };
        // In a party a boss is a third tougher (Joep: the Drowned King went
        // down too easily); alone a little weaker than its level makes it
        // (Joep: solo, the first boss was nearly impossible; two players
        // felt right).
        let base = if n <= 1 { 0.9 } else { 1.3 };
        base * (1.0 + 0.15 * each)
    }

    /// The summoning is over: in it comes, unless the fight already ended.
    pub fn summoned(&mut self, k: usize, at: Vec3, tag: i32, hp_scale: f64, champ: bool) {
        *self.run.incoming.entry(tag).or_insert(1) -= 1;
        if self.run.state != State::Fight && !kinds()[k].boss {
            return;
        }
        self.spawned(k, at, tag, hp_scale, champ);
    }

    fn spawned(&mut self, k: usize, at: Vec3, tag: i32, hp_scale: f64, champ: bool) {
        let id = self.add_enemy(k, at, hp_scale, champ, tag);
        if kinds()[k].boss {
            self.run.boss = Some(id);
        }
    }

    pub fn run_step(&mut self, dt: f64) {
        match self.run.state {
            State::Fight => {
                self.fight(dt);
                self.shrines(dt);
                self.practice_step(dt);
                self.ward_step();
                self.door_step();
            }
            State::Cleared => {
                self.shrines(dt);
                self.portal_step(dt);
            }
            State::Dead | State::Won => {
                self.run.end_t -= dt;
                if self.run.end_t <= 0.0 {
                    let died = self.run.state == State::Dead;
                    self.runs_ended += 1;
                    self.new_run();
                    if died {
                        self.ev(Ev::YouDiedEnd);
                    }
                }
            }
            State::Travel => {}
        }
        if !matches!(self.run.state, State::Dead | State::Won) {
            self.learn_step(dt);
        }
        for w in &mut self.run.wards {
            if w.opening {
                w.open = (w.open - dt as f32 / 1.6).max(0.0);
            } else if w.back {
                // The arena's door rises out of nothing.
                w.open = (w.open + dt as f32 / 1.2).min(1.0);
            }
        }
        self.run.wards.retain(|w| !w.opening || w.open > 0.0);
        for a in &mut self.run.altars {
            a.rise = if a.taken {
                (a.rise - dt as f32 / 0.4).max(0.0)
            } else {
                (a.rise + dt as f32 / 0.6).min(1.0)
            };
        }
        if matches!(self.run.state, State::Fight | State::Cleared)
            && !self.wizards.is_empty()
            && !self.any_wizard_alive()
        {
            self.run.state = State::Dead;
            // The world goes black and two words say it all; then a new run.
            self.run.end_t = 4.2;
            self.ev(Ev::YouDied);
            self.ev(Ev::RunOver { won: false });
        }
    }

    /// A wall of mist just past each shrine's clearing, across all the
    /// ground you can walk there.
    fn build_wards(&mut self) {
        for k in 0..2 {
            let c = self.level.clearings[k];
            let mut wd = self.ward_wall(self.level.clearing_at[k] + c.z + 1.5);
            wd.k = k as i32;
            self.run.wards.push(wd);
        }
        self.ward_step();
    }

    /// A practice ward `at` metres along the path that opens on `check`.
    fn add_ward(&mut self, at: f32, check: Check) {
        let mut wd = self.ward_wall(at);
        wd.k = -1;
        wd.at = at;
        wd.check = Some(check);
        self.run.wards.push(wd);
    }

    /// A wall `at` metres along the path, across it as the path runs
    /// there and over all the ground you can walk.
    fn ward_wall(&self, at: f32) -> Ward {
        let l = &self.level;
        let c = l.point_at(at);
        let n = (l.point_at(at + 1.0) - l.point_at(at - 1.0)).normalize_or(Vec2::NEG_Y);
        let across = Vec2::new(-n.y, n.x);
        let reach = l.half.x.max(l.half.y) * 2.0;
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        while lo > -reach && l.bound(c + across * (lo - 0.5)) < 0.0 {
            lo -= 0.5;
        }
        while hi < reach && l.bound(c + across * (hi + 0.5)) < 0.0 {
            hi += 0.5;
        }
        Ward {
            k: -1,
            c,
            n,
            at,
            lo: lo - 1.0,
            hi: hi + 1.0,
            holds: false,
            check: None,
            open: 1.0,
            opening: false,
            back: false,
        }
    }

    /// Opens every wall whose shrine has risen and given each living wizard
    /// a tome (or has none left they may take), and sets where the rest hold.
    fn ward_step(&mut self) {
        for i in 0..self.run.wards.len() {
            self.run.wards[i].holds = false;
            let wd = self.run.wards[i].clone();
            if wd.opening {
                continue;
            }
            if wd.back {
                self.run.wards[i].holds = true;
                continue;
            }
            // Knob `wards` 0: no walls hold the way (the practice strip's
            // stops still run their lessons as they open).
            if crate::knobs::wards() < 0.5 {
                self.open_ward(i);
                if let Some(c) = wd.check {
                    self.practice_open(c);
                }
                continue;
            }
            let done = if let Some(c) = wd.check {
                let ok = self.practice_check(c);
                if ok {
                    self.open_ward(i);
                    self.practice_open(c);
                    continue;
                }
                false
            } else {
                let key = format!("{}_{}", self.run.biome_index, wd.k);
                let risen = self.run.risen.contains(&key) || self.run.altars.iter().any(|a| a.shrine == key);
                let left = self.run.altars.iter().any(|a| a.shrine == key && !a.taken);
                let mut done = risen;
                if risen && left && crate::knobs::ward_needs_tomes() > 0.5 {
                    for &w in &self.wizards {
                        if self.alive(w) && !self.wiz(w).took.contains(&key) {
                            done = false;
                        }
                    }
                }
                done
            };
            if done {
                self.open_ward(i);
            } else {
                self.run.wards[i].holds = true;
            }
        }
    }

    /// Once every wizard is in the boss's arena, its way back shuts behind
    /// them with a wall of mist like the others (Joep: no walking back), and
    /// the arena is theirs to spread out in.
    fn door_step(&mut self) {
        if self.run.sealed || self.run.practice.is_some() || crate::knobs::wards() < 0.5 || self.level.clearings.len() < 3 {
            return;
        }
        let r = self.level.clearings[2].z;
        let at = self.level.clearing_at[2] - r - 0.5;
        let inside = self.wizards.iter().all(|&w| {
            let p = self.pos(w);
            self.level.progress_of(Vec2::new(p.x, p.z)) > at + 2.5
        });
        if !inside || self.wizards.is_empty() {
            return;
        }
        let mut wd = self.ward_wall(at);
        wd.n = -wd.n;
        (wd.lo, wd.hi) = (-wd.hi, -wd.lo);
        wd.k = -2;
        wd.back = true;
        wd.open = 0.0;
        wd.holds = true;
        self.run.wards.push(wd);
        self.run.sealed = true;
        let m = self.run.wards.last().map(|w| w.mid()).unwrap_or_default();
        self.burst(Vec3::new(m.x, 1.2, m.y), fx::rgb(0.9, 0.85, 0.7), 30, 3.5, 0.2, None);
        self.ev(Ev::Shake(0.25));
    }

    fn open_ward(&mut self, i: usize) {
        let wd = &mut self.run.wards[i];
        wd.opening = true;
        let m = wd.mid();
        let at = Vec3::new(m.x, 1.6, m.y);
        self.burst(at, fx::rgb(0.9, 0.85, 0.7), 24, 3.0, 0.15, None);
    }

    /// How far along the level's path the front living wizard is.
    pub fn lead_progress(&self) -> f32 {
        let mut best = -1.0f32;
        for &w in &self.wizards {
            if self.alive(w) {
                best = best.max(self.level.progress_of(xz(self.pos(w))));
            }
        }
        best
    }

    fn fight(&mut self, dt: f64) {
        if self.run.boss_fell {
            // Beating the boss ends the level: the rest of its horde falls
            // with it and nothing more comes.
            self.run.boss_fell = false;
            self.run.encounters.clear();
            self.run.spawn_queue.clear();
            self.run.incoming.clear();
            for e in self.enemies.clone() {
                if self.alive(e) {
                    let hp = self.a(e).hp;
                    self.take(e, hp + 1.0);
                }
            }
        }
        // The lead only matters a few times a second.
        self.run.lead_t -= dt;
        if self.run.lead_t <= 0.0 {
            self.run.lead_t = 0.2;
            self.run.lead = self.lead_progress();
        }
        // The next pack holds off while the field is still full.
        let alive_count = self.near_count();
        while !self.run.encounters.is_empty()
            && self.run.lead >= self.run.encounters[0].at
            && alive_count + self.run.spawn_queue.len() < 9
        {
            let enc = self.run.encounters.remove(0);
            self.run.wave_at = self.run.lead;
            let players = self.wizards.len().max(1);
            match enc.kind {
                EncKind::Wave => self.queue_wave(players, 1.0),
                EncKind::Guard(k) => {
                    self.run.tag = k as i32;
                    self.queue_wave(players, 1.5);
                    self.run.tag = -1;
                    self.run.pending.push(k);
                    self.run.guard_quiet.insert(k, 0.0);
                }
                EncKind::Boss => {
                    let c = self.level.clearings[2];
                    let b = enemy::kind(self.level.biome().boss);
                    self.spawn_enemy(b, Vec3::new(c.x, 0.0, c.y), false, -1);
                    let sub = self.level.biome().name.to_string();
                    self.ev(Ev::Banner {
                        title: kinds()[b].name.to_string(),
                        sub,
                        c: fx::rgb(1.0, 0.7, 0.3),
                    });
                    self.queue_wave(players, 0.5);
                }
            }
        }
        self.run.spawn_t -= dt;
        if !self.run.spawn_queue.is_empty() && self.run.spawn_t <= 0.0 {
            self.run.spawn_t = 0.35;
            let (k, tag) = self.run.spawn_queue.remove(0);
            let at = self.spawn_spot(k);
            self.spawn_enemy(k, at, kinds()[k].spawn != Spawn::Under, tag);
        }
        // A shrine rises as soon as its own guards are beaten, whatever
        // packs came after them.
        for k in self.run.pending.clone() {
            let q = if self.guards_left(k as i32) == 0 {
                self.run.guard_quiet.get(&k).copied().unwrap_or(0.0) + dt
            } else {
                0.0
            };
            self.run.guard_quiet.insert(k, q);
            if q >= 1.5 {
                self.raise_shrine(k);
                self.run.pending.retain(|&p| p != k);
            }
        }
        let alive_count = self.near_count();
        self.run.quiet_t = if self.run.spawn_queue.is_empty() && alive_count == 0 {
            self.run.quiet_t + dt
        } else {
            0.0
        };
        if self.run.quiet_t < 1.5 {
            return;
        }
        // The field is clear: the shrines of beaten clearings rise.
        for k in std::mem::take(&mut self.run.pending) {
            self.raise_shrine(k);
        }
        let boss_up = self
            .run
            .boss
            .is_some_and(|b| self.ecs.contains(b) && self.alive(b));
        if self.run.encounters.is_empty() && !boss_up {
            self.level_cleared();
        }
    }

    fn guards_left(&self, k: i32) -> usize {
        let mut n = self.run.incoming.get(&k).copied().unwrap_or(0).max(0) as usize;
        n += self.run.spawn_queue.iter().filter(|q| q.1 == k).count();
        for &e in &self.enemies {
            if self.alive(e) && self.enemy(e).guard == k && self.near(e) {
                n += 1;
            }
        }
        n
    }

    fn near(&self, e: Id) -> bool {
        let p = self.pos(e);
        self.wizards
            .iter()
            .any(|&w| self.alive(w) && self.pos(w).distance(p) < STRAY_DIST)
    }

    fn near_count(&self) -> usize {
        self.enemies
            .iter()
            .filter(|&&e| self.alive(e) && self.near(e))
            .count()
    }

    /// Where a monster of `k` comes in, by what it is.
    fn spawn_spot(&mut self, k: usize) -> Vec3 {
        let wz: Vec<Vec3> = self.wizards.iter().map(|&w| self.pos(w)).collect();
        let at = self.run.wave_at;
        if at < 0.0 {
            return self.level.spawn_point(&wz);
        }
        let l = &mut self.level;
        match kinds()[k].spawn {
            Spawn::Far => l.spawn_ahead(at, 15.0, 22.0, &wz, 0.0),
            Spawn::Under => l.spawn_ahead(at, 4.0, 10.0, &wz, 0.0),
            Spawn::Flank => l.spawn_ahead(at, -12.0, -5.0, &wz, 0.0),
            Spawn::Water => l.spawn_ahead(at, 8.0, 16.0, &wz, 1.0),
            Spawn::Ahead => l.spawn_ahead(at, 7.0, 15.0, &wz, 0.0),
        }
    }

    /// `p` moved back to this side of every wall of mist still up: nothing
    /// comes in behind one (Joep).
    pub(crate) fn before_walls(&self, mut p: Vec3) -> Vec3 {
        for wd in self.run.wards.iter().filter(|w| !w.opening && !w.back) {
            let q = Vec2::new(p.x, p.z);
            let prog = self.level.progress_of(q);
            let (d, within) = wd.past(q);
            let behind = (within && d > -2.0 && d < 25.0 && (prog - wd.at).abs() < 30.0) || (prog > wd.at - 2.0 && prog < wd.at + 40.0);
            if behind {
                // As far to the side of the path, a few metres short of it.
                let side = q - self.level.point_at(prog);
                let mut back = self.level.point_at(wd.at - 3.5) + side;
                // Where the path bends, the side can still reach past it.
                let (d, within) = wd.past(back);
                if within && d > -2.0 {
                    back -= wd.n * (d + 2.5);
                }
                p = self.level.clamp_inside(Vec3::new(back.x, p.y, back.y));
            }
        }
        p
    }

    /// Where a wizard with nothing to fight walks to (bots use it).
    pub fn walk_goal(&self, pos: Vec3) -> Vec3 {
        let mine = self.level.progress_of(xz(pos));
        // Far behind the front, follow the path itself: straight at a point
        // up ahead runs into the walls where the path bends.
        let run = if self.run.lead - mine > 8.0 {
            mine + 6.0
        } else {
            self.run.lead.max(mine) + 5.0
        };
        let p = self.level.point_at(run);
        Vec3::new(p.x, 0.0, p.y)
    }

    /// A tome `w` may still take, near enough to walk to.
    pub fn tome_for(&self, pos: Vec3, w: &Wizard) -> Option<usize> {
        self.run
            .altars
            .iter()
            .position(|a| !a.taken && !w.took.contains(&a.shrine) && a.pos.distance(pos) < 30.0)
    }

    /// A shrine of tomes rises at clearing `k` (2 is the boss's arena): one
    /// for each wizard and one spare, and the fallen get back up.
    fn raise_shrine(&mut self, k: usize) {
        let c = self.level.clearings[k];
        let title = if k < 2 {
            "A shrine rises".to_string()
        } else {
            format!(
                "{} is slain",
                kinds()[enemy::kind(self.level.biome().boss)].name
            )
        };
        for id in self.wizards.clone() {
            if !self.alive(id) {
                self.revive_id(id, 0.3);
            }
        }
        self.teach(k);
        let n = self.wizards.len() + 1;
        let mut taken: Vec<usize> = Vec::new();
        for &w in &self.wizards {
            // Tomes stack (Joep): only a one-off already carried is left out.
            taken.extend(self.wiz(w).relics.iter().copied().filter(|&r| !relics::stacks(r)));
        }
        taken.extend(self.run.altars.iter().map(|a| a.relic));
        // Only tomes someone here can use: their elements, their forms.
        // Bots know everything, so they count only when nobody else is here.
        let humans: Vec<Id> = self.wizards.iter().copied().filter(|&w| self.wiz(w).bot.is_none()).collect();
        let who = if humans.is_empty() { self.wizards.clone() } else { humans };
        let knows: Vec<(Vec<El>, Vec<crate::grammar::Form>)> = who
            .iter()
            .map(|&w| {
                let wz = self.wiz(w);
                (wz.open_elements(), crate::grammar::FORMS.iter().copied().filter(|&f| wz.form_open(f)).collect())
            })
            .collect();
        let has = |need: relics::Need| {
            knows.iter().any(|(els, forms)| match need {
                relics::Need::Nothing => true,
                relics::Need::El(e) => e.parts().iter().all(|p| els.contains(p)),
                relics::Need::Form(f) => forms.contains(&f),
            })
        };
        let picks = relics::pick(n, &mut self.run.rng, &taken, &has);
        // Joep, level 4: two wizards carrying all 32 tomes got the banner,
        // no tome and a wall that never opened. With none left, say so; the
        // shrine still counts as risen, so its wall opens.
        self.ev(Ev::Banner {
            title,
            sub: if picks.is_empty() { "You carry every tome there is".into() } else { "Each wizard may take one tome".into() },
            c: fx::rgb(0.9, 0.75, 0.4),
        });
        let shrine = format!("{}_{}", self.run.biome_index, k);
        if !self.run.risen.contains(&shrine) {
            self.run.risen.push(shrine.clone());
        }
        // Across the path, facing the way on. Wizards who ran on past the
        // clearing find it ahead of them, never behind (Joep).
        let mut along = self.level.clearing_at[k];
        // This clearing's wall, while it is shut: it stays up until everyone
        // took a tome, so nothing may stand past it.
        let wall = self.run.wards.iter().find(|w| w.k == k as i32 && !w.opening).cloned();
        if k < 2 {
            along = self
                .wizards
                .iter()
                .filter(|&&w| self.alive(w))
                .map(|&w| { let p = self.pos(w); self.level.progress_of(Vec2::new(p.x, p.z)) + 8.0 })
                .fold(along, f32::max)
                .min(self.level.length() - 4.0);
        }
        let dir = (self.level.point_at(along + 1.0) - self.level.point_at(along - 1.0))
            .normalize_or_zero();
        let mut across = Vec2::new(-dir.y, dir.x);
        let mut spacing: f32 = 3.6;
        let mut mid = if along > self.level.clearing_at[k] { self.level.point_at(along) } else { Vec2::new(c.x, c.y) }
            + if k == 2 { dir * -2.0 } else { Vec2::ZERO };
        // Ahead would be past the wall: a row
        // along this side of it instead, which nobody can be beyond.
        let half = (picks.len() as f32 - 1.0) * 0.5 * spacing;
        let past = |wd: &Ward| {
            [mid - across * half, mid, mid + across * half].iter().any(|&p| {
                let (d, within) = wd.past(p);
                within && d > -2.0
            })
        };
        if let Some(wd) = wall.filter(|wd| past(wd)) {
            let n = picks.len().max(2) as f32 - 1.0;
            spacing = spacing.min((wd.hi - wd.lo - 2.0).max(0.0) / n);
            // Centred where the path crosses the wall, not the wall's own
            // middle: in an open level the wall spans the whole field and
            // its middle can be far out in the grass (Joep: tomes off the
            // path, one inside a building).
            let half = n * 0.5 * spacing;
            let t = 0.0f32.clamp((wd.lo + 1.0 + half).min(0.0), (wd.hi - 1.0 - half).max(0.0));
            mid = wd.c + wd.across() * t - wd.n * 2.0;
            across = wd.across();
        }
        for (i, &relic) in picks.iter().enumerate() {
            let p = mid + across * (i as f32 - (picks.len() - 1) as f32 * 0.5) * spacing;
            // On ground you can walk, clear of props.
            let q = self.level.clamp_inside(Vec3::new(p.x, 0.0, p.y));
            let p = self.level.statics.push_out(Vec2::new(q.x, q.z), 0.9);
            let pos = Vec3::new(p.x, 0.0, p.y);
            self.burst(pos + Vec3::Y, fx::rgb(1.0, 0.8, 0.4), 16, 3.0, 0.15, None);
            self.run.altars.push(Altar {
                pos,
                relic,
                taken: false,
                hold: 0.0,
                fill_c: fx::rgb(1.0, 0.8, 0.45),
                shrine: shrine.clone(),
                rise: 0.0,
            });
        }
    }

    /// What a shrine teaches every wizard present: the next step of the
    /// unlock ladder (wizard::STAGE_*), given once the fight is over
    /// (learn_step).
    fn teach(&mut self, k: usize) {
        // One step of the ladder (wizard::STAGE_*) at a level's first
        // shrine and one at its boss: it runs over five levels (Joep).
        if k < 2 {
            if self.run.taught {
                return;
            }
            self.run.taught = true;
        }
        // After the tomes, as the wizard walks on through the wall (Joep:
        // not at the same time as the tomes).
        let gate = self.run.wards.iter().find(|w| w.k == k as i32).cloned();
        for id in self.wizards.clone() {
            let u = self.learned_by(id).max(0);
            self.run.learn.push((id, (u + 1).min(crate::wizard::STAGES as i32 - 1), gate.clone()));
        }
    }

    /// What `id` will know once its waiting unlocks are given.
    fn learned_by(&self, id: Id) -> i32 {
        let u = self.wiz(id).unlocks;
        self.run.learn.iter().filter(|l| l.0 == id).map(|l| l.1).fold(u, i32::max)
    }

    /// Unlocks up to `n` for `id`, at the next calm moment.
    pub(crate) fn learn_later(&mut self, id: Id, n: i32) {
        self.run.learn.push((id, n, None));
    }

    /// Waiting unlocks are given once it has been calm for 2 s: no monster
    /// near a wizard or still to come, no wizard hurt (Joep: never
    /// mid-fight, new things want a quiet moment).
    fn learn_step(&mut self, dt: f64) {
        if self.run.learn.is_empty() {
            self.run.calm_t = 0.0;
            return;
        }
        let hurt = self.wizards.iter().any(|&w| self.a(w).hurt_t > 0.0);
        let calm = self.near_count() == 0 && self.run.spawn_queue.is_empty() && !hurt;
        self.run.calm_t = if calm { self.run.calm_t + dt } else { 0.0 };
        let calm = self.run.calm_t >= 2.0;
        for (id, n, gate) in std::mem::take(&mut self.run.learn) {
            if !self.ecs.contains(id) {
                continue;
            }
            let ready = match &gate {
                // Through it: on along the path, and past the wall itself.
                Some(wd) => {
                    let q = xz(self.pos(id));
                    let (d, within) = wd.past(q);
                    self.alive(id) && self.level.progress_of(q) > wd.at + 0.5 && (!within || d > 0.5)
                }
                None => calm,
            };
            if ready {
                self.unlock_to(id, n);
            } else {
                self.run.learn.push((id, n, gate));
            }
        }
    }

    /// Skip this level's fights (the live tools' and shots' shortcut):
    /// every monster falls, no more come, the walls open, the boss's shrine
    /// and the portal rise, and the wizards stand before the way out.
    pub fn skip_level(&mut self) {
        self.run.encounters.clear();
        self.run.spawn_queue.clear();
        self.run.pending.clear();
        for id in self.enemies.clone() {
            self.take(id, 1e9);
        }
        self.flush_deaths();
        for wd in &mut self.run.wards {
            wd.opening = true;
        }
        self.level_cleared();
        let pa = self.level.exit_at;
        let n = self.wizards.len() as f32;
        for (i, id) in self.wizards.clone().into_iter().enumerate() {
            self.am(id).pos = Vec3::new(pa.x + (i as f32 - (n - 1.0) * 0.5) * 1.2, 0.0, pa.y + 3.5);
        }
    }

    fn level_cleared(&mut self) {
        if self.run.biome_index == crate::biome::biomes().len() - 1 {
            self.run.state = State::Won;
            self.run.end_t = 12.0;
            let sub = format!(
                "Your names will be sung. {} monsters slain.",
                self.run.kills
            );
            self.ev(Ev::Banner {
                title: "The Pit is sealed".into(),
                sub,
                c: fx::rgb(1.0, 0.8, 0.35),
            });
            self.ev(Ev::RunOver { won: true });
            return;
        }
        self.run.state = State::Cleared;
        self.raise_shrine(2);
        let pa = self.level.exit_at;
        self.run.portal = Some(Vec3::new(pa.x, 0.0, pa.y));
        self.run.portal_t = 0.0;
    }

    fn shrines(&mut self, dt: f64) {
        for i in 0..self.run.altars.len() {
            if self.run.altars[i].taken {
                continue;
            }
            // The ring fills while a wizard who has not taken a tome here
            // stands in it; full, the tome is theirs.
            let (apos, shrine) = (self.run.altars[i].pos, self.run.altars[i].shrine.clone());
            let mut taker = None;
            for &w in &self.wizards {
                if !self.alive(w) || self.wiz(w).took.contains(&shrine) {
                    continue;
                }
                if xz(self.pos(w) - apos).length() < ALTAR_R {
                    taker = Some(w);
                    break;
                }
            }
            let robe = taker.map(|w| fx::lighten(self.wiz(w).robe, 0.35));
            let a = &mut self.run.altars[i];
            a.hold = if taker.is_some() {
                (a.hold + dt).min(crate::knobs::tome_time())
            } else {
                (a.hold - dt * 2.0).max(0.0)
            };
            if let Some(c) = robe {
                a.fill_c = c;
            }
            if let (Some(w), true) = (taker, a.hold >= crate::knobs::tome_time()) {
                a.taken = true;
                let relic = a.relic;
                self.wiz_mut(w).took.push(shrine);
                self.give_relic(w, relic);
                self.burst(
                    apos + Vec3::Y * 1.3,
                    fx::rgb(1.0, 0.8, 0.4),
                    30,
                    4.0,
                    0.15,
                    None,
                );
                let c = robe.unwrap();
                self.ev(Ev::Ring {
                    at: apos,
                    c,
                    r: ALTAR_R + 0.4,
                    time: 0.4,
                });
                if let Some(r) = relics::all().get(relic) {
                    let text = format!("{} took {}: {}", self.wiz(w).name, r.name, r.text);
                    self.ev(Ev::Toast { text, c });
                }
                // Once every living wizard has a tome from this shrine, the
                // other altars there sink away too (their labels fade with them).
                let shrine = self.run.altars[i].shrine.clone();
                let done = self.wizards.iter().all(|&w| !self.alive(w) || self.wiz(w).took.contains(&shrine));
                if done {
                    for a in &mut self.run.altars {
                        if a.shrine == shrine {
                            a.taken = true;
                        }
                    }
                }
            }
        }
    }

    fn portal_step(&mut self, dt: f64) {
        let Some(p) = self.run.portal else { return };
        let living: Vec<Id> = self
            .wizards
            .iter()
            .copied()
            .filter(|&w| self.alive(w))
            .collect();
        let inside = living
            .iter()
            .filter(|&&w| self.pos(w).distance(p) < PORTAL_R)
            .count();
        if !living.is_empty() && inside == living.len() {
            self.run.portal_t += dt;
            if self.run.portal_t >= PORTAL_TIME {
                self.travel();
            }
        } else {
            self.run.portal_t = (self.run.portal_t - dt * 2.0).max(0.0);
        }
    }

    /// Through the portal: the wizards vanish into it in a flash, the
    /// screen goes black, and it opens again on the next level.
    fn travel(&mut self) {
        self.run.state = State::Travel;
        let at = self.run.portal.unwrap_or_default();
        let c = fx::rgb(0.55, 0.75, 1.0);
        self.burst(at + Vec3::Y * 1.4, c, 40, 6.0, 0.35, None);
        self.ev(Ev::LightPop {
            at,
            c,
            energy: 6.0,
            range: 10.0,
            time: 0.5,
        });
        for w in self.wizards.clone() {
            let robe = fx::lighten(self.wiz(w).robe, 0.3);
            let p = self.pos(w);
            self.burst(p + Vec3::Y, robe, 14, 3.0, 0.25, None);
        }
        self.ev(Ev::Fade {
            to: 1.0,
            time: 0.35,
        });
        self.later(0.4, Later::Travel);
    }

    pub fn travelled(&mut self) {
        if self.run.state == State::Travel {
            self.next_level();
        }
        self.ev(Ev::Fade { to: 0.0, time: 0.6 });
    }

    fn next_level(&mut self) {
        // Every level starts fresh: full health and mana, the fallen up.
        for id in self.wizards.clone() {
            if !self.alive(id) {
                self.revive_id(id, 1.0);
            }
            let mut a = self.am(id);
            a.hp = a.max_hp;
            drop(a);
            let mut w = self.wiz_mut(id);
            w.mana = w.mana_max();
        }
        self.run.biome_index += 1;
        self.enter_level();
    }

    // --- The practice strip ---------------------------------------------

    /// Does the first level need the strip: someone at the couch knows
    /// nothing.
    fn practice_wanted(&self) -> bool {
        if self.settings.force_practice {
            return true;
        }
        if self.settings.unlock_all || self.settings.forced_unlocks >= 0 || crate::knobs::practice() < 0.5 {
            return false;
        }
        self.wizards.iter().any(|&w| {
            let w = self.wiz(w);
            w.bot.is_none() && w.unlocks < 0
        })
    }

    fn practice_build(&mut self) {
        let start = self.level.progress_of(self.level.start);
        let mut p = Practice {
            start,
            ..Default::default()
        };
        let across = self.practice_across(start, practice::URNS_AT);
        let c = self.level.point_at(start + practice::URNS_AT);
        for i in 0..3 {
            let q = c + across * (i as f32 - 1.0) * 2.6;
            if let Some(u) = self.level.prop("urn", Some(Vec3::new(q.x, 0.0, q.y)), None) {
                p.urns.push(u);
            }
        }
        self.run.practice = Some(p);
        self.add_ward(start + practice::WARD_1, Check::Urns);
        self.add_ward(start + practice::WARD_2, Check::Archer);
        self.add_ward(start + practice::WARD_3, Check::Caster);
        self.add_ward(start + practice::WARD_4, Check::Shrine);
    }

    fn practice_across(&self, start: f32, at: f32) -> Vec2 {
        let l = &self.level;
        let dir = (l.point_at(start + at + 1.0) - l.point_at(start + at - 1.0)).normalize_or_zero();
        Vec2::new(-dir.y, dir.x)
    }

    /// Wizards at the couch who still have to learn their first elements.
    fn learners(&self) -> Vec<Id> {
        self.wizards
            .iter()
            .copied()
            .filter(|&w| {
                let wz = self.wiz(w);
                self.alive(w) && wz.bot.is_none() && wz.unlocks < 0
            })
            .collect()
    }

    fn beaten(&self, e: Option<Id>) -> bool {
        e.is_some_and(|e| !self.ecs.contains(e) || !self.alive(e))
    }

    fn practice_check(&self, c: Check) -> bool {
        let Some(p) = &self.run.practice else {
            return true;
        };
        let all = |f: fn(&Wizard) -> bool| self.learners().iter().all(|&w| f(&self.wiz(w)));
        match c {
            Check::Urns => p.urns.iter().all(|&u| self.level.props[u].broken),
            Check::Archer => self.beaten(p.archer) && (all(|w| w.rolled) || p.beaten_t > practice::GRACE),
            Check::Caster => self.beaten(p.caster) && (all(|w| w.deflected) || p.beaten_t > practice::GRACE),
            Check::Shrine => p.shrine.is_some() && self.learners().is_empty(),
        }
    }

    fn practice_open(&mut self, c: Check) {
        match c {
            Check::Urns => {
                let e = self.practice_spawn("practice_archer", practice::ARCHER_AT);
                self.run.practice.as_mut().unwrap().archer = Some(e);
            }
            Check::Archer => {
                let e = self.practice_spawn("practice_caster", practice::CASTER_AT);
                self.run.practice.as_mut().unwrap().caster = Some(e);
            }
            Check::Caster => self.practice_shrine(),
            Check::Shrine => {}
        }
    }

    fn practice_spawn(&mut self, kind: &str, at: f32) -> Id {
        self.run.practice.as_mut().unwrap().beaten_t = 0.0;
        for w in self.wizards.clone() {
            let mut wz = self.wiz_mut(w);
            wz.rolled = false;
            wz.deflected = false;
        }
        let start = self.run.practice.as_ref().unwrap().start;
        let c = self.level.point_at(start + at);
        let p = Vec3::new(c.x, 0.0, c.y);
        self.summon_circle(p);
        self.add_enemy(enemy::kind(kind), p, 1.0, false, -1)
    }

    /// A low stone with the two first elements floating over it, and the
    /// ring to stand in.
    fn practice_shrine(&mut self) {
        let start = self.run.practice.as_ref().unwrap().start;
        let c = self.level.point_at(start + practice::SHRINE_AT);
        let mut els: Vec<El> = Vec::new();
        for w in self.learners() {
            let order = ELEMENT_ORDER[self.wiz(w).element_order % ELEMENT_ORDER.len()];
            for &e in &order[..2] {
                if !els.contains(&e) {
                    els.push(e);
                }
            }
        }
        if els.is_empty() {
            els = vec![El::Fire, El::Lightning];
        }
        let at = Vec3::new(c.x, 0.0, c.y);
        self.burst(at + Vec3::Y, fx::rgb(1.0, 0.8, 0.45), 16, 3.0, 0.15, None);
        let p = self.run.practice.as_mut().unwrap();
        p.shrine = Some(at);
        p.shrine_els = els;
    }

    fn practice_step(&mut self, dt: f64) {
        if let Some(p) = &self.run.practice {
            let lesson = if p.caster.is_some() { p.caster } else { p.archer };
            if self.beaten(lesson) {
                self.run.practice.as_mut().unwrap().beaten_t += dt;
            }
        }
        let Some(at) = self.run.practice.as_ref().and_then(|p| p.shrine) else {
            return;
        };
        let mut best = 0.0f64;
        for w in self.learners() {
            let d = xz(self.pos(w) - at).length();
            let seat = self.wiz(w).seat;
            let p = self.run.practice.as_mut().unwrap();
            let h = p.hold.get(&seat).copied().unwrap_or(0.0);
            let h = if d < practice::SHRINE_R {
                (h + dt).min(practice::LEARN_TIME)
            } else {
                (h - dt * 2.0).max(0.0)
            };
            p.hold.insert(seat, h);
            best = best.max(h);
            if h >= practice::LEARN_TIME {
                p.hold.remove(&seat);
                self.unlock_to(w, 0);
                let c = fx::lighten(self.wiz(w).robe, 0.35);
                self.burst(at + Vec3::Y * 1.2, c, 24, 3.5, 0.15, None);
            }
        }
        self.run.practice.as_mut().unwrap().fill = best / practice::LEARN_TIME;
    }

    /// The nearest urn still standing, for bots.
    pub fn next_urn(&self, pos: Vec3) -> Option<Vec3> {
        let p = self.run.practice.as_ref()?;
        p.urns
            .iter()
            .map(|&u| &self.level.props[u])
            .filter(|u| !u.broken)
            .map(|u| u.pos)
            .min_by(|a, b| a.distance(pos).total_cmp(&b.distance(pos)))
    }
}
