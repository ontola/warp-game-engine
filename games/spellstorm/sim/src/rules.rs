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
//! - `monsters`, `biomes`: every monster kind and every level
//!   (mods/base/monsters.luau, biomes.luau), replacing the lists.
//! - `cast(g, e)`, `hit(g, e)`, `tick(g)`: hooks the sim calls inline.
//!
//! The base files are the only copy: without installed rules (the sim's own
//! tests, or inside a hook) the built-in files embedded in BUILTIN answer.
use crate::el::{self, Effect, El, Kind};
use crate::biome::Biome;
use crate::enemy::Kind as Monster;
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
pub const BUILTIN: &[(&str, &str)] = &[("base/biomes.luau", include_str!("../../mods/base/biomes.luau")), ("base/monsters.luau", include_str!("../../mods/base/monsters.luau")), ("base/spells.luau", include_str!("../../mods/base/spells.luau")), ("base/tomes.luau", include_str!("../../mods/base/tomes.luau"))];

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
    /// This host installed its monster and biome lists (later ones keep their keys).
    seen_lists: bool,
    /// The built-in BASE rules: refresh never installs anything from them.
    base: bool,
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
    let mut r = Rules { host, seen: u64::MAX, compose: None, cache: HashMap::new(), recent: vec![], seen_lists: false, base: false };
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
    el::set_effects(None);
    crate::relics::set_tomes(None);
    crate::enemy::set_kinds(None);
    crate::biome::set_biomes(None);
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
            let mut effects = builtin_effects();
            let mut tomes: Option<Vec<Tome>> = None;
            let mut monsters: Option<Vec<Monster>> = None;
            let mut biomes: Option<Vec<Biome>> = None;
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
                if let Ok(Some(t)) = x.raw_get::<Option<Table>>("biomes") {
                    match read_biomes(&t) {
                        Ok(list) => biomes = Some(list),
                        Err(e) => errors.push(format!("{}: biomes: {}", m.name, script::short(&e))),
                    }
                }
                if let Ok(Some(t)) = x.raw_get::<Option<Table>>("monsters") {
                    match read_monsters(&t) {
                        Ok(list) => monsters = Some(list),
                        Err(e) => errors.push(format!("{}: monsters: {}", m.name, script::short(&e))),
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
            el::set_effects(Some(effects));
            if tomes.is_some() {
                crate::relics::set_tomes(tomes);
            }
            // Monsters and levels alive hold indices into these lists, so
            // while the game runs only their values may change.
            let kinds_now = crate::enemy::kinds();
            let biomes_now = crate::biome::biomes();
            let kinds_same = monsters.as_ref().is_none_or(|l| l.len() + 1 == kinds_now.len() && l.iter().zip(kinds_now).all(|(a, b)| a.key == b.key));
            let biomes_same = biomes.as_ref().is_none_or(|l| l.len() == biomes_now.len() && l.iter().zip(biomes_now).all(|(a, b)| a.id == b.id));
            if !self.seen_lists || (kinds_same && biomes_same) {
                let with_none: Vec<Monster>;
                let kinds_next: &[Monster] = match &monsters {
                    Some(l) => { with_none = l.iter().cloned().chain([crate::enemy::none_kind()]).collect(); &with_none }
                    None => kinds_now,
                };
                match check_biomes(biomes.as_deref().unwrap_or(biomes_now), kinds_next) {
                    Ok(()) => {
                        if monsters.is_some() { crate::enemy::set_kinds(monsters); }
                        if biomes.is_some() { crate::biome::set_biomes(biomes); }
                        self.seen_lists = true;
                    }
                    Err(e) => errors.push(format!("{e} (keeping the monsters and biomes before)")),
                }
            } else {
                errors.push("monsters or biomes were added, removed or reordered: that takes effect at the next start (changed values still reload)".into());
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
        if r.is_none() && !self.base {
            self.refresh();
        }
        if r.is_none() && self.base {
            self.recent.extend(self.host.drain_log());
        }
        r
    }
}

/// The spell `slots` and `form` make, with its rate fitted
/// (crate::balance): the installed rules' grammar, else the built-in
/// mods/base/spells.luau. Inside a hook the installed host is busy, so the
/// built-in grammar answers there too.
pub(crate) fn compose(slots: &[El], form: Form, mods: &Mods) -> Spell {
    if installed() {
        if let Ok(mut g) = RULES.try_lock() {
            if let Some(s) = g.as_mut().and_then(|r| r.compose_fitted(slots, form, mods)) {
                return s;
            }
        }
    }
    let mut g = BASE.lock().unwrap();
    let r = g.get_or_insert_with(Rules::base);
    r.compose_fitted(slots, form, mods).unwrap_or_else(|| panic!("mods/base/spells.luau failed to compose {slots:?} {form:?}: {}", r.recent.join(" | ")))
}

/// The built-in rules (BUILTIN), never watched or installed: the grammar
/// when no rules are installed and inside hooks.
static BASE: Mutex<Option<Rules>> = Mutex::new(None);

impl Rules {
    fn base() -> Rules {
        let mut host = Host::new(Limits::default(), api).expect("Luau host");
        for (name, src) in BUILTIN {
            host.add_source(name, src).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let compose = host.modules().iter().enumerate().find_map(|(i, m)| m.exports.as_ref()?.raw_get::<Option<Function>>("compose").ok().flatten().map(|f| (i, f)));
        assert!(compose.is_some(), "mods/base/spells.luau exports no compose");
        Rules { seen: host.generation(), host, compose, cache: HashMap::new(), recent: vec![], seen_lists: true, base: true }
    }

    /// One composed spell, fitted; cached per generation. None when the
    /// grammar failed (logged) or there is none.
    fn compose_fitted(&mut self, slots: &[El], form: Form, mods: &Mods) -> Option<Spell> {
        if self.host.generation() != self.seen {
            self.refresh();
        }
        let k = format!("{slots:?}{form:?}{mods:?}");
        if let Some(s) = self.cache.get(&k) {
            return s.clone();
        }
        let out = self.raw(slots, form, mods).map(|(mut s, fit)| {
            if fit && s.n.has_rate && crate::balance::measured(&s) {
                // The factor comes from the spell without tomes, so a tome
                // still makes you stronger.
                if let Some((mut base, _)) = self.raw(slots, form, &Mods::default()) {
                    s.n.rate *= crate::balance::fit(&mut base);
                }
            }
            s
        });
        self.cache.insert(k, out.clone());
        out
    }
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
    for (k, v) in [("dmg", m.dmg), ("heal", m.heal), ("reach", m.reach), ("charge", m.charge), ("enchant_time", m.enchant_time), ("sword", m.sword), ("afflicted", m.afflicted), ("speed", m.speed), ("hp", m.hp), ("mana", m.mana), ("regen", m.regen), ("cost", m.cost), ("souls", m.souls), ("magnet", m.magnet), ("guard", m.guard), ("status", m.status), ("push", m.push), ("bolt", m.bolt), ("swing", m.swing), ("leech", m.leech), ("thorns", m.thorns), ("wind", m.wind), ("chain", m.chain), ("nova", m.nova), ("raise", m.raise)] {
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
                    "leech" => Mod::Leech(f()?),
                    "thorns" => Mod::Thorns(f()?),
                    "wind" => Mod::Wind(f()?),
                    "chain" => Mod::Chain(f()?),
                    "nova" => Mod::Nova(f()?),
                    "raise" => Mod::Raise(f()?),
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

/// Read `what` from the built-in module `file`, once per process.
fn builtin<T>(file: &str, what: &str, read: impl FnOnce(&Table) -> mlua::Result<T>) -> T {
    let mut host = Host::new(Limits::default(), api).expect("Luau host");
    let (name, src) = BUILTIN.iter().find(|(n, _)| *n == file).unwrap();
    host.add_source(name, src).unwrap_or_else(|e| panic!("{name}: {e}"));
    let t: Table = host.modules()[0].exports.as_ref().unwrap().raw_get(what).unwrap_or_else(|e| panic!("{name}: {what}: {e}"));
    read(&t).unwrap_or_else(|e| panic!("{name}: {what}: {}", script::short(&e)))
}

/// The monster kinds from the built-in mods/base/monsters.luau: what the
/// game uses until rules with their own list are installed, and what the
/// sim's tests use.
pub fn builtin_kinds() -> &'static [Monster] {
    static KINDS: std::sync::OnceLock<Vec<Monster>> = std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        let mut k = builtin("base/monsters.luau", "monsters", read_monsters);
        k.push(crate::enemy::none_kind());
        k
    })
}

/// Every element's effect from the built-in mods/base/spells.luau.
pub fn builtin_effects() -> [Effect; 13] {
    static EFFECTS: std::sync::OnceLock<[Effect; 13]> = std::sync::OnceLock::new();
    *EFFECTS.get_or_init(|| {
        builtin("base/spells.luau", "effects", |t| {
            let mut out = [Effect { dmg: f64::NAN, poison: 0.0, push: 0.0, burn: 0.0, chill: 0.0, wet: 0.0, shock: 0.0, slow: 0.0 }; 13];
            read_effects(t, &mut out)?;
            match el::ALL.iter().find(|e| out[e.idx()].dmg.is_nan()) {
                Some(e) => Err(mlua::Error::runtime(format!("no effect for {}", key(*e)))),
                None => Ok(out),
            }
        })
    })
}

/// The tomes from the built-in mods/base/tomes.luau.
pub fn builtin_tomes() -> Vec<Tome> {
    static TOMES: std::sync::OnceLock<Vec<Tome>> = std::sync::OnceLock::new();
    TOMES.get_or_init(|| builtin("base/tomes.luau", "tomes", read_tomes)).clone()
}

/// The biomes from the built-in mods/base/biomes.luau.
pub fn builtin_biomes() -> &'static [Biome] {
    static BIOMES: std::sync::OnceLock<Vec<Biome>> = std::sync::OnceLock::new();
    BIOMES.get_or_init(|| builtin("base/biomes.luau", "biomes", read_biomes))
}

