//! The spell rules in Luau (`games/spellstorm/mods`, hosted by
//! engine/crates/script): which spell a queue and a form make, what each
//! element does on a hit, the tomes on the shrines, and hooks into casting
//! and hitting. They hot-reload while the game runs.
//!
//! A module returns a table; these keys mean something (later modules in
//! path order win, so `mods/live/x.luau` can override `mods/base`):
//! - `compose(slots, form, mods) -> Spell`: the grammar (mods/base/spells.luau);
//! - `effects`: element name -> { dmg, poison, push, burn, chill, wet, shock, slow };
//! - `tomes`: the shrines' tomes (mods/base/tomes.luau), replacing the list;
//! - `cast(g, e)`, `hit(g, e)`, `tick(g)`: hooks the sim calls inline.
//!
//! Without installed rules (the sim's own tests) the built-in Rust grammar,
//! effects and tomes stand in; the Luau base files are their exact port and
//! a test keeps them so.
use crate::el::{self, Effect, El, Kind};
use crate::grammar::{Form, Numbers, Shape, Spell, FORMS};
use crate::relics::{Mod, Mods, Tome};
use crate::world::{Id, World};
use glam::Vec3;
use script::mlua::{self, Function, Lua, Table, Value, Vector};
use script::{Host, Limits};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The base modules, for a binary run away from the repository.
pub const BUILTIN: &[(&str, &str)] = &[("base/spells.luau", include_str!("../../mods/base/spells.luau")), ("base/tomes.luau", include_str!("../../mods/base/tomes.luau"))];

/// Methods on `g`, the live world during a hook: name, Luau signature, what
/// it does (README.md lists them).
pub const API: &[(&str, &str, &str)] = &[
    ("time", "(self): number", "Seconds since the level began."),
    ("random", "(self): number", "Uniform in [0, 1), from the run's seed: the same every replay."),
    ("wizards", "(self): {number}", "Every wizard's id, in seat order."),
    ("enemies", "(self): {number}", "Every living monster's id."),
    ("near", "(self, at: vector, r: number): {number}", "Living monsters within r metres of `at` (on the floor plane)."),
    ("pos", "(self, id: number): vector", "Where an actor stands."),
    ("aim", "(self, id: number): vector", "The way a wizard aims (flat, unit)."),
    ("health", "(self, id: number): number", "An actor's health."),
    ("max_health", "(self, id: number): number", "An actor's max health."),
    ("alive", "(self, id: number): boolean", "The actor lives."),
    ("is_wizard", "(self, id: number): boolean", "The actor is a wizard (else a monster)."),
    ("hurt", "(self, id: number, amount: number, element: string?, by: number?)", "Hit an actor with `amount` units of an element (default earth), as a spell from `by` does."),
    ("nova", "(self, at: vector, radius: number, element: string, power: number?, by: number?)", "A burst of one element at `at` (power 1: a plain nova's hit)."),
    ("burst", "(self, at: vector, color: vector, n: number?, speed: number?, size: number?)", "Sparks flying out (looks only)."),
    ("shake", "(self, amount: number)", "Shake the camera."),
    ("mana", "(self, id: number): number", "A wizard's mana."),
    ("set_mana", "(self, id: number, mana: number)", "Set a wizard's mana (up to their max)."),
];

/// Hook keys a module can export, with their Luau signatures.
pub const HOOKS: &[(&str, &str, &str)] = &[
    ("cast", "(g: Game, e: Cast) -> (boolean | Spell)?", "A wizard cast (mana paid; wind-up spells when let go). Return false to skip the built-in effect (do your own), a Spell table to cast that instead, nil to keep it."),
    ("hit", "(g: Game, e: Hit) -> number?", "Before a spell lands on an actor: return the new strength (nil keeps it). Modules chain."),
    ("tick", "(g: Game) -> ()", "End of every sim tick (60 a second). Keep it cheap."),
];

struct Rules {
    host: Host,
    seen: u64,
    /// The module whose `compose` is used.
    compose: Option<(usize, Function)>,
    cache: HashMap<String, Option<Spell>>,
    recent: Vec<String>,
}

static RULES: Mutex<Option<Rules>> = Mutex::new(None);
/// Rules are installed; and which hooks some module exports.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static HIT: AtomicBool = AtomicBool::new(false);
static CAST: AtomicBool = AtomicBool::new(false);
static TICK: AtomicBool = AtomicBool::new(false);

