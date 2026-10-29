//! Monsters (enemy.gd). Every kind is a row in KINDS; bosses are kinds with
//! `boss`. Ranged monsters fire bullet patterns ("shots", taken in turn),
//! slams throw a ring of bullets, and bosses cycle through BOSS_PATTERNS.
//! Bullets are slow and bright: the game is to walk, roll and swing through
//! them. Every attack shows itself in the body while the monster winds up
//! (the app poses it from `wind_k`); nothing is drawn on the floor.

use crate::El;
use crate::actor::Actor;
use crate::fx::{self, Death, Ev};
use crate::rng::Rand;
use crate::spells::NovaN;
use crate::world::{Id, MONSTERS, WIZARDS, World, flat, lerp_angle, signed_angle, yaw_rot};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attack {
    Melee,
    Ranged,
    Slam,
    Burrow,
    Dash,
    Lob,
}

/// Where a monster of a kind comes in (Run._spawn_spot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spawn {
    Ahead,
    Far,
    Under,
    Flank,
    Water,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shot {
    Fan3,
    Fan5,
    Ring10,
    Burst3,
    Wave7,
    Slow1,
    Snipe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossPattern {
    Spiral,
    Rings,
    Nova,
    Fan9,
}

pub const BOSS_PATTERNS: [BossPattern; 4] = [
    BossPattern::Spiral,
    BossPattern::Rings,
    BossPattern::Nova,
    BossPattern::Fan9,
];
pub const BOSS_WIND: f64 = 0.8;
/// How far to each side of where it faces a melee blow reaches.
pub const MELEE_ARC: f32 = 1.1;

#[derive(Debug)]
pub struct Kind {
    pub key: &'static str,
    pub name: &'static str,
    /// Whose body it wears; scaled by `size` when it borrows one.
    pub model: Option<&'static str>,
    pub hp: f64,
    pub speed: f64,
    pub dmg: f64,
    pub reach: f32,
    /// Ranged kinds keep this far off.
    pub keep: Option<f32>,
    pub el: El,
    pub size: f32,
    pub attack: Attack,
    pub shots: &'static [Shot],
    pub wind: Option<f64>,
    /// A slam's ring of bullets.
    pub ring: usize,
    pub mass: f64,
    pub undead: bool,
    pub resist: &'static [(El, f64)],
    pub heals_from: Option<El>,
    /// Pops in a burst of mud when it dies.
    pub burst: bool,
    pub always_wet: bool,
    pub practice: bool,
    /// Raises these out of the ground (necromancer).
    pub raise: Option<&'static str>,
    /// Never two in one wave.
    pub solo: bool,
    pub spawn: Spawn,
    /// A dash's speed and time.
    pub dash: (f32, f64),
    /// Leaves this on the ground where it pushed off.
    pub trail: Option<El>,
    pub pack: usize,
    pub shield: bool,
    pub boss: bool,
    pub summon: &'static [&'static str],
    pub nova: El,
}

const K: Kind = Kind {
    key: "",
    name: "",
    model: None,
    hp: 40.0,
    speed: 3.0,
    dmg: 9.0,
    reach: 1.3,
    keep: None,
    el: El::Earth,
    size: 1.0,
    attack: Attack::Melee,
    shots: &[Shot::Fan3],
    wind: None,
    ring: 0,
    mass: 1.0,
    undead: false,
    resist: &[],
    heals_from: None,
    burst: false,
    always_wet: false,
    practice: false,
    raise: None,
    solo: false,
    spawn: Spawn::Ahead,
    dash: (0.0, 0.0),
    trail: None,
    pack: 1,
    shield: false,
    boss: false,
    summon: &[],
    nova: El::Void,
};

use El::*;

const BONES: &[(El, f64)] = &[(Frost, 0.3), (Earth, -0.3), (Void, 0.5)];

