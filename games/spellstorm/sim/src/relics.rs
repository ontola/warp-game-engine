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
    /// Max mana, added.
    Mana(f64),
    /// Mana regen, times.
    Regen(f64),
    /// What loading and casting cost, times.
    Cost(f64),
    /// Mana from souls, times.
    Souls(f64),
    /// How far souls fly to you, times.
    Magnet(f64),
    /// Damage you take, times.
    Guard(f64),
    /// How deep your burns, poisons and chills go, times.
    Status(f64),
    /// How far your spells knock monsters back, times.
    Push(f64),
    /// Bolt damage, times.
    Bolt(f64),
    /// The staff swing's recovery, times.
    Swing(f64),
    /// Share of the damage you deal that heals you, added.
    Leech(f64),
    /// Share of a melee blow that hits its striker back, added.
    Thorns(f64),
    /// Killing blows survived each level, added.
    Wind(f64),
    /// More monsters your lightning jumps to, added.
    Chain(f64),
    /// How far bursts around you reach, times.
    Nova(f64),
    /// How fast you raise a fallen friend, times.
    Raise(f64),
}

/// What a tome needs before it is any use to a wizard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Need {
    Nothing,
    /// The element, or for a fused one both its parts.
    El(El),
    Form(crate::grammar::Form),
}

impl Mod {
    pub fn need(self) -> Need {
        use crate::grammar::Form;
        match self {
            M::El(e, _) => Need::El(e),
            M::Echo => Need::Form(Form::Nova),
            M::Nova(_) => Need::Form(Form::Nova),
            M::Chain(_) => Need::El(Lightning),
            M::Planter => Need::Form(Form::Plant),
            M::EnchantTime(_) => Need::Form(Form::Enchant),
            _ => Need::Nothing,
        }
    }
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

pub const ALL: [Relic; 38] = [
    r("Black Grimoire", "Poison and Void +40%", &[M::El(Poison, 1.4), M::El(Void, 1.4)]),
    r("Ember and Rime Codex", "Fire, Frost, Steam and Ice +40%", &[M::El(Fire, 1.4), M::El(Frost, 1.4), M::El(Steam, 1.4), M::El(Ice, 1.4)]),
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
    r("Storm Sigil", "Lightning, Earth and Magma +40%", &[M::El(Lightning, 1.4), M::El(Earth, 1.4), M::El(Magma, 1.4)]),
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
    r("Whetstone", "Sword hits twice as hard", &[M::Sword(2.0)]),
    r("Thick Hide", "+30 max health", &[M::Hp(30.0)]),
    r("Glass Staff", "+30% to all your damage, -20 max health", &[M::Dmg(1.3), M::Hp(-20.0)]),
    r("Zephyr Feather", "You move 20% faster", &[M::Speed(1.2)]),
    r("Red Chalice", "Blood you drink heals twice as much", &[M::Heal(2.0)]),
    r(
        "Pilgrim Boots",
        "Nothing on the ground can hurt or slow you",
        &[M::Firewalker],
    ),
    r("Hunter's Mark", "+20% on wet, burning, chilled or poisoned monsters", &[M::Afflicted(1.2)]),
    r("Pyre Psalter", "Fire and Magma +50%", &[M::El(Fire, 1.5), M::El(Magma, 1.5)]),
    r("Frozen Heart", "Frost and Ice +50%", &[M::El(Frost, 1.5), M::El(Ice, 1.5)]),
    r("Thunder Rod", "Lightning and Storm +50%", &[M::El(Lightning, 1.5), M::El(Storm, 1.5)]),
    r("Tomb Dust", "Poison +60%", &[M::El(Poison, 1.6)]),
    r("Deep Well", "+40 max mana", &[M::Mana(40.0)]),
    r("Moon Font", "Mana comes back 50% faster", &[M::Regen(1.5)]),
    r("Miser's Ledger", "Loading and casting cost 20% less", &[M::Cost(0.8)]),
    r("Soul Jar", "Souls give 50% more mana", &[M::Souls(1.5)]),
    r("Lodestone", "Souls fly to you from afar and give 20% more", &[M::Magnet(3.0), M::Souls(1.2)]),
    r("Iron Skin", "You take 20% less damage", &[M::Guard(0.8)]),
    r("Plague Primer", "Your burns, poisons and chills go 50% deeper", &[M::Status(1.5)]),
    r("Ram's Horn", "Spells knock monsters back twice as far, -10% hurt", &[M::Push(2.0), M::Guard(0.9)]),
    r("Keen Gem", "Your bolts hit 80% harder", &[M::Bolt(1.8)]),
    r("Quicksilver Grip", "Swing recovers twice as fast, hits 30% harder", &[M::Swing(0.5), M::Sword(1.3)]),
    r("Blood Pact", "+40% to all your damage, but you take 20% more", &[M::Dmg(1.4), M::Guard(1.2)]),
    r("Wayfarer's Charm", "+20 max health and you move 15% faster", &[M::Hp(20.0), M::Speed(1.15)]),
    r("Vampire Fang", "Heal 3% of the damage you deal", &[M::Leech(0.03)]),
    r("Bramble Mail", "Monsters that strike you take half of it back", &[M::Thorns(0.5)]),
    r("Phoenix Feather", "Once a level a killing blow leaves you at 1 health", &[M::Wind(1.0)]),
    r("Forked Rod", "Your chain lightning jumps to one more monster", &[M::Chain(1.0)]),
    r("Wide Sigil", "Bursts around you reach 30% further", &[M::Nova(1.3)]),
    r("Shepherd's Crook", "You raise fallen friends twice as fast", &[M::Raise(2.0)]),
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
    pub mana: f64,
    pub regen: f64,
    pub cost: f64,
    pub souls: f64,
    pub magnet: f64,
    pub guard: f64,
    pub status: f64,
    pub push: f64,
    pub bolt: f64,
    pub swing: f64,
    pub leech: f64,
    pub thorns: f64,
    pub wind: f64,
    pub chain: f64,
    pub nova: f64,
    pub raise: f64,
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
            mana: 0.0,
            regen: 1.0,
            cost: 1.0,
            souls: 1.0,
            magnet: 1.0,
            guard: 1.0,
            status: 1.0,
            push: 1.0,
            bolt: 1.0,
            swing: 1.0,
            leech: 0.0,
            thorns: 0.0,
            wind: 0.0,
            chain: 0.0,
            nova: 1.0,
            raise: 1.0,
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
                    M::Mana(f) => m.mana += f,
                    M::Regen(f) => m.regen *= f,
                    M::Cost(f) => m.cost *= f,
                    M::Souls(f) => m.souls *= f,
                    M::Magnet(f) => m.magnet *= f,
                    M::Guard(f) => m.guard *= f,
                    M::Status(f) => m.status *= f,
                    M::Push(f) => m.push *= f,
                    M::Bolt(f) => m.bolt *= f,
                    M::Swing(f) => m.swing *= f,
                    M::Leech(f) => m.leech += f,
                    M::Thorns(f) => m.thorns += f,
                    M::Wind(f) => m.wind += f,
                    M::Chain(f) => m.chain += f,
                    M::Nova(f) => m.nova *= f,
                    M::Raise(f) => m.raise *= f,
                }
            }
        }
        // Tomes stack (Joep: you keep getting stronger), but the ones that
        // shrink a number stop short of nothing: no free spells, no
        // instant wind-ups, never untouchable.
        m.charge = m.charge.max(0.25);
        m.cost = m.cost.max(0.3);
        m.guard = m.guard.max(0.3);
        m.swing = m.swing.max(0.25);
        m.speed = m.speed.min(2.0);
        m.reach = m.reach.min(2.5);
        m.leech = m.leech.min(0.25);
        m.wind = m.wind.min(3.0);
        m.chain = m.chain.min(5.0);
        m.nova = m.nova.min(2.0);
        m.raise = m.raise.min(5.0);
        m
    }
}

