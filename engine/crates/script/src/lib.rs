//! Luau scripting for Warp games.
//!
//! A [`Host`] loads every `*.luau` file (not `*.d.luau`) under a folder as a module, each in
//! its own sandboxed environment, and reloads a file when it changes while the
//! game runs. A module returns a table of exports (hooks and data) that the
//! game reads; the game decides which hooks exist and what they get.
//!
//! Rules that keep scripts safe to run inside a deterministic simulation:
//! - no clock, no files, no `os`, no `math.random`: the game hands out time
//!   and random numbers itself;
//! - every call runs under a step budget counted in VM interrupts (function
//!   calls and loop back-edges), so a runaway loop stops at the same point on
//!   every machine instead of freezing the game;
//! - a memory cap for the whole VM;
//! - a load error keeps the previous version running; a runtime error turns
//!   that module's hooks off until the file changes again.
//!
//! Each module has a `state` table that survives reloads, so a running match
//! keeps what its scripts remembered while their code is replaced.
use mlua::{Function, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

mod imports;
use std::collections::BTreeSet;
pub use mlua;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Bytes for the whole VM.
    pub memory: usize,
    /// VM interrupts (calls and loop iterations) per hook call or load.
    pub steps: u32,
}

impl Default for Limits {
    fn default() -> Self { Limits { memory: 64 << 20, steps: 200_000 } }
}

pub struct Module {
    /// Path under the root with `/` separators, e.g. `base/cards.luau`.
    pub name: String,
    dependencies: BTreeSet<String>,
    path: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    /// What the module returned; `None` until it first loads.
    pub exports: Option<Table>,
    /// The module's own globals from its last good load.
    env: Option<Table>,
    state: Table,
    /// The last load error, while the previous version (if any) still runs.
    pub error: Option<String>,
    /// A hook failed at runtime: this module's hooks stay off until it reloads.
    pub broken: bool,
    /// The last version that ran for a while (exports, globals), to fall
    /// back to when a fresh edit fails at runtime.
    proven: Option<(Table, Table)>,
    /// Calls the current version has completed, up to PROVEN_AFTER.
    calls: u32,
}

/// Successful calls after which a version counts as working.
const PROVEN_AFTER: u32 = 30;

impl Module {
    /// The module's `state` table, which survives reloads.
    pub fn state(&self) -> &Table { &self.state }
}

pub struct Host {
    lua: Lua,
    root: Option<PathBuf>,
    /// Read-only names every module sees: safe libraries plus the game's API.
    base: Table,
    console: Table,
    modules: Vec<Module>,
    steps: Arc<AtomicU32>,
    limits: Limits,
    log: Arc<Mutex<Vec<String>>>,
    /// Bumped on every successful load or removal.
    generation: u64,
    hooks: Vec<String>,
    /// Top-level folders under the root that hold no modules.
    ignored: Vec<String>,
    /// Load only this module of the root (a one-file game).
    only: Option<String>,
    libraries: Option<imports::Libraries>,
}

const BASE: &[&str] = &[
    "assert", "error", "ipairs", "next", "pairs", "pcall", "xpcall", "select", "tonumber", "tostring", "type", "typeof",
    "unpack", "rawequal", "rawget", "rawset", "rawlen", "setmetatable", "getmetatable",
    "string", "table", "utf8", "bit32", "buffer", "vector",
];