/// The `biomes` list.
fn read_biomes(t: &Table) -> mlua::Result<Vec<Biome>> {
    use crate::biome::WallStyle;
    let mut out: Vec<Biome> = vec![];
    for (i, row) in t.sequence_values::<Table>().enumerate() {
        let row = row?;
        let id: String = row.raw_get::<Option<String>>("id")?.ok_or_else(|| mlua::Error::runtime(format!("biome {} has no id", i + 1)))?;
        let bad = |f: &str, why: &str| mlua::Error::runtime(format!("{id}: {f} {why}"));
        let num = |f: &str| -> mlua::Result<f32> {
            let v: Option<f64> = row.raw_get(f)?;
            v.filter(|x| x.is_finite()).map(|x| x as f32).ok_or_else(|| bad(f, "must be a number"))
        };
        let flag = |f: &str| -> mlua::Result<bool> { Ok(row.raw_get::<Option<bool>>(f)?.unwrap_or(false)) };
        let word = |f: &str| -> mlua::Result<String> { row.raw_get::<Option<String>>(f)?.ok_or_else(|| bad(f, "must be a name")) };
        let rgb = |f: &str| -> mlua::Result<[f32; 3]> {
            let c: Table = row.raw_get::<Option<Table>>(f)?.ok_or_else(|| bad(f, "must be { r, g, b }"))?;
            let mut out = [0.0; 3];
            for (j, o) in out.iter_mut().enumerate() {
                let x: f64 = c.raw_get::<Option<f64>>(j + 1)?.ok_or_else(|| bad(f, "must be { r, g, b }"))?;
                if !(0.0..=4.0).contains(&x) { return Err(bad(f, "channels go from 0 to 4")); }
                *o = x as f32;
            }
            Ok(out)
        };
        let names_in = |t: Option<Table>, f: &str| -> mlua::Result<&'static [&'static str]> {
            let mut v = vec![];
            if let Some(t) = t {
                for n in t.sequence_values::<String>() { v.push(leak(n?)); }
            } else {
                return Err(bad(f, "must be a list (or {})"));
            }
            Ok(Box::leak(v.into_boxed_slice()))
        };
        let names = |f: &str| names_in(row.raw_get::<Option<Table>>(f)?, f);
        let zt: Table = row.raw_get::<Option<Table>>("zones")?.ok_or_else(|| bad("zones", "must be three lists of props"))?;
        if zt.raw_len() != 3 { return Err(bad("zones", "must be three lists of props")); }
        let zones = [names_in(zt.raw_get(1)?, "zones")?, names_in(zt.raw_get(2)?, "zones")?, names_in(zt.raw_get(3)?, "zones")?];
        let mut enemies = vec![];
        for e in row.raw_get::<Option<Table>>("enemies")?.ok_or_else(|| bad("enemies", "must be a list of { kind, count }"))?.sequence_values::<Table>() {
            let e = e?;
            let k: String = e.raw_get::<Option<String>>(1)?.ok_or_else(|| bad("enemies", "entries are { kind, count }"))?;
            let n: f64 = e.raw_get::<Option<f64>>(2)?.ok_or_else(|| bad("enemies", "entries are { kind, count }"))?;
            if !(1.0..=32.0).contains(&n) || n.fract() != 0.0 { return Err(bad("enemies", "counts are whole numbers from 1 to 32")); }
            enemies.push((leak(k), n as u32));
        }
        if enemies.is_empty() { return Err(bad("enemies", "needs at least one kind")); }
        let mut fields = vec![];
        for e in row.raw_get::<Option<Table>>("fields")?.ok_or_else(|| bad("fields", "must be a list (or {})"))?.sequence_values::<String>() {
            fields.push(el_of(&e?)?);
        }
        let wall_style = match word("wall_style")?.as_str() {
            "masonry" => WallStyle::Masonry, "ossuary" => WallStyle::Ossuary, "rock" => WallStyle::Rock,
            "castle" => WallStyle::Castle, "cliff" => WallStyle::Cliff,
            w => return Err(bad("wall_style", &format!("{w:?} is not masonry, ossuary, rock, castle or cliff"))),
        };
        let path_style = num("path_style")?;
        if !(0.0..=8.0).contains(&path_style) || path_style.fract() != 0.0 { return Err(bad("path_style", "must be a whole number from 0 to 8")); }
        if out.iter().any(|b| b.id == id) { return Err(bad("id", "is used twice")); }
        out.push(Biome {
            name: leak(word("name")?),
            moss: num("moss")?, moss_c: rgb("moss_c")?, zones,
            floor_a: rgb("floor_a")?, floor_b: rgb("floor_b")?, tiles: num("tiles")?, wet: num("wet")?,
            wall: rgb("wall")?, fog: rgb("fog")?, ambient: rgb("ambient")?, moon: rgb("moon")?,
            moon_energy: num("moon_energy")?, mist: num("mist")?, mist_c: rgb("mist_c")?,
            props: names("props")?, enemies: Box::leak(enemies.into_boxed_slice()), boss: leak(word("boss")?),
            fields: Box::leak(fields.into_boxed_slice()), path_style: path_style as i32,
            path_a: rgb("path_a")?, path_b: rgb("path_b")?, nave: flag("nave")?, corridor: flag("corridor")?,
            outside: names("outside")?, wall_style, wall_props: names("wall_props")?, wall_every: num("wall_every")?,
            isles: flag("isles")?, low_walls: flag("low_walls")?, water_props: names("water_props")?,
            density: num("density")?, outside_lava: flag("outside_lava")?,
            id: leak(id),
        });
    }
    if out.is_empty() { return Err(mlua::Error::runtime("the list is empty")); }
    Ok(out)
}