/// The mods folder: SPELLSTORM_MODS, else `games/spellstorm/mods` above the
/// working directory or the executable, else `mods` beside the executable.
pub fn find_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("SPELLSTORM_MODS") {
        return Some(d.into());
    }
    let exe = std::env::current_exe().ok();
    for start in [std::env::current_dir().ok(), exe.clone()].into_iter().flatten() {
        for dir in start.ancestors() {
            let d = dir.join("games/spellstorm/mods");
            if d.join("base/spells.luau").is_file() {
                return Some(d);
            }
        }
    }
    let beside = exe?.parent()?.join("mods");
    beside.join("base/spells.luau").is_file().then_some(beside)
}

/// Load the rules from `dir` (watched from then on), or the built-in base
/// modules. Replaces any rules installed before.
pub fn install(dir: Option<&Path>) -> Result<Vec<String>, String> {
    let mut host = Host::new(Limits::default(), api)?;
    match dir {
        Some(d) => {
            host.watch(d);
        }
        None => {
            for (name, src) in BUILTIN {
                host.add_source(name, src)?;
            }
        }
    }
    let mut r = Rules { host, seen: u64::MAX, compose: None, cache: HashMap::new(), recent: vec![] };
    let lines = r.refresh();
    *RULES.lock().unwrap() = Some(r);
    ACTIVE.store(true, Ordering::Relaxed);
    Ok(lines)
}

/// Back to the built-in Rust rules.
pub fn uninstall() {
    *RULES.lock().unwrap() = None;
    ACTIVE.store(false, Ordering::Relaxed);
    for f in [&HIT, &CAST, &TICK] {
        f.store(false, Ordering::Relaxed);
    }
    el::set_effects(el::EFFECT);
    crate::relics::set_tomes(None);
}

pub fn installed() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Pick up edited files; returns what loaded and any errors.
pub fn poll() -> Vec<String> {
    let mut g = RULES.lock().unwrap();
    let Some(r) = g.as_mut() else { return vec![] };
    r.host.poll();
    r.refresh()
}

/// The loaded modules, their errors and the latest log lines.
pub fn status() -> String {
    let g = RULES.lock().unwrap();
    let Some(r) = g.as_ref() else { return "no Luau rules: the built-in Rust ones".into() };
    let mut out = vec![];
    for m in r.host.modules() {
        let state = if m.broken { "hooks off after an error" } else if m.exports.is_none() { "not loaded" } else { "ok" };
        out.push(format!("{}: {state}{}", m.name, m.error.as_ref().map(|e| format!(" | {e}")).unwrap_or_default()));
    }
    out.extend(r.recent.iter().rev().take(8).rev().map(|l| format!("  {l}")));
    out.join("\n")
}

impl Rules {
    /// Re-read the data after a (re)load; the log lines since the last call.
    fn refresh(&mut self) -> Vec<String> {
        if self.host.generation() != self.seen {
            self.seen = self.host.generation();
            self.cache.clear();
            self.compose = None;
            let mut effects = el::EFFECT;
            let mut tomes: Option<Vec<Tome>> = None;
            let mut errors = vec![];
            for (i, m) in self.host.modules().iter().enumerate() {
                let Some(x) = m.exports.as_ref() else { continue };
                if let Ok(Some(f)) = x.raw_get::<Option<Function>>("compose") {
                    self.compose = Some((i, f));
                }
                if let Ok(Some(t)) = x.raw_get::<Option<Table>>("effects") {
                    if let Err(e) = read_effects(&t, &mut effects) {
                        errors.push(format!("{}: effects: {e}", m.name));
                    }
                }
                if let Ok(Some(t)) = x.raw_get::<Option<Table>>("tomes") {
                    match read_tomes(&t) {
                        Ok(list) if !list.is_empty() => tomes = Some(list),
                        Ok(_) => errors.push(format!("{}: tomes: the list is empty (keeping the one before)", m.name)),
                        Err(e) => errors.push(format!("{}: tomes: {e}", m.name)),
                    }
                }
            }
            el::set_effects(effects);
            if tomes.is_some() {
                crate::relics::set_tomes(tomes);
            }
            HIT.store(self.host.has("hit"), Ordering::Relaxed);
            CAST.store(self.host.has("cast"), Ordering::Relaxed);
            TICK.store(self.host.has("tick"), Ordering::Relaxed);
            for e in errors {
                self.say(e);
            }
        }
        let lines = self.host.drain_log();
        for l in &lines {
            eprintln!("mods: {l}");
        }
        self.recent.extend(lines.iter().cloned());
        let n = self.recent.len();
        if n > 40 {
            self.recent.drain(..n - 40);
        }
        lines
    }

