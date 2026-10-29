//! Tomes found on shrines (relics.gd). Each changes numbers in a way you can
//! say in one sentence; none changes what a spell is.

use crate::el::El;
use gdcompat::Pcg;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mod {
    El(El, f64),
    Charge(f64),
    Reach(f64),
    Echo,
    Planter,
    EnchantTime(f64),
    Sword(f64),
    Hp(f64),
    Dmg(f64),
    Speed(f64),
    Heal(f64),
    Firewalker,
    Afflicted(f64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relic {
    pub name: &'static str,
    pub text: &'static str,
    pub mods: &'static [Mod],
}

use El::*;
use Mod as M;

const fn r(name: &'static str, text: &'static str, mods: &'static [Mod]) -> Relic {
    Relic { name, text, mods }
}

pub const ALL: [Relic; 16] = [
    r(
        "Black Grimoire",
        "Poison and Void +30%",
        &[M::El(Poison, 1.3), M::El(Void, 1.3)],
    ),
    r(
        "Ember and Rime Codex",
        "Fire, Frost, Steam and Ice +35%",
        &[
            M::El(Fire, 1.35),
            M::El(Frost, 1.35),
            M::El(Steam, 1.35),
            M::El(Ice, 1.35),
        ],
    ),
    r(
        "Tide Scroll",
        "Water, Air, Mud and Storm +50%",
        &[
            M::El(Water, 1.5),
            M::El(Air, 1.5),
            M::El(Mud, 1.5),
            M::El(Storm, 1.5),
        ],
    ),
    r(
        "Storm Sigil",
        "Lightning, Earth and Magma +35%",
        &[
            M::El(Lightning, 1.35),
            M::El(Earth, 1.35),
            M::El(Magma, 1.35),
        ],
    ),
    r(
        "Quick Hands",
        "Aimed spells wind up 40% faster",
        &[M::Charge(0.6)],
    ),
    r(
        "Long Nozzle",
        "Sprays and beams reach 35% further",
        &[M::Reach(1.35)],
    ),
    r("Echo Stone", "Bursts around you go off twice", &[M::Echo]),
    r(
        "Mason's Chisel",
        "Keep 2 sentries and 5 traps out at once",
        &[M::Planter],
    ),
    r(
        "Runed Staff",
        "Enchantments last twice as long",
        &[M::EnchantTime(2.0)],
    ),
    r("Whetstone", "Sword hits 60% harder", &[M::Sword(1.6)]),
    r("Thick Hide", "+30 max health", &[M::Hp(30.0)]),
    r(
        "Glass Staff",
        "+15% to all your damage, -30 max health",
        &[M::Dmg(1.15), M::Hp(-30.0)],
    ),
    r("Zephyr Feather", "You move 20% faster", &[M::Speed(1.2)]),
    r(
        "Red Chalice",
        "Blood you drink heals 60% more",
        &[M::Heal(1.6)],
    ),
    r(
        "Pilgrim Boots",
        "Nothing on the ground can hurt or slow you",
        &[M::Firewalker],
    ),
    r(
        "Hunter's Mark",
        "+25% on wet, burning, chilled or poisoned monsters",
        &[M::Afflicted(1.25)],
    ),
];

/// A tome as the game uses it: the built-in ones, or the Luau rules'
/// (mods/base/tomes.luau).
#[derive(Clone, Debug, PartialEq)]
pub struct Tome {
    pub name: String,
    pub text: String,
    pub mods: Vec<Mod>,
}

static TOMES: std::sync::RwLock<Option<std::sync::Arc<Vec<Tome>>>> = std::sync::RwLock::new(None);

/// Every tome a shrine may offer, by index.
pub fn all() -> std::sync::Arc<Vec<Tome>> {
    if let Some(t) = TOMES.read().unwrap().as_ref() {
        return t.clone();
    }
    let built: std::sync::Arc<Vec<Tome>> = std::sync::Arc::new(ALL.iter().map(|r| Tome { name: r.name.into(), text: r.text.into(), mods: r.mods.to_vec() }).collect());
    *TOMES.write().unwrap() = Some(built.clone());
    built
}

/// The tomes from now on; None goes back to the built-in ones.
pub fn set_tomes(t: Option<Vec<Tome>>) {
    *TOMES.write().unwrap() = t.map(std::sync::Arc::new);
}

/// A wizard's tomes folded together: factors multiply, health adds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mods {
    pub el: [f64; 13],
    pub dmg: f64,
    pub heal: f64,
    pub reach: f64,
    pub charge: f64,
    pub enchant_time: f64,
    pub sword: f64,
    pub afflicted: f64,
    pub speed: f64,
    pub hp: f64,
    pub echo: bool,
    pub planter: bool,
    pub firewalker: bool,
}

impl Default for Mods {
    fn default() -> Mods {
        Mods {
            el: [1.0; 13],
            dmg: 1.0,
            heal: 1.0,
            reach: 1.0,
            charge: 1.0,
            enchant_time: 1.0,
            sword: 1.0,
            afflicted: 1.0,
            speed: 1.0,
            hp: 0.0,
            echo: false,
            planter: false,
            firewalker: false,
        }
    }
}

impl Mods {
    pub fn of(relics: &[usize]) -> Mods {
        let mut m = Mods::default();
        let all = all();
        for &i in relics {
            let Some(t) = all.get(i) else { continue };
            for md in &t.mods {
                match *md {
                    M::El(e, f) => m.el[e.idx()] *= f,
                    M::Charge(f) => m.charge *= f,
                    M::Reach(f) => m.reach *= f,
                    M::Echo => m.echo = true,
                    M::Planter => m.planter = true,
                    M::EnchantTime(f) => m.enchant_time *= f,
                    M::Sword(f) => m.sword *= f,
                    M::Hp(f) => m.hp += f,
                    M::Dmg(f) => m.dmg *= f,
                    M::Speed(f) => m.speed *= f,
                    M::Heal(f) => m.heal *= f,
                    M::Firewalker => m.firewalker = true,
                    M::Afflicted(f) => m.afflicted *= f,
                }
            }
        }
        m
    }
}

/// `n` different tomes, none of them in `avoid` (indices into ALL).
pub fn pick(n: usize, rng: &mut Pcg, avoid: &[usize]) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..all().len()).filter(|i| !avoid.contains(i)).collect();
    let mut out = Vec::new();
    while out.len() < n && !pool.is_empty() {
        let i = rng.next_u32() as usize % pool.len();
        out.push(pool.remove(i));
    }
    out
}
