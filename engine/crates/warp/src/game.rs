//! A game folder as an `app::Game`: the Luau modules' hooks at a fixed
//! tick, the world drawn from the entities, hot reload of scripts and
//! sound patches, debug commands, and live control over MCP.
use crate::api::{self, Cam, Core, HudItem, Phase, Rng, Shared};
use crate::config::GameToml;
use crate::luajson;
use crate::sky::SkyChoice;
use crate::sounds::{check_params, Bank, SoundOut};
use crate::world::World;
use app::{Buttons, Controls, Draw, Frame, Game};
use audio::synth::{Ear, SoundSender};
use glam::Vec3;
use render::atmosphere::{AtmosphereCache, BakeQuality};
use render::hud::{FontFace, HudDrawList, TextRun};
use render::sky::{bake_live, PreparedSky};
use render::{primitives, Camera, Lighting};
use script::mlua::{self, Function, Table, Value};
use script::{Host, Limits};
use serde_json::{json, Value as Json};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Top-level folders of a game that hold no Luau modules.
pub const NOT_MODULES: &[&str] = &["sounds", "assets", "scripts"];

/// How often (in ticks) files are checked for edits: about 4 times a second at 60 Hz.
const POLL_TICKS: u64 = 15;

/// Options from the command line that override `game.toml`.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub players: Option<usize>,
    pub seed: Option<u64>,
    pub sky: Option<String>,
}

pub struct LuaGame {
    pub dir: PathBuf,
    pub cfg: GameToml,
    host: Host,
    core: Shared,
    /// Modules whose `init` has run (by name), so a reload does not re-run it.
    inited: BTreeSet<String>,
    /// The light of the applied sky, before `light.set` overrides.
    light: Lighting,
    applied_sky: Option<SkyChoice>,
    sky_upload: Option<PreparedSky>,
    log: VecDeque<String>,
    pub live: Option<live::Live>,
    /// The `logs` tool's buffer (usable while `live` is out serving).
    live_logs: Option<live::Logs>,
    seed: u64,
    default_sky: SkyChoice,
    /// Print log lines to stdout (off for tests that read `logs()`).
    pub echo: bool,
}

const LOG_KEEP: usize = 200;

impl LuaGame {
    /// Load a game folder. `sound` is where sounds go (the speakers, an
    /// offline mixer, or None for silence).
    pub fn load(dir: &Path, over: &Overrides, sound: Option<SoundSender>) -> Result<Self, String> {
        let mut cfg = GameToml::load(dir)?;
        if let Some(p) = over.players {
            if !(1..=4).contains(&p) { return Err(format!("--players {p}: 1 to 4")); }
            cfg.players = p;
        }
        if let Some(s) = over.seed { cfg.seed = s; }
        let default_sky = SkyChoice::parse(over.sky.as_deref().unwrap_or(&cfg.sky))?;
        let sounds_dir = dir.join("sounds");
        let mut bank = Bank::new(sounds_dir.is_dir().then_some(sounds_dir));
        let lines = bank.scan();
        let core: Shared = Arc::new(Mutex::new(Core {
            world: World::new(dir, cfg.max_entities),
            sounds: SoundOut::new(bank, sound),
            cams: vec![Cam::default(); cfg.players],
            hud: Vec::new(),
            screen: cfg.size,
            views: Vec::new(),
            rng: Rng(cfg.seed),
            sky: default_sky,
            light: Default::default(),
            tick: 0,
            tick_rate: cfg.tick_rate,
            players: cfg.players,
            title: cfg.title.clone(),
            phase: Phase::Sim,
        }));
        let shared = core.clone();
        let mut host = Host::new(Limits { memory: cfg.memory_mb << 20, steps: cfg.steps }, move |lua, base| api::register(lua, base, shared))?;
        host.ignore(NOT_MODULES);
        let mut g = LuaGame { dir: dir.to_path_buf(), seed: cfg.seed, cfg, host, core, inited: BTreeSet::new(), light: Lighting::default(), applied_sky: None, sky_upload: None,
            log: VecDeque::new(), live: None, live_logs: None, default_sky, echo: true };
        for l in lines { g.say(l); }
        g.host.watch(dir.to_path_buf());
        g.flush_log();
        g.start_new();
        Ok(g)
    }

    pub fn core(&self) -> std::sync::MutexGuard<'_, Core> { api::lock(&self.core) }