impl Host {
    /// A VM with the safe libraries; `api` adds the game's names (functions,
    /// constants, tables) before everything is frozen.
    pub fn new(limits: Limits, api: impl FnOnce(&Lua, &Table) -> mlua::Result<()>) -> Result<Self, String> {
        let libs = StdLib::MATH | StdLib::STRING | StdLib::TABLE | StdLib::UTF8 | StdLib::BIT | StdLib::BUFFER | StdLib::VECTOR;
        let lua = Lua::new_with(libs, LuaOptions::default()).map_err(err)?;
        lua.set_memory_limit(limits.memory).map_err(err)?;
        let steps = Arc::new(AtomicU32::new(limits.steps));
        let budget = steps.clone();
        lua.set_interrupt(move |_| {
            if budget.fetch_sub(1, Ordering::Relaxed) == 0 {
                budget.store(0, Ordering::Relaxed);
                return Err(mlua::Error::runtime("step budget used up (a loop that never ends?)"));
            }
            Ok(VmState::Continue)
        });
        let log = Arc::new(Mutex::new(Vec::new()));
        let base = (|| -> mlua::Result<Table> {
            let g = lua.globals();
            let base = lua.create_table()?;
            for &name in BASE {
                let v: Value = g.raw_get(name)?;
                if let Value::Table(t) = &v { t.set_readonly(true); }
                base.raw_set(name, v)?;
            }
            // math without its own random generator (not seeded by the game).
            let math = lua.create_table()?;
            g.raw_get::<Table>("math")?.for_each(|k: String, v: Value| if k == "random" || k == "randomseed" { Ok(()) } else { math.raw_set(k, v) })?;
            math.set_readonly(true);
            base.raw_set("math", math)?;
            let sink = log.clone();
            base.raw_set("print", lua.create_function(move |_, args: mlua::Variadic<Value>| {
                let line: Vec<String> = args.iter().map(|v| v.to_string().unwrap_or_else(|_| v.type_name().into())).collect();
                sink.lock().unwrap().push(line.join("\t"));
                Ok(())
            })?)?;
            api(&lua, &base)?;
            base.set_readonly(true);
            Ok(base)
        })().map_err(err)?;
        let console = env(&lua, &base, &lua.create_table().map_err(err)?).map_err(err)?;
        Ok(Host { lua, root: None, base, console, modules: Vec::new(), steps, limits, log, generation: 0, hooks: Vec::new(), ignored: Vec::new(), only: None, libraries: None })
    }

    /// Load every `*.luau` under `root` and watch it from now on ([`Host::poll`]).
    pub fn watch(&mut self, root: impl Into<PathBuf>) -> Vec<String> {
        self.root = Some(root.into());
        self.poll()
    }

    /// Enable local *.module.luau libraries for a folder game before watching it.
    pub fn enable_libraries(&mut self) { self.libraries = Some(Default::default()); }

    /// Skip these top-level folders of the root when scanning for modules
    /// (a game folder whose `sounds/` hold synth patches, not modules).
    pub fn ignore(&mut self, folders: &[&str]) { self.ignored = folders.iter().map(|f| f.to_string()).collect(); }

    /// Load and watch only the root's module `name` (a path relative to the
    /// root, such as `game.luau`): a one-file game in a folder of other things.
    pub fn only(&mut self, name: &str) { self.only = Some(name.to_string()); }

    /// A module from a string that never reloads (built-in content, tests).
    pub fn add_source(&mut self, name: &str, source: &str) -> Result<(), String> {
        let state = self.lua.create_table().map_err(err)?;
        self.modules.push(Module { dependencies: BTreeSet::new(), name: name.into(), path: None, stamp: None, exports: None, env: None, state, error: None, broken: false, proven: None, calls: 0 });
        let i = self.modules.len() - 1;
        self.load(i, source);
        self.modules[i].error.clone().map_or(Ok(()), Err)
    }

    /// Load a watched module from `source` instead of its file (a trial
    /// edit). The file is not touched; editing it on disk loads it again.
    /// Returns the load error, if any (the previous version keeps running).
    pub fn load_text(&mut self, name: &str, source: &str) -> Result<(), String> {
        let i = self.modules.iter().position(|m| m.name == name).ok_or_else(|| format!("no module {name}"))?;
        self.load(i, source);
        self.modules[i].error.clone().map_or(Ok(()), Err)
    }

    /// Reload what changed under the root: new and edited files load (in
    /// path order), deleted ones drop. Returns the names that changed.
    /// Load `name` again on the next poll even if its time and size match
    /// (an edit that keeps the length within the file system's clock tick).
    pub fn forget(&mut self, name: &str) {
        if let Some(libs) = &mut self.libraries { libs.forget(name); }
        if let Some(m) = self.modules.iter_mut().find(|m| m.name == name) { m.stamp = None; }
    }