    fn say(&mut self, line: String) {
        eprintln!("mods: {line}");
        self.recent.push(line);
    }

    /// The Luau grammar's spell, its rate not yet fitted; None when it
    /// failed (logged) or there is none.
    fn raw(&mut self, slots: &[El], form: Form, mods: &Mods) -> Option<(Spell, bool)> {
        let (m, f) = self.compose.clone()?;
        let r = self.host.run(m, "compose", |lua| {
            let list = lua.create_sequence_from(slots.iter().map(|e| key(*e)))?;
            let t: Table = f.call((list, form.name(), mods_table(lua, mods)?))?;
            spell_from(&t, slots, form)
        });
        if r.is_none() {
            self.refresh();
        }
        r
    }
}

/// The spell `slots` and `form` make under the Luau rules, fitted like the
/// built-in grammar; None without rules (or inside a hook), when the
/// built-in grammar answers.
pub(crate) fn compose(slots: &[El], form: Form, mods: &Mods) -> Option<Spell> {
    if !installed() {
        return None;
    }
    let mut g = RULES.try_lock().ok()?;
    let r = g.as_mut()?;
    if r.host.generation() != r.seen {
        r.refresh();
    }
    let k = format!("{slots:?}{form:?}{mods:?}");
    if let Some(s) = r.cache.get(&k) {
        return s.clone();
    }
    let out = r.raw(slots, form, mods).map(|(mut s, fit)| {
        if fit && s.n.has_rate && crate::balance::measured(&s) {
            // The factor comes from the spell without tomes, so a tome
            // still makes you stronger (grammar::numbers).
            if let Some((mut base, _)) = r.raw(slots, form, &Mods::default()) {
                s.n.rate *= crate::balance::fit(&mut base);
            }
        }
        s
    });
    r.cache.insert(k, out.clone());
    out
}

/// What the `cast` hooks made of a cast.
pub(crate) enum Cast {
    Keep,
    Skip,
    Instead(Box<Spell>),
}

/// `seat` and `aim` are the caster's (whose Wizard is out of the world
/// while it acts).
pub(crate) fn cast(w: &mut World, caster: Id, seat: usize, aim: Vec3, s: &Spell) -> Cast {
    if !CAST.load(Ordering::Relaxed) {
        return Cast::Keep;
    }
    let Ok(mut g) = RULES.try_lock() else { return Cast::Keep };
    let Some(r) = g.as_mut() else { return Cast::Keep };
    let mut out = Cast::Keep;
    for (m, f) in r.host.each("cast") {
        let res = r.host.run(m, "cast", |lua| {
            let e = lua.create_table()?;
            e.raw_set("caster", id_num(caster))?;
            e.raw_set("seat", seat + 1)?;
            e.raw_set("pos", vector(w.pos(caster)))?;
            e.raw_set("aim", vector(aim))?;
            e.raw_set("spell", spell_table(lua, s)?)?;
            let v: Value = lua.scope(|sc| f.call((sc.create_any_userdata_ref_mut(&mut *w)?, e)))?;
            Ok(match v {
                Value::Boolean(false) => Cast::Skip,
                Value::Table(t) => Cast::Instead(Box::new(spell_from(&t, &s.slots, s.form)?.0)),
                _ => Cast::Keep,
            })
        });
        match res {
            Some(Cast::Keep) | None => {}
            Some(c) => {
                out = c;
                break;
            }
        }
    }
    r.refresh();
    out
}

/// The `hit` hooks' strength for a spell landing on `target`.
pub(crate) fn hit(w: &mut World, target: Id, src: Option<Id>, mix: &[(El, f64)], scale: f64) -> f64 {
    if !HIT.load(Ordering::Relaxed) {
        return scale;
    }
    let Ok(mut g) = RULES.try_lock() else { return scale };
    let Some(r) = g.as_mut() else { return scale };
    let mut scale = scale;
    for (m, f) in r.host.each("hit") {
        r.host.run(m, "hit", |lua| {
            let e = lua.create_table()?;
            e.raw_set("target", id_num(target))?;
            e.raw_set("by", src.map(id_num))?;
            e.raw_set("strength", scale)?;
            let els = lua.create_table()?;
            for &(el, n) in mix {
                els.raw_set(key(el), n)?;
            }
            e.raw_set("elements", els)?;
            let v: Option<f64> = lua.scope(|sc| f.call((sc.create_any_userdata_ref_mut(&mut *w)?, e)))?;
            if let Some(v) = v.filter(|v| v.is_finite()) {
                scale = v.clamp(0.0, 1e6);
            }
            Ok(())
        });
    }
    scale
}

