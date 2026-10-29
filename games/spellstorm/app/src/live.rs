//! Turning Spellstorm while it runs. Always: `knobs.txt` (the sim's balance
//! knobs, spellstorm::knobs) and the game's own `surface.wgsl` are read from
//! the repository when it is there, and an edit of either applies on the
//! next frame (a shader that does not compile keeps the last good one).
//! With `play --live [PORT]`: MCP (engine/crates/live, docs in
//! engine/docs/live-control.md) with the state, screenshots, logs, the
//! knobs (set_knob, trial, keep, revert), new runs, levels and settings.
use crate::play::Spellstorm;
use live::{Content, Features, LiveGame, Manifest, Reload, Reply, Root, ToolSpec};
use serde_json::{json, Value};
use spellstorm::knobs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const SURFACE: &str = include_str!("surface.wgsl");

/// games/spellstorm in the repository: SPELLSTORM_DIR, else found from the
/// working directory or the program upward (a build in target/ finds it).
pub fn game_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("SPELLSTORM_DIR").map(PathBuf::from).filter(|d| d.is_dir()) {
        return Some(d);
    }
    let starts = [std::env::current_dir().ok(), std::env::current_exe().ok()];
    for s in starts.into_iter().flatten() {
        for a in s.ancestors() {
            for d in [a.join("games/spellstorm"), a.to_path_buf()] {
                if d.join("app/src/surface.wgsl").is_file() && d.join("sim").is_dir() {
                    return Some(d);
                }
            }
        }
    }
    None
}

