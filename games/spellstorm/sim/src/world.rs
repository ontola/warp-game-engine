//! The game world (main.gd's Game): wizards, monsters and every spell let
//! loose, on a small ECS (hecs). Actors keep their health and statuses in
//! an `Actor` component; a wizard adds `Wizard`, a monster `Enemy`, and each
//! spell in flight is an entity with its own component. One tick runs the
//! systems in Godot's tree order: the game (crossing streams, floating
//! numbers), the ground's fields, the spells and pickups in the order they
//! were made, the run, the wizards, the monsters.

use crate::El;
use crate::actor::Actor;
use crate::enemy::Enemy;
use crate::fx::{self, C, Ev};
use crate::level::Level;
use crate::relics::Mods;
use crate::rng::{Pcg, Rand};
use crate::run::Run;
use crate::wizard::{Pad, Wizard, STAGES};
use glam::{Vec2, Vec3};
use hecs::Entity;
use std::collections::BTreeMap;

pub type Id = Entity;

/// One tick: Godot's physics runs at 60 Hz.
pub const DT: f64 = 1.0 / 60.0;

/// An element mix landing in one hit: each element and how many of it.
pub type Mix = Vec<(El, f64)>;

pub fn mix_of(counts: &[(El, u8)]) -> Mix {
    counts.iter().map(|&(e, n)| (e, n as f64)).collect()
}

pub fn one(e: El) -> Mix {
    vec![(e, 1.0)]
}

pub const WIZARDS: u8 = 0;
pub const MONSTERS: u8 = 1;

/// Room settings (main.gd _on_setting).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub ff: f64,
    pub monster_hp: f64,
    pub monster_damage: f64,
    pub monster_count: f64,
    /// "Unlocks: All" in the menu.
    pub unlock_all: bool,
    /// --unlocks N: everyone at this many unlocks (tests, screenshots).
    pub forced_unlocks: i32,
    pub force_practice: bool,
    /// Bots always cast this recipe (screenshots).
    pub recipe: i32,
    /// The level the host's "Level" setting last sent us to (0 the first).
    pub level: i32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            ff: 0.25,
            monster_hp: 1.0,
            monster_damage: 1.0,
            monster_count: 1.0,
            unlock_all: false,
            forced_unlocks: -1,
            force_practice: false,
            recipe: -1,
            level: 0,
        }
    }
}

impl Settings {
    pub fn set(&mut self, key: &str, value: &str) {
        match key {
            "friendly_fire" => {
                self.ff = match value {
                    "off" => 0.0,
                    "half" => 0.5,
                    "full" => 1.0,
                    _ => 0.25,
                }
            }
            "difficulty" => {
                let (hp, dmg, n) = match value {
                    "easy" => (0.7, 0.6, 0.8),
                    "hard" => (1.4, 1.35, 1.25),
                    _ => (1.0, 1.0, 1.0),
                };
                self.monster_hp = hp;
                self.monster_damage = dmg;
                self.monster_count = n;
            }
            "unlocks" => self.unlock_all = value == "all",
            _ => {}
        }
    }
}

/// Who sits where (main.gd _seat).
#[derive(Clone, Debug)]
pub struct Seat {
    pub name: String,
    pub robe: C,
    pub bot: bool,
    /// Unlocks this player earned before (-1: not even the practice strip).
    pub unlocks: i32,
    pub player_id: String,
}

pub const COLORS: [C; 6] = [
    fx::rgb(0.62, 0.12, 0.12),
    fx::rgb(0.15, 0.3, 0.65),
    fx::rgb(0.2, 0.5, 0.25),
    fx::rgb(0.55, 0.42, 0.12),
    fx::rgb(0.45, 0.18, 0.55),
    fx::rgb(0.15, 0.5, 0.52),
];
pub const NAMES: [&str; 6] = ["Morwen", "Aldric", "Isolde", "Thorne", "Ysolde", "Cael"];

impl Seat {
    pub fn bot(i: usize) -> Seat {
        Seat {
            name: NAMES[i % NAMES.len()].to_string(),
            robe: COLORS[i % COLORS.len()],
            bot: true,
            unlocks: -1,
            player_id: String::new(),
        }
    }

    pub fn player(i: usize, name: &str) -> Seat {
        Seat {
            name: name.to_string(),
            robe: COLORS[i % COLORS.len()],
            bot: false,
            unlocks: -1,
            player_id: String::new(),
        }
    }
}

/// Something that happens a moment later (Godot's timers and tweens).
#[derive(Clone, Debug)]
pub enum Later {
    /// A meteor shower's next rock.
    Meteor {
        caster: Id,
        spell: Box<crate::Spell>,
        at: Vec3,
    },
    /// The echo tome's second nova.
    Nova {
        caster: Id,
        spell: Box<crate::Spell>,
    },
    /// A glob of the venom cannon lands.
    Venom {
        caster: Id,
        at: Vec3,
        spell: Box<crate::Spell>,
        puddle: bool,
    },
    /// A toad's glob lands.
    Lob {
        caster: Id,
        at: Vec3,
        el: El,
        dmg: f64,
    },
    /// A dead monster's fuse is out.
    BlowUp {
        id: Id,
        el: El,
        r: f64,
        dmg: f64,
        field: f64,
    },
    /// A summoning circle is done.
    Spawn {
        kind: usize,
        at: Vec3,
        tag: i32,
        hp_scale: f64,
        champ: bool,
    },
    /// Through the portal, once the screen is black.
    Travel,
    Shake(f32),
}