pub(crate) fn tick(w: &mut World) {
    if !TICK.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut g) = RULES.try_lock() else { return };
    let Some(r) = g.as_mut() else { return };
    for (m, f) in r.host.each("tick") {
        r.host.run(m, "tick", |lua| lua.scope(|sc| f.call::<()>(sc.create_any_userdata_ref_mut(&mut *w)?)));
    }
    r.refresh();
}

/// A Luau line against the live world (`g`), for the console and the live
/// tools: `g:nova(g:pos(g:wizards()[1]), 4, "fire")`.
pub fn console(w: &mut World, line: &str) -> Result<String, String> {
    let mut g = RULES.lock().unwrap();
    let r = g.as_mut().ok_or("no Luau rules installed")?;
    let f = r.host.console_line(line)?;
    let console = r.host.console().clone();
    let lua = r.host.lua().clone();
    let out = lua.scope(|s| {
        console.raw_set("g", s.create_any_userdata_ref_mut(&mut *w)?)?;
        f.call::<mlua::MultiValue>(())
    });
    let _ = console.raw_set("g", Value::Nil);
    let out = out.map_err(|e| script::short(&e))?;
    let printed = r.host.drain_log();
    let shown: Vec<String> = out.iter().map(|v| v.to_string().unwrap_or_else(|_| v.type_name().into())).collect();
    Ok(printed.into_iter().chain(shown).collect::<Vec<_>>().join("\n"))
}

/// An element's name in Luau: "fire".
pub fn key(e: El) -> String {
    e.name().to_lowercase()
}

fn el_of(name: &str) -> mlua::Result<El> {
    el::ALL.iter().copied().find(|e| key(*e) == name).ok_or_else(|| mlua::Error::runtime(format!("no element called {name:?}")))
}

fn shape_of(name: &str) -> mlua::Result<Shape> {
    use Shape::*;
    [Staff, Stomp, Fizzle, Spray, Beam, Strike, Projectile, Nova, Trap, Sentry, Enchant, Overload]
        .into_iter()
        .find(|s| s.name() == name)
        .ok_or_else(|| mlua::Error::runtime(format!("no spell shape called {name:?}")))
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Solid => "solid",
        Kind::Energy => "energy",
        Kind::Spirit => "spirit",
        Kind::Flow => "flow",
    }
}

fn vector(v: Vec3) -> Vector {
    Vector::new(v.x, v.y, v.z)
}

fn v3(v: Vector) -> Vec3 {
    Vec3::new(v.x(), v.y(), v.z())
}

fn id_num(id: Id) -> f64 {
    id.to_bits().get() as f64
}

fn id_of(w: &World, n: f64) -> mlua::Result<Id> {
    hecs::Entity::from_bits(n as u64).filter(|id| w.ecs.contains(*id)).ok_or_else(|| mlua::Error::runtime(format!("no actor {n}")))
}

fn mods_table(lua: &Lua, m: &Mods) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let els = lua.create_table()?;
    for e in el::ALL {
        els.raw_set(key(e), m.el[e.idx()])?;
    }
    t.raw_set("el", els)?;
    for (k, v) in [("dmg", m.dmg), ("heal", m.heal), ("reach", m.reach), ("charge", m.charge), ("enchant_time", m.enchant_time), ("sword", m.sword), ("afflicted", m.afflicted), ("speed", m.speed), ("hp", m.hp), ("mana", m.mana), ("regen", m.regen), ("cost", m.cost), ("souls", m.souls), ("magnet", m.magnet), ("guard", m.guard), ("status", m.status), ("push", m.push), ("bolt", m.bolt), ("swing", m.swing)] {
        t.raw_set(k, v)?;
    }
    for (k, v) in [("echo", m.echo), ("planter", m.planter), ("firewalker", m.firewalker)] {
        t.raw_set(k, v)?;
    }
    Ok(t)
}

