//! Growing Guns' Luau API (engine/crates/script hosts it): the cards and
//! game rules in `games/growing-guns/mods`, hot-reloaded while playing.
//!
//! A module returns a table; these keys mean something:
//! - `cards`: card definitions (see `mods/base/cards.luau`); a card's
//!   `apply(w)` mutates the player's Weapon (Godot field names);
//! - `hit(g, e)`, `kill(g, e)`, `tick(g)`, `round_start(g)`: hooks the sim
//!   calls inline (World::hooks), in module path order.
//!
//! `g` is the live match (only valid during the call); players are 1..n.
//! Everything runs on the host inside the sim tick, so a match with the same
//! mods replays exactly; clients only load the card data for their menus.
use glam::Vec3;
use growing_guns::cards::{Card, CardDef};
use growing_guns::weapon::{self, Weapon};
use growing_guns::{HitInfo, HookSlot, Hooks, World};
use script::mlua::{self, Function, Lua, Table, Value, Vector};
use script::{Host, Limits};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The base cards, for a binary run away from the repository.
const BUILTIN: &str = include_str!("../../mods/base/cards.luau");

/// Methods on `g`: name, Luau signature, what it does. API.md and the type
/// definitions come from this list, and a test checks every entry exists.
pub const API: &[(&str, &str, &str)] = &[
    ("players", "(self): number", "Number of players."),
    ("alive", "(self, p: number): boolean", "Player p is alive."),
    ("bot", "(self, p: number): boolean", "Player p is a bot."),
    ("pos", "(self, p: number): vector", "Position (capsule centre), metres."),
    ("set_pos", "(self, p: number, v: vector)", "Teleport."),
    ("vel", "(self, p: number): vector", "Velocity, m/s."),
    ("set_vel", "(self, p: number, v: vector)", "Set velocity (knockback, launch pads)."),
    ("aim", "(self, p: number): vector", "Unit vector the player looks along."),
    ("health", "(self, p: number): number", "Health (0 when dead)."),
    ("set_health", "(self, p: number, hp: number)", "Set health, capped at max health. Does not kill: use damage."),
    ("max_health", "(self, p: number): number", "100 plus the hp_bonus stat."),
    ("damage", "(self, p: number, amount: number, by: number?): boolean", "Damage at the start of the next tick, credited to player `by`; can kill, skips the hit hooks. False over quota."),
    ("ammo", "(self, p: number): number", "Rounds in the magazine."),
    ("set_ammo", "(self, p: number, n: number)", "Set rounds in the magazine."),
    ("score", "(self, p: number): number", "Rounds won."),
    ("stat", "(self, p: number, name: string): any", "A field of player p's Weapon (see stats()): number, boolean, vector or the special's name."),
    ("set_stat", "(self, p: number, name: string, value: any)", "Change a Weapon field for this round (cards are re-applied each round)."),
    ("stats", "(self): {string}", "Names of the Weapon fields."),
    ("special_cooldown", "(self, p: number): number", "Seconds until player p's RMB special is ready."),
    ("modifier", "(self): string?", "This round's modifier id, or nil."),
    ("cards", "(self, p: number): {string}", "Ids of the cards player p holds, in pick order."),
    ("card_ids", "(self): {string}", "Ids of every card on offer this match."),
    ("give", "(self, p: number, id: string): boolean", "Give a card at the start of the next tick (stats and pick hook). False over quota."),
    ("explode", "(self, at: vector, radius: number, damage: number, by: number?): boolean", "Explosion at the start of the next tick. False over quota (32 per tick)."),
    ("tick", "(self): number", "Sim ticks since the match started (60 per second)."),
    ("time", "(self): number", "Match time in seconds."),
    ("round", "(self): number", "Round number, from 1."),
    ("between_rounds", "(self): boolean", "The round is over (intermission and card picks)."),
    ("random", "(self): number", "Uniform in [0, 1), from the match seed: the same every replay."),
];

/// Hook keys a module can export, with their Luau signatures.
pub const HOOKS: &[(&str, &str, &str)] = &[
    ("hit", "(g: Game, e: Hit) -> number?", "Before damage lands. Return the damage to deal (nil keeps it). Modules chain."),
    ("kill", "(g: Game, e: Kill) -> ()", "A player died."),
    ("tick", "(g: Game) -> ()", "End of every sim tick (60 per second). Keep it cheap."),
    ("round_start", "(g: Game) -> ()", "A new round's arena and spawns are ready."),
];