fn stamp(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// knobs.txt, watched.
pub struct Tuning {
    pub path: Option<PathBuf>,
    seen: Option<SystemTime>,
    /// Some values are set only in the running game (`trial`).
    pub trial: bool,
}

impl Tuning {
    pub fn new(dir: Option<&Path>) -> Tuning {
        let mut t = Tuning { path: dir.map(|d| d.join("knobs.txt")), seen: None, trial: false };
        if let Some(p) = &t.path {
            if !p.exists() {
                let _ = std::fs::write(p, knobs::file_text());
            }
        }
        t.scan();
        t
    }

    /// Load the file when it changed since last time: log lines, if any.
    pub fn scan(&mut self) -> Option<String> {
        let p = self.path.clone()?;
        let s = stamp(&p);
        if s == self.seen {
            return None;
        }
        self.seen = s;
        let text = std::fs::read_to_string(&p).ok()?;
        let errors = knobs::load(&text);
        self.trial = false;
        Some(if errors.is_empty() { "knobs.txt: loaded".into() } else { format!("knobs.txt: loaded, but {}", errors.join("; ")) })
    }

    /// Set a knob; `persist` writes it into knobs.txt too.
    pub fn set(&mut self, name: &str, value: f64, persist: bool) -> Result<String, String> {
        let old = knobs::value(name).ok_or_else(|| format!("no knob called {name} (knobs lists them)"))?;
        if !knobs::set(name, value) {
            return Err(format!("{value} is not a number"));
        }
        if !persist {
            self.trial = true;
            return Ok(format!("{name}: {old} -> {value} (only in the running game: keep saves it, revert undoes it)"));
        }
        let p = self.path.clone().ok_or("no knobs.txt (the game runs outside the repository); use persist false")?;
        let text = std::fs::read_to_string(&p).unwrap_or_else(|_| knobs::file_text());
        let mut found = false;
        let lines: Vec<String> = text
            .lines()
            .map(|l| {
                let (body, note) = l.split_once('#').map_or((l, None), |(b, n)| (b, Some(n)));
                if body.split_once('=').is_some_and(|(n, _)| n.trim() == name) {
                    found = true;
                    match note {
                        Some(n) => format!("{name} = {value}  #{n}"),
                        None => format!("{name} = {value}"),
                    }
                } else {
                    l.to_string()
                }
            })
            .collect();
        let mut out = lines.join("\n") + "\n";
        if !found {
            let doc = knobs::LIST.iter().find(|k| k.0 == name).map_or("", |k| k.2.trim());
            out += &format!("{name} = {value}  # {doc}\n");
        }
        std::fs::write(&p, out).map_err(|e| e.to_string())?;
        self.seen = stamp(&p);
        Ok(format!("{name}: {old} -> {value} (saved in knobs.txt)"))
    }

    /// Save every value now (trials too) into knobs.txt.
    pub fn keep(&mut self) -> Result<String, String> {
        if !self.trial {
            return Ok("nothing temporary to keep".into());
        }
        let p = self.path.clone().ok_or("no knobs.txt (the game runs outside the repository)")?;
        std::fs::write(&p, knobs::file_text()).map_err(|e| e.to_string())?;
        self.seen = stamp(&p);
        self.trial = false;
        Ok("kept: knobs.txt holds the values now".into())
    }

    /// Back to knobs.txt as saved.
    pub fn revert(&mut self) -> String {
        if !self.trial {
            return "nothing temporary to undo".into();
        }
        self.seen = None;
        self.scan().unwrap_or_else(|| {
            knobs::reset();
            "back to the defaults".into()
        })
    }

    pub fn list(&self) -> String {
        let mut s = String::new();
        for k in knobs::LIST {
            let v = knobs::value(k.0).unwrap_or(k.1);
            let mark = if v != k.1 { format!(" (default {})", k.1) } else { String::new() };
            s += &format!("{} = {v}{mark}  # {}\n", k.0, k.2.trim());
        }
        if self.trial {
            s += "(some values are temporary: keep saves them, revert undoes them)\n";
        }
        s
    }
}

/// The game's surface.wgsl, watched: an edit compiles on the next frame.
pub struct Shader {
    path: Option<PathBuf>,
    seen: Option<SystemTime>,
    pub good: String,
    pending: Option<String>,
    pub error: Option<String>,
}

impl Shader {
    pub fn new(dir: Option<&Path>) -> Shader {
        let mut s = Shader { path: dir.map(|d| d.join("app/src/surface.wgsl")), seen: None, good: SURFACE.to_string(), pending: None, error: None };
        s.seen = s.path.as_deref().and_then(stamp);
        // The file on disk may be newer than this build.
        if let Some(text) = s.path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            s.good = text;
        }
        s
    }

    pub fn scan(&mut self) -> Option<String> {
        let p = self.path.clone()?;
        let st = stamp(&p);
        if st == self.seen {
            return None;
        }
        self.seen = st;
        let text = std::fs::read_to_string(&p).ok()?;
        if text == self.good && self.error.is_none() {
            return None;
        }
        self.pending = Some(text);
        Some("surface.wgsl: compiles on the next frame".into())
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Compile the pending source with `reload`; on an error the last good
    /// one goes back in. Returns a log line.
    pub fn apply(&mut self, mut reload: impl FnMut() -> Result<(), String>) -> Option<String> {
        let new = self.pending.take()?;
        render::set_game_surface(&new);
        match reload() {
            Ok(()) => {
                self.good = new;
                self.error = None;
                Some("surface.wgsl: compiled".into())
            }
            Err(e) => {
                render::set_game_surface(&self.good);
                let line = format!("surface.wgsl: {e}");
                self.error = Some(line.clone());
                if let Err(e) = reload() {
                    return Some(format!("{line}; the last good version failed too: {e}"));
                }
                Some(format!("{line} (the last good version keeps running)"))
            }
        }
    }
}

pub struct SsLive {
    pub live: live::Live,
}

/// `--live [PORT]` (see live::Config::from_args).
pub fn start(args: &[String], dir: Option<&Path>) -> Result<Option<SsLive>, String> {
    let Some(cfg) = live::Config::from_args("spellstorm", args)? else { return Ok(None) };
    let mut roots = Vec::new();
    if let Some(d) = dir {
        roots.push(Root::new("game", d, Reload::Game, "games/spellstorm: knobs.txt (the balance knobs) and app/src/surface.wgsl (the game's floor, stone and cloth shading); a write applies on the next frame, a shader that does not compile keeps the last good one").extensions(&["txt", "wgsl", "md"]));
    }
    let shaders = render::shaders::dir();
    if let Some(d) = &shaders {
        roots.push(Root::new("shaders", d, Reload::Shaders, "the engine's WGSL shading chain from KERN_SHADER_DIR; a write recompiles it on the next frame, a failure keeps the old shaders").extensions(&["wgsl"]));
    }
    let manifest = Manifest {
        name: "Spellstorm".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        about: "A co-op wizard roguelite on Warp (a port of the Godot game): up to 4 wizards combine elements into spells (RT cast, RB sword and nova, LS trap, RS enchant) through biomes of monsters, shrines of tomes and a boss each. 60 Hz deterministic Rust sim (hecs); the balance numbers are live knobs.".into(),
        guide: format!(
            "- Look first: `state`, `screenshot`, `knobs`, `logs`.\n\
             - Balance: `knobs` lists them; `set_knob` changes one now and saves it in knobs.txt (`persist: false` only in the running game, then `keep` or `revert`). Walking and rolling speeds, cooldowns and reach, mana, damage and health scales, wave spacing, revives, tome time and the mist walls' rules (wards, ward_needs_tomes, practice).\n\
             - Runs: `new_run`, `start_at` (a biome from 0), `setting` (friendly_fire off|half|full, difficulty easy|normal|hard, unlocks earned|all), `command` (the same as a console line; `help`).\n\
             - Shading: edit `game/app/src/surface.wgsl` with write_file (Spellstorm's floor, stone and cloth, ported from Godot's shaders); it compiles on the next frame. {}\n\
             - Spells, monsters and rules are Rust (games/spellstorm/sim): they change with a rebuild, not live.",
            if shaders.is_some() { "The engine's own shaders are in `shaders/` (reload_shaders)." } else { "Start with KERN_SHADER_DIR=engine/crates/render/src/shaders to edit the engine's shaders too." }
        ),
        tools: tools(),
        roots,
        features: Features { state: true, screenshot: true, luau: false, shaders: shaders.is_some(), sound: false },
    };
    Ok(Some(SsLive { live: live::start(cfg, manifest)? }))
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new("knobs", "Every balance knob: name, value now (and its default when changed) and what it does.", obj(json!({}), &[])).read_only(),
        ToolSpec::new("set_knob", "Set one knob now. With persist (the default) the value is saved in knobs.txt, ready to commit; with persist false only the running game changes (`keep` saves, `revert` undoes).",
            obj(json!({ "name": { "type": "string" }, "value": { "type": "number" }, "persist": { "type": "boolean" } }), &["name", "value"])),
        ToolSpec::new("keep", "Save the temporary knob values (set_knob with persist false) into knobs.txt.", obj(json!({}), &[])),
        ToolSpec::new("revert", "Undo the temporary knob values: back to knobs.txt as saved.", obj(json!({}), &[])),
        ToolSpec::new("new_run", "Start a fresh run for everyone, at the first biome.", obj(json!({}), &[])),
        ToolSpec::new("start_at", "Start a run at a biome (0 the chapel; the state lists the run).", obj(json!({ "biome": { "type": "integer", "minimum": 0 } }), &["biome"])),
        ToolSpec::new("setting", "A party setting: friendly_fire off|half|full, difficulty easy|normal|hard, unlocks earned|all.",
            obj(json!({ "key": { "type": "string" }, "value": { "type": "string" } }), &["key", "value"])),
        ToolSpec::new("command", "One console line: knobs, knob NAME VALUE (saved), trial NAME VALUE (temporary), keep, revert, new_run, biome N, skip (past this level's fights to its shrine and portal), setting KEY VALUE, help.",
            obj(json!({ "line": { "type": "string" } }), &["line"])),
    ]
}