/// Marks an entity that ticks with the spells, and when it was made: they
/// run in that order, as Godot's children of the Fx node do.
#[derive(Clone, Copy, Debug)]
pub struct Order(pub u64);

pub struct World {
    pub ecs: hecs::World,
    pub level: Level,
    pub run: Run,
    pub settings: Settings,
    /// Godot's global randf(): every gameplay roll that is not the run's.
    pub rng: Pcg,
    pub tick: u64,
    pub time: f64,
    /// Seat order.
    pub wizards: Vec<Id>,
    /// Spawn order, living and dying (Game.enemies).
    pub enemies: Vec<Id>,
    /// Dead monsters still lying there while their death plays.
    pub corpses: Vec<Id>,
    /// What is due later: seconds left, and what.
    pub later: Vec<(f64, Later)>,
    /// Fields on the ground, in the order they were laid.
    pub fields: Vec<Id>,
    /// Spells, bullets and pickups, in the order they were made.
    pub fx: Vec<Id>,
    next_order: u64,
    pub events: Vec<Ev>,
    pub stats: BTreeMap<&'static str, u64>,
    /// Spell keys cast this session (the grimoire just counts them).
    pub grimoire: BTreeMap<String, String>,
    pub confluences: BTreeMap<String, String>,
    confluence_t: BTreeMap<(u64, u64), f64>,
    dmg_t: f64,
    /// The host paused the game or a menu is open: nothing moves.
    pub paused: bool,
    /// The run's seed; each new run takes the next one.
    pub seed: u64,
    /// Runs this session that ended in a wipe or a win (the HUD's run
    /// counter after a death says the next one's number).
    pub runs_ended: u32,
}

impl World {
    pub fn new(seats: &[Seat], seed: u64, settings: Settings) -> World {
        let mut w = World {
            ecs: hecs::World::new(),
            level: Level::build(0, seed.wrapping_mul(31)),
            run: Run::default(),
            settings,
            rng: Pcg::new(seed ^ 0x5eed),
            tick: 0,
            time: 0.0,
            wizards: Vec::new(),
            enemies: Vec::new(),
            corpses: Vec::new(),
            later: Vec::new(),
            fields: Vec::new(),
            fx: Vec::new(),
            next_order: 0,
            events: Vec::new(),
            stats: BTreeMap::new(),
            grimoire: BTreeMap::new(),
            confluences: BTreeMap::new(),
            confluence_t: BTreeMap::new(),
            dmg_t: 0.0,
            paused: false,
            seed,
            runs_ended: 0,
        };
        w.seat(seats);
        w.new_run();
        w
    }

    /// Put the wizards in their seats (main.gd _seat).
    pub fn seat(&mut self, seats: &[Seat]) {
        for id in self.wizards.drain(..) {
            let _ = self.ecs.despawn(id);
        }
        let mut used: Vec<C> = Vec::new();
        for (i, s) in seats.iter().enumerate() {
            let mut robe = s.robe;
            if used.contains(&robe) {
                robe = COLORS[i % COLORS.len()];
            }
            used.push(robe);
            let (actor, wiz) = Wizard::make(i, s, robe, &self.settings);
            let id = self.ecs.spawn((actor, wiz));
            self.wizards.push(id);
        }
        self.unlocks_changed();
    }

    /// A player joins the run in progress (Joep): a wizard beside a living
    /// one, with as many tomes as the most any human here carries, picked at
    /// random among those their elements can use. Returns its index.
    pub fn join(&mut self, seat: &Seat) -> usize {
        let i = self.wizards.len();
        let used: Vec<C> = self.wizards.iter().map(|&w| self.wiz(w).robe).collect();
        let robe = if used.contains(&seat.robe) { COLORS.iter().copied().find(|c| !used.contains(c)).unwrap_or(COLORS[i % COLORS.len()]) } else { seat.robe };
        let n = self.wizards.iter().filter(|&&w| self.wiz(w).bot.is_none()).map(|&w| self.wiz(w).relics.len()).max().unwrap_or(0);
        let near = self.wizards.iter().copied().find(|&w| self.alive(w)).map(|w| self.pos(w));
        // What the others learned, they know too.
        let unlocks = self.wizards.iter().filter(|&&w| self.wiz(w).bot.is_none()).map(|&w| self.wiz(w).unlocks).max();
        let (actor, mut wiz) = Wizard::make(i, seat, robe, &self.settings);
        if let Some(u) = unlocks {
            wiz.unlocks = wiz.unlocks.max(u);
        }
        let id = self.ecs.spawn((actor, wiz));
        self.wizards.push(id);
        self.unlocks_changed();
        if let Some(p) = near {
            let at = p + Vec3::new(1.2, 0.0, 0.8);
            let mut a = self.am(id);
            a.pos = at;
            a.vel = Vec3::ZERO;
        }
        let (els, forms) = {
            let wz = self.wiz(id);
            (wz.open_elements(), crate::grammar::FORMS.iter().copied().filter(|&f| wz.form_open(f)).collect::<Vec<_>>())
        };
        let has = |need: crate::relics::Need| match need {
            crate::relics::Need::Nothing => true,
            crate::relics::Need::El(e) => e.parts().iter().all(|p| els.contains(p)),
            crate::relics::Need::Form(f) => forms.contains(&f),
        };
        let mut rng = Pcg::new(self.seed ^ (i as u64 + 1).wrapping_mul(0x9e37_79b9));
        // Stacking tomes may come twice; a one-off only once.
        let mut picks: Vec<usize> = Vec::new();
        while picks.len() < n {
            let avoid: Vec<usize> = picks.iter().copied().filter(|&r| !crate::relics::stacks(r)).collect();
            let more = crate::relics::pick(n - picks.len(), &mut rng, &avoid, &has);
            if more.is_empty() {
                break;
            }
            picks.extend(more);
        }
        for r in picks {
            self.give_relic(id, r);
        }
        i
    }