    fn say(&mut self, line: String) {
        if self.echo { println!("{line}"); }
        self.log.push_back(line.clone());
        while self.log.len() > LOG_KEEP { self.log.pop_front(); }
        if let Some(l) = &self.live_logs { l.push(line); }
    }

    fn flush_log(&mut self) { for l in self.host.drain_log() { self.say(l); } }

    /// Recent log lines: prints, loads, errors, reloads.
    pub fn logs(&self) -> Vec<String> { self.log.iter().cloned().collect() }

    /// Run `hook` in every module that exports it, in path order.
    fn hook(&mut self, hook: &str, args: impl Fn() -> mlua::Result<mlua::MultiValue>) {
        for (m, f) in self.host.each(hook) { self.call_one(m, hook, &f, &args); }
    }

    fn call_one(&mut self, m: usize, hook: &str, f: &Function, args: &impl Fn() -> mlua::Result<mlua::MultiValue>) {
        self.host.run(m, hook, |_| f.call::<()>(args()?));
        self.flush_log();
    }

    /// `init` for every loaded module that has not had it yet.
    fn start_new(&mut self) {
        let fresh: Vec<(usize, String)> = self.host.modules().iter().enumerate()
            .filter(|(_, m)| m.exports.is_some() && !self.inited.contains(&m.name)).map(|(i, m)| (i, m.name.clone())).collect();
        for (i, name) in fresh {
            self.inited.insert(name);
            let f = self.host.modules()[i].exports.as_ref().and_then(|e| e.raw_get::<Option<Function>>("init").ok().flatten());
            if let Some(f) = f { self.call_one(i, "init", &f, &|| Ok(mlua::MultiValue::new())); }
        }
    }

    /// Reload edited scripts and sound patches now. Returns what changed.
    pub fn reload(&mut self) -> Vec<String> {
        let mut lines = self.core().sounds.bank.scan();
        let changed = self.host.poll();
        self.flush_log();
        // Changed modules that already ran init get `reload()` instead.
        for name in &changed {
            let Some(i) = self.host.modules().iter().position(|m| &m.name == name) else { continue };
            let m = &self.host.modules()[i];
            if m.error.is_some() || !self.inited.contains(name) { continue; }
            if let Some(f) = m.exports.as_ref().and_then(|e| e.raw_get::<Option<Function>>("reload").ok().flatten()) {
                self.call_one(i, "reload", &f, &|| Ok(mlua::MultiValue::new()));
            }
        }
        self.inited.retain(|n| self.host.modules().iter().any(|m| &m.name == n));
        self.start_new();
        lines.extend(changed.iter().map(|n| match self.host.modules().iter().find(|m| &m.name == n) {
            Some(m) => match &m.error { Some(e) => format!("{n}: {e}"), None => format!("{n}: reloaded") },
            None => format!("{n}"),
        }));
        for l in &lines { if l.starts_with("sounds/") { self.say(l.clone()); } }
        lines
    }

    /// Start over: clear the world and every module's `state`, reseed, run `init`.
    pub fn restart(&mut self) {
        {
            let mut c = self.core();
            c.world.clear();
            c.sounds.stop_all();
            c.rng = Rng(self.seed);
            c.tick = 0;
            c.sky = self.default_sky;
            c.light = Default::default();
            let n = c.players;
            c.cams = vec![Cam::default(); n];
        }
        for m in self.host.modules() { let _ = m.state().clear(); }
        self.inited.clear();
        self.say("restarted".into());
        self.start_new();
    }

    fn main_state(&self) -> Option<Table> {
        let m = self.host.modules();
        m.iter().find(|m| m.name == "main.luau").or(m.first()).map(|m| m.state().clone())
    }

    /// Errors to show: load errors and modules whose hooks are off.
    pub fn errors(&self) -> Vec<String> {
        let mut out: Vec<String> = self.host.modules().iter().filter_map(|m| m.error.clone().or_else(|| m.broken.then(|| format!("{}: a hook failed; its hooks are off until the file changes (see logs)", m.name)))).collect();
        out.extend(self.core().sounds.bank.errors.iter().map(|(f, e)| format!("sounds/{f}: {e}")));
        if self.host.modules().is_empty() { out.push("no Luau modules loaded".into()); }
        out
    }