impl Spellstorm {
    /// A console line (live `command`, and the shell's `command`).
    pub fn console(&mut self, line: &str) -> Result<String, String> {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let rest: Vec<&str> = words.collect();
        let num = |i: usize| -> Result<f64, String> { rest.get(i).ok_or("missing a value")?.parse::<f64>().map_err(|e| e.to_string()) };
        let out = match cmd {
            "knobs" => self.tuning.list(),
            "knob" | "trial" => {
                let name = rest.first().ok_or("knob NAME VALUE")?;
                let r = self.tuning.set(name, num(1)?, cmd == "knob")?;
                self.world.knobs_changed();
                r
            }
            "keep" => self.tuning.keep()?,
            "revert" => {
                let r = self.tuning.revert();
                self.world.knobs_changed();
                r
            }
            "new_run" => {
                self.world.new_run();
                "a new run".into()
            }
            "biome" => {
                let b = num(0)? as i32;
                self.world.start_at(b);
                format!("a run at biome {b}")
            }
            "skip" => {
                self.world.skip_level();
                "the level's fights skipped: shrine and portal up".into()
            }
            "setting" => {
                let (k, v) = (rest.first().ok_or("setting KEY VALUE")?, rest.get(1).ok_or("setting KEY VALUE")?);
                self.world.set_option(k, v);
                format!("{k} = {v}")
            }
            "help" | "" => "knobs | knob NAME VALUE (saved in knobs.txt) | trial NAME VALUE (only the running game) | keep | revert | new_run | biome N | skip (this level's fights) | setting KEY VALUE".into(),
            _ => return Err(format!("unknown command {cmd} (help lists them)")),
        };
        Ok(out)
    }