fn spell_table(lua: &Lua, s: &Spell) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("form", s.form.name())?;
    t.raw_set("shape", s.shape.name())?;
    t.raw_set("name", s.name.as_str())?;
    t.raw_set("key", s.key.as_str())?;
    t.raw_set("primary", s.primary.map(key))?;
    t.raw_set("slots", lua.create_sequence_from(s.slots.iter().map(|e| key(*e)))?)?;
    t.raw_set("payload", lua.create_sequence_from(s.payload.iter().map(|e| key(*e)))?)?;
    t.raw_set("riders", lua.create_sequence_from(s.riders.iter().map(|e| key(*e)))?)?;
    let n = lua.create_table()?;
    let x = &s.n;
    if x.has_rate {
        n.raw_set("rate", x.rate)?;
    }
    for (k, v) in [("range", x.range), ("angle", x.angle), ("duration", x.duration), ("width", x.width), ("radius", x.radius), ("spread", x.spread), ("speed", x.speed), ("charge_time", x.charge_time), ("splash", x.splash), ("field", x.field), ("field_time", x.field_time), ("stun", x.stun), ("interval", x.interval)] {
        if v != 0.0 {
            n.raw_set(k, v)?;
        }
    }
    if x.pieces != 0 {
        n.raw_set("pieces", x.pieces)?;
    }
    if x.pierce {
        n.raw_set("pierce", true)?;
    }
    t.raw_set("n", n)?;
    Ok(t)
}

/// A Spell from Luau's table, and whether its rate is to be fitted. The
/// slots are the queue's unless the table names its own.
fn spell_from(t: &Table, slots: &[El], form: Form) -> mlua::Result<(Spell, bool)> {
    let slots: Vec<El> = match t.raw_get::<Option<Vec<String>>>("slots")? {
        Some(v) => v.iter().map(|s| el_of(s)).collect::<mlua::Result<_>>()?,
        None => slots.to_vec(),
    };
    let form = match t.raw_get::<Option<String>>("form")? {
        Some(f) => FORMS.into_iter().find(|x| x.name() == f).ok_or_else(|| mlua::Error::runtime(format!("no form called {f:?}")))?,
        None => form,
    };
    let list = |k: &str| -> mlua::Result<Vec<El>> { t.raw_get::<Option<Vec<String>>>(k)?.unwrap_or_default().iter().map(|s| el_of(s)).collect() };
    let mut counts: Vec<(El, u8)> = Vec::new();
    for &e in &slots {
        match counts.iter_mut().find(|c| c.0 == e) {
            Some(c) => c.1 += 1,
            None => counts.push((e, 1)),
        }
    }
    let shape = shape_of(&t.raw_get::<String>("shape")?)?;
    let n: Option<Table> = t.raw_get("n")?;
    let get = |k: &str| -> mlua::Result<Value> { n.as_ref().map_or(Ok(Value::Nil), |n| n.raw_get(k)) };
    let num = |k: &str| -> mlua::Result<f64> { Ok(get(k)?.as_number().or(get(k)?.as_integer().map(|i| i as f64)).unwrap_or(0.0)) };
    let rate = match get("rate")? {
        Value::Nil => None,
        _ => Some(num("rate")?),
    };
    let numbers = Numbers {
        has_rate: rate.is_some(),
        rate: rate.unwrap_or(0.0),
        range: num("range")?,
        angle: num("angle")?,
        duration: num("duration")?,
        width: num("width")?,
        radius: num("radius")?,
        pieces: num("pieces")?.round() as i32,
        spread: num("spread")?,
        speed: num("speed")?,
        charge_time: num("charge_time")?,
        splash: num("splash")?,
        pierce: get("pierce")?.as_boolean().unwrap_or(false),
        field: num("field")?,
        field_time: num("field_time")?,
        stun: num("stun")?,
        interval: num("interval")?,
    };
    let fit = get("fit")?.as_boolean().unwrap_or(true);
    let spell = Spell {
        form,
        power: slots.len(),
        counts,
        primary: t.raw_get::<Option<String>>("primary")?.map(|p| el_of(&p)).transpose()?,
        shape,
        name: t.raw_get::<Option<String>>("name")?.unwrap_or_default(),
        payload: list("payload")?,
        riders: list("riders")?,
        key: t.raw_get::<Option<String>>("key")?.unwrap_or_default(),
        n: numbers,
        slots,
    };
    Ok((spell, fit))
}