    pub fn state_json(&self) -> Json {
        let errors = self.errors();
        let game = self.main_state().map(|t| luajson::to_json(&Value::Table(t))).unwrap_or(Json::Null);
        let modules: Vec<Json> = self.host.modules().iter().map(|m| json!({ "name": m.name, "error": m.error, "broken": m.broken })).collect();
        let c = self.core();
        let mut tags = std::collections::BTreeMap::<&str, u32>::new();
        for e in c.world.entities.values() { *tags.entry(e.tag.as_str()).or_default() += 1; }
        let cams: Vec<Json> = c.cams.iter().map(|k| json!({ "pos": k.pos.to_array(), "target": k.target.to_array(), "fov": k.fov })).collect();
        let sounds = json!({ "loaded": c.sounds.bank.patches.keys().collect::<Vec<_>>(), "played": c.sounds.played,
            "recent": c.sounds.recent.iter().map(|(n, t)| json!({ "sound": n, "tick": t })).collect::<Vec<_>>() });
        json!({
            "title": c.title, "tick": c.tick, "time": c.tick as f64 / c.tick_rate as f64, "players": c.players,
            "sky": c.sky.name(), "entities": c.world.entities.len(), "max_entities": c.world.capacity(), "tags": tags,
            "cameras": cams, "sounds": sounds, "modules": modules, "errors": errors, "state": game,
        })
    }

    /// One Luau chunk in the console, with `state` bound to main.luau's.
    pub fn luau(&mut self, code: &str) -> Result<String, String> {
        if let Some(s) = self.main_state() { let _ = self.host.console().raw_set("state", s); }
        let f = self.host.console_line(code)?;
        let r = f.call::<mlua::MultiValue>(()).map_err(|e| script::short(&e));
        self.flush_log();
        let vals = r?;
        Ok(if vals.is_empty() { "ok".into() } else { vals.iter().map(luajson::show).collect::<Vec<_>>().join("\t") })
    }