    /// A fresh run (main.gd new_run): the next seed, the first biome.
    pub fn new_run(&mut self) {
        let s = self.seed;
        self.seed += 1;
        self.run_start(s);
    }

    /// A fresh run that starts in `biome`, or a random one for -1
    /// (main.gd start_at).
    pub fn start_at(&mut self, biome: i32) {
        use crate::rng::Rand;
        self.new_run();
        let n = crate::biome::biomes().len();
        let b = if biome < 0 { self.rng.pick(n) } else { (biome as usize).min(n - 1) };
        if b > 0 {
            self.run.biome_index = b;
            self.enter_level();
        }
    }

    /// A setting changed in the menu (main.gd _on_setting).
    /// Knobs were turned (live): what they set at the start applies now too.
    pub fn knobs_changed(&mut self) {
        for &id in &self.wizards {
            if let Ok(mut a) = self.ecs.get::<&mut Actor>(id) {
                a.speed = crate::knobs::wizard_speed() * a.mods.speed;
            }
        }
    }

    pub fn set_option(&mut self, key: &str, value: &str) {
        // Joep: "Level" in the host's settings goes straight there, a fresh
        // run in that level ("3. The Sunken Isles" or "3"). Only a change
        // moves: the value the host sends on connecting keeps the run.
        if key == "level" {
            let n: i32 = value.split(|c: char| !c.is_ascii_digit()).find(|t| !t.is_empty()).and_then(|t| t.parse().ok()).unwrap_or(1);
            let b = (n - 1).clamp(0, crate::biome::biomes().len() as i32 - 1);
            if b != self.settings.level {
                self.settings.level = b;
                self.start_at(b);
            }
            return;
        }
        self.settings.set(key, value);
        if key == "unlocks" {
            self.unlocks_changed();
        }
    }

    pub fn order(&mut self) -> Order {
        self.next_order += 1;
        Order(self.next_order)
    }

    /// Spawn a spell, bullet or pickup that ticks with the rest.
    pub fn spawn_fx<T: hecs::Component>(&mut self, c: T) -> Id {
        let o = self.order();
        let id = self.ecs.spawn((o, c));
        self.fx.push(id);
        id
    }

    pub fn ev(&mut self, e: Ev) {
        self.events.push(e);
    }

    pub fn burst(&mut self, at: Vec3, c: C, n: u32, speed: f32, size: f32, el: Option<El>) {
        self.events.push(Ev::Burst {
            at,
            c,
            n,
            speed,
            size,
            el,
        });
    }

    pub fn shake(&mut self, amount: f32) {
        self.events.push(Ev::Shake(amount));
    }

    pub fn count(&mut self, what: &'static str) {
        *self.stats.entry(what).or_insert(0) += 1;
    }

    pub fn stat(&self, what: &str) -> u64 {
        self.stats.get(what).copied().unwrap_or(0)
    }

    // --- Lookups -------------------------------------------------------