/// May tome `i` be taken again? Every tome adds up but the ones that only
/// switch something on (Echo Stone, Mason's Chisel, Pilgrim Boots).
pub fn stacks(i: usize) -> bool {
    all().get(i).is_some_and(|t| !t.mods.iter().any(|m| matches!(m, M::Echo | M::Planter | M::Firewalker)))
}

/// Is tome `i` any use to someone who `has` what a need asks for? A tome
/// with several mods needs only one of them (the fire half of a fire and
/// frost tome is use enough).
pub fn usable(i: usize, has: &dyn Fn(Need) -> bool) -> bool {
    let all = all();
    let Some(t) = all.get(i) else { return false };
    t.mods.is_empty() || t.mods.iter().any(|m| has(m.need()))
}

/// `n` different tomes, none of them in `avoid` (indices into ALL), only
/// ones that are `usable`. When too few are, any others fill up.
pub fn pick(n: usize, rng: &mut Pcg, avoid: &[usize], has: &dyn Fn(Need) -> bool) -> Vec<usize> {
    let open: Vec<usize> = (0..all().len()).filter(|i| !avoid.contains(i)).collect();
    let (mut pool, mut rest): (Vec<usize>, Vec<usize>) = open.into_iter().partition(|&i| usable(i, has));
    let mut out = Vec::new();
    for pool in [&mut pool, &mut rest] {
        while out.len() < n && !pool.is_empty() {
            let i = rng.next_u32() as usize % pool.len();
            out.push(pool.remove(i));
        }
    }
    // Every tome already carried (Joep: two wizards held all 32 and the
    // shrines stood empty): repeats then, of the usable ones first.
    if out.is_empty() {
        let (mut pool, mut rest): (Vec<usize>, Vec<usize>) = (0..all().len()).partition(|&i| usable(i, has));
        for pool in [&mut pool, &mut rest] {
            while out.len() < n && !pool.is_empty() {
                let i = rng.next_u32() as usize % pool.len();
                out.push(pool.remove(i));
            }
        }
    }
    out
}