/// Every monster a biome names exists, and its boss is a boss.
fn check_biomes(biomes: &[Biome], kinds: &[Monster]) -> Result<(), String> {
    for b in biomes {
        for (k, _) in b.enemies {
            if !kinds.iter().any(|m| m.key == *k) { return Err(format!("{}: enemies: no monster called {k:?}", b.id)); }
        }
        match kinds.iter().find(|m| m.key == b.boss) {
            Some(m) if m.boss => {}
            Some(_) => return Err(format!("{}: boss: {:?} is not a boss (boss = true in monsters.luau)", b.id, b.boss)),
            None => return Err(format!("{}: boss: no monster called {:?}", b.id, b.boss)),
        }
    }
    Ok(())
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// The `monsters` list: one table per kind, every field given (the base
/// file fills them from its DEFAULT row).
fn read_monsters(t: &Table) -> mlua::Result<Vec<Monster>> {
    use crate::enemy::{Attack, Shot, Spawn};
    let mut out: Vec<Monster> = vec![];
    for (i, row) in t.sequence_values::<Table>().enumerate() {
        let row = row?;
        let key: String = row.raw_get::<Option<String>>("key")?.ok_or_else(|| mlua::Error::runtime(format!("monster {} has no key", i + 1)))?;
        let bad = |f: &str, why: &str| mlua::Error::runtime(format!("{key}: {f} {why}"));
        let num = |f: &str| -> mlua::Result<f64> {
            let v: Option<f64> = row.raw_get(f)?;
            v.filter(|x| x.is_finite()).ok_or_else(|| bad(f, "must be a number"))
        };
        let opt_num = |f: &str| -> mlua::Result<Option<f64>> {
            match row.raw_get::<Value>(f)? { Value::Nil => Ok(None), v => v.as_number().or(v.as_integer().map(|i| i as f64)).filter(|x| x.is_finite()).map(Some).ok_or_else(|| bad(f, "must be a number or nil")) }
        };
        let flag = |f: &str| -> mlua::Result<bool> { Ok(row.raw_get::<Option<bool>>(f)?.unwrap_or(false)) };
        let opt_str = |f: &str| -> mlua::Result<Option<String>> { row.raw_get::<Option<String>>(f) };
        let el = |f: &str| -> mlua::Result<El> { el_of(&opt_str(f)?.ok_or_else(|| bad(f, "must be an element"))?) };
        let opt_el = |f: &str| -> mlua::Result<Option<El>> { opt_str(f)?.map(|n| el_of(&n)).transpose() };
        let count = |f: &str| -> mlua::Result<usize> {
            let x = num(f)?;
            if x < 0.0 || x.fract() != 0.0 || x > 64.0 { return Err(bad(f, "must be a whole number from 0 to 64")); }
            Ok(x as usize)
        };
        let word = |f: &str| -> mlua::Result<String> { opt_str(f)?.ok_or_else(|| bad(f, "must be a name")) };
        let attack = match word("attack")?.as_str() {
            "melee" => Attack::Melee, "ranged" => Attack::Ranged, "slam" => Attack::Slam,
            "burrow" => Attack::Burrow, "dash" => Attack::Dash, "lob" => Attack::Lob,
            w => return Err(bad("attack", &format!("{w:?} is not melee, ranged, slam, burrow, dash or lob"))),
        };
        let spawn = match word("spawn")?.as_str() {
            "ahead" => Spawn::Ahead, "far" => Spawn::Far, "under" => Spawn::Under, "flank" => Spawn::Flank, "water" => Spawn::Water,
            w => return Err(bad("spawn", &format!("{w:?} is not ahead, far, under, flank or water"))),
        };
        let mut shots = vec![];
        for s in row.raw_get::<Option<Table>>("shots")?.ok_or_else(|| bad("shots", "must be a list"))?.sequence_values::<String>() {
            shots.push(match s?.as_str() {
                "fan3" => Shot::Fan3, "fan5" => Shot::Fan5, "ring10" => Shot::Ring10, "burst3" => Shot::Burst3,
                "wave7" => Shot::Wave7, "slow1" => Shot::Slow1, "snipe" => Shot::Snipe,
                w => return Err(bad("shots", &format!("{w:?} is not fan3, fan5, ring10, burst3, wave7, slow1 or snipe"))),
            });
        }
        if shots.is_empty() { return Err(bad("shots", "needs at least one pattern")); }
        let mut resist = vec![];
        if let Some(r) = row.raw_get::<Option<Table>>("resist")? {
            for pair in r.pairs::<String, f64>() {
                let (e, x) = pair?;
                if !(-1.0..=1.0).contains(&x) { return Err(bad("resist", "values go from -1 to 1")); }
                resist.push((el_of(&e)?, x));
            }
        }
        resist.sort_by_key(|(e, _)| e.idx());
        let mut summon = vec![];
        if let Some(r) = row.raw_get::<Option<Table>>("summon")? {
            for k in r.sequence_values::<String>() { summon.push(leak(k?)); }
        }
        let dash: Table = row.raw_get::<Option<Table>>("dash")?.ok_or_else(|| bad("dash", "must be { speed, seconds }"))?;
        let (hp, speed, dmg, size, mass) = (num("hp")?, num("speed")?, num("dmg")?, num("size")?, num("mass")?);
        if hp <= 0.0 || speed < 0.0 || dmg < 0.0 || size <= 0.0 || mass <= 0.0 { return Err(bad("hp, size and mass", "must be above 0, speed and dmg not below")); }
        if out.iter().any(|k| k.key == key) { return Err(bad("key", "is used twice")); }
        out.push(Monster {
            name: leak(word("name")?),
            model: opt_str("model")?.map(leak),
            hp, speed, dmg,
            reach: num("reach")? as f32,
            keep: opt_num("keep")?.map(|x| x as f32),
            el: el("el")?,
            size: size as f32,
            attack,
            shots: Box::leak(shots.into_boxed_slice()),
            wind: opt_num("wind")?,
            ring: count("ring")?,
            mass,
            undead: flag("undead")?,
            resist: Box::leak(resist.into_boxed_slice()),
            heals_from: opt_el("heals_from")?,
            burst: opt_el("burst")?,
            always_wet: flag("always_wet")?,
            practice: flag("practice")?,
            raise: opt_str("raise")?.map(leak),
            solo: flag("solo")?,
            spawn,
            dash: (dash.raw_get::<f64>(1)? as f32, dash.raw_get::<f64>(2)?),
            trail: opt_el("trail")?,
            pack: count("pack")?.max(1),
            shield: flag("shield")?,
            boss: flag("boss")?,
            summon: Box::leak(summon.into_boxed_slice()),
            nova: el("nova")?,
            tempo: opt_num("tempo")?.unwrap_or(1.0).clamp(0.5, 3.0),
            key: leak(key),
        });
    }
    if out.is_empty() { return Err(mlua::Error::runtime("the list is empty")); }
    // Names other kinds point at must exist.
    for k in &out {
        for other in k.summon.iter().chain(k.raise.iter()).chain(k.model.iter()) {
            if !out.iter().any(|o| o.key == *other) { return Err(mlua::Error::runtime(format!("{}: no monster called {other:?}", k.key))); }
        }
    }
    Ok(out)
}