    pub fn a(&self, id: Id) -> hecs::Ref<'_, Actor> {
        self.ecs.get::<&Actor>(id).expect("actor")
    }

    pub fn am(&mut self, id: Id) -> hecs::RefMut<'_, Actor> {
        self.ecs.get::<&mut Actor>(id).expect("actor")
    }

    pub fn actor(&self, id: Id) -> Option<hecs::Ref<'_, Actor>> {
        self.ecs.get::<&Actor>(id).ok()
    }

    pub fn pos(&self, id: Id) -> Vec3 {
        self.actor(id).map(|a| a.pos).unwrap_or(Vec3::ZERO)
    }

    pub fn alive(&self, id: Id) -> bool {
        self.actor(id).is_some_and(|a| a.alive)
    }

    pub fn team(&self, id: Id) -> u8 {
        self.actor(id).map(|a| a.team).unwrap_or(MONSTERS)
    }

    pub fn mods(&self, id: Id) -> Mods {
        self.actor(id).map(|a| a.mods).unwrap_or_default()
    }

    /// Every actor a spell can touch: all wizards (a fallen wizard's grave
    /// can still be hit) and the monsters that live.
    pub fn living(&self) -> Vec<Id> {
        let mut out = self.wizards.clone();
        for &e in &self.enemies {
            if self.alive(e) {
                out.push(e);
            }
        }
        out
    }

    pub fn living_enemies(&self) -> Vec<Id> {
        self.enemies
            .iter()
            .copied()
            .filter(|&e| self.alive(e))
            .collect()
    }

    pub fn any_wizard_alive(&self) -> bool {
        self.wizards.iter().any(|&w| self.alive(w))
    }

    // --- Hits ----------------------------------------------------------

    /// Land `mix` on `target` with strength `scale` (1 for a burst, dt for a
    /// stream). `ff` scales damage from a teammate. Returns damage done
    /// (Actor.hit_spell, with the shieldbearer's shield in front).
    pub fn hit(
        &mut self,
        target: Id,
        mix: &[(El, f64)],
        scale: f64,
        src: Option<Id>,
        dir: Vec3,
        ff: f64,
    ) -> f64 {
        let scale = crate::rules::hit(self, target, src, mix, scale);
        let (src_team, mods) =
            match src.and_then(|s| self.ecs.get::<&Actor>(s).ok().map(|a| (a.team, a.mods))) {
                Some((t, m)) => (Some(t), m),
                None => (None, Mods::default()),
            };
        let Ok(mut a) = self.ecs.get::<&mut Actor>(target) else {
            return 0.0;
        };
        if !a.alive {
            return 0.0;
        }
        let ally = src_team == Some(a.team) && src != Some(target);
        if a.invuln > 0.0 && !ally {
            return 0.0;
        }
        let shield = a.shield && dir.length() >= 0.01;
        let before = a.push.length();
        let keep = a.armor;
        if shield {
            let fwd = Vec3::new(a.yaw.sin(), 0.0, a.yaw.cos());
            let front = dir.normalize().dot(fwd) < -0.35;
            let conducts = mix
                .iter()
                .any(|(e, _)| matches!(e, El::Lightning | El::Storm));
            if front && a.open_t <= 0.0 && !conducts {
                a.armor *= 0.2;
                a.shown.resist = true;
                if scale > 0.2 {
                    let at = a.pos + fwd * 0.5 + Vec3::Y;
                    self.events.push(Ev::Burst {
                        at,
                        c: fx::rgb(1.0, 0.85, 0.5),
                        n: 6,
                        speed: 3.0,
                        size: 0.12,
                        el: None,
                    });
                }
            }
        }
        let mut dmg = 0.0;
        let mut heal = 0.0;
        let mut shove = 0.0;
        let mut shock_hit = false;
        // Read before this hit adds its own statuses (Hunter's Mark).
        let afflicted = a.afflicted();
        let mut top_el = None;
        let mut top_d = 0.0;
        for &(e, count) in mix {
            let n = count * scale * mods.el[e.idx()] * if src_team == Some(WIZARDS) { crate::knobs::spell_damage() } else { 1.0 };
            let eff = e.effect();
            let mult = 1.0 - a.resistance(e);
            if eff.dmg > 0.0 {
                if mult > 1.05 {
                    a.shown.weak = true;
                } else if mult < 0.95 {
                    a.shown.resist = true;
                }
            }
            if a.heals_from.is_some_and(|h| e.contains(h)) {
                heal += eff.dmg * n;
                continue;
            }
            if e == El::Void && a.undead {
                heal += eff.dmg * n * 0.5;
                continue;
            }
            let mut d = eff.dmg * n * mult;
            if d > top_d {
                top_d = d;
                top_el = Some(e);
            }
            if eff.shock > 0.0 {
                if a.wet > 0.0 {
                    d *= 2.0;
                }
                shock_hit = true;
                a.stun = a.stun.max(if scale > 0.2 { 0.3 } else { 0.08 });
            }
            if e == El::Earth && a.frozen > 0.0 {
                d *= 2.0;
                a.frozen = 0.0;
                let at = a.pos + Vec3::Y * 0.8;
                self.events.push(Ev::Burst {
                    at,
                    c: fx::el_c(El::Ice),
                    n: 14,
                    speed: 5.0,
                    size: 0.35,
                    el: None,
                });
            }
            dmg += d;
            shove += eff.push * n * mult.max(0.0) * mods.push;
            if a.status(&eff, n * mult.max(0.0) * mods.status) {
                let at = a.pos + Vec3::Y;
                self.events.push(Ev::Puff {
                    at,
                    c: fx::el_c(El::Steam),
                    size: 1.2,
                });
            }
        }
        if ally {
            dmg *= ff;
            shove *= ff.max(0.35);
        }
        dmg *= mods.dmg * a.armor;
        if afflicted {
            dmg *= mods.afflicted;
        }
        if heal > 0.0 {
            a.healed(heal);
        }
        if shove > 0.0 && dir.length() > 0.01 {
            let m = a.mass;
            a.push += dir.normalize() * (shove * 2.2 / m) as f32;
        }
        if dmg > 0.0 {
            a.last_el = top_el;
            a.last_dir = dir;
            a.last_melee = a.melee_hit;
            a.take(dmg);
            a.hurt(dmg, dir);
            if shock_hit {
                a.flash = 0.12;
            }
        }
        if shield {
            a.armor = keep;
            if a.push.length() - before > 3.0 {
                a.open_t = 1.8;
            }
        }
        // Vampire Fang heals the striker, Bramble Mail strikes back.
        let back = if a.melee_hit { a.mods.thorns * dmg } else { 0.0 };
        drop(a);
        if let (Some(s), false, true) = (src, ally, dmg > 0.0) {
            if mods.leech > 0.0 {
                if let Ok(mut c) = self.ecs.get::<&mut Actor>(s) {
                    c.healed(dmg * mods.leech);
                }
            }
            if back > 0.0 {
                let per = El::Earth.effect().dmg.max(0.1);
                self.hit(s, &[(El::Earth, 1.0)], back / per, Some(target), Vec3::ZERO, 1.0);
            }
        }
        dmg
    }

    /// Plain damage, no element (burning, poison, a monster's blast).
    pub fn take(&mut self, id: Id, dmg: f64) {
        if let Ok(mut a) = self.ecs.get::<&mut Actor>(id) {
            a.take(dmg);
        }
    }

    /// Whatever died since the last flush gets its death: souls and blood,
    /// a grave, the boss's fall. Run after each step of each thing.
    pub fn flush_deaths(&mut self) {
        loop {
            let mut dying: Vec<Id> = Vec::new();
            for (id, a) in self.ecs.query_mut::<&mut Actor>() {
                if a.dying {
                    a.dying = false;
                    dying.push(id);
                }
            }
            if dying.is_empty() {
                return;
            }
            // In the order they were made, as a list walk would meet them.
            dying.sort_by_key(|id| id.to_bits());
            for id in dying {
                if self.ecs.satisfies::<&Wizard>(id).unwrap_or(false) {
                    self.wizard_died(id);
                } else if self.ecs.satisfies::<&Enemy>(id).unwrap_or(false) {
                    self.enemy_died(id);
                }
            }
        }
    }

    // --- Moving --------------------------------------------------------

    /// Godot's tick_status: water, burning and poison, then the timers.
    pub fn tick_status(&mut self, id: Id, dt: f64) {
        let wading = self.level.in_water(self.pos(id));
        let mut ripple = None;
        let mut a = self.am(id);
        a.wading = wading;
        if wading && a.alive {
            a.wet = a.wet.max(1.5);
            a.ripple_t -= dt;
            if a.ripple_t <= 0.0 {
                a.ripple_t = if a.vel.length() > 0.5 { 0.45 } else { 1.2 };
                ripple = Some((a.pos, a.radius + 0.5));
            }
        }
        if a.burn > 0.0 {
            a.burn -= dt;
            a.last_el = Some(El::Fire);
            a.last_melee = false;
            let d = crate::balance::BURN_DPS * dt * (1.0 - a.resistance(El::Fire)) * a.armor;
            a.take(d);
        }
        if a.poison > 0.0 {
            // The dose seeps out: fast while it is big, the last of it slowly.
            let seep = a.poison.min(
                (a.poison / crate::balance::POISON_TIME).max(crate::balance::POISON_FLOOR) * dt,
            );
            a.poison -= seep;
            if a.poison < 0.01 {
                a.poison = 0.0;
            }
            if a.burn <= 0.0 {
                a.last_el = Some(El::Poison);
            }
            a.last_melee = false;
            let d = seep * (1.0 - a.resistance(El::Poison)) * a.armor;
            a.take(d);
        }
        a.cool(dt);
        drop(a);
        if let Some((at, r)) = ripple {
            self.ev(Ev::Ripple { at, r, time: 0.9 });
        }
    }

    /// Move toward `desired` (a velocity), knockback on top (Actor.drive).
    pub fn drive(&mut self, id: Id, desired: Vec3, dt: f64) {
        let a = self.am(id);
        let mut desired = if a.can_act() { desired } else { Vec3::ZERO };
        desired.y = 0.0;
        let mut target = desired;
        if a.slick > 0.0 {
            // On ice you keep sliding where you were going.
            target = a.vel.lerp(desired * 1.3, (1.5 * dt) as f32) - a.push;
        }
        let mut v = target + a.push;
        v.y = 0.0;
        drop(a);
        self.move_and_slide(id, v, dt);
        let mut a = self.am(id);
        a.push *= (-5.0 * dt).exp() as f32;
    }

    /// Godot's move_and_slide for a floating body: slide along the walls
    /// and props, then out of other bodies in the way, then stay inside.
    pub fn move_and_slide(&mut self, id: Id, v: Vec3, dt: f64) {
        let (from, r, blocks) = {
            let a = self.a(id);
            (a.pos, a.radius, a.blocks)
        };
        let mut p = self.level.statics.slide(
            Vec2::new(from.x, from.z),
            Vec2::new(v.x, v.z) * dt as f32,
            r,
        );
        if blocks {
            for (oid, o) in self.ecs.query::<&Actor>().iter() {
                if oid == id || !o.solid {
                    continue;
                }
                let d = p - Vec2::new(o.pos.x, o.pos.z);
                let want = r + o.radius;
                let l = d.length();
                if l < want && l > 1e-4 {
                    p += d / l * (want - l);
                }
            }
            p = self.level.statics.push_out(p, r);
        }
        let mut at = Vec3::new(p.x, 0.0, p.y);
        at = self.keep_inside(id, at);
        let mut a = self.am(id);
        a.vel = (at - from) / dt as f32;
        a.vel.y = 0.0;
        a.pos = at;
    }

    /// Walls have gaps where they meet and a hard shove can squeeze
    /// through: in a walled level nobody ends up outside the band. A wizard
    /// also waits at a shut wall of mist (Ward::holds).
    fn keep_inside(&self, id: Id, mut p: Vec3) -> Vec3 {
        // A wizard waits at a shut wall of mist (Ward::holds), and monsters
        // never get past one that is still up (Joep): whatever is ahead of
        // the wizards stays ahead. Round a wall's end too, in open levels
        // where nothing walls the band in: there the path tells.
        let (wizard, monster) = {
            let a = self.a(id);
            (a.wizard, a.team != WIZARDS)
        };
        if wizard || monster {
            for wd in self.run.wards.iter().filter(|w| if wizard { w.holds } else { !w.opening && !w.back }) {
                let q = Vec2::new(p.x, p.z);
                let (d, within) = wd.past(q);
                // (Where the path loops back near the wall, its earlier
                // stretch is no part of it: the path tells which side. In a
                // sharp bend that stretch can lie just past the wall's line,
                // and holding it there was an invisible wall (Joep, 25 Nov).)
                let prog = self.level.progress_of(q);
                let s = if wd.back { -1.0 } else { 1.0 };
                let ahead = (prog - wd.at) * s;
                if within && d > -0.5 && d < 2.0 && ahead > -3.0 && ahead < 12.0 {
                    p.x -= wd.n.x * (d + 0.5);
                    p.z -= wd.n.y * (d + 0.5);
                    continue;
                }
                // Round its end, past it along the path: back to this side,
                // straight back through the wall's plane. Only near the wall:
                // where the path winds, a later stretch can be nearest from
                // open ground far from it, and moving a wizard by the path
                // between there and the wall threw them through it (Joep:
                // "teleporting through the mist wall"). And only a hold, never a
                // jump: a wizard found further past (a shove, a join) stays.
                let t = (q - wd.c).dot(wd.across());
                if ahead > -0.5 && ahead < 25.0 && d > -0.5 && d < 2.0 && t > wd.lo - 2.5 && t < wd.hi + 2.5 {
                    p.x -= wd.n.x * (d + 0.5);
                    p.z -= wd.n.y * (d + 0.5);
                }
            }
        }
        if !self.level.corridor {
            return p;
        }
        let q = Vec2::new(p.x, p.z);
        let d = self.level.bound(q);
        if d > -0.4 {
            let o = self.level.bound_out(q);
            p.x -= o.x * (d + 0.4);
            p.z -= o.y * (d + 0.4);
        }
        p
    }

    /// The first wall or prop between `a` and `b`: where, and which prop.
    pub fn ray(&self, a: Vec3, b: Vec3) -> Option<(Vec3, Option<usize>)> {
        self.level
            .statics
            .ray(a, b, 0.0)
            .map(|h| (a + (b - a) * h.t, h.prop))
    }

    // --- One tick ------------------------------------------------------

    pub fn step(&mut self) {
        self.events.clear();
        self.tick += 1;
        // Paused, even the clock stops: the shaders and effects run on it.
        if self.paused {
            return;
        }
        self.time += DT;
        let dt = DT;
        self.confluence(dt);
        self.dmg_t -= dt;
        if self.dmg_t <= 0.0 {
            self.dmg_t = 0.3;
            for id in self.wizards.clone().into_iter().chain(self.enemies.clone()) {
                self.numbers(id);
            }
        }
        for id in self.fields.clone() {
            if self.ecs.contains(id) {
                self.field_step(id, dt);
                self.flush_deaths();
            }
        }
        self.fields.retain(|&f| self.ecs.contains(f));
        for id in self.fx.clone() {
            if self.ecs.contains(id) {
                self.fx_step(id, dt);
                self.flush_deaths();
            }
        }
        self.fx.retain(|&f| self.ecs.contains(f));
        self.run_step(dt);
        self.flush_deaths();
        for id in self.wizards.clone() {
            self.wizard_step(id, dt);
            self.flush_deaths();
        }
        for id in self.enemies.clone() {
            if self.ecs.contains(id) {
                self.enemy_step(id, dt);
                self.flush_deaths();
            }
        }
        self.enemies.retain(|&e| self.ecs.contains(e));
        self.corpse_step(dt);
        self.later_step(dt);
        self.flush_deaths();
        crate::rules::tick(self);
        self.flush_deaths();
    }

    /// Do `what` in `seconds`.
    pub fn later(&mut self, seconds: f64, what: Later) {
        self.later.push((seconds, what));
    }

    /// Timers fire after the nodes have ticked, in the order they were set.
    fn later_step(&mut self, dt: f64) {
        let mut due = Vec::new();
        for (t, what) in std::mem::take(&mut self.later) {
            let t = t - dt;
            if t <= 0.0 {
                due.push(what);
            } else {
                self.later.push((t, what));
            }
        }
        for what in due {
            match what {
                Later::Meteor { caster, spell, at } => {
                    if self.ecs.contains(caster) {
                        self.overload(caster, &spell, at);
                    }
                }
                Later::Nova { caster, spell } => {
                    if self.ecs.contains(caster) {
                        let at = self.pos(caster);
                        self.nova(
                            caster,
                            &spell,
                            crate::spells::nova_n(&spell.n),
                            at,
                            true,
                            None,
                            1.0,
                        );
                    }
                }
                Later::Lob {
                    caster,
                    at,
                    el,
                    dmg,
                } => self.lob_land(caster, at, el, dmg),
                Later::Venom { caster, at, spell, puddle } => self.venom_splat(caster, at, &spell, puddle),
                Later::BlowUp {
                    id,
                    el,
                    r,
                    dmg,
                    field,
                } => self.blow_up(id, el, r, dmg, field),
                Later::Spawn {
                    kind,
                    at,
                    tag,
                    hp_scale,
                    champ,
                } => self.summoned(kind, at, tag, hp_scale, champ),
                Later::Travel => self.travelled(),
                Later::Shake(k) => self.shake(k),
            }
            self.flush_deaths();
        }
    }

    /// Small floating numbers for what an actor took and healed since the
    /// last time: white on monsters, gold and bigger on a weakness, grey and
    /// smaller when it resists, red on wizards, green for healing.
    pub(crate) fn numbers(&mut self, id: Id) {
        let Some(a) = self.actor(id) else { return };
        let s = a.shown;
        // The killing blow shows a little bigger.
        let killed = !a.alive && !a.wizard;
        let top = a.pos + Vec3::Y * if a.wizard { 2.6 } else { 1.4 + a.radius * 1.4 };
        let wizard = a.wizard;
        let pos = a.pos;
        drop(a);
        // The killing blow sits in the middle: drawing at a death would
        // change every roll after it.
        let side = if killed { Vec3::ZERO } else { Vec3::new(self.rng.range(-0.35, 0.35), 0.0, 0.0) };
        if s.dmg >= 1.0 {
            let mut text = format!("{}", s.dmg.round() as i64);
            let mut c = fx::rgb(0.95, 0.92, 0.85);
            let mut size = 30.0;
            if wizard {
                c = fx::rgb(1.0, 0.3, 0.25);
            } else if s.weak {
                text.push('!');
                c = fx::rgb(1.0, 0.72, 0.15);
                size = 42.0;
            } else if s.resist {
                c = fx::rgb(0.6, 0.62, 0.68);
                size = 24.0;
            }
            if killed {
                size *= 1.25;
            }
            self.ev(Ev::Text {
                at: top + side,
                text,
                c,
                size,
                time: 0.8,
            });
        } else if s.resist && !wizard {
            // Shrugged off: a dull grey glance off it, and the word where
            // the number would be (Joep).
            self.burst(pos + Vec3::Y, fx::rgb(0.6, 0.62, 0.68), 6, 2.5, 0.1, None);
            self.ev(Ev::Text {
                at: top + side,
                text: "RESIST".into(),
                c: fx::rgb(0.6, 0.62, 0.68),
                size: 24.0,
                time: 0.8,
            });
        }
        if s.heal >= 1.0 {
            let text = format!("+{}", s.heal.round() as i64);
            self.ev(Ev::Text {
                at: top - side + Vec3::Y * 0.25,
                text,
                c: fx::rgb(0.4, 1.0, 0.45),
                size: 28.0,
                time: 0.9,
            });
        }
        self.am(id).shown = Default::default();
    }

    /// Two wizards' streams that cross meet in the middle: the elements of
    /// both mix by the queue rules (water and fire make steam) and burst.
    fn confluence(&mut self, dt: f64) {
        let streams = self.streams();
        for v in self.confluence_t.values_mut() {
            *v -= dt;
        }
        for i in 0..streams.len() {
            for j in i + 1..streams.len() {
                let (ia, ca, fa, ta, sa) = &streams[i];
                let (ib, cb, fb, tb, sb) = &streams[j];
                if ca == cb {
                    continue;
                }
                let Some(hit) = segments(
                    Vec2::new(fa.x, fa.z),
                    Vec2::new(ta.x, ta.z),
                    Vec2::new(fb.x, fb.z),
                    Vec2::new(tb.x, tb.z),
                ) else {
                    continue;
                };
                let key = (ia.to_bits().get(), ib.to_bits().get());
                if self.confluence_t.get(&key).copied().unwrap_or(0.0) > 0.0 {
                    continue;
                }
                self.confluence_t.insert(key, 0.45);
                let mut q = crate::queue::Queue::new();
                q.size = 10;
                for &e in sa.iter().chain(sb.iter()) {
                    q.add(e);
                }
                let s = crate::grammar::compose(&q.slots, crate::Form::Nova, &Mods::default());
                if s.shape != crate::Shape::Nova {
                    continue;
                }
                let key = s.key.clone();
                let name = s.name.clone();
                let n = crate::spells::NovaN {
                    radius: 2.4,
                    rate: 0.9,
                };
                self.nova(*ca, &s, n, Vec3::new(hit.x, 0.0, hit.y), false, None, 1.0);
                self.confluences.entry(key).or_insert(name);
            }
        }
        self.confluence_t.retain(|_, v| *v > -1.0);
    }

    /// A spell was cast (Game.on_cast): counted, and new ones go in the
    /// grimoire.
    pub fn on_cast(&mut self, s: &crate::Spell) {
        if s.key.is_empty()
            || matches!(
                s.shape,
                crate::Shape::Staff | crate::Shape::Stomp | crate::Shape::Fizzle
            )
        {
            return;
        }
        self.count("casts");
        if !self.grimoire.contains_key(&s.key) {
            self.grimoire.insert(s.key.clone(), s.name.clone());
            self.ev(Ev::Grimoire {
                key: s.key.clone(),
                name: s.name.clone(),
            });
        }
    }

    pub fn summon_circle(&mut self, at: Vec3) {
        let c = fx::rgb(0.8, 0.08, 0.05);
        self.ev(Ev::Ring {
            at,
            c,
            r: 1.2,
            time: 0.8,
        });
        self.burst(at, c, 12, 2.0, 0.25, None);
        self.ev(Ev::LightPop {
            at,
            c,
            energy: 2.0,
            range: 4.0,
            time: 0.9,
        });
        self.ev(Ev::Summon { at });
    }

    /// Mana, queue and forms for everyone: the most learned player's stage.
    pub fn group_stage(&self) -> usize {
        if self.settings.forced_unlocks >= 0 {
            return self.settings.forced_unlocks.clamp(0, STAGES as i32 - 1) as usize;
        }
        if self.settings.unlock_all {
            return STAGES - 1;
        }
        let mut s = -1;
        for &id in &self.wizards {
            let w = self.ecs.get::<&Wizard>(id).unwrap();
            if w.bot.is_none() {
                s = s.max(w.unlocks.clamp(0, STAGES as i32 - 1));
            }
        }
        if s < 0 { STAGES - 1 } else { s as usize }
    }

    /// Everyone's queue and mana follow the stage when it changes.
    pub fn unlocks_changed(&mut self) {
        let stage = self.group_stage();
        for id in self.wizards.clone() {
            let mut w = self.ecs.get::<&mut Wizard>(id).unwrap();
            w.refresh(stage, &self.settings);
        }
    }

    /// A player's controller this tick.
    pub fn set_pad(&mut self, seat: usize, pad: &Pad) {
        if let Some(&id) = self.wizards.get(seat) {
            let mut w = self.ecs.get::<&mut Wizard>(id).unwrap();
            if w.bot.is_none() {
                w.input = pad.clone();
            }
        }
    }

    pub fn wiz(&self, id: Id) -> hecs::Ref<'_, Wizard> {
        self.ecs.get::<&Wizard>(id).expect("wizard")
    }

    pub fn wiz_mut(&mut self, id: Id) -> hecs::RefMut<'_, Wizard> {
        self.ecs.get::<&mut Wizard>(id).expect("wizard")
    }

    /// The streams running now: (entity, caster, from, to, slots).
    fn streams(&self) -> Vec<(Id, Id, Vec3, Vec3, Vec<El>)> {
        let mut out = Vec::new();
        for &id in &self.fx {
            if let Ok(s) = self.ecs.get::<&crate::spells::Stream>(id) {
                if s.running {
                    out.push((id, s.caster, s.from, s.to, s.slots.clone()));
                }
            }
        }
        out
    }
}