pub static KINDS: [Kind; 26] = [
    Kind {
        key: "ghoul",
        name: "Ghoul",
        hp: 40.0,
        speed: 4.3,
        dmg: 9.0,
        reach: 1.3,
        el: Earth,
        size: 0.9,
        ..K
    },
    Kind {
        key: "skeleton",
        name: "Skeleton",
        hp: 50.0,
        speed: 3.3,
        dmg: 10.0,
        reach: 1.4,
        undead: true,
        resist: BONES,
        ..K
    },
    // A floating eye that keeps its distance and stares out fans of void.
    Kind {
        key: "watcher",
        name: "Watcher",
        hp: 34.0,
        speed: 3.0,
        dmg: 9.0,
        reach: 10.0,
        keep: Some(8.0),
        el: Void,
        attack: Attack::Ranged,
        shots: &[Shot::Fan3],
        resist: &[(Void, 0.6), (Lightning, -0.4)],
        ..K
    },
    Kind {
        key: "cultist",
        name: "Cultist",
        hp: 42.0,
        speed: 3.0,
        dmg: 11.0,
        reach: 9.0,
        keep: Some(7.0),
        el: Fire,
        attack: Attack::Ranged,
        shots: &[Shot::Fan5, Shot::Ring10],
        resist: &[(Fire, 0.6)],
        ..K
    },
    Kind {
        key: "bile",
        name: "Bile Horror",
        hp: 95.0,
        speed: 2.1,
        dmg: 12.0,
        reach: 1.5,
        el: Water,
        size: 1.25,
        mass: 2.0,
        resist: &[(Water, 1.0), (Earth, 0.2)],
        burst: true,
        ..K
    },
    Kind {
        key: "imp",
        name: "Imp",
        hp: 30.0,
        speed: 5.2,
        dmg: 8.0,
        reach: 7.0,
        keep: Some(5.0),
        el: Fire,
        size: 0.7,
        attack: Attack::Ranged,
        shots: &[Shot::Burst3],
        heals_from: Some(Fire),
        resist: &[(Frost, -0.5), (Water, -0.5)],
        ..K
    },
    Kind {
        key: "brute",
        name: "Brute",
        hp: 240.0,
        speed: 2.7,
        dmg: 24.0,
        reach: 2.2,
        size: 1.7,
        attack: Attack::Slam,
        ring: 10,
        mass: 3.5,
        resist: &[(Earth, 0.3)],
        ..K
    },
    Kind {
        key: "frost_brute",
        name: "Frost Brute",
        hp: 260.0,
        speed: 2.5,
        dmg: 22.0,
        reach: 2.2,
        el: Frost,
        size: 1.7,
        attack: Attack::Slam,
        ring: 10,
        mass: 3.5,
        resist: &[(Frost, 0.9), (Fire, -0.4)],
        ..K
    },
    Kind {
        key: "wraith",
        name: "Wraith",
        hp: 45.0,
        speed: 3.8,
        dmg: 10.0,
        reach: 8.0,
        keep: Some(6.0),
        el: Frost,
        attack: Attack::Ranged,
        shots: &[Shot::Wave7],
        undead: true,
        resist: &[(Frost, 1.0), (Earth, 0.5)],
        ..K
    },
    Kind {
        key: "drowned",
        name: "Drowned",
        hp: 55.0,
        speed: 3.1,
        dmg: 11.0,
        reach: 1.4,
        el: Water,
        resist: &[(Water, 1.0), (Fire, 0.4)],
        always_wet: true,
        ..K
    },
    Kind {
        key: "demon",
        name: "Demon",
        hp: 130.0,
        speed: 3.5,
        dmg: 16.0,
        reach: 1.7,
        el: Fire,
        size: 1.35,
        mass: 2.0,
        resist: &[(Fire, 1.0), (Frost, -0.3)],
        ..K
    },
    // Hangs back among the graves and looses one fast arrow. Brittle bones:
    // earth and life break it.
    Kind {
        key: "bone_archer",
        name: "Bone Archer",
        model: Some("skeleton"),
        hp: 32.0,
        speed: 3.2,
        dmg: 14.0,
        reach: 16.0,
        keep: Some(11.0),
        attack: Attack::Ranged,
        shots: &[Shot::Snipe],
        wind: Some(1.1),
        undead: true,
        resist: BONES,
        spawn: Spawn::Far,
        ..K
    },
    // The practice strip at the start of the first level: they stand
    // still, shoot slowly and hit softly.
    Kind {
        key: "practice_archer",
        name: "Bone Archer",
        model: Some("skeleton"),
        hp: 24.0,
        speed: 0.0,
        dmg: 5.0,
        reach: 18.0,
        keep: Some(11.0),
        attack: Attack::Ranged,
        shots: &[Shot::Snipe],
        wind: Some(1.5),
        undead: true,
        practice: true,
        ..K
    },
    Kind {
        key: "practice_caster",
        name: "Cultist",
        model: Some("cultist"),
        hp: 30.0,
        speed: 0.0,
        dmg: 4.0,
        reach: 14.0,
        keep: Some(9.0),
        el: Fire,
        attack: Attack::Ranged,
        shots: &[Shot::Slow1],
        practice: true,
        ..K
    },
    // Keeps far back and raises skeletons; when it dies, everything it
    // raised falls apart. Its robes burn.
    Kind {
        key: "necromancer",
        name: "Necromancer",
        hp: 60.0,
        speed: 2.7,
        dmg: 9.0,
        reach: 12.0,
        keep: Some(10.0),
        el: Void,
        attack: Attack::Ranged,
        shots: &[Shot::Fan3],
        raise: Some("skeleton"),
        solo: true,
        resist: &[(Void, 0.8), (Fire, -0.3)],
        spawn: Spawn::Far,
        ..K
    },
    // Tunnels under the floor where nothing hits it and bursts up under
    // you. Chilled, frozen or bogged in mud it is stuck up top.
    Kind {
        key: "lurker",
        name: "Lurker",
        hp: 70.0,
        speed: 3.4,
        dmg: 12.0,
        reach: 1.5,
        size: 1.1,
        attack: Attack::Burrow,
        mass: 1.6,
        resist: &[(Earth, 0.4), (Frost, -0.3)],
        spawn: Spawn::Under,
        ..K
    },
    // A spark that dashes straight through you; double damage if you are
    // wet. Lightning feeds it, earth grounds it.
    Kind {
        key: "sprite",
        name: "Storm Sprite",
        hp: 26.0,
        speed: 5.5,
        dmg: 7.0,
        reach: 6.5,
        keep: Some(4.5),
        el: Lightning,
        size: 0.7,
        attack: Attack::Dash,
        dash: (15.0, 0.32),
        wind: Some(0.5),
        heals_from: Some(Lightning),
        resist: &[(Earth, -0.6)],
        ..K
    },
    // Squats in the water and lobs globs of bog where it saw you a moment
    // ago, slowing you and leaving you soaked.
    Kind {
        key: "toad",
        name: "Bog Toad",
        hp: 55.0,
        speed: 2.4,
        dmg: 7.0,
        reach: 12.0,
        keep: Some(8.0),
        el: Mud,
        attack: Attack::Lob,
        wind: Some(0.7),
        resist: &[(Water, 1.0), (Earth, 0.3), (Fire, -0.4)],
        always_wet: true,
        spawn: Spawn::Water,
        ..K
    },
    // A tower shield in front turns aside almost everything that hits it
    // there. Get round it, arc lightning through the metal, or knock it
    // aside with earth or a gust.
    Kind {
        key: "shieldbearer",
        name: "Shieldbearer",
        hp: 120.0,
        speed: 2.4,
        dmg: 14.0,
        reach: 1.6,
        size: 1.15,
        mass: 2.5,
        shield: true,
        resist: &[(Lightning, -0.3)],
        ..K
    },
    // Hunts in packs from behind or the side, leaping from a few metres and
    // leaving fire where it pushed off.
    Kind {
        key: "hellhound",
        name: "Hellhound",
        hp: 38.0,
        speed: 5.6,
        dmg: 10.0,
        reach: 5.5,
        el: Fire,
        size: 0.85,
        attack: Attack::Dash,
        dash: (13.0, 0.3),
        wind: Some(0.55),
        trail: Some(Fire),
        resist: &[(Fire, 1.0), (Water, -0.5), (Frost, -0.3)],
        pack: 3,
        spawn: Spawn::Flank,
        ..K
    },
    // Bosses: one per biome, in the arena at the end of its level.
    Kind {
        key: "gravekeeper",
        name: "The Gravekeeper",
        model: Some("skeleton"),
        hp: 1100.0,
        speed: 2.6,
        dmg: 26.0,
        reach: 2.6,
        size: 2.3,
        attack: Attack::Slam,
        mass: 8.0,
        undead: true,
        resist: &[(Void, 0.8), (Frost, 0.3)],
        boss: true,
        ring: 16,
        summon: &["skeleton", "skeleton", "watcher"],
        nova: Void,
        ..K
    },
    Kind {
        key: "abbot",
        name: "The Bloated Abbot",
        model: Some("bile"),
        hp: 1300.0,
        speed: 2.0,
        dmg: 26.0,
        reach: 2.6,
        el: Water,
        size: 2.4,
        attack: Attack::Slam,
        mass: 8.0,
        resist: &[(Water, 1.0)],
        boss: true,
        ring: 16,
        summon: &["cultist", "ghoul", "ghoul"],
        nova: Mud,
        ..K
    },
    Kind {
        key: "drowned_king",
        name: "The Drowned King",
        model: Some("drowned"),
        hp: 1400.0,
        speed: 2.6,
        dmg: 26.0,
        reach: 2.6,
        el: Water,
        size: 2.3,
        attack: Attack::Slam,
        mass: 8.0,
        resist: &[(Water, 1.0), (Fire, 0.5)],
        boss: true,
        ring: 16,
        summon: &["drowned", "drowned", "bile"],
        nova: Water,
        always_wet: true,
        ..K
    },
    Kind {
        key: "warden",
        name: "The Frost Warden",
        model: Some("frost_brute"),
        hp: 1600.0,
        speed: 2.4,
        dmg: 30.0,
        reach: 2.8,
        el: Frost,
        size: 2.4,
        attack: Attack::Slam,
        mass: 9.0,
        resist: &[(Frost, 1.0), (Fire, -0.3)],
        boss: true,
        ring: 16,
        summon: &["wraith", "wraith", "skeleton"],
        nova: Ice,
        ..K
    },
    Kind {
        key: "pit_lord",
        name: "The Pit Lord",
        model: Some("demon"),
        hp: 2200.0,
        speed: 2.8,
        dmg: 34.0,
        reach: 3.0,
        el: Fire,
        size: 2.6,
        attack: Attack::Slam,
        mass: 10.0,
        resist: &[(Fire, 1.0), (Frost, -0.3)],
        boss: true,
        ring: 16,
        summon: &["imp", "imp", "demon"],
        nova: Magma,
        ..K
    },
    // Not in Godot's table: a stand-in so `Enemy::default()` (the component
    // lifted out while it steps) points at something.
    Kind {
        key: "none",
        name: "",
        ..K
    },
];