    /// Between ticks: the files on disk, then the live requests.
    pub fn live_tick(&mut self) {
        self.scan_t += 1;
        if self.scan_t % 15 == 0 {
            if let Some(line) = self.tuning.scan() {
                self.world.knobs_changed();
                self.log(line);
            }
            if let Some(line) = self.shader.scan() {
                self.log(line);
            }
        }
        let Some(mut l) = self.live.take() else { return };
        l.live.serve(&mut Ctx { g: self });
        self.live = Some(l);
    }

    pub fn log(&self, line: String) {
        match &self.live {
            Some(l) => l.live.log(line.clone()),
            None => {}
        }
        println!("{line}");
    }

    /// On a drawn frame: a pending surface.wgsl, and the live shader reloads.
    pub fn live_frame(&mut self, frame: &mut app::Frame) {
        let gpu = frame.gpu;
        if self.shader.pending() && frame.renderer().is_some() {
            let line = self.shader.apply(|| frame.renderer_mut().ok_or("no renderer yet".to_string())?.reload_shaders(gpu).map(|_| ()));
            if let Some(line) = line {
                self.log(line);
            }
        }
        if let Some(l) = self.live.as_mut() {
            if l.live.wants_shader_reload() {
                let result = match frame.renderer_mut() {
                    Some(r) => r.reload_shaders(gpu),
                    None => Err("no renderer yet".into()),
                };
                l.live.shaders_reloaded(result);
            }
        }
    }

    pub fn live_wants_capture(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.live.wants_capture() || l.live.wants_shader_reload())
    }

    pub fn live_captured(&mut self, size: [u32; 2], rgba: Vec<u8>) {
        if let Some(l) = self.live.as_mut() {
            l.live.captured(size, rgba);
        }
    }
}

struct Ctx<'a> {
    g: &'a mut Spellstorm,
}

fn text(t: String) -> Reply {
    Ok(vec![Content::Text(t)])
}

impl LiveGame for Ctx<'_> {
    fn state(&mut self) -> Result<Value, String> {
        let mut v = self.g.world.snapshot();
        v["knobs"] = knobs::LIST.iter().map(|k| (k.0.to_string(), json!(knobs::value(k.0)))).collect::<serde_json::Map<_, _>>().into();
        if let Some(e) = &self.g.shader.error {
            v["shader_error"] = json!(e);
        }
        Ok(v)
    }

    fn call(&mut self, tool: &str, args: &Value) -> Reply {
        let g = &mut *self.g;
        match tool {
            "knobs" => text(g.tuning.list()),
            "set_knob" => {
                let name = args["name"].as_str().ok_or("set_knob needs name")?;
                let value = args["value"].as_f64().or_else(|| args["value"].as_str().and_then(|s| s.parse().ok())).ok_or("set_knob needs a number value")?;
                let r = g.tuning.set(name, value, args["persist"].as_bool().unwrap_or(true))?;
                g.world.knobs_changed();
                text(r)
            }
            "keep" => text(g.tuning.keep()?),
            "revert" => text(g.console("revert")?),
            "new_run" => text(g.console("new_run")?),
            "start_at" => text(g.console(&format!("biome {}", args["biome"].as_i64().ok_or("start_at needs biome")?))?),
            "setting" => text(g.console(&format!("setting {} {}", args["key"].as_str().ok_or("setting needs key")?, args["value"].as_str().ok_or("setting needs value")?))?),
            "command" => text(g.console(args["line"].as_str().ok_or("command needs line")?)?),
            _ => Err(format!("unknown tool {tool}")),
        }
    }

    fn file_written(&mut self, _root: &str, path: &str) -> Option<String> {
        let g = &mut *self.g;
        if path.ends_with("knobs.txt") {
            let line = g.tuning.scan();
            g.world.knobs_changed();
            return line;
        }
        if path.ends_with("surface.wgsl") {
            return g.shader.scan().or(Some("surface.wgsl: unchanged".into()));
        }
        None
    }
}