    pub fn help() -> &'static str {
        "commands: lua CODE (run Luau against the game; `state` is main.luau's) | reload (scripts and sounds now) | restart (clear the world and state, run init) | sky NAME | sounds | entities [TAG] | logs | help"
    }

    fn input(&self, lua: &mlua::Lua, controls: &[Controls]) -> mlua::Result<Table> {
        const NAMES: &[(&str, u32)] = &[
            ("south", Buttons::SOUTH), ("east", Buttons::EAST), ("west", Buttons::WEST), ("north", Buttons::NORTH),
            ("left_shoulder", Buttons::LEFT_SHOULDER), ("right_shoulder", Buttons::RIGHT_SHOULDER), ("left_trigger", Buttons::LEFT_TRIGGER),
            ("right_trigger", Buttons::RIGHT_TRIGGER), ("select", Buttons::SELECT), ("start", Buttons::START),
            ("up", Buttons::UP), ("down", Buttons::DOWN), ("left", Buttons::LEFT), ("right", Buttons::RIGHT),
            ("jump", Buttons::SOUTH), ("fire", Buttons::RIGHT_TRIGGER), ("aim", Buttons::LEFT_TRIGGER),
        ];
        let all = lua.create_table()?;
        for (i, c) in controls.iter().enumerate() {
            let p = lua.create_table()?;
            p.raw_set("move_x", c.mv[0])?;
            p.raw_set("move_y", c.mv[1])?;
            p.raw_set("look_x", c.look[0])?;
            p.raw_set("look_y", c.look[1])?;
            let (held, pressed) = (lua.create_table()?, lua.create_table()?);
            for &(n, b) in NAMES { held.raw_set(n, c.held.has(b))?; pressed.raw_set(n, c.pressed.has(b))?; }
            p.raw_set("held", held)?;
            p.raw_set("pressed", pressed)?;
            all.raw_set(i + 1, p)?;
        }
        Ok(all)
    }

    /// The sky the scripts asked for, applied to the scene and the light.
    fn update_sky(&mut self) {
        let want = self.core().sky;
        if self.applied_sky == Some(want) { return; }
        self.applied_sky = Some(want);
        let mut c = api::lock(&self.core);
        match want {
            SkyChoice::Gradient => {
                c.world.scene.sky = Some(primitives::gradient_sky([0.25, 0.45, 0.8], [0.75, 0.82, 0.9], [0.35, 0.33, 0.3]));
                self.light = Lighting::default();
                c.world.rebuild = true;
                self.sky_upload = None;
            }
            SkyChoice::Preset(settings) => {
                static CACHE: std::sync::OnceLock<Mutex<AtmosphereCache>> = std::sync::OnceLock::new();
                let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
                match bake_live(0, settings, &mut cache, BakeQuality::default()) {
                    Ok(p) => {
                        let mut light = Lighting::default();
                        p.apply_scene(&mut c.world.scene, &mut light);
                        self.light = light;
                        self.sky_upload = Some(p);
                    }
                    Err(e) => { drop(c); self.say(format!("sky {}: {e}", want.name())); }
                }
            }
        }
    }

    fn lighting(&self) -> Lighting {
        let mut l = self.light.clone();
        let o = self.core().light;
        if let Some(s) = o.sun { l.sun_dir = s; }
        if let Some(v) = o.sun_lux { l.sun_lux = v; }
        if let Some(v) = o.sky_lux { l.sky_lux = v; }
        if let Some(v) = o.ev100 { l.ev100 = v; }
        if let Some(v) = o.fog[0] { l.fog_start = v; }
        if let Some(v) = o.fog[1] { l.fog_end = v; }
        if let Some(v) = o.fog[2] { l.fog_density = v; }
        l
    }

    fn hud(&self, size: [u32; 2]) -> HudDrawList {
        let mut list = HudDrawList::new(size);
        let full = [0.0, 0.0, size[0] as f32, size[1] as f32];
        let rgba = |c: [f32; 4]| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        for (k, item) in self.core().hud.iter().enumerate() {
            match item {
                HudItem::Box { rect, color } => list.rect(*rect, *color, full),
                HudItem::Text { text, pos, size: fs, color, align, width } => list.text.push(TextRun {
                    id: k as u64 + 1, text: text.clone(), position: *pos, bounds: full, font_size: *fs, color: rgba(*color), font: FontFace::Body, align: *align,
                    width: if *width > 0.0 { *width } else { (size[0] as f32 - pos[0]).max(fs * 4.0) },
                }),
            }
        }
        // Script errors stay on screen until fixed: live iteration needs to see them.
        let errors = self.errors();
        let s = (size[1] as f32 / 720.0).max(0.5);
        for (i, e) in errors.iter().take(6).enumerate() {
            list.text.push(TextRun { id: 10_000 + i as u64, text: e.chars().take(200).collect(), position: [12.0 * s, size[1] as f32 - (24.0 + 22.0 * (errors.len().min(6) - i) as f32) * s],
                bounds: full, font_size: 16.0 * s, color: [255, 110, 90, 255], font: FontFace::Body, align: render::hud::TextAlign::Left, width: size[0] as f32 - 24.0 * s });
        }
        list
    }

    /// Listeners at the cameras.
    fn ears(&self) {
        let mut c = self.core();
        let ears = c.cams.iter().map(|k| {
            let f = (k.target - k.pos).try_normalize().unwrap_or(Vec3::NEG_Z);
            let r = f.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
            Ear { pos: k.pos.to_array(), forward: f.to_array(), right: r.to_array() }
        }).collect();
        c.sounds.ears(ears);
    }

    pub fn command_line(&mut self, cmd: &str) -> Result<String, String> {
        let (head, rest) = cmd.trim().split_once(' ').map_or((cmd.trim(), ""), |(a, b)| (a, b.trim()));
        match head {
            "help" | "" => Ok(Self::help().into()),
            "lua" => self.luau(rest),
            "reload" => { let l = self.reload(); Ok(if l.is_empty() { "nothing changed on disk".into() } else { l.join("\n") }) }
            "restart" => { self.restart(); Ok(format!("restarted at seed {}", self.seed)) }
            "sky" => { let s = SkyChoice::parse(rest)?; self.core().sky = s; Ok(format!("sky {}", s.name())) }
            "sounds" => Ok(self.core().sounds.bank.patches.values().map(|p| format!("{} ({})", p.name, p.params.iter().map(|(n, d)| format!("{n}={d}")).collect::<Vec<_>>().join(" "))).collect::<Vec<_>>().join("\n")),
            "entities" => {
                let c = self.core();
                let lines: Vec<String> = c.world.entities.iter().filter(|(_, e)| rest.is_empty() || e.tag == rest)
                    .map(|(id, e)| format!("{id} {} {} pos {:.2} {:.2} {:.2}", e.shape.name(), if e.tag.is_empty() { "-" } else { &e.tag }, e.pos.x, e.pos.y, e.pos.z)).collect();
                Ok(if lines.is_empty() { "no entities".into() } else { lines.join("\n") })
            }
            "logs" => Ok(self.logs().join("\n")),
            other => Err(format!("unknown command {other:?}; {}", Self::help())),
        }
    }

    /// Start live control: the generic MCP tools, plus `reload`, `restart` and `command`.
    pub fn start_live(&mut self, cfg: live::Config) -> Result<(), String> {
        let tools = vec![
            live::ToolSpec::new("reload", "Reload edited Luau modules and sound patches now (they also reload by themselves within a quarter second of a save). Returns what changed and any load error. A reload keeps the world and each module's `state` table and calls the module's `reload()` hook; `init` does not run again.", json!({ "type": "object", "properties": {} })),
            live::ToolSpec::new("restart", "Start the game over: remove every entity, clear every module's `state`, reseed the random generator and run `init` again.", json!({ "type": "object", "properties": {} })),
            live::ToolSpec::new("command", "One debug console line. `help` lists them: lua CODE, reload, restart, sky NAME, sounds, entities [TAG], logs.", json!({ "type": "object", "properties": { "line": { "type": "string" } }, "required": ["line"] })),
        ];
        let manifest = live::Manifest {
            name: self.live_name(),
            version: env!("CARGO_PKG_VERSION").into(),
            about: format!("\"{}\", a Warp game made of Luau scripts (main.luau), synth patches (sounds/*.luau) and assets, run by the `warp` executable.", self.cfg.title),
            guide: "Edit the game with write_file in the `game` folder: main.luau holds init(), tick(dt, input) and draw(); sounds/NAME.luau is the patch `NAME`. A saved file reloads at once: the world and the `state` table stay, so change code without losing the play session; call `restart` to run init again. The Luau API is in the folder's types.d.luau. `state` is the game's JSON (its `state` field is main.luau's state table); `run_luau` runs Luau against the live game; `screenshot` shows the frame; `render_sound` measures a patch without speakers. Errors appear in `state.errors`, `logs` and on screen.".into(),
            tools,
            roots: vec![live::Root::new("game", &self.dir, live::Reload::Game, "The game folder: main.luau and other modules, sounds/*.luau patches, game.toml, types.d.luau. A write reloads the scripts and sounds and reports load errors.").extensions(&["luau", "toml", "md", "json"])],
            features: live::Features { state: true, screenshot: true, luau: true, shaders: false, sound: true },
        };
        let live = live::start(cfg, manifest)?;
        for l in &self.log { live.log(l.clone()); }
        self.live_logs = Some(live.logs().clone());
        self.live = Some(live);
        Ok(())
    }

    pub fn live_name(&self) -> String {
        let name = self.dir.canonicalize().ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_else(|| self.cfg.title.clone());
        let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        slug.trim_matches('-').to_string()
    }
}