/// The kind called `key` (an index into KINDS).
pub fn kind(key: &str) -> usize {
    KINDS
        .iter()
        .position(|k| k.key == key)
        .unwrap_or_else(|| panic!("no monster kind {key}"))
}

pub struct Affix {
    pub name: &'static str,
    pub el: Option<El>,
    pub armor: Option<f64>,
    pub speed: Option<f64>,
    pub leech: Option<f64>,
}

const A0: Affix = Affix {
    name: "",
    el: None,
    armor: None,
    speed: None,
    leech: None,
};

pub static AFFIXES: [Affix; 6] = [
    Affix {
        name: "Fire Enchanted",
        el: Some(Fire),
        ..A0
    },
    Affix {
        name: "Cold Enchanted",
        el: Some(Frost),
        ..A0
    },
    Affix {
        name: "Lightning Enchanted",
        el: Some(Lightning),
        ..A0
    },
    Affix {
        name: "Stone Skin",
        armor: Some(0.6),
        ..A0
    },
    Affix {
        name: "Extra Fast",
        speed: Some(1.45),
        ..A0
    },
    Affix {
        name: "Vampiric",
        leech: Some(0.5),
        ..A0
    },
];
const PREFIX: [&str; 10] = [
    "Bone", "Gore", "Rot", "Ash", "Grim", "Blood", "Murk", "Hollow", "Vile", "Grave",
];
const SUFFIX: [&str; 10] = [
    "maw", "rend", "gut", "skull", "howl", "fang", "shade", "bane", "claw", "spite",
];

/// A shot fired a moment later (Enemy._later).
#[derive(Clone, Debug)]
enum Pending {
    Fan {
        count: usize,
        spread: f32,
        speed: f32,
        el: El,
        dmg: f64,
        size: f32,
    },
    Ring {
        count: usize,
        speed: f32,
        el: El,
        dmg: f64,
        offset: f32,
        size: f32,
    },
    Boss {
        pattern: BossPattern,
        el: El,
        dmg: f64,
    },
}

#[derive(Clone, Debug)]
pub struct Enemy {
    pub kind: usize,
    pub elite: Option<usize>,
    pub elite_name: String,
    pub target: Option<Id>,
    /// Winding up an attack: time left of `wind_time()`.
    pub wind: f64,
    cool: f64,
    think: f64,
    special: f64,
    summon_t: f64,
    strafe: f32,
    shot_i: usize,
    pattern_i: usize,
    pending: Vec<(f64, Pending)>,
    pub spiral_t: f64,
    spiral_ang: f32,
    spiral_step: f64,
    /// Raised by a necromancer, and falls apart with it.
    pub master: Option<Id>,
    raise_t: f64,
    pub dash_t: f64,
    pub dash_dir: Vec3,
    dash_hit: Vec<Id>,
    /// A lurker under the floor, and how long it stays up once it surfaced.
    pub under: bool,
    surface_t: f64,
    /// The heave over a lurker about to burst up (0 to 1).
    pub heave: f32,
    /// Which way the attack winding up will go: follows the target until
    /// three quarters in, then held.
    pub lock: Vec3,
    aim_lock: Vec3,
    pub boss_wind: f64,
    best_d: f32,
    stuck_t: f64,
    detour_t: f64,
    /// Rising out of the ground (raised, erupted): the app eases the body up.
    pub rise_t: f64,
    pub rise_from: f32,
    /// A melee blow's lunge, for the app.
    pub lunge_t: f64,
    pub lunge_dir: Vec3,
    /// Which shrine's guard it is (-1: none).
    pub guard: i32,
    /// Dead: freed after this many more seconds, once its death has played.
    pub gone: f64,
    /// Dead and swelling to burst (imps, sprites, watchers, demons).
    pub fuse: f64,
    pub fuse_t: f64,
    pub how: Option<Death>,
}

impl Default for Enemy {
    fn default() -> Enemy {
        Enemy {
            kind: KINDS.len() - 1,
            elite: None,
            elite_name: String::new(),
            target: None,
            wind: 0.0,
            cool: 0.0,
            think: 0.0,
            special: 4.0,
            summon_t: 8.0,
            strafe: 1.0,
            shot_i: 0,
            pattern_i: 0,
            pending: Vec::new(),
            spiral_t: 0.0,
            spiral_ang: 0.0,
            spiral_step: 0.0,
            master: None,
            raise_t: 5.0,
            dash_t: 0.0,
            dash_dir: Vec3::ZERO,
            dash_hit: Vec::new(),
            under: false,
            surface_t: 0.0,
            heave: 0.0,
            lock: Vec3::ZERO,
            aim_lock: Vec3::ZERO,
            boss_wind: 0.0,
            best_d: f32::INFINITY,
            stuck_t: 0.0,
            detour_t: 0.0,
            rise_t: 0.0,
            rise_from: 0.0,
            lunge_t: 0.0,
            lunge_dir: Vec3::ZERO,
            guard: -1,
            gone: f64::INFINITY,
            fuse: 0.0,
            fuse_t: 0.0,
            how: None,
        }
    }
}

impl Enemy {
    pub fn cfg(&self) -> &'static Kind {
        &KINDS[self.kind]
    }

    pub fn affix(&self) -> Option<&'static Affix> {
        self.elite.map(|i| &AFFIXES[i])
    }

    /// The body it wears.
    pub fn model(&self) -> &'static str {
        let c = self.cfg();
        c.model.unwrap_or(c.key)
    }

    /// What its attacks are made of.
    pub fn attack_el(&self) -> El {
        self.affix().and_then(|a| a.el).unwrap_or(self.cfg().el)
    }

    pub fn wind_time(&self) -> f64 {
        let c = self.cfg();
        c.wind
            .unwrap_or(if c.attack == Attack::Slam { 0.9 } else { 0.6 })
    }

    /// How far through the wind-up it is (0 to 1), boss patterns too, or 0.
    pub fn wind_k(&self) -> f64 {
        if self.wind > 0.0 {
            1.0 - self.wind / self.wind_time()
        } else if self.boss_wind > 0.0 {
            1.0 - self.boss_wind / BOSS_WIND
        } else {
            0.0
        }
    }

    /// Enemy.setup: a monster of kind `k`, its health scaled, maybe an elite.
    pub fn setup(
        k: usize,
        hp_scale: f64,
        champion: bool,
        at: Vec3,
        rng: &mut impl Rand,
    ) -> (Actor, Enemy) {
        let c = &KINDS[k];
        let speed = c.speed * rng.range(0.92, 1.08) as f64 * crate::knobs::monster_speed();
        let mut a = Actor::new(MONSTERS, at, c.hp * hp_scale, 0.42 * c.size, speed);
        a.mass = c.mass;
        a.undead = c.undead;
        for &(e, r) in c.resist {
            a.resist[e.idx()] = r;
        }
        a.heals_from = c.heals_from;
        a.shield = c.shield;
        let mut e = Enemy {
            kind: k,
            ..Default::default()
        };
        if champion && !c.boss {
            let i = rng.pick(AFFIXES.len());
            let x = &AFFIXES[i];
            e.elite = Some(i);
            a.max_hp *= 2.2;
            let name = format!(
                "{}{}",
                PREFIX[rng.pick(PREFIX.len())],
                SUFFIX[rng.pick(SUFFIX.len())]
            );
            e.elite_name = format!("{} the {}", name, x.name);
            if let Some(el) = x.el {
                a.resist[el.idx()] = 1.0;
            }
            if let Some(ar) = x.armor {
                a.armor = ar;
            }
            if let Some(s) = x.speed {
                a.speed *= s;
            }
        }
        a.hp = a.max_hp;
        e.strafe = if rng.f() < 0.5 { 1.0 } else { -1.0 };
        if c.always_wet {
            a.wet = 999.0;
        }
        if c.attack == Attack::Burrow {
            e.under = true;
            a.solid = false;
        }
        (a, e)
    }
}