pub struct Scripts { inner: Mutex<Inner> }

struct Inner {
    host: Host,
    seen: u64,
    /// Card id -> the module that defined it and its `pick` function.
    picks: Vec<(String, usize, Function)>,
    /// Card id -> its `apply(w)`.
    applies: Vec<(String, usize, Function)>,
    /// Recent log lines, for the `mods` command.
    recent: Vec<String>,
}

/// The mods folder: GG_MODS, else `games/growing-guns/mods` above the
/// working directory or the executable.
pub fn find_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("GG_MODS") { return Some(d.into()); }
    // A packaged game: `mods` beside the executable.
    if let Some(d) = crate::sounds::beside_exe("mods") && d.is_dir() { return Some(d); }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok()];
    for start in starts.into_iter().flatten() {
        for dir in start.ancestors() {
            let d = dir.join("games/growing-guns/mods");
            if d.is_dir() { return Some(d); }
        }
    }
    None
}

/// Load the mods and hook them into `world` (cards and rules).
pub fn attach(world: &mut World) -> Result<Arc<Scripts>, String> { attach_dir(world, find_dir()) }

pub fn attach_dir(world: &mut World, dir: Option<PathBuf>) -> Result<Arc<Scripts>, String> {
    let mut host = Host::new(Limits::default(), |lua, api| { register(lua)?; godot_helpers(lua, api) })?;
    match &dir {
        Some(d) => { host.watch(d); }
        None => host.add_source("base/cards.luau", BUILTIN)?,
    }
    let s = Arc::new(Scripts { inner: Mutex::new(Inner { host, seen: u64::MAX, picks: Vec::new(), applies: Vec::new(), recent: Vec::new() }) });
    s.refresh(world);
    world.hooks = HookSlot(Some(s.clone()));
    Ok(s)
}

impl Scripts {
    /// Pick up edited files; returns what was reloaded and any errors.
    pub fn poll(&self, world: &mut World) -> Vec<String> {
        self.inner.lock().unwrap().host.poll();
        self.refresh(world)
    }

    /// Re-read card data after a (re)load and collect log lines.
    fn refresh(&self, world: &mut World) -> Vec<String> {
        let mut st = self.inner.lock().unwrap();
        if st.host.generation() != st.seen {
            st.seen = st.host.generation();
            let (defs, picks, applies, errors) = collect_cards(&st.host);
            world.cards = Arc::new(world.cards.merged(defs));
            st.picks = picks;
            st.applies = applies;
            for e in errors { st.host_log(e); }
        }
        let lines = st.host.drain_log();
        st.recent.extend(lines.iter().cloned());
        let n = st.recent.len();
        if n > 40 { st.recent.drain(..n - 40); }
        lines
    }

    /// A Luau line against the live match (`g`), for the debug console.
    pub fn console(&self, world: &mut World, line: &str) -> Result<String, String> {
        let mut st = self.inner.lock().unwrap();
        let f = st.host.console_line(line)?;
        let console = st.host.console().clone();
        let lua = st.host.lua().clone();
        let r = lua.scope(|s| {
            console.raw_set("g", s.create_any_userdata_ref_mut(world)?)?;
            f.call::<mlua::MultiValue>(())
        });
        let _ = console.raw_set("g", Value::Nil);
        let out = r.map_err(|e| script::short(&e))?;
        let printed = st.host.drain_log();
        let shown: Vec<String> = out.iter().map(|v| v.to_string().unwrap_or_else(|_| v.type_name().into())).collect();
        Ok(printed.into_iter().chain(shown).collect::<Vec<_>>().join("\n"))
    }

    /// Loaded modules, their errors and the latest log lines.
    pub fn status(&self) -> String {
        let st = self.inner.lock().unwrap();
        let mut out = Vec::new();
        for m in st.host.modules() {
            let state = if m.broken { "hooks off after an error" } else if m.exports.is_none() { "not loaded" } else { "ok" };
            out.push(format!("{}: {state}{}", m.name, m.error.as_ref().map(|e| format!(" | {e}")).unwrap_or_default()));
        }
        out.extend(st.recent.iter().rev().take(8).rev().map(|l| format!("  {l}")));
        out.join("\n")
    }