fn read_effects(t: &Table, out: &mut [Effect; 13]) -> mlua::Result<()> {
    for pair in t.pairs::<String, Table>() {
        let (name, f) = pair?;
        let e = el_of(&name)?;
        let num = |k: &str| -> mlua::Result<f64> { Ok(f.raw_get::<Option<f64>>(k)?.unwrap_or(0.0)) };
        out[e.idx()] = Effect { dmg: num("dmg")?, poison: num("poison")?, push: num("push")?, burn: num("burn")?, chill: num("chill")?, wet: num("wet")?, shock: num("shock")?, slow: num("slow")? };
    }
    Ok(())
}

fn read_tomes(t: &Table) -> mlua::Result<Vec<Tome>> {
    let mut out = vec![];
    for (i, v) in t.sequence_values::<Table>().enumerate() {
        let v = v?;
        let name: String = v.raw_get::<Option<String>>("name")?.ok_or_else(|| mlua::Error::runtime(format!("tome {} has no name", i + 1)))?;
        let text: String = v.raw_get::<Option<String>>("text")?.unwrap_or_default();
        let mut mods = vec![];
        if let Some(m) = v.raw_get::<Option<Table>>("mods")? {
            for pair in m.pairs::<String, Value>() {
                let (k, val) = pair?;
                let f = || val.as_number().or(val.as_integer().map(|i| i as f64)).ok_or_else(|| mlua::Error::runtime(format!("{name}: {k} must be a number")));
                mods.push(match k.as_str() {
                    "charge" => Mod::Charge(f()?),
                    "reach" => Mod::Reach(f()?),
                    "enchant_time" => Mod::EnchantTime(f()?),
                    "sword" => Mod::Sword(f()?),
                    "hp" => Mod::Hp(f()?),
                    "dmg" => Mod::Dmg(f()?),
                    "speed" => Mod::Speed(f()?),
                    "heal" => Mod::Heal(f()?),
                    "afflicted" => Mod::Afflicted(f()?),
                    "mana" => Mod::Mana(f()?),
                    "regen" => Mod::Regen(f()?),
                    "cost" => Mod::Cost(f()?),
                    "souls" => Mod::Souls(f()?),
                    "magnet" => Mod::Magnet(f()?),
                    "guard" => Mod::Guard(f()?),
                    "status" => Mod::Status(f()?),
                    "push" => Mod::Push(f()?),
                    "bolt" => Mod::Bolt(f()?),
                    "swing" => Mod::Swing(f()?),
                    "echo" if val.as_boolean() == Some(true) => Mod::Echo,
                    "planter" if val.as_boolean() == Some(true) => Mod::Planter,
                    "firewalker" if val.as_boolean() == Some(true) => Mod::Firewalker,
                    "echo" | "planter" | "firewalker" => continue,
                    other => Mod::El(el_of(other).map_err(|_| mlua::Error::runtime(format!("{name}: no tome mod called {other}")))?, f()?),
                });
            }
        }
        // Table order is not fixed: keep the mods in one order, so the same
        // file always gives the same tome.
        mods.sort_by_key(|m| format!("{m:?}"));
        out.push(Tome { name, text, mods });
    }
    Ok(out)
}