/// Where two segments cross (Geometry2D.segment_intersects_segment).
pub fn segments(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> Option<Vec2> {
    let r = a1 - a0;
    let s = b1 - b0;
    let den = r.perp_dot(s);
    if den.abs() < 1e-9 {
        return None;
    }
    let t = (b0 - a0).perp_dot(s) / den;
    let u = (b0 - a0).perp_dot(r) / den;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
        Some(a0 + r * t)
    } else {
        None
    }
}

/// Godot's `Vector3.rotated(UP, a)`.
pub fn yaw_rot(v: Vec3, a: f32) -> Vec3 {
    let (s, c) = a.sin_cos();
    Vec3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

/// Godot's `a.signed_angle_to(b, UP)` for flat vectors.
pub fn signed_angle(a: Vec3, b: Vec3) -> f32 {
    let cross = a.cross(b);
    let angle = cross.length().atan2(a.dot(b));
    if cross.y < 0.0 { -angle } else { angle }
}

pub fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Godot's `lerp_angle`.
pub fn lerp_angle(from: f32, to: f32, w: f32) -> f32 {
    let diff = (to - from) % std::f32::consts::TAU;
    let d = (2.0 * diff) % std::f32::consts::TAU - diff;
    from + d * w
}

pub fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    if a == b {
        return if x < a { 0.0 } else { 1.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Godot's `Vector3.move_toward`.
pub fn move_toward(from: Vec3, to: Vec3, delta: f32) -> Vec3 {
    let d = to - from;
    let l = d.length();
    if l <= delta || l < 1e-6 {
        to
    } else {
        from + d / l * delta
    }
}

#[cfg(test)]
mod ward_tests {
    use super::*;

    /// Walks up to every shut wall of mist along the path, in many levels,
    /// and reports where the hold moves a wizard who is still on this side.
    #[test]
    fn walls_hold_only_past_them() {
        let mut bad = vec![];
        for seed in 1..40u64 {
            for b in 0..crate::biome::biomes().len() {
                let mut w = World::new(&[Seat::player(0, "Joep")], seed, Settings::default());
                w.run.biome_index = b;
                w.enter_level();
                let id = w.wizards[0];
                for wd in w.run.wards.clone() {
                    if !wd.holds || wd.back {
                        continue;
                    }
                    // Open ground round the wall: whatever the path calls
                    // earlier than the wall must stay where it is.
                    for gx in -40..=40 {
                        for gz in -40..=40 {
                            let q = wd.c + Vec2::new(gx as f32, gz as f32) * 0.3;
                            if w.level.bound(q) > -0.5 || w.level.progress_of(q) > wd.at - 1.5 {
                                continue;
                            }
                            let p = Vec3::new(q.x, 0.0, q.y);
                            let out = w.keep_inside(id, p);
                            let (d, _) = wd.past(q);
                            if d < -0.6 && out.distance(p) > 0.05 || d >= -0.6 && w.level.progress_of(q) < wd.at - 3.0 && out.distance(p) > 0.05 {
                                bad.push(format!("seed {seed} biome {b} wall at {:.1}: ground prog {:.1} d {d:.2} moved {:.2} m", wd.at, w.level.progress_of(q), out.distance(p)));
                            }
                        }
                    }
                    // Just past it on the path: held back to this side.
                    for k in 1..4 {
                        let q = w.level.point_at(wd.at + 0.3 * k as f32);
                        let (d0, within) = wd.past(q);
                        if !within || d0 < -0.4 {
                            continue;
                        }
                        let out = w.keep_inside(id, Vec3::new(q.x, 0.0, q.y));
                        let (d1, _) = wd.past(Vec2::new(out.x, out.z));
                        if d1 > -0.4 {
                            bad.push(format!("seed {seed} biome {b} wall at {:.1}: not held at d {d0:.2}", wd.at));
                        }
                    }
                    let mut s = wd.at - 25.0;
                    while s < wd.at - 1.0 {
                        let c = w.level.point_at(s);
                        let n = (w.level.point_at(s + 0.5) - w.level.point_at(s - 0.5)).normalize_or(Vec2::Y);
                        let across = Vec2::new(-n.y, n.x);
                        for off in [-1.5f32, 0.0, 1.5] {
                            let q = c + across * off;
                            if w.level.corridor && w.level.bound(q) > -0.5 {
                                continue;
                            }
                            let p = Vec3::new(q.x, 0.0, q.y);
                            let out = w.keep_inside(id, p);
                            if out.distance(p) > 0.05 {
                                bad.push(format!("seed {seed} biome {b} wall at {:.1}: path {:.1} off {off} moved {:.2} m", wd.at, s, out.distance(p)));
                            }
                        }
                        s += 0.5;
                    }
                }
            }
        }
        for l in bad.iter().take(30) {
            eprintln!("{l}");
        }
        assert!(bad.is_empty(), "{} spots on this side of a wall were moved", bad.len());
    }
}