/// A monster's attack as a spell: one unit of `el`, scaled to do `dmg`.
pub fn attack_scale(el: El, dmg: f64) -> f64 {
    let e = el.effect().dmg;
    dmg / if e > 0.0 { e } else { 8.0 }
}

impl World {
    pub fn enemy(&self, id: Id) -> hecs::Ref<'_, Enemy> {
        self.ecs.get::<&Enemy>(id).expect("enemy")
    }

    pub fn enemy_mut(&mut self, id: Id) -> hecs::RefMut<'_, Enemy> {
        self.ecs.get::<&mut Enemy>(id).expect("enemy")
    }

    /// A new monster in the fight (Game.add_enemy).
    pub fn add_enemy(
        &mut self,
        k: usize,
        at: Vec3,
        hp_scale: f64,
        champion: bool,
        guard: i32,
    ) -> Id {
        let at = flat(self.level.clamp_inside(at));
        let (a, mut e) = Enemy::setup(k, hp_scale, champion, at, &mut self.rng);
        e.guard = guard;
        let id = self.ecs.spawn((a, e));
        self.enemies.push(id);
        id
    }

    fn damage_scale(&self) -> f64 {
        self.settings.monster_damage * crate::knobs::monster_damage()
    }

    /// One monster's tick (Enemy._physics_process). Its Enemy component is
    /// lifted out while it runs.
    pub fn enemy_step(&mut self, id: Id, dt: f64) {
        let mut e = std::mem::take(&mut *self.enemy_mut(id));
        self.enemy_tick(id, &mut e, dt);
        if let Ok(mut c) = self.ecs.get::<&mut Enemy>(id) {
            *c = e;
        }
    }

    fn enemy_tick(&mut self, id: Id, e: &mut Enemy, dt: f64) {
        self.tick_status(id, dt);
        e.rise_t = (e.rise_t - dt).max(0.0);
        e.lunge_t = (e.lunge_t - dt).max(0.0);
        e.boss_wind = (e.boss_wind - dt).max(0.0);
        if !self.alive(id) {
            return;
        }
        let c = e.cfg();
        e.think -= dt;
        e.cool = (e.cool - dt).max(0.0);
        let lost = e
            .target
            .is_none_or(|t| !self.ecs.contains(t) || (!self.alive(t) && self.any_wizard_alive()));
        if e.think <= 0.0 || lost {
            e.think = 0.5;
            e.target = self.pick_target(id, e.target);
        }
        let mut want = Vec3::ZERO;
        {
            let mut a = self.am(id);
            a.open_t = (a.open_t - dt).max(0.0);
        }
        if e.dash_t > 0.0 {
            self.dashing(id, e, dt);
            return;
        }
        if e.under {
            self.tunnel(id, e, dt);
            return;
        }
        if c.attack == Attack::Burrow {
            e.surface_t -= dt;
            // Back down when its time up top is over, unless the cold or
            // the mud holds it.
            let (can, chill, slow) = {
                let a = self.a(id);
                (a.can_act(), a.chill, a.slow)
            };
            if e.surface_t <= 0.0 && e.wind <= 0.0 && can && chill <= 0.0 && slow <= 0.0 {
                self.go_under(id, e, false);
                return;
            }
        }
        if c.raise.is_some() {
            self.raise_dead(id, e, dt);
        }
        if let Some(t) = e.target.filter(|_| self.a(id).can_act()) {
            let (pos, yaw) = {
                let a = self.a(id);
                (a.pos, a.yaw)
            };
            let to = flat(self.pos(t) - pos);
            let d = to.length();
            let dir = to / d.max(0.01);
            let reach = c.reach + self.a(t).radius;
            if e.wind > 0.0 {
                e.wind -= dt;
                let k = 1.0 - e.wind / e.wind_time();
                if k < 0.75 {
                    e.lock = self.aim(id, e);
                }
                if e.wind <= 0.0 {
                    self.monster_strike(id, e);
                }
            } else if let Some(keep) = c.keep {
                if d > reach {
                    want = dir;
                } else if d < keep - 1.0 {
                    want = -dir;
                } else {
                    want = dir.cross(Vec3::Y) * e.strafe * 0.5;
                }
                if d <= reach && e.cool <= 0.0 {
                    self.begin(id, e);
                }
            } else {
                if d > reach * 0.8 {
                    want = dir;
                }
                if d <= reach && e.cool <= 0.0 {
                    self.begin(id, e);
                }
            }
            want = self.unstick(id, e, t, want, d, reach, dt);
            want += self.separation(id) * 0.8;
            if want.length() > 1.0 {
                want = want.normalize();
            }
            let face = if want.length() < 0.2 || e.wind > 0.0 {
                dir
            } else {
                want.normalize()
            };
            self.am(id).yaw = lerp_angle(yaw, face.x.atan2(face.z), (8.0 * dt) as f32);
        }
        if c.boss {
            self.boss(id, e, dt);
            if e.spiral_t > 0.0 {
                want *= 0.25;
            }
        }
        self.run_pending(id, e, dt);
        let spd = self.a(id).move_speed();
        self.drive(id, want * spd as f32, dt);
    }

    fn begin(&mut self, id: Id, e: &mut Enemy) {
        e.wind = e.wind_time();
        e.lock = self.aim(id, e);
        // A glint over the head: something is coming.
        let at = self.pos(id) + Vec3::Y * (1.3 * e.cfg().size + 0.4);
        self.ev(Ev::Flash {
            at,
            c: fx::rgb(1.0, 0.85, 0.55),
            size: 0.7,
            time: 0.25,
        });
    }

    /// Which way the target is, or ahead.
    fn aim(&self, id: Id, e: &Enemy) -> Vec3 {
        let a = self.a(id);
        let Some(t) = e.target.filter(|&t| self.ecs.contains(t)) else {
            return yaw_rot(Vec3::NEG_Z, a.yaw);
        };
        let d = flat(self.pos(t) - a.pos);
        if d.length() > 0.01 {
            d.normalize()
        } else {
            Vec3::NEG_Z
        }
    }

    fn monster_strike(&mut self, id: Id, e: &mut Enemy) {
        let c = e.cfg();
        e.cool = if c.attack != Attack::Slam {
            self.rng.range(1.2, 1.8) as f64
        } else {
            2.2
        };
        let Some(t) = e.target.filter(|&t| self.ecs.contains(t)) else {
            return;
        };
        let dmg = c.dmg * self.damage_scale();
        let el = e.attack_el();
        let scale = attack_scale(el, dmg);
        let mix = vec![(el, 1.0)];
        let pos = self.pos(id);
        let attack = if c.attack == Attack::Burrow {
            Attack::Melee
        } else {
            c.attack
        };
        match attack {
            Attack::Dash => {
                // Off it goes, straight through where the target stood.
                e.dash_t = c.dash.1;
                e.dash_dir = if e.lock != Vec3::ZERO {
                    e.lock
                } else {
                    self.aim(id, e)
                };
                e.dash_hit.clear();
                e.cool = self.rng.range(2.2, 3.0) as f64;
                if let Some(tr) = c.trail {
                    self.add_field(pos, tr, 0.9, 2.5, None);
                }
            }
            Attack::Lob => {
                let at = {
                    let ta = self.a(t);
                    ta.pos + ta.vel * 0.35
                };
                self.lob(id, at, el, dmg);
                e.cool = self.rng.range(1.8, 2.6) as f64;
            }
            Attack::Melee => {
                let to = flat(self.pos(t) - pos);
                let tr = self.a(t).radius;
                // Only where it showed: out of the slice is out of reach.
                let side = if to.length() > 0.3 {
                    signed_angle(e.lock, to).abs()
                } else {
                    0.0
                };
                if to.length() <= c.reach + tr + 0.6
                    && side <= MELEE_ARC + tr.atan2(to.length().max(0.1))
                {
                    let n = to.normalize_or_zero();
                    self.am(t).melee_hit = true;
                    if c.shield {
                        // A shove with the shield as well as the mace.
                        let mut ta = self.am(t);
                        let m = ta.mass as f32;
                        ta.push += n * 7.0 / m;
                    }
                    let dealt = self.hit(t, &mix, scale, Some(id), n, 1.0);
                    if let Ok(mut ta) = self.ecs.get::<&mut Actor>(t) {
                        ta.melee_hit = false;
                    }
                    if let Some(l) = e.affix().and_then(|a| a.leech) {
                        self.am(id).healed(dealt * l);
                    }
                    e.lunge_t = 0.2;
                    e.lunge_dir = n;
                }
            }
            Attack::Slam => {
                let r = c.reach + 0.8;
                let ring_c = fx::mix(fx::el_c(el), fx::rgb(0.6, 0.5, 0.4), 0.5);
                self.ev(Ev::Ring {
                    at: pos,
                    c: ring_c,
                    r,
                    time: 0.3,
                });
                let at = pos + (self.pos(t) - pos).normalize_or_zero() * 1.2;
                self.burst(at, fx::rgb(0.5, 0.4, 0.3), 20, 5.0, 0.15, None);
                self.shake(if c.boss { 0.6 } else { 0.3 });
                if c.ring > 0 {
                    let off = self.rng.f() * TAU;
                    self.ring_shot(id, c.ring, 6.0, el, dmg * 0.6, off, 0.24);
                }
                for w in self.wizards.clone() {
                    let to = flat(self.pos(w) - pos);
                    let (alive, wr) = {
                        let a = self.a(w);
                        (a.alive, a.radius)
                    };
                    if alive && to.length() < r + wr {
                        self.am(w).melee_hit = true;
                        self.hit(w, &mix, scale, Some(id), to.normalize_or_zero(), 1.0);
                        self.am(w).melee_hit = false;
                    }
                }
            }
            Attack::Ranged => {
                let shot = c.shots[e.shot_i % c.shots.len()];
                self.volley(id, e, shot, el, dmg);
                e.shot_i += 1;
            }
            Attack::Burrow => {}
        }
    }

    /// One of the small monsters' patterns.
    fn volley(&mut self, id: Id, e: &mut Enemy, shot: Shot, el: El, dmg: f64) {
        match shot {
            Shot::Fan3 => self.fan(id, e, 3, 22.0, 9.0, el, dmg, 0.22),
            Shot::Fan5 => self.fan(id, e, 5, 56.0, 7.0, el, dmg * 0.8, 0.22),
            Shot::Ring10 => {
                let off = self.rng.f() * TAU;
                self.ring_shot(id, 10, 5.0, el, dmg * 0.8, off, 0.24);
            }
            Shot::Burst3 => {
                for i in 0..3 {
                    self.later_shot(
                        id,
                        e,
                        0.16 * i as f64,
                        Pending::Fan {
                            count: 1,
                            spread: 0.0,
                            speed: 9.5,
                            el,
                            dmg: dmg * 0.7,
                            size: 0.22,
                        },
                    );
                }
            }
            Shot::Wave7 => self.fan(id, e, 7, 84.0, 4.6, el, dmg * 0.7, 0.26),
            // One big slow ball: easy to knock back with the staff.
            Shot::Slow1 => self.fan(id, e, 1, 0.0, 4.2, el, dmg, 0.32),
            Shot::Snipe => {
                // One fast arrow down the line it showed.
                let dir = if e.aim_lock != Vec3::ZERO {
                    e.aim_lock
                } else {
                    self.aim(id, e)
                };
                let at = self.pos(id) + dir * (self.a(id).radius + 0.3);
                self.fire_bullet(id, at, dir, 17.0, el, attack_scale(el, dmg), 0.16, true);
                e.aim_lock = Vec3::ZERO;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fan(
        &mut self,
        id: Id,
        e: &Enemy,
        count: usize,
        spread_deg: f32,
        speed: f32,
        el: El,
        dmg: f64,
        size: f32,
    ) {
        let aim = self.aim(id, e);
        let (pos, r) = {
            let a = self.a(id);
            (a.pos, a.radius)
        };
        for i in 0..count {
            let k = if count == 1 {
                0.0
            } else {
                i as f32 / (count - 1) as f32 - 0.5
            };
            let dir = yaw_rot(aim, spread_deg.to_radians() * k);
            self.fire_bullet(
                id,
                pos + dir * (r + 0.3),
                dir,
                speed,
                el,
                attack_scale(el, dmg),
                size,
                false,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ring_shot(
        &mut self,
        id: Id,
        count: usize,
        speed: f32,
        el: El,
        dmg: f64,
        offset: f32,
        size: f32,
    ) {
        let (pos, r) = {
            let a = self.a(id);
            (a.pos, a.radius)
        };
        for i in 0..count {
            let dir = yaw_rot(Vec3::NEG_Z, offset + TAU * i as f32 / count as f32);
            self.fire_bullet(
                id,
                pos + dir * (r + 0.3),
                dir,
                speed,
                el,
                attack_scale(el, dmg),
                size,
                false,
            );
        }
    }

    fn later_shot(&mut self, id: Id, e: &mut Enemy, seconds: f64, p: Pending) {
        if seconds <= 0.0 {
            self.fire_pending(id, e, p);
        } else {
            e.pending.push((seconds, p));
        }
    }

    fn fire_pending(&mut self, id: Id, e: &mut Enemy, p: Pending) {
        match p {
            Pending::Fan {
                count,
                spread,
                speed,
                el,
                dmg,
                size,
            } => self.fan(id, e, count, spread, speed, el, dmg, size),
            Pending::Ring {
                count,
                speed,
                el,
                dmg,
                offset,
                size,
            } => self.ring_shot(id, count, speed, el, dmg, offset, size),
            Pending::Boss { pattern, el, dmg } => {
                if self.alive(id) {
                    self.boss_pattern(id, e, pattern, el, dmg);
                }
            }
        }
    }

    fn run_pending(&mut self, id: Id, e: &mut Enemy, dt: f64) {
        if e.pending.is_empty() {
            return;
        }
        let all = std::mem::take(&mut e.pending);
        let mut keep = Vec::new();
        for (t, p) in all {
            let t = t - dt;
            if t <= 0.0 {
                if self.a(id).can_act() {
                    self.fire_pending(id, e, p);
                }
            } else {
                keep.push((t, p));
            }
        }
        // Patterns queued while firing (a boss's rings) come after the rest.
        keep.append(&mut e.pending);
        e.pending = keep;
    }

    fn boss(&mut self, id: Id, e: &mut Enemy, dt: f64) {
        let c = e.cfg();
        e.special -= dt;
        e.summon_t -= dt;
        let el = c.nova;
        let dmg = c.dmg * 0.5 * self.damage_scale();
        let can = self.a(id).can_act();
        if e.spiral_t > 0.0 {
            e.spiral_t -= dt;
            e.spiral_step -= dt;
            if e.spiral_step <= 0.0 && can {
                e.spiral_step = 0.1;
                e.spiral_ang += 0.33;
                self.ring_shot(id, 3, 6.0, el, dmg, e.spiral_ang, 0.26);
            }
        }
        if e.special <= 0.0 && can {
            e.special = self.rng.range(4.5, 6.0) as f64;
            let pattern = BOSS_PATTERNS[e.pattern_i % BOSS_PATTERNS.len()];
            e.pattern_i += 1;
            // Shown first like every attack, in the body: a glint, and it
            // rears up and swells before the pattern comes.
            let at = self.pos(id) + Vec3::Y * (1.3 * c.size + 0.4);
            self.ev(Ev::Flash {
                at,
                c: fx::rgb(1.0, 0.85, 0.55),
                size: 1.2,
                time: 0.3,
            });
            e.boss_wind = BOSS_WIND;
            self.later_shot(id, e, BOSS_WIND, Pending::Boss { pattern, el, dmg });
        }
        if e.summon_t <= 0.0 {
            e.summon_t = self.rng.range(10.0, 14.0) as f64;
            let pos = self.pos(id);
            for k in c.summon {
                let off = Vec3::new(self.rng.range(-3.0, 3.0), 0.0, self.rng.range(-3.0, 3.0));
                self.spawn_enemy(kind(k), pos + off, false, -1);
            }
        }
    }

    /// A boss's pattern once its wind-up is over.
    fn boss_pattern(&mut self, id: Id, e: &mut Enemy, pattern: BossPattern, el: El, dmg: f64) {
        match pattern {
            BossPattern::Spiral => {
                e.spiral_t = 3.2;
                e.spiral_step = 0.0;
            }
            BossPattern::Rings => {
                for i in 0..3 {
                    let off = if i % 2 == 1 { 0.5 } else { 0.0 } * TAU / 20.0;
                    self.later_shot(
                        id,
                        e,
                        0.55 * i as f64,
                        Pending::Ring {
                            count: 20,
                            speed: 5.2,
                            el,
                            dmg,
                            offset: off,
                            size: 0.28,
                        },
                    );
                }
            }
            BossPattern::Fan9 => {
                for i in 0..3 {
                    self.later_shot(
                        id,
                        e,
                        0.45 * i as f64,
                        Pending::Fan {
                            count: 9,
                            spread: 90.0,
                            speed: 7.5,
                            el,
                            dmg,
                            size: 0.22,
                        },
                    );
                }
            }
            BossPattern::Nova => {
                let d = e.cfg().dmg * 0.7 * self.damage_scale();
                let n = NovaN {
                    radius: 7.0,
                    rate: attack_scale(el, d),
                };
                let at = self.pos(id);
                self.burst_nova(id, el, n, at, Some(WIZARDS));
                self.shake(0.5);
            }
        }
    }

    /// Dashing: a straight run, hitting every wizard it passes through once.
    fn dashing(&mut self, id: Id, e: &mut Enemy, dt: f64) {
        e.dash_t -= dt;
        if !self.a(id).can_act() {
            e.dash_t = 0.0;
            return;
        }
        let c = e.cfg();
        let v = e.dash_dir * c.dash.0 + self.a(id).push;
        self.move_and_slide(id, flat(v), dt);
        if let Some(tr) = c.trail {
            if self.rng.f() < 0.3 {
                let at = self.pos(id) + Vec3::Y * 0.4;
                self.burst(at, fx::el_c(tr), 3, 1.0, 0.2, None);
            }
        }
        let dmg = c.dmg * self.damage_scale();
        let el = e.attack_el();
        let scale = attack_scale(el, dmg);
        let (pos, r) = {
            let a = self.a(id);
            (a.pos, a.radius)
        };
        for w in self.wizards.clone() {
            let (alive, wp, wr) = {
                let a = self.a(w);
                (a.alive, a.pos, a.radius)
            };
            if !alive || e.dash_hit.contains(&w) {
                continue;
            }
            if wp.distance(pos) < r + wr + 0.25 {
                e.dash_hit.push(w);
                self.am(w).melee_hit = true;
                self.hit(w, &[(el, 1.0)], scale, Some(id), e.dash_dir, 1.0);
                self.am(w).melee_hit = false;
                if let Some(l) = e.affix().and_then(|a| a.leech) {
                    self.am(id).healed(dmg * l);
                }
            }
        }
    }

    /// A glob of bog arcs over to `at` (a ring shows where) and bursts there
    /// after its flight: a little damage, slowed and soaked, and a puddle.
    fn lob(&mut self, id: Id, at: Vec3, el: El, dmg: f64) {
        let at = self.level.clamp_inside(at);
        let flight = 0.9;
        self.ev(Ev::Ring {
            at,
            c: fx::lighten(fx::el_c(el), 0.3),
            r: 1.6,
            time: flight as f32,
        });
        let from = self.pos(id) + Vec3::Y * 0.8;
        self.ev(Ev::Lob {
            from,
            to: at,
            el,
            time: flight as f32,
        });
        self.later(
            flight,
            crate::world::Later::Lob {
                caster: id,
                at,
                el,
                dmg,
            },
        );
    }

    /// The glob lands (the end of Enemy._lob's tween).
    pub fn lob_land(&mut self, caster: Id, at: Vec3, el: El, dmg: f64) {
        if self.ecs.contains(caster) {
            let n = NovaN {
                radius: 1.6,
                rate: attack_scale(el, dmg),
            };
            self.burst_nova(caster, el, n, at, Some(WIZARDS));
        }
        self.add_field(at, El::Water, 1.4, 5.0, None);
    }

    /// A lurker goes down into the floor: nothing can hit it there.
    fn go_under(&mut self, id: Id, e: &mut Enemy, at_once: bool) {
        e.under = true;
        e.wind = 0.0;
        self.am(id).solid = false;
        if !at_once {
            let at = self.pos(id) + Vec3::Y * 0.2;
            self.burst(at, fx::rgb(0.3, 0.22, 0.15), 14, 3.0, 0.15, None);
        }
    }

    /// Under the floor: close in on the target, shake the ground over it,
    /// burst up.
    fn tunnel(&mut self, id: Id, e: &mut Enemy, dt: f64) {
        {
            let mut a = self.am(id);
            a.invuln = a.invuln.max(0.1);
        }
        let Some(t) = e.target.filter(|&t| self.ecs.contains(t)) else {
            self.drive(id, Vec3::ZERO, dt);
            return;
        };
        let to = flat(self.pos(t) - self.pos(id));
        if e.wind > 0.0 {
            e.wind -= dt;
            e.heave = ((0.7 - e.wind) / 0.7).clamp(0.0, 1.0) as f32;
            self.drive(id, Vec3::ZERO, dt);
            if e.wind <= 0.0 {
                self.erupt(id, e);
            }
            return;
        }
        if to.length() < 1.2 {
            // It is right under you: the ground heaves and a ring warns you.
            e.wind = 0.7;
            let at = self.pos(id);
            self.ev(Ev::Ring {
                at,
                c: fx::rgb(0.5, 0.35, 0.2),
                r: 1.9,
                time: 0.7,
            });
            return;
        }
        let spd = self.a(id).speed * 1.35;
        self.drive(id, to.normalize() * spd as f32, dt);
    }

    fn erupt(&mut self, id: Id, e: &mut Enemy) {
        e.under = false;
        e.heave = 0.0;
        {
            let mut a = self.am(id);
            a.invuln = 0.0;
            a.solid = true;
        }
        e.rise_t = 0.22;
        e.rise_from = -1.4;
        e.surface_t = 4.5;
        e.cool = 1.0;
        let at = self.pos(id);
        self.burst(
            at + Vec3::Y * 0.2,
            fx::rgb(0.3, 0.22, 0.15),
            12,
            5.0,
            0.14,
            None,
        );
        self.shake(0.3);
        let dmg = e.cfg().dmg * self.damage_scale();
        let el = e.attack_el();
        let scale = attack_scale(el, dmg);
        for w in self.wizards.clone() {
            let to = flat(self.pos(w) - at);
            let (alive, wr) = {
                let a = self.a(w);
                (a.alive, a.radius)
            };
            if alive && to.length() < 1.9 + wr {
                let dir = if to.length() > 0.1 {
                    to.normalize()
                } else {
                    Vec3::NEG_Z
                };
                self.am(w).melee_hit = true;
                self.hit(w, &[(el, 1.0)], scale, Some(id), dir, 1.0);
                let mut a = self.am(w);
                a.melee_hit = false;
                a.stun = a.stun.max(0.45);
            }
        }
    }

    /// A necromancer raises two skeletons out of the ground near it now
    /// and then, never more than four at once.
    fn raise_dead(&mut self, id: Id, e: &mut Enemy, dt: f64) {
        e.raise_t -= dt;
        let Some(t) = e.target.filter(|&t| self.ecs.contains(t)) else {
            return;
        };
        if e.raise_t > 0.0 || !self.a(id).can_act() || self.pos(t).distance(self.pos(id)) > 18.0 {
            return;
        }
        e.raise_t = self.rng.range(7.0, 9.0) as f64;
        let mine = self
            .enemies
            .iter()
            .filter(|&&o| {
                self.ecs
                    .get::<&Enemy>(o)
                    .is_ok_and(|x| x.master == Some(id))
                    && self.alive(o)
            })
            .count();
        e.wind = 0.0;
        let c = e.cfg();
        let (pos, max_hp) = {
            let a = self.a(id);
            (a.pos, a.max_hp)
        };
        for _ in 0..2usize.min(4usize.saturating_sub(mine)) {
            let a = self.rng.f() * TAU;
            let r = self.rng.range(1.8, 3.0);
            let at = self
                .level
                .clamp_inside(pos + Vec3::new(a.cos(), 0.0, a.sin()) * r);
            let k = kind(c.raise.unwrap());
            let o = self.add_enemy(k, at, max_hp / c.hp * 0.8, false, -1);
            self.am(o).stun = 0.7;
            let mut oe = self.enemy_mut(o);
            oe.master = Some(id);
            oe.rise_t = 0.75;
            oe.rise_from = -1.6;
            drop(oe);
            self.burst(
                at + Vec3::Y * 0.3,
                fx::rgb(0.45, 1.0, 0.5),
                14,
                2.5,
                0.2,
                None,
            );
            self.ev(Ev::Ring {
                at,
                c: fx::rgb(0.45, 1.0, 0.5),
                r: 1.2,
                time: 0.5,
            });
        }
    }

    /// The nearest living wizard. With every wizard down, the nearest
    /// grave: the monsters gather round it and beat on it instead.
    fn pick_target(&self, id: Id, current: Option<Id>) -> Option<Id> {
        let pos = self.pos(id);
        let anyone = self.any_wizard_alive();
        let mut best = None;
        let mut bd = f32::INFINITY;
        for &w in &self.wizards {
            if anyone && !self.alive(w) {
                continue;
            }
            let mut d = pos.distance(self.pos(w));
            if Some(w) == current {
                d -= 2.0;
            }
            if d < bd {
                bd = d;
                best = Some(w);
            }
        }
        best
    }

    fn separation(&self, id: Id) -> Vec3 {
        let (pos, r) = {
            let a = self.a(id);
            (a.pos, a.radius)
        };
        let mut out = Vec3::ZERO;
        for &o in &self.enemies {
            if o == id {
                continue;
            }
            let Some(oa) = self.actor(o) else { continue };
            if !oa.alive {
                continue;
            }
            let d = flat(pos - oa.pos);
            let l = d.length();
            let want = r + oa.radius + 0.3;
            if l < want && l > 0.01 {
                out += d / l * (want - l) / want;
            }
        }
        out
    }

    /// Monsters walk straight at their prey, which leaves one stuck in a
    /// corner now and then. One that has not got any closer for a while,
    /// or that is far off, follows the level's path instead.
    #[allow(clippy::too_many_arguments)]
    fn unstick(
        &self,
        id: Id,
        e: &mut Enemy,
        t: Id,
        want: Vec3,
        d: f32,
        reach: f32,
        dt: f64,
    ) -> Vec3 {
        if want.length() < 0.1 || d <= reach * 1.5 {
            e.best_d = f32::INFINITY;
            e.stuck_t = 0.0;
            return want;
        }
        if d < e.best_d - 0.3 {
            e.best_d = d;
            e.stuck_t = 0.0;
        } else {
            e.stuck_t += dt;
        }
        if e.stuck_t > 1.5 {
            e.detour_t = 3.0;
            e.stuck_t = 0.0;
        }
        e.detour_t = (e.detour_t - dt).max(0.0);
        if e.detour_t <= 0.0 && d < 25.0 {
            return want;
        }
        if e.detour_t <= 0.0 {
            e.best_d = f32::INFINITY;
        }
        let p = self.pos(id);
        let tp = self.pos(t);
        let mine = self.level.progress_of(Vec2::new(p.x, p.z));
        let theirs = self.level.progress_of(Vec2::new(tp.x, tp.z));
        if (theirs - mine).abs() < 3.0 {
            return want;
        }
        let q = self.level.point_at(mine + (theirs - mine).signum() * 4.0);
        let to = flat(Vec3::new(q.x, 0.0, q.y) - p);
        if to.length() > 0.2 {
            to.normalize() * want.length()
        } else {
            want
        }
    }

    /// A monster fell (Enemy.die and Game.on_enemy_died).
    pub fn enemy_died(&mut self, id: Id) {
        let (pos, max_hp, how) = {
            let mut a = self.am(id);
            a.solid = false;
            a.blocks = false;
            (a.pos, a.max_hp, a.death_style())
        };
        if self.run.boss == Some(id) {
            self.run.boss_fell = true;
        }
        let (c, elite, model) = {
            let e = self.enemy(id);
            (e.cfg(), e.affix(), e.model())
        };
        if c.raise.is_some() {
            for o in self.enemies.clone() {
                let raised = self
                    .ecs
                    .get::<&Enemy>(o)
                    .is_ok_and(|x| x.master == Some(id));
                if raised && self.alive(o) {
                    let hp = self.a(o).hp;
                    self.take(o, hp + 1.0);
                }
            }
        }
        self.run.kills += 1;
        self.count("kills");
        self.enemies.retain(|&o| o != id);
        // Health only comes from blood (no spell heals): bosses spill a lot,
        // elites always some, the rest fairly often.
        if c.boss {
            self.drop_globes(pos, 6);
        } else if elite.is_some() {
            let n = if self.rng.f() < 0.5 { 2 } else { 1 };
            self.drop_globes(pos, n);
        } else if self.rng.f() < 0.34 {
            self.drop_globes(pos, 1);
        }
        self.drop_souls(pos, max_hp);
        if c.burst {
            let n = NovaN {
                radius: 2.6,
                rate: attack_scale(El::Mud, 18.0),
            };
            self.burst_nova(id, El::Mud, n, pos, Some(WIZARDS));
            self.add_field(pos, El::Mud, 2.0, 7.0, None);
        }
        if let Some(el) = elite.and_then(|x| x.el) {
            let n = NovaN {
                radius: 2.8,
                rate: attack_scale(el, 14.0),
            };
            self.burst_nova(id, el, n, pos, Some(WIZARDS));
        }
        // How the body goes (Enemy._death).
        let big = if c.model.is_some() { c.size } else { 1.0 };
        let mut gone = 4.0;
        let mut fuse = None;
        match model {
            "skeleton" => gone = 6.0,
            "frost_brute" => {
                self.burst(pos + Vec3::Y, fx::el_c(El::Ice), 20, 6.0, 0.25, None);
                self.ev(Ev::Ring {
                    at: pos,
                    c: fx::el_c(El::Frost),
                    r: 3.0 * big,
                    time: 0.4,
                });
                self.shake(0.5);
            }
            "imp" => fuse = Some((El::Fire, 0.35, 2.4, 12.0, 1.0)),
            "sprite" => fuse = Some((El::Lightning, 0.2, 2.2, 8.0, 0.0)),
            "watcher" => fuse = Some((El::Void, 0.5, 2.6, 12.0, 0.0)),
            "demon" => {
                fuse = Some(if c.boss {
                    (El::Fire, 1.4, 5.0, 30.0, 3.0)
                } else {
                    (El::Fire, 0.8, 3.2, 20.0, 2.0)
                });
            }
            "necromancer" => {
                self.burst(
                    pos + Vec3::Y * 1.3,
                    fx::rgb(0.45, 1.0, 0.5),
                    24,
                    3.0,
                    0.2,
                    None,
                );
                self.ev(Ev::Puff {
                    at: pos + Vec3::Y,
                    c: fx::rgba(0.3, 0.5, 0.3, 0.5),
                    size: 1.4,
                });
                gone = 3.5;
            }
            "cultist" | "hellhound" => gone = 3.6,
            "wraith" => {
                self.ev(Ev::Puff {
                    at: pos + Vec3::Y,
                    c: fx::rgba(0.55, 0.75, 0.9, 0.6),
                    size: 1.4 * big,
                });
                gone = 3.0;
            }
            "drowned" | "toad" => {
                self.burst(
                    pos + Vec3::Y * 0.5,
                    fx::el_c(El::Water),
                    18,
                    4.0,
                    0.25,
                    None,
                );
                self.add_field(pos, El::Water, 1.6 * big as f64, 6.0, None);
                gone = 3.5;
            }
            "bile" | "ghoul" | "lurker" => {}
            // Brutes topple like felled trees and shake the floor.
            _ => self.later(0.5, crate::world::Later::Shake(0.7)),
        }
        {
            let mut e = self.enemy_mut(id);
            e.wind = 0.0;
            e.pending.clear();
            e.how = Some(how);
            if let Some((el, t, r, dmg, field)) = fuse {
                e.fuse = t;
                gone = t + 4.0;
                drop(e);
                self.later(
                    t,
                    crate::world::Later::BlowUp {
                        id,
                        el,
                        r,
                        dmg,
                        field,
                    },
                );
                e = self.enemy_mut(id);
            }
            e.gone = gone;
        }
        if c.boss {
            self.shake(1.2);
        }
        self.corpses.push(id);
        let kind = c.key;
        self.ev(Ev::MonsterDied {
            id: id.to_bits().get(),
            kind,
            how,
        });
    }

    /// The fuse is out: a burst of `el` on everyone within `r`, monsters
    /// too, and a field of `field` radius left behind (Enemy._blow_up).
    pub fn blow_up(&mut self, id: Id, el: El, r: f64, dmg: f64, field: f64) {
        if !self.ecs.contains(id) {
            return;
        }
        let at = self.pos(id);
        let n = NovaN {
            radius: r,
            rate: attack_scale(el, dmg * self.damage_scale()),
        };
        self.burst_nova(id, el, n, at, None);
        self.shake(0.4 + r as f32 * 0.1);
        if field > 0.0 {
            self.add_field(at, el, field, 5.0, None);
        }
        let c = fx::darken(fx::el_c(el), 0.5);
        self.burst(at + Vec3::Y * 0.8, c, 6, 6.0, 0.08, None);
    }

    /// The dead lie a while for their death to play, then go.
    pub fn corpse_step(&mut self, dt: f64) {
        for id in self.corpses.clone() {
            let done = match self.ecs.get::<&mut Enemy>(id) {
                Ok(mut e) => {
                    e.gone -= dt;
                    e.fuse_t += dt;
                    e.rise_t = (e.rise_t - dt).max(0.0);
                    e.gone <= 0.0
                }
                Err(_) => true,
            };
            if done {
                let _ = self.ecs.despawn(id);
            }
        }
        self.corpses.retain(|&c| self.ecs.contains(c));
    }
}

/// The swell of a monster about to burst, 0 to 1.
pub fn fuse_k(e: &Enemy) -> f32 {
    if e.fuse <= 0.0 {
        0.0
    } else {
        (e.fuse_t / e.fuse).min(1.0) as f32
    }
}