    /// Call `hook` in every module that has it, with `g` bound to the world.
    fn each(&self, hook: &str, world: &mut World, mut call: impl FnMut(&Lua, &Function, mlua::AnyUserData) -> mlua::Result<()>) {
        let mut st = self.inner.lock().unwrap();
        if !st.host.has(hook) { return; }
        for (m, f) in st.host.each(hook) {
            st.host.run(m, hook, |lua| lua.scope(|s| call(lua, &f, s.create_any_userdata_ref_mut(&mut *world)?)));
        }
    }
}

impl Inner {
    fn host_log(&mut self, line: String) { eprintln!("mods: {line}"); self.recent.push(line); }
}

impl Hooks for Scripts {
    fn apply_card(&self, def: &CardDef, w: &mut Weapon) {
        let mut st = self.inner.lock().unwrap();
        let Some((_, m, f)) = st.applies.iter().find(|(i, _, _)| *i == def.id).cloned() else { return };
        st.host.run(m, &format!("apply {}", def.id), |lua| lua.scope(|s| f.call::<()>(s.create_any_userdata_ref_mut(w)?)));
    }

    fn pick(&self, w: &mut World, player: usize, card: Card) {
        let mut st = self.inner.lock().unwrap();
        let Some(id) = w.cards.get(card).map(|d| d.id.clone()) else { return };
        let Some((_, m, f)) = st.picks.iter().find(|(i, _, _)| *i == id).cloned() else { return };
        st.host.run(m, &format!("pick {id}"), |lua| lua.scope(|s| f.call::<()>((s.create_any_userdata_ref_mut(w)?, player + 1))));
    }

    fn hit(&self, w: &mut World, h: &HitInfo) -> i32 {
        let mut damage = h.damage;
        self.each("hit", w, |lua, f, g| {
            let e = lua.create_table()?;
            e.raw_set("victim", h.victim + 1)?;
            e.raw_set("attacker", h.attacker + 1)?;
            e.raw_set("damage", damage)?;
            e.raw_set("head", h.head)?;
            e.raw_set("explosion", h.explosion)?;
            if let Some(d) = f.call::<Option<f64>>((g, e))? { damage = d.round().clamp(0.0, 1e6) as i32; }
            Ok(())
        });
        damage
    }

    fn kill(&self, w: &mut World, victim: usize, attacker: Option<usize>) {
        self.each("kill", w, |lua, f, g| {
            let e = lua.create_table()?;
            e.raw_set("victim", victim + 1)?;
            e.raw_set("attacker", attacker.map(|a| a + 1))?;
            f.call::<()>((g, e))
        });
    }

    fn tick(&self, w: &mut World) { self.each("tick", w, |_, f, g| f.call::<()>(g)); }
    fn round_start(&self, w: &mut World) { self.each("round_start", w, |_, f, g| f.call::<()>(g)); }
}

/// Card definitions from every module's `cards`, in module order.
type Fns = Vec<(String, usize, Function)>;

fn collect_cards(host: &Host) -> (Vec<CardDef>, Fns, Fns, Vec<String>) {
    let (mut defs, mut picks, mut applies, mut errors) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (m, module) in host.modules().iter().enumerate() {
        let Some(list) = module.exports.as_ref().and_then(|e| e.raw_get::<Option<Table>>("cards").ok().flatten()) else { continue };
        for (k, t) in list.sequence_values::<Table>().enumerate() {
            match t.map_err(|e| script::short(&e)).and_then(|t| card(&t).map(|d| (d, t))) {
                Ok((d, t)) => {
                    if defs.iter().any(|x: &CardDef| x.id == d.id) {
                        errors.push(format!("{}: card {}: id {} is used twice; the later one wins", module.name, k + 1, d.id));
                        defs.retain(|x| x.id != d.id);
                        picks.retain(|p: &(String, usize, Function)| p.0 != d.id);
                        applies.retain(|p: &(String, usize, Function)| p.0 != d.id);
                    }
                    if let Ok(Some(f)) = t.raw_get::<Option<Function>>("pick") { picks.push((d.id.clone(), m, f)); }
                    if let Ok(Some(f)) = t.raw_get::<Option<Function>>("apply") { applies.push((d.id.clone(), m, f)); }
                    defs.push(d);
                }
                Err(e) => errors.push(format!("{}: card {}: {e}", module.name, k + 1)),
            }
        }
    }
    (defs, picks, applies, errors)
}