/// The names every module sees: the elements, the queue's size, and `g`'s
/// methods on the borrowed world.
fn api(lua: &Lua, base: &Table) -> mlua::Result<()> {
    let els = lua.create_table()?;
    for e in el::ALL {
        let t = lua.create_table()?;
        t.raw_set("index", e.idx())?;
        t.raw_set("name", e.name())?;
        t.raw_set("kind", kind_name(e.kind()))?;
        t.raw_set("parts", lua.create_sequence_from(e.parts().into_iter().map(key))?)?;
        let c = e.color();
        t.raw_set("color", Vector::new(c[0], c[1], c[2]))?;
        t.set_readonly(true);
        els.raw_set(key(e), t)?;
    }
    els.set_readonly(true);
    base.raw_set("ELEMENTS", els)?;
    base.raw_set("QUEUE_SIZE", crate::queue::SIZE)?;
    base.raw_set("ENCHANT_TIME", crate::balance::ENCHANT_TIME)?;
    lua.register_userdata_type::<World>(|r| {
        use script::mlua::UserDataMethods;
        r.add_method("time", |_, w, ()| Ok(w.time));
        r.add_method_mut("random", |_, w, ()| Ok(w.rng.randf() as f64));
        r.add_method("wizards", |_, w, ()| Ok(w.wizards.iter().map(|&id| id_num(id)).collect::<Vec<_>>()));
        r.add_method("enemies", |_, w, ()| Ok(w.enemies.iter().filter(|&&id| w.alive(id)).map(|&id| id_num(id)).collect::<Vec<_>>()));
        r.add_method("near", |_, w, (at, rad): (Vector, f64)| {
            let at = v3(at);
            Ok(w.enemies.iter().filter(|&&id| w.alive(id) && crate::world::flat(w.pos(id) - at).length() as f64 <= rad).map(|&id| id_num(id)).collect::<Vec<_>>())
        });
        r.add_method("pos", |_, w, id: f64| Ok(vector(w.pos(id_of(w, id)?))));
        r.add_method("aim", |_, w, id: f64| {
            let id = id_of(w, id)?;
            let d = w.ecs.get::<&crate::wizard::Wizard>(id).map(|z| z.aim_dir).unwrap_or(Vec3::Z);
            Ok(vector(d))
        });
        r.add_method("health", |_, w, id: f64| Ok(w.actor(id_of(w, id)?).map(|a| a.hp).unwrap_or(0.0)));
        r.add_method("max_health", |_, w, id: f64| Ok(w.actor(id_of(w, id)?).map(|a| a.max_hp).unwrap_or(0.0)));
        r.add_method("alive", |_, w, id: f64| Ok(id_of(w, id).is_ok_and(|id| w.alive(id))));
        r.add_method("is_wizard", |_, w, id: f64| Ok(id_of(w, id).is_ok_and(|id| w.wizards.contains(&id))));
        r.add_method_mut("hurt", |_, w, (id, amount, element, by): (f64, f64, Option<String>, Option<f64>)| {
            let id = id_of(w, id)?;
            let e = element.as_deref().map(el_of).transpose()?.unwrap_or(El::Earth);
            let by = by.map(|b| id_of(w, b)).transpose()?;
            let from = by.map(|b| w.pos(b)).unwrap_or(w.pos(id));
            let dir = crate::world::flat(w.pos(id) - from).normalize_or_zero();
            w.hit(id, &[(e, 1.0)], amount.clamp(0.0, 1e6), by, dir, 1.0);
            w.flush_deaths();
            Ok(())
        });
        r.add_method_mut("nova", |_, w, (at, radius, element, power, by): (Vector, f64, String, Option<f64>, Option<f64>)| {
            let e = el_of(&element)?;
            let by = match by {
                Some(b) => id_of(w, b)?,
                None => *w.wizards.first().ok_or_else(|| mlua::Error::runtime("no wizard to cast it"))?,
            };
            let n = crate::spells::NovaN { radius: radius.clamp(0.1, 30.0), rate: power.unwrap_or(1.0).clamp(0.0, 100.0) * 1.25 };
            let team = if w.team(by) == crate::world::WIZARDS { crate::world::MONSTERS } else { crate::world::WIZARDS };
            w.burst_nova(by, e, n, v3(at), Some(team));
            w.flush_deaths();
            Ok(())
        });
        r.add_method_mut("burst", |_, w, (at, c, n, speed, size): (Vector, Vector, Option<f64>, Option<f64>, Option<f64>)| {
            w.burst(v3(at), [c.x(), c.y(), c.z(), 1.0], n.unwrap_or(16.0).clamp(1.0, 200.0) as u32, speed.unwrap_or(4.0) as f32, size.unwrap_or(0.2) as f32, None);
            Ok(())
        });
        r.add_method_mut("shake", |_, w, amount: f64| {
            w.shake(amount.clamp(0.0, 3.0) as f32);
            Ok(())
        });
        r.add_method("mana", |_, w, id: f64| Ok(w.ecs.get::<&crate::wizard::Wizard>(id_of(w, id)?).map(|z| z.mana).unwrap_or(0.0)));
        r.add_method_mut("set_mana", |_, w, (id, mana): (f64, f64)| {
            let id = id_of(w, id)?;
            if let Ok(mut z) = w.ecs.get::<&mut crate::wizard::Wizard>(id) {
                z.mana = mana.clamp(0.0, z.mana_max());
            }
            Ok(())
        });
    })?;
    Ok(())
}