impl Game for LuaGame {
    fn local_players(&self) -> usize { self.cfg.players }

    fn tick(&mut self, controls: &[Controls]) {
        if let Some(mut l) = self.live.take() { l.serve(self); self.live = Some(l); }
        let tick = self.core().tick;
        if tick % POLL_TICKS == 0 && tick > 0 { self.reload(); }
        let dt = 1.0 / self.cfg.tick_rate;
        let input = match self.input(self.host.lua(), controls) { Ok(t) => t, Err(e) => { self.say(format!("input: {e}")); return; } };
        self.hook("tick", || Ok(mlua::MultiValue::from_vec(vec![Value::Number(dt as f64), Value::Table(input.clone())])));
        self.core().tick += 1;
        self.ears();
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let players = self.cfg.players;
        let views = frame.rects(players);
        {
            let mut c = self.core();
            c.screen = frame.size;
            c.views = views;
            c.hud.clear();
            c.phase = Phase::Draw;
        }
        self.hook("draw", || Ok(mlua::MultiValue::new()));
        self.core().phase = Phase::Sim;
        self.update_sky();
        let light = self.lighting();
        let mut c = api::lock(&self.core);
        if std::mem::take(&mut c.world.rebuild) || frame.renderer().is_none() {
            frame.set_scene(&c.world.scene, &light);
            self.sky_upload = None;
        } else if let Some(p) = self.sky_upload.take() {
            let gpu = frame.gpu;
            let mut l = light.clone();
            if let Some(r) = frame.renderer_mut() && let Err(e) = r.update_sky(gpu, &p.sky, &mut l) { eprintln!("sky unchanged on the GPU: {e}"); }
        }
        let r = frame.renderer().ok_or("no renderer")?;
        c.world.flush(frame.gpu, r)?;
        let cameras = c.cams.iter().map(|k| Camera { pos: k.pos, target: k.target, fov_y: k.fov.to_radians(), near: 0.05 }).collect();
        drop(c);
        let hud = self.hud(frame.size);
        Ok(Draw { cameras, light, hud })
    }