fn card(t: &Table) -> Result<CardDef, String> {
    let s = |k: &str| t.raw_get::<Option<String>>(k).map_err(|_| format!("{k} must be a string"));
    let id = s("id")?.ok_or("needs an id")?;
    let ops = |k: &str| -> Result<Vec<(String, f32)>, String> {
        let Some(m) = t.raw_get::<Option<Table>>(k).map_err(|_| format!("{id}: {k} must be a table of field = number"))? else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for kv in m.pairs::<String, f64>() {
            let (name, v) = kv.map_err(|_| format!("{id}: {k} must be a table of field = number"))?;
            if !weapon::FIELDS.iter().any(|(f, kind)| *f == name && (*kind == "f32" || *kind == "i32")) {
                return Err(format!("{id}: {k}: {name} is not a numeric Weapon field (see stats())"));
            }
            out.push((name, v as f32));
        }
        // Table order is not defined: apply in the fixed field order.
        out.sort_by_key(|(n, _)| weapon::FIELDS.iter().position(|(f, _)| f == n));
        Ok(out)
    };
    let color = match t.raw_get::<Value>("color").map_err(|e| script::short(&e))? {
        Value::Vector(v) => [v.x(), v.y(), v.z()],
        Value::Table(c) if c.raw_len() == 3 => [c.raw_get(1).unwrap_or(1.0), c.raw_get(2).unwrap_or(1.0), c.raw_get(3).unwrap_or(1.0)],
        Value::Nil => [1.0; 3],
        _ => return Err(format!("{id}: color must be Color(r, g, b) or {{r, g, b}}")),
    };
    let tags: Vec<String> = t.raw_get::<Option<Vec<String>>>("tags").map_err(|_| format!("{id}: tags must be a list of strings"))?.unwrap_or_default();
    let rarity = s("rarity")?.unwrap_or_else(|| "common".into());
    if rarity != "common" && rarity != "rare" { return Err(format!("{id}: rarity must be \"common\" or \"rare\"")); }
    Ok(CardDef {
        name: s("name")?.unwrap_or_else(|| id.to_uppercase()),
        desc: s("desc")?.or(s("text")?).unwrap_or_default(),
        color,
        firepower: tags.iter().any(|t| t == "firepower") || t.raw_get::<Option<bool>>("firepower").ok().flatten().unwrap_or(false),
        rare: rarity == "rare",
        mul: ops("mul")?,
        add: ops("add")?,
        enabled: true,
        id,
    })
}

fn v3(v: Vector) -> Vec3 { Vec3::new(v.x(), v.y(), v.z()) }
fn vector(v: Vec3) -> Vector { Vector::new(v.x, v.y, v.z) }

/// Lua player number (1..n) to an index.
fn pl(w: &World, p: i64) -> mlua::Result<usize> {
    if p >= 1 && (p as usize) <= w.players.len() { Ok(p as usize - 1) } else { Err(mlua::Error::runtime(format!("no player {p} (players are 1..{})", w.players.len()))) }
}

fn to_lua(lua: &Lua, v: weapon::Value) -> mlua::Result<Value> {
    Ok(match v {
        weapon::Value::Num(x) => Value::Number(x as f64),
        weapon::Value::Bool(b) => Value::Boolean(b),
        weapon::Value::Vec(v) => Value::Vector(vector(v)),
        weapon::Value::Text(t) => Value::String(lua.create_string(&t)?),
    })
}

fn from_lua(v: Value) -> mlua::Result<weapon::Value> {
    Ok(match v {
        Value::Number(x) => weapon::Value::Num(x as f32),
        Value::Integer(x) => weapon::Value::Num(x as f32),
        Value::Boolean(b) => weapon::Value::Bool(b),
        Value::Vector(v) => weapon::Value::Vec(v3(v)),
        Value::String(s) => weapon::Value::Text(s.to_str()?.to_string()),
        other => return Err(mlua::Error::runtime(format!("a Weapon field can't be a {}", other.type_name()))),
    })
}

fn field(w: &Weapon, name: &str) -> mlua::Result<weapon::Value> {
    w.get(name).ok_or_else(|| mlua::Error::runtime(format!("no Weapon field {name} (see stats())")))
}