    pub fn poll(&mut self) -> Vec<String> {
        let Some(root) = self.root.clone() else { return Vec::new() };
        let mut found = Vec::new();
        scan(&root, &root, &mut found);
        found.retain(|(name, _)| !self.ignored.iter().any(|f| name.strip_prefix(f.as_str()).is_some_and(|r| r.starts_with('/'))));
        if let Some(only) = &self.only { found.retain(|(name, _)| name == only); }
        found.sort();
        let edited = self.libraries.as_mut().map(|libs| libs.refresh(&root, &mut found)).unwrap_or_default();
        let mut changed = Vec::new();
        let before = self.modules.len();
        self.modules.retain(|m| m.path.is_none() || found.iter().any(|(n, _)| *n == m.name));
        if self.modules.len() != before { self.generation += 1; changed.push("(removed)".into()); }
        for (name, path) in found {
            let stamp = std::fs::metadata(&path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
            let i = match self.modules.iter().position(|m| m.name == name) {
                Some(i) if self.modules[i].stamp == stamp && self.modules[i].dependencies.is_disjoint(&edited) => continue,
                Some(i) => i,
                None => {
                    let Ok(state) = self.lua.create_table() else { continue };
                    self.modules.push(Module { dependencies: BTreeSet::new(), name: name.clone(), path: Some(path.clone()), stamp: None, exports: None, env: None, state, error: None, broken: false, proven: None, calls: 0 });
                    self.modules.len() - 1
                }
            };
            self.modules[i].stamp = stamp;
            match std::fs::read_to_string(&path) {
                Ok(src) => self.load(i, &src),
                Err(e) => self.modules[i].error = Some(format!("{name}: {e}")),
            }
            changed.push(name);
        }
        // Keep path order (the order hooks run in) after additions.
        self.modules.sort_by(|a, b| (a.path.is_some(), &a.name).cmp(&(b.path.is_some(), &b.name)));
        changed
    }

    fn load(&mut self, i: usize, source: &str) {
        let m = &self.modules[i];
        let name = m.name.clone();
        let mut dependencies = BTreeSet::new();
        let result = env(&self.lua, &self.base, &m.state).and_then(|e| {
            self.steps.store(self.limits.steps, Ordering::Relaxed);
            let value = match &self.libraries {
                Some(libs) => libs.eval(&self.lua, &self.base, &e, &name, source, &mut dependencies),
                None => self.lua.load(source).set_name(format!("@{name}")).set_environment(e.clone()).eval::<Value>(),
            };
            value.map(|v| (v, e))
        });
        let m = &mut self.modules[i];
        match result {
            Ok((Value::Table(t), e)) => { m.exports = Some(t); m.env = Some(e); m.error = None; m.broken = false; m.calls = 0; }
            Ok((Value::Nil, e)) => { m.exports = self.lua.create_table().ok(); m.env = Some(e); m.error = None; m.broken = false; m.calls = 0; }
            Ok((v, _)) => m.error = Some(format!("{name}: must return a table of exports, not a {}", v.type_name())),
            Err(e) => m.error = Some(short(&e)),
        }
        if m.error.is_none() { m.dependencies = dependencies; } else { m.dependencies.extend(dependencies); }
        match &m.error {
            None => { self.generation += 1; self.say(format!("loaded {name}")); }
            Some(e) => { let keep = if m.exports.is_some() { " (keeping the previous version)" } else { "" }; let e = format!("{e}{keep}"); self.say(e); }
        }
        self.rehook();
    }

    fn rehook(&mut self) {
        self.hooks = self.modules.iter().filter_map(|m| m.exports.as_ref()).flat_map(|t| t.pairs::<String, Value>().filter_map(|kv| kv.ok()).filter(|(_, v)| v.is_function()).map(|(k, _)| k).collect::<Vec<_>>()).collect();
        self.hooks.sort();
        self.hooks.dedup();
    }

    /// Changes on every successful load, reload or removal: the game
    /// re-reads exported data (cards, tuning) when this moves.
    pub fn generation(&self) -> u64 { self.generation }

    pub fn modules(&self) -> &[Module] { &self.modules }

    pub fn lua(&self) -> &Lua { &self.lua }

    /// Any loaded module exports a function under this name.
    pub fn has(&self, hook: &str) -> bool { self.hooks.binary_search_by(|h| h.as_str().cmp(hook)).is_ok() }

    /// The modules (in path order) that export `hook` and still run.
    pub fn each(&self, hook: &str) -> Vec<(usize, Function)> {
        if !self.has(hook) { return Vec::new(); }
        self.modules.iter().enumerate()
            .filter(|(_, m)| !m.broken)
            .filter_map(|(i, m)| m.exports.as_ref()?.raw_get::<Option<Function>>(hook).ok().flatten().map(|f| (i, f)))
            .collect()
    }

    /// Run one call on behalf of `module` with a fresh step budget. A
    /// failure is logged. A fresh edit that fails falls back to the version
    /// that ran before it (the error stays shown until the next good load);
    /// otherwise the module's hooks are off until it reloads.
    pub fn run<R>(&mut self, module: usize, what: &str, f: impl FnOnce(&Lua) -> mlua::Result<R>) -> Option<R> {
        self.steps.store(self.limits.steps, Ordering::Relaxed);
        match f(&self.lua) {
            Ok(r) => {
                let m = &mut self.modules[module];
                if m.calls < PROVEN_AFTER {
                    m.calls += 1;
                    if m.calls == PROVEN_AFTER && let (Some(x), Some(e)) = (&m.exports, &m.env) { m.proven = Some((x.clone(), e.clone())); }
                }
                Some(r)
            }
            Err(e) => {
                let m = &mut self.modules[module];
                let fresh = m.calls < PROVEN_AFTER;
                let line = match m.proven.clone().filter(|(x, _)| fresh && m.exports.as_ref() != Some(x)) {
                    Some((x, env)) => {
                        m.exports = Some(x);
                        m.env = Some(env);
                        m.calls = PROVEN_AFTER;
                        let line = format!("{} ({what}): {} (back to the version that ran before this edit)", m.name, short(&e));
                        m.error = Some(line.clone());
                        self.rehook();
                        line
                    }
                    None => {
                        m.broken = true;
                        format!("{} ({what}): {} (its hooks are off until the file changes)", m.name, short(&e))
                    }
                };
                self.say(line);
                None
            }
        }
    }

    /// Run Rust code that builds or reads Luau values (hook arguments, a
    /// returned table) with the memory cap lifted; see [`unguarded`].
    pub fn unguarded<R>(&self, f: impl FnOnce(&Lua) -> R) -> R { unguarded(&self.lua, self.limits.memory, f) }

    /// The limits this host was made with.
    pub fn limits(&self) -> Limits { self.limits }

    /// Steps left in the current call's budget.
    pub fn steps_left(&self) -> u32 { self.steps.load(Ordering::Relaxed) }

    /// A function for a console line: an expression (`1 + 2`) or statements,
    /// in an environment that persists between lines. The game binds its
    /// live objects into [`Host::console`] around the call.
    pub fn console_line(&mut self, line: &str) -> Result<Function, String> {
        self.steps.store(self.limits.steps, Ordering::Relaxed);
        let load = |src: &str| self.lua.load(src).set_name("=console").set_environment(self.console.clone()).into_function();
        load(&format!("return {line}")).or_else(|_| load(line)).map_err(|e| short(&e))
    }

    pub fn console(&self) -> &Table { &self.console }

    /// Let console lines see a module: its globals, then its exports (so
    /// `spawn("chaser", 100, 100)` or a function the module returns works),
    /// then the base library. Console assignments stay in the console.
    pub fn console_in(&self, module: Option<usize>) -> mlua::Result<()> {
        let m = module.and_then(|i| self.modules.get(i));
        let (env, exports) = (m.and_then(|m| m.env.clone()), m.and_then(|m| m.exports.clone()));
        let base = self.base.clone();
        let index = self.lua.create_function(move |_, (_, k): (Value, Value)| {
            for t in [&env, &exports].into_iter().flatten() {
                let v: Value = t.raw_get(k.clone())?;
                if !v.is_nil() { return Ok(v); }
            }
            base.raw_get::<Value>(k)
        })?;
        let meta = self.lua.create_table()?;
        meta.raw_set("__index", index)?;
        meta.raw_set("__metatable", false)?;
        self.console.set_metatable(Some(meta))?;
        Ok(())
    }

    /// Lines from `print` and from loads and failures, oldest first.
    pub fn drain_log(&mut self) -> Vec<String> { std::mem::take(&mut *self.log.lock().unwrap()) }

    fn say(&self, line: String) { self.log.lock().unwrap().push(line); }
}

/// A fresh module environment: its own globals, `state` kept across
/// reloads, everything else read from the frozen base.
fn env(lua: &Lua, base: &Table, state: &Table) -> mlua::Result<Table> {
    let e = lua.create_table()?;
    e.raw_set("state", state.clone())?;
    let meta = lua.create_table()?;
    meta.raw_set("__index", base.clone())?;
    meta.raw_set("__metatable", false)?;
    e.set_metatable(Some(meta))?;
    Ok(e)
}

fn scan(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() { scan(root, &p, out); continue; }
        // `*.d.luau` are type definitions, not modules.
        if p.extension().is_some_and(|e| e == "luau") && !p.to_string_lossy().ends_with(".d.luau") {
            let name = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push((name, p));
        }
    }
}