    fn title(&self) -> Option<String> {
        let e = self.errors();
        Some(if e.is_empty() { format!("tick {}", self.core().tick) } else { format!("{} error(s): {}", e.len(), e[0]) })
    }

    fn state(&self) -> Option<String> { Some(self.state_json().to_string()) }

    fn command(&mut self, cmd: &str) -> Result<String, String> { self.command_line(cmd) }

    fn wants_capture(&self) -> bool { self.live.as_ref().is_some_and(|l| l.wants_capture()) }

    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) { if let Some(l) = &mut self.live { l.captured(size, rgba); } }
}

impl live::LiveGame for LuaGame {
    fn state(&mut self) -> Result<Json, String> { Ok(self.state_json()) }

    fn call(&mut self, tool: &str, args: &Json) -> live::Reply {
        let text = match tool {
            "reload" => { let l = self.reload(); if l.is_empty() { "nothing changed on disk".to_string() } else { l.join("\n") } }
            "restart" => { self.restart(); format!("restarted at seed {}", self.seed) }
            "command" => self.command_line(args["line"].as_str().ok_or("command needs `line`")?)?,
            other => return Err(format!("no tool {other}")),
        };
        let errors = self.errors();
        Ok(vec![live::Content::Text(if errors.is_empty() { text } else { format!("{text}\nerrors:\n{}", errors.join("\n")) })])
    }

    fn luau(&mut self, code: &str) -> Result<String, String> { LuaGame::luau(self, code) }

    fn file_written(&mut self, _root: &str, path: &str) -> Option<String> {
        let lines = self.reload();
        let mine: Vec<&String> = lines.iter().filter(|l| l.starts_with(path) || l.contains(path)).collect();
        Some(if mine.is_empty() { format!("{path}: written (no module or patch changed: {})", if lines.is_empty() { "nothing reloaded".into() } else { lines.join("; ") }) } else { mine.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n") })
    }

    fn sound(&mut self) -> Option<&mut dyn live::SoundHook> { Some(self) }
}

impl live::SoundHook for LuaGame {
    fn sounds(&mut self) -> Vec<live::SoundInfo> {
        self.reload();
        self.core().sounds.bank.patches.values().map(|p| live::SoundInfo { name: p.name.clone(), params: p.params.clone(),
            about: format!("{}{}", if p.max_length > 0.0 { format!("up to {:.2} s", p.max_length) } else { "no fixed length".into() }, if p.looping { ", loops" } else { "" }) }).collect()
    }

    fn play(&mut self, name: &str, params: &[(String, f32)], at: Option<[f32; 3]>) -> Result<String, String> {
        let mut c = self.core();
        let tick = c.tick;
        let id = c.sounds.play(name, params, at, 0.0, 1.0, tick)?;
        Ok(format!("playing {name} (handle {id}){}{}", at.map_or(" at the listener".into(), |p| format!(" at {p:?}")), if c.sounds.has_output() { "" } else { "; this run has no audio output, render_sound measures it" }))
    }

    fn set_param(&mut self, sound: &str, param: &str, value: f32) -> Result<String, String> {
        let old = self.core().sounds.bank.set_default(sound, param, value)?;
        Ok(format!("{sound}.{param}: {old} -> {value} for every later play (values a script passes still win; saving the file resets it)"))
    }

    fn render(&mut self, name: &str, params: &[(String, f32)], seconds: Option<f32>) -> Result<live::Work, String> {
        let patch = self.core().sounds.bank.find(name)?;
        check_params(&patch, params)?;
        let opts = audio::cli::Opts { set: params.to_vec(), seconds, ..Default::default() };
        Ok(Box::new(move || audio::cli::report(&patch, &opts).map(|t| vec![live::Content::Text(t)])))
    }
}