/// `g`'s methods on the borrowed World.
fn register(lua: &Lua) -> mlua::Result<()> {
    lua.register_userdata_type::<World>(|r| {
        use script::mlua::UserDataMethods;
        r.add_method("players", |_, w, ()| Ok(w.players.len()));
        r.add_method("alive", |_, w, p: i64| Ok(w.players[pl(w, p)?].alive));
        r.add_method("bot", |_, w, p: i64| Ok(w.players[pl(w, p)?].bot));
        r.add_method("pos", |_, w, p: i64| Ok(vector(w.players[pl(w, p)?].pos)));
        r.add_method_mut("set_pos", |_, w, (p, v): (i64, Vector)| { let i = pl(w, p)?; w.players[i].pos = v3(v); Ok(()) });
        r.add_method("vel", |_, w, p: i64| Ok(vector(w.players[pl(w, p)?].vel)));
        r.add_method_mut("set_vel", |_, w, (p, v): (i64, Vector)| { let i = pl(w, p)?; w.players[i].vel = v3(v); Ok(()) });
        r.add_method("aim", |_, w, p: i64| Ok(vector(w.players[pl(w, p)?].forward())));
        r.add_method("health", |_, w, p: i64| Ok(w.players[pl(w, p)?].health));
        r.add_method_mut("set_health", |_, w, (p, hp): (i64, f64)| {
            let i = pl(w, p)?;
            let pl = &mut w.players[i];
            if pl.alive { pl.health = (hp.round() as i32).clamp(1, pl.max_health()); }
            Ok(())
        });
        r.add_method("max_health", |_, w, p: i64| Ok(w.players[pl(w, p)?].max_health()));
        r.add_method_mut("damage", |_, w, (p, amount, by): (i64, f64, Option<i64>)| {
            let i = pl(w, p)?;
            let by = by.map(|b| pl(w, b)).transpose()?;
            Ok(w.queue_damage(i, amount.round().clamp(0.0, 1e6) as i32, by))
        });
        r.add_method("ammo", |_, w, p: i64| Ok(w.players[pl(w, p)?].ammo));
        r.add_method_mut("set_ammo", |_, w, (p, n): (i64, f64)| { let i = pl(w, p)?; w.players[i].ammo = n.round().clamp(0.0, 100_000.0) as u32; Ok(()) });
        r.add_method("score", |_, w, p: i64| Ok(w.players[pl(w, p)?].score));
        r.add_method("stat", |lua, w, (p, name): (i64, String)| to_lua(lua, field(&w.players[pl(w, p)?].weapon, &name)?));
        r.add_method_mut("set_stat", |_, w, (p, name, v): (i64, String, Value)| {
            let i = pl(w, p)?;
            w.players[i].weapon.set(&name, from_lua(v)?).map_err(mlua::Error::runtime)
        });
        r.add_method("stats", |_, _, ()| Ok(weapon::FIELDS.iter().map(|f| f.0).chain(["special"]).collect::<Vec<_>>()));
        r.add_method("special_cooldown", |_, w, p: i64| Ok(w.players[pl(w, p)?].special_cd));
        r.add_method("modifier", |_, w, ()| Ok(w.modifier.map(|m| m.id())));
        r.add_method("cards", |_, w, p: i64| { let i = pl(w, p)?; Ok(w.players[i].cards.iter().filter_map(|&c| w.cards.get(c).map(|d| d.id.clone())).collect::<Vec<_>>()) });
        r.add_method("card_ids", |_, w, ()| Ok(w.cards.enabled().map(|(_, d)| d.id.clone()).collect::<Vec<_>>()));
        r.add_method_mut("give", |_, w, (p, id): (i64, String)| {
            let i = pl(w, p)?;
            let card = w.cards.find(&id).ok_or_else(|| mlua::Error::runtime(format!("unknown card {id}")))?;
            Ok(w.queue_give(i, card))
        });
        r.add_method_mut("explode", |_, w, (at, radius, damage, by): (Vector, f64, f64, Option<i64>)| {
            if w.queued_explosions() >= growing_guns::SCRIPT_QUOTA { return Ok(false); }
            let by = by.map(|b| pl(w, b)).transpose()?.unwrap_or(0);
            w.queue_explosion(v3(at), (radius as f32).clamp(0.1, 30.0), (damage as f32).clamp(0.0, 10_000.0), by);
            Ok(true)
        });
        r.add_method("tick", |_, w, ()| Ok(w.tick));
        r.add_method("time", |_, w, ()| Ok(w.tick as f64 / growing_guns::TICK_HZ as f64));
        r.add_method("round", |_, w, ()| Ok(w.round));
        r.add_method("between_rounds", |_, w, ()| Ok(w.between_rounds()));
        r.add_method_mut("random", |_, w, ()| Ok(w.script_rng.f() as f64));
    })?;
    // A card's `apply(w)` gets the Weapon itself: w.damage_mult *= 1.5.
    lua.register_userdata_type::<Weapon>(|r| {
        use script::mlua::{MetaMethod, UserDataMethods};
        r.add_meta_method(MetaMethod::Index, |lua, w, name: String| match w.get(&name) {
            Some(v) => to_lua(lua, v),
            None => Err(mlua::Error::runtime(format!("no Weapon field {name}"))),
        });
        r.add_meta_method_mut(MetaMethod::NewIndex, |_, w, (name, v): (String, Value)| w.set(&name, from_lua(v)?).map_err(mlua::Error::runtime));
    })?;
    Ok(())
}