fn err(e: mlua::Error) -> String { short(&e) }

/// Run Rust code that builds or reads Luau values with the memory cap
/// lifted, then put `cap` back. While a cap is set, mlua guards every
/// allocating call against an out-of-memory error, which makes building a
/// table from Rust about four times slower (engine/docs/luau-or-rust.md).
/// The cap still holds for scripts as long as `f` runs no Luau: convert
/// values here, call hooks outside.
pub fn unguarded<R>(lua: &Lua, cap: usize, f: impl FnOnce(&Lua) -> R) -> R {
    struct Restore<'a>(&'a Lua, usize);
    impl Drop for Restore<'_> { fn drop(&mut self) { let _ = self.0.set_memory_limit(self.1); } }
    let _ = lua.set_memory_limit(0);
    let _restore = Restore(lua, cap);
    f(lua)
}

/// The message without the Rust-side wrapping and the stack traceback.
pub fn short(e: &mlua::Error) -> String {
    let mut e = e;
    loop {
        match e {
            mlua::Error::CallbackError { cause, .. } => e = cause,
            mlua::Error::WithContext { cause, .. } => e = cause,
            _ => break,
        }
    }
    let s = e.to_string();
    let s = s.split("\nstack traceback").next().unwrap_or(&s);
    s.trim_start_matches("runtime error: ").trim_start_matches("syntax error: ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host::new(Limits { memory: 8 << 20, steps: 10_000 }, |lua, api| api.raw_set("twice", lua.create_function(|_, x: f64| Ok(x * 2.0))?)).unwrap()
    }

    #[test]
    fn modules_get_the_api_and_return_exports() {
        let mut h = host();
        h.add_source("a.luau", "return { f = function(x) return twice(x) + 1 end }").unwrap();
        assert!(h.has("f") && !h.has("g"));
        let (m, f) = h.each("f").pop().unwrap();
        assert_eq!(h.run(m, "f", |_| f.call::<f64>(20.0)), Some(41.0));
    }

    #[test]
    fn sandbox_hides_clock_files_and_random() {
        let mut h = host();
        h.add_source("a.luau", "return { os = os, io = io, rnd = math.random, getfenv = getfenv, load = loadstring, req = require, floor = math.floor }").unwrap();
        let e = h.modules()[0].exports.clone().unwrap();
        for k in ["os", "io", "rnd", "getfenv", "load", "req"] { assert!(e.raw_get::<Value>(k).unwrap().is_nil(), "{k} visible"); }
        assert!(e.raw_get::<Value>("floor").unwrap().is_function());
        // The shared base and libraries are frozen.
        assert!(h.add_source("b.luau", "math.floor = nil").is_err());
        assert!(h.add_source("c.luau", "getmetatable(_G).__index.twice = nil").is_err());
    }

    #[test]
    fn runaway_loops_stop_at_the_budget() {
        let mut h = host();
        h.add_source("a.luau", "return { spin = function() while true do pcall(function() while true do end end) end end, ok = function() return 1 end }").unwrap();
        let (m, f) = h.each("spin").pop().unwrap();
        assert_eq!(h.run(m, "spin", |_| f.call::<()>(())), None);
        assert!(h.modules()[0].broken);
        assert!(h.drain_log().iter().any(|l| l.contains("step budget")));
        assert!(h.each("ok").is_empty(), "a broken module's hooks are off");
    }

    #[test]
    fn hot_reload_keeps_state_and_survives_bad_edits() {
        let dir = std::env::temp_dir().join(format!("warp-script-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("sub/m.luau");
        std::fs::write(&file, "state.n = (state.n or 0) + 1\nreturn { n = function() return state.n end, v = function() return 1 end }").unwrap();
        let mut h = host();
        assert_eq!(h.watch(&dir), vec!["sub/m.luau".to_string()]);
        let call = |h: &mut Host, k: &str| { let (m, f) = h.each(k).pop().unwrap(); h.run(m, k, |_| f.call::<f64>(())).unwrap() };
        assert_eq!(call(&mut h, "v"), 1.0);
        // Edit: new code, state carried over (n counts loads).
        std::fs::write(&file, "state.n = (state.n or 0) + 1\nreturn { n = function() return state.n end, v = function() return 2 end }   ").unwrap();
        assert_eq!(h.poll(), vec!["sub/m.luau".to_string()]);
        assert_eq!((call(&mut h, "v"), call(&mut h, "n")), (2.0, 2.0));
        // A syntax error keeps version 2 running and reports the line.
        std::fs::write(&file, "return {\n v = function() return 3 end,\n oops oops }").unwrap();
        h.poll();
        assert_eq!(call(&mut h, "v"), 2.0);
        let e = h.modules()[0].error.clone().unwrap();
        assert!(e.contains("sub/m.luau:3"), "{e}");
        assert!(h.poll().is_empty(), "unchanged files do not reload");
        assert_eq!(h.modules()[0].state().get::<f64>("n").unwrap(), 2.0);
        // Ignored folders hold no modules.
        std::fs::create_dir_all(dir.join("sounds")).unwrap();
        std::fs::write(dir.join("sounds/beep.luau"), "return 1").unwrap();
        h.ignore(&["sounds"]);
        assert!(h.poll().is_empty(), "sounds/ is ignored");
        std::fs::remove_file(&file).unwrap();
        h.poll();
        assert!(h.modules().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_edit_that_fails_falls_back_to_the_version_before() {
        let dir = std::env::temp_dir().join(format!("warp-script-back-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("m.luau");
        std::fs::write(&file, "return { v = function() return 1 end }").unwrap();
        let mut h = host();
        h.watch(&dir);
        let call = |h: &mut Host| { let (m, f) = h.each("v").pop()?; h.run(m, "v", |_| f.call::<f64>(())) };
        for _ in 0..PROVEN_AFTER { assert_eq!(call(&mut h), Some(1.0)); }
        std::fs::write(&file, "return { v = function() return nil + 1 end }   ").unwrap();
        h.poll();
        assert_eq!(call(&mut h), None, "the edit fails");
        assert!(!h.modules()[0].broken);
        assert!(h.modules()[0].error.as_deref().unwrap().contains("back to the version"));
        assert_eq!(call(&mut h), Some(1.0), "the version before runs again");
        std::fs::write(&file, "return { v = function() return 3 end }").unwrap();
        h.poll();
        assert_eq!(call(&mut h), Some(3.0));
        assert!(h.modules()[0].error.is_none(), "a good load clears the message");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_console_sees_a_modules_globals_and_exports() {
        let mut h = host();
        h.add_source("a.luau", "function spawn(n) return n + 1 end local function hidden() end return { peek = function() return 7 end }").unwrap();
        assert!(h.console_line("spawn").unwrap().call::<Value>(()).unwrap().is_nil());
        h.console_in(Some(0)).unwrap();
        assert_eq!(h.console_line("spawn(41)").unwrap().call::<f64>(()).unwrap(), 42.0);
        assert_eq!(h.console_line("peek()").unwrap().call::<f64>(()).unwrap(), 7.0);
        assert_eq!(h.console_line("twice(2)").unwrap().call::<f64>(()).unwrap(), 4.0);
        assert!(h.console_line("hidden").unwrap().call::<Value>(()).unwrap().is_nil());
    }

    #[test]
    fn console_lines_are_expressions_or_statements() {
        let mut h = host();
        let f = h.console_line("x = 20").unwrap();
        f.call::<()>(()).unwrap();
        let f = h.console_line("twice(x) + 2").unwrap();
        assert_eq!(f.call::<f64>(()).unwrap(), 42.0);
        assert!(h.console_line("1 +").is_err());
    }

    #[test]
    fn memory_cap_holds() {
        let mut h = host();
        let r = h.add_source("a.luau", "local s = string.rep('x', 1e8) return {}");
        assert!(r.is_err());
    }

    #[test]
    fn unguarded_puts_the_cap_back() {
        let mut h = host();
        let t = h.unguarded(|lua| lua.create_table_from((1..=100).map(|i| (i, i)))).unwrap();
        assert_eq!(t.raw_len(), 100);
        // The cap is back for scripts after the conversion.
        assert!(h.add_source("b.luau", "local s = string.rep('x', 1e8) return {}").is_err());
    }
}