/// Godot-named helpers, so card code reads like `card_library.gd`.
fn godot_helpers(lua: &Lua, api: &Table) -> mlua::Result<()> {
    let v = |lua: &Lua, x: f32, y: f32, z: f32| -> mlua::Result<Value> { let _ = lua; Ok(Value::Vector(Vector::new(x, y, z))) };
    api.raw_set("Color", lua.create_function(move |lua, (r, g, b): (f32, f32, f32)| v(lua, r, g, b))?)?;
    api.raw_set("Vector3", lua.create_function(move |lua, (x, y, z): (f32, f32, f32)| v(lua, x, y, z))?)?;
    api.raw_set("deg_to_rad", lua.create_function(|_, d: f64| Ok(d.to_radians()))?)?;
    api.raw_set("max", lua.create_function(|_, (a, b): (f64, f64)| Ok(a.max(b)))?)?;
    api.raw_set("min", lua.create_function(|_, (a, b): (f64, f64)| Ok(a.min(b)))?)?;
    api.raw_set("maxf", lua.create_function(|_, (a, b): (f64, f64)| Ok(a.max(b)))?)?;
    api.raw_set("minf", lua.create_function(|_, (a, b): (f64, f64)| Ok(a.min(b)))?)?;
    api.raw_set("clampf", lua.create_function(|_, (x, a, b): (f64, f64, f64)| Ok(x.clamp(a, b)))?)?;
    api.raw_set("lerp", lua.create_function(|_, (a, b, t): (Value, Value, f64)| match (a, b) {
        (Value::Vector(a), Value::Vector(b)) => Ok(Value::Vector(vector(v3(a).lerp(v3(b), t as f32)))),
        (a, b) => {
            let n = |v: &Value| v.as_number().or(v.as_integer().map(|i| i as f64));
            match (n(&a), n(&b)) { (Some(a), Some(b)) => Ok(Value::Number(a + (b - a) * t)), _ => Err(mlua::Error::runtime("lerp wants two numbers or two vectors")) }
        }
    })?)?;
    // Bone warps (cosmetic): warp(w, "Neck", Vector3(1, 1.5, 1)).
    api.raw_set("warp", lua.create_function(|_, (w, bone, by): (mlua::AnyUserData, String, Vector)| { w.borrow_mut_scoped::<Weapon, _>(|w| w.warp(&bone, v3(by)))?; Ok(()) })?)?;
    Ok(())
}

/// The API reference for people and models writing mods (mods/API.md).
pub fn api_doc() -> String {
    let mut s = String::from("<!-- Generated by `growing-guns mods-api` from app/src/scripts.rs; do not edit. -->\n");
    s += "# Growing Guns mod API\n\n";
    s += "Every `*.luau` file under `mods/` is a module. It hot-reloads when saved: a load error keeps the previous version, a runtime error turns that module's hooks off until the file changes. Modules run on the host inside the 60 Hz sim tick, in path order; the same mods and seed replay exactly.\n\n";
    s += "Sandbox: no `os`, `io`, `require`, `loadstring` or `math.random` (use `g:random()`). Each call gets a step budget (calls and loop iterations), the VM 64 MB. `state` is a per-module table that survives reloads. `print` goes to the game log. Do not depend on `pairs` order over tables keyed by tables or functions.\n\n";
    s += "## Module exports\n\nA module returns a table of this type (every key optional):\n\n```luau\n";
    s += &exports_type();
    s += "```\n\n";
    for (k, _, doc) in HOOKS { s += &format!("- `{k}`: {doc}\n"); }
    s += "\n```luau\ntype Hit = { victim: number, attacker: number, damage: number, head: boolean, explosion: boolean }\ntype Kill = { victim: number, attacker: number? }  -- attacker nil: lava\n```\n\n";
    s += "Dash is retired. The `dash_iframes`, `dash_end_melee` and `dash_spawn_bomb` weapon fields remain readable for old replay compatibility but have no gameplay effect. `dmg_up`, `big_mag` and `rapid_fire` have three times the ordinary draw weight.\n\n";
    s += "## Cards\n\nThe pool is `mods/base/cards.luau`, a port of Godot's `card_library.gd`. A card's `apply(w)` mutates the player's `Weapon` exactly like the GDScript card does; the Godot helpers exist under the same names: `Color(r, g, b)`, `Vector3(x, y, z)`, `deg_to_rad`, `max`, `min`, `maxf`, `minf`, `clampf`, `lerp(a, b, t)` (numbers or vectors) and `warp(w, bone, scale)` for cosmetic bone scales. Weapons are rebuilt from the cards every round, so `apply` must only depend on `w`.\n\n```luau\ntype Card = {\n  id: string,             -- stable; what `give P ID` uses\n  name: string?, desc: string?,\n  color: vector?,         -- Color(r, g, b), sRGB 0..1\n  tags: { string }?,      -- \"firepower\": improves the gun (every offer has one)\n  rarity: string?,        -- \"common\" (weight 1) or \"rare\" (0.15, more for trailing players)\n  apply: ((w: Weapon) -> ())?,  -- the card's effect\n  mul: { [string]: number }?,  -- or as data: field multipliers, then\n  add: { [string]: number }?,  -- field additions\n  pick: ((g: Game, p: number) -> ())?,  -- runs once when taken (match effects)\n}\n```\n\n## `Weapon` fields\n\nNames match Godot's `weapon.gd`; read and write them as `w.field` in `apply`, or with `g:stat` / `g:set_stat`. Numbers unless noted.\n\n";
    for (name, kind) in weapon::FIELDS {
        let k = match *kind { "i32" => "whole number", "bool" => "boolean", "vec" => "vector", "color" => "colour (vector)", _ => "number" };
        s += &format!("- `{name}` ({k})\n");
    }
    s += "- `special` (string): one of grenade, teleport, zoom, sword, air_strike, ion_cannon, cluster_grenade\n";
    s += "\n## `g`: the match\n\nPlayers are numbers 1..`g:players()`. `g` is only valid during the call that received it.\n\n";
    for (k, sig, doc) in API { s += &format!("- `g:{k}{}`: {doc}\n", sig.replacen("self, ", "", 1).replacen("(self)", "()", 1)); }
    s
}

fn exports_type() -> String {
    let mut s = String::from("type Exports = {\n\tcards: { Card }?,\n");
    for (k, sig, _) in HOOKS { s += &format!("\t{k}: ({sig})?,\n"); }
    s + "}\n"
}

/// Luau type definitions for editors and type checking (mods/types.d.luau).
pub fn api_types() -> String {
    let mut s = String::from("-- Generated by `growing-guns mods-api`; do not edit.\n\nexport type Game = {\n");
    for (k, sig, doc) in API {
        let (args, ret) = sig.split_once("):").map(|(a, r)| (format!("{a})"), r.trim().to_string())).unwrap_or((sig.to_string(), "()".into()));
        s += &format!("\t-- {doc}\n\t{k}: {} -> {ret},\n", args.replacen("self", "self: Game", 1));
    }
    s += "}\n\nexport type Hit = { victim: number, attacker: number, damage: number, head: boolean, explosion: boolean }\nexport type Kill = { victim: number, attacker: number? }\n";
    s += &format!("export {}", exports_type());
    s += "export type Card = { id: string, name: string?, text: string?, color: { number }?, firepower: boolean?, mul: { [string]: number }?, add: { [string]: number }?, pick: ((g: Game, p: number) -> ())? }\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use growing_guns::Input;

    fn world() -> (World, Arc<Scripts>) {
        let mut w = World::new(3, &[true; 4]);
        let s = attach_dir(&mut w, None).unwrap();
        (w, s)
    }

    #[test]
    fn base_cards_load_from_luau() {
        let (mut w, s) = world();
        let ids: Vec<String> = w.cards.enabled().map(|(_, d)| d.id.clone()).collect();
        assert_eq!(ids.len(), 49, "card pool without dash modifiers");
        for id in ["ninja", "dodge", "backblast"] { assert!(w.cards.find(id).is_none()); }
        assert_eq!(&ids[..3], ["dmg_up", "healthy", "steady_hands"]);
        let rare = w.cards.enabled().filter(|(_, d)| d.rare).count();
        assert_eq!(rare, 14, "card_library.gd has 14 rare cards");
        // Explosive: 4 m first, then +2.5 m; the special swaps and bodies scale.
        let e = w.cards.find("explosive").unwrap();
        w.give(0, e);
        w.give(0, e);
        let wp = &w.players[0].weapon;
        assert_eq!((wp.explosive_radius, wp.explosive_damage), (6.5, 50.0));
        assert_eq!(s.console(&mut w, "g:stat(1, 'explosive_radius')").unwrap(), "6.5");
        for id in ["sniper", "slenderman", "shotgun", "chilling_rounds"] { let c = w.cards.find(id).unwrap(); w.give(1, c); }
        let wp = &w.players[1].weapon;
        assert_eq!(wp.special, growing_guns::Special::Zoom);
        assert_eq!(wp.body_scale_axes, Vec3::new(0.72, 1.8, 0.72));
        assert_eq!(wp.bone_warps.len(), 3);
        assert_eq!(wp.extra_projectiles, 2);
        assert!((wp.spread - (0.0008f32.max(2f32.to_radians()) + 2f32.to_radians())).abs() < 1e-6, "{}", wp.spread);
        assert!((wp.slow_on_hit - 0.48).abs() < 1e-6 && wp.slow_duration == 2.6);
        // Weapons are rebuilt from the cards each round.
        w.rebuild_weapon(1);
        assert_eq!(w.players[1].weapon.special, growing_guns::Special::Zoom);
    }

    #[test]
    fn every_documented_method_exists() {
        let (mut w, s) = world();
        for (name, _, _) in API {
            assert_eq!(s.console(&mut w, &format!("typeof(g.{name})")).unwrap(), "function", "{name}");
        }
    }

    #[test]
    fn api_doc_is_current() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../mods");
        for (file, text) in [("API.md", api_doc()), ("types.d.luau", api_types())] {
            let path = dir.join(file);
            if std::env::var_os("BLESS").is_some() { std::fs::write(&path, &text).unwrap(); }
            // Git may check out the generated files with Windows line endings.
            let actual = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            assert_eq!(actual, text, "{file} is stale: BLESS=1 cargo test -p growing-guns-app api_doc");
        }
    }

    #[test]
    fn hooks_change_the_match_and_errors_switch_them_off() {
        let mut w = World::new(3, &[true; 4]);
        let s = attach_dir(&mut w, None).unwrap();
        {
            let mut st = s.inner.lock().unwrap();
            st.host.add_source("test/rules.luau", r#"
                state.kills = state.kills or 0
                return {
                    hit = function(g, e) return e.damage * 3 end,
                    kill = function(g, e) state.kills += 1 end,
                    tick = function(g) if g:tick() == 30 then g:explode(g:pos(1), 3, 10) end end,
                }"#).unwrap();
            st.host.add_source("test/bad.luau", "return { tick = function(g) local x = nil; x.y = 1 end }").unwrap();
        }
        s.refresh(&mut w);
        let mut kills = 0;
        for _ in 0..60 * 20 {
            w.step(&[Input::default(); 4]);
            kills += w.events.iter().filter(|e| matches!(e, growing_guns::Event::Kill { .. })).count();
        }
        assert!(kills > 0);
        // The console shows log lines first: here the bad module's error.
        let out = s.console(&mut w, "1").unwrap();
        assert!(out.contains("bad.luau:1: attempt to index nil") && out.ends_with("\n1"), "{out}");
        let status = s.status();
        assert!(status.contains("test/bad.luau: hooks off after an error"), "{status}");
        assert!(status.contains("test/rules.luau: ok"), "{status}");
    }

    /// Same mods, same seed: the same match, with hooks in the loop.
    #[test]
    fn scripted_matches_replay_exactly() {
        let run = || {
            let mut w = World::new(5, &[true; 4]);
            let s = attach_dir(&mut w, None).unwrap();
            s.inner.lock().unwrap().host.add_source("r.luau", "return { hit = function(g, e) return e.damage + math.floor(g:random() * 10) end }").unwrap();
            s.refresh(&mut w);
            for _ in 0..60 * 30 { w.step(&[]); }
            w.digest()
        };
        assert_eq!(run(), run());
    }
}
