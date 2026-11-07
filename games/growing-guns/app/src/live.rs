//! Growing Guns under live control (engine/crates/live, docs in
//! engine/docs/live-control.md): `play --live [PORT]` serves MCP so Claude,
//! ChatGPT or an in-game assistant can read the match, hand out cards, tune
//! weapon fields, add bots, skip rounds, write Luau mods and synth patches,
//! reload shaders and take screenshots while you play. `serve --live` runs
//! the same game headless in real time (no window), for tests and servers.
//!
//! Online: the host runs the only simulation and streams snapshots, so a
//! change made on the host between ticks reaches every client with the next
//! snapshot (weapons and cards travel in `Player`; new arenas through
//! `Session::mark_arena`). Gameplay tools are refused on a client. Mods run
//! on the host only; clients need the same card files for the names in
//! their menus. `spawn_bots` works offline, or on a host with nobody joined.
use crate::play::Gg;
use crate::Args;
use growing_guns::weapon::{self, Value as Field};
use crate::sounds::Bank;
use audio::synth::{CompiledPatch as Patch, Play};
use live::{Content, Features, LiveGame, Manifest, Reload, Reply, Root, SoundHook, SoundInfo, ToolSpec, Work};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

pub struct GgLive {
    live: live::Live,
    synth: Synth,
}

/// `--live [PORT]` and friends (see live::Config::from_args). Under
/// GameNight (`party`) it is on without them, on a free local port, so the
/// session assistant in the app can change the game (`WARP_LIVE=0`: off).
pub fn start(args: &Args, party: bool) -> Result<Option<GgLive>, String> {
    let cfg = match live::Config::from_args("growing-guns", &args.list)? {
        Some(cfg) => cfg,
        None if party && std::env::var("WARP_LIVE").as_deref() != Ok("0") => {
            let mut cfg = live::Config::new("growing-guns");
            cfg.port = 0;
            cfg.announce = false;
            cfg
        }
        None => return Ok(None),
    };
    let mut roots = Vec::new();
    if let Some(d) = crate::scripts::find_dir() {
        roots.push(Root::new("mods", d, Reload::Game, "Luau cards and rules (read API.md and types.d.luau first); a write reloads the module and reports errors").extensions(&["luau", "md"]));
    }
    let sounds = crate::sounds::find_patches(args.value("--sfx"));
    if let Some(d) = &sounds {
        roots.push(Root::new("sounds", d, Reload::Game, "procedural synth patches (Luau, see engine/docs/audio.md; `warp-audio units` lists the units); a write reloads the patch, which the game's own triggers use at once").extensions(&["luau"]));
    }
    let shaders = render::shaders::dir();
    if let Some(d) = &shaders {
        roots.push(Root::new("shaders", d, Reload::Shaders, "WGSL shading chain from KERN_SHADER_DIR; a write recompiles it on the next frame, a failure keeps the old shaders").extensions(&["wgsl"]));
    }
    let manifest = Manifest {
        name: "Growing Guns".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        about: "A last-man-standing arena shooter on Warp: up to 8 players (humans and bots) in a procedural castle arena; the loser of each round picks a card that changes their gun (52 cards in mods/base/cards.luau); first to ten round wins takes the match. 60 Hz deterministic sim, host-authoritative online play.".into(),
        guide: format!("- Look first: `match_state`, `screenshot`, `list_cards`, `logs`.\n\
            - The feel of the match (damage, fire rate, run speed, bullet speed, explosions, health): `knobs` lists them, `set_knob` changes one for everyone at once and saves it in its mod (mods/base/knobs.luau). Start there for \"more damage\", \"faster\" and the like.\n\
            - Gameplay (host or offline only, applied between ticks, streamed to online clients): `give_card`, `set_weapon` (Weapon fields as in Godot's weapon.gd; weapons are rebuilt from cards each round, so a field change lasts until the round ends; make it permanent with a card), `spawn_bots`, `next_round`, `restart_match`, `command` (the debug console: teleport, health, modifier, boom, bots pause...).\n\
            - Rules and cards are Luau in `mods/`: `load_mod` (or `write_file` mods/NAME.luau) adds or replaces a module and returns its load errors; `run_luau` runs a line against the match as `g` (g:give(1, 'explosive'), g:set_stat(1, 'damage_mult', 3)).\n\
            - Sound: every game sound is a synth patch in `sounds/` (one file per sound, named as the game triggers it: shot, explosion, reload...). `play_sound` lists and plays them, `render_sound` measures one offline (levels and a text spectrogram), `set_param` changes a parameter's default for the game's own triggers too; edit a patch with `write_file` and it is live at once. {}\n\
            - Shaders: {}",
            if sounds.is_some() { "" } else { "(no sounds folder found)" },
            if shaders.is_some() { "edit WGSL in `shaders/`, then `reload_shaders` (a write reloads too)." } else { "start the game with KERN_SHADER_DIR=engine/crates/render/src/shaders to edit shaders live." }),
        tools: tools(),
        roots,
        features: Features { state: true, screenshot: true, luau: true, shaders: shaders.is_some(), sound: true },
    };
    let live = live::start(cfg, manifest)?;
    Ok(Some(GgLive { live, synth: Synth { fallback: None, dir: sounds, plays: 0 } }))
}

fn obj(props: Value, required: &[&str]) -> Value { json!({ "type": "object", "properties": props, "required": required }) }

fn tools() -> Vec<ToolSpec> {
    let player = json!({ "type": "integer", "minimum": 0, "description": "player index from 0 (match_state lists them)" });
    vec![
        ToolSpec::new("match_state", "Players (bot, alive, health, score, cards, special, changed weapon fields), round, phase, modifier. With player: that player's whole Weapon; with fields: the names and kinds of every Weapon field.",
            obj(json!({ "player": player, "fields": { "type": "boolean" } }), &[])).read_only(),
        ToolSpec::new("knobs", "The match's tweakable values (`-- @knobs` blocks in the mods: damage, fire_rate, move_speed, bullet_speed, explosions, health, as multipliers of what the cards give) with their values and ranges.", obj(json!({}), &[])).read_only(),
        ToolSpec::new("set_knob", "Set one knob for every player at once, mid-round too; saved in its mod file (ready to commit) and hot-reloaded. The Gamepad Party app's settings show it.",
            obj(json!({ "name": { "type": "string" }, "value": { "type": "string", "description": "one Luau value, such as 1.5" } }), &["name", "value"])),
        ToolSpec::new("list_cards", "Every card on offer: id, name, description, rarity.", obj(json!({}), &[])).read_only(),
        ToolSpec::new("give_card", "Give a player a card now (its stats, pick hook and the gun's growth), as if picked.",
            obj(json!({ "player": player, "card": { "type": "string", "description": "card id, see list_cards" } }), &["player", "card"])),
        ToolSpec::new("set_weapon", "Set Weapon fields of one player (or all, without player) until the round ends: numbers, booleans, [x, y, z] vectors/colours, special as a name (grenade, teleport, zoom, sword, air_strike, ion_cannon, cluster_grenade).",
            obj(json!({ "player": player, "fields": { "type": "object", "description": "field: value, e.g. {\"damage_mult\": 3, \"extra_projectiles\": 4}" } }), &["fields"])),
        ToolSpec::new("spawn_bots", "Add bot players now (up to 8 players). Offline, or on a host before anyone joins.",
            obj(json!({ "count": { "type": "integer", "minimum": 1, "maximum": 7 } }), &["count"])),
        ToolSpec::new("next_round", "End this round and start the next one (new arena; cards kept).", obj(json!({}), &[])),
        ToolSpec::new("restart_match", "Start the match over: scores and cards cleared.", obj(json!({}), &[])),
        ToolSpec::new("load_mod", "Add or replace a Luau module in mods/ (cards and rule hooks, see mods/API.md) and load it now; returns its status or load error. A load error keeps the previous version.",
            obj(json!({ "name": { "type": "string", "description": "module path without .luau, e.g. live/low_gravity" }, "source": { "type": "string" } }), &["name", "source"])),
        ToolSpec::new("command", "A debug console line (`help` lists them): teleport P X Y Z, health P N, ammo P N, special P NAME, modifier ID, bots pause|resume, boom X Y Z R, arena SEED, cage, extend...",
            obj(json!({ "line": { "type": "string" } }), &["line"])),
    ]
}

impl GgLive {
    /// Where the bridge (and the GameNight host) find the port and token.
    pub fn token_file(&self) -> Option<PathBuf> { self.live.token_file().map(PathBuf::from) }
}

/// The game thread's part, once per tick.
pub fn tick(gg: &mut Gg) {
    let Some(mut l) = gg.live.take() else { return };
    let GgLive { live, synth } = &mut *l;
    live.serve(&mut Ctx { gg, synth });
    gg.live = Some(l);
}

/// Frame work: shader reloads need the renderer.
pub fn frame(gg: &mut Gg, frame: &mut app::Frame) {
    let Some(l) = gg.live.as_mut() else { return };
    if !l.live.wants_shader_reload() { return; }
    let gpu = frame.gpu;
    let result = match frame.renderer_mut() { Some(r) => r.reload_shaders(gpu), None => Err("no renderer yet".into()) };
    l.live.shaders_reloaded(result);
}

/// A drawn frame is wanted (a screenshot, or a shader reload headless).
pub fn wants_capture(gg: &Gg) -> bool { gg.live.as_ref().is_some_and(|l| l.live.wants_capture() || l.live.wants_shader_reload()) }

pub fn captured(gg: &mut Gg, size: [u32; 2], rgba: Vec<u8>) { if let Some(l) = gg.live.as_mut() { l.live.captured(size, rgba); } }

/// `serve`: the game headless in real time, until `--seconds` (0: forever).
pub fn serve(args: &Args) -> Result<(), String> {
    if live::Config::from_args("growing-guns", &args.list)?.is_none() { return Err("serve needs --live [PORT]".into()); }
    let total = args.num("--total", 4usize).clamp(1, 8);
    let world = growing_guns::World::new_with(args.num("--seed", 3u64), &vec![true; total], |w| crate::play::match_options(args, w, false));
    // Every seat is a bot; the single view follows player 0 (for screenshots).
    let mut game = Gg::new(world, crate::session::Session::Local, vec![0]);
    game.world.release_cage();
    game.live = start(args, false)?.map(Box::new);
    // The game's sound runs as when playing, into a mixer drained in real
    // time and thrown away: triggers, patches and play_sound all work, silently.
    let (mut mix, tx, _crowd) = crate::sounds::offline(None);
    game.set_sounds(crate::sounds::Sounds::new(tx, crate::sounds::find_patches(args.value("--sfx")), None));
    std::thread::Builder::new().name("serve-audio".into()).spawn(move || {
        let mut buf = vec![0.0f32; (audio::SAMPLE_RATE / 100.0) as usize * 2];
        let start = std::time::Instant::now();
        for block in 1u64.. {
            mix.render(&mut buf, 2);
            if let Some(wait) = std::time::Duration::from_millis(10 * block).checked_sub(start.elapsed()) { std::thread::sleep(wait); }
        }
    }).map_err(|e| e.to_string())?;
    let gpu = render::Gpu::new()?;
    let cfg = app::Config { size: { let (w, h) = args.size((960, 540)); [w, h] }, tick_hz: growing_guns::TICK_HZ as f32, ..Default::default() };
    println!("serve: headless, {total} bots, GPU {}", gpu.info.name);
    app::run_headless(&cfg, &mut game, &gpu, args.num("--seconds", 0.0f32))
}

struct Ctx<'a> { gg: &'a mut Gg, synth: &'a mut Synth }

impl Ctx<'_> {
    /// Gameplay changes only where the simulation runs.
    fn authority(&self) -> Result<(), String> {
        if self.gg.session().is_client() { return Err("this game is a client of an online match: make gameplay changes on the host (they reach every client through its snapshots)".into()); }
        Ok(())
    }

    fn player(&self, args: &Value) -> Result<Option<usize>, String> {
        match args.get("player") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => {
                let p = v.as_u64().ok_or("player is an index from 0")? as usize;
                if p < self.gg.world.players.len() { Ok(Some(p)) } else { Err(format!("no player {p}: players are 0..{}", self.gg.world.players.len() - 1)) }
            }
        }
    }

    fn command(&mut self, line: &str) -> Reply { app::Game::command(self.gg, line).map(|t| vec![Content::Text(t)]) }

    fn match_state(&self, args: &Value) -> Reply {
        let w = &self.gg.world;
        let detail = self.player(args)?;
        let base = weapon::Weapon::default();
        let players: Vec<Value> = w.players.iter().enumerate().map(|(i, p)| {
            let fields: serde_json::Map<String, Value> = weapon::FIELDS.iter().filter_map(|(name, _)| {
                let v = p.weapon.get(name)?;
                (detail == Some(i) || base.get(name).as_ref() != Some(&v)).then(|| (name.to_string(), field_json(&v)))
            }).collect();
            json!({ "index": i, "bot": p.bot, "local": self.gg.slots.contains(&i), "alive": p.alive, "health": p.health, "max_health": p.max_health(),
                "score": p.score, "ammo": p.ammo, "mag": p.mag_size(), "special": p.weapon.special.name(),
                "cards": p.cards.iter().map(|&c| w.cards.get(c).map_or("?".to_string(), |d| d.id.clone())).collect::<Vec<_>>(),
                "offer": p.offer.as_ref().map(|o| o.iter().map(|&c| w.cards.get(c).map_or("?".to_string(), |d| d.id.clone())).collect::<Vec<_>>()),
                "pos": p.pos.to_array().map(|x| (x as f64 * 100.0).round() / 100.0),
                if detail == Some(i) { "weapon" } else { "weapon_changes" }: fields })
        }).collect();
        let session = if self.gg.session().is_client() { "client" } else if self.gg.session().ticket().is_some() { "host" } else { "local" };
        let mut out = json!({ "tick": w.tick, "seconds": w.tick as f64 / growing_guns::TICK_HZ as f64, "round": w.round, "phase": w.phase, "rounds_to_win": w.rounds_to_win,
            "modifier": w.modifier.map(|m| m.id()), "coop": w.coop, "session": session, "bots_paused": w.bots_paused, "players": players });
        if args.get("fields").and_then(Value::as_bool) == Some(true) {
            out["weapon_fields"] = json!(weapon::FIELDS.iter().map(|(n, k)| json!({ "name": n, "kind": k })).chain([json!({ "name": "special", "kind": "name" })]).collect::<Vec<_>>());
        }
        Ok(vec![Content::Json(out)])
    }

    fn set_weapon(&mut self, args: &Value) -> Reply {
        self.authority()?;
        let fields = args.get("fields").and_then(Value::as_object).ok_or("fields must be an object of field: value")?;
        let who: Vec<usize> = match self.player(args)? { Some(p) => vec![p], None => (0..self.gg.world.players.len()).collect() };
        let mut done = Vec::new();
        for (name, v) in fields {
            let value = match v {
                Value::Number(n) => Field::Num(n.as_f64().unwrap_or(0.0) as f32),
                Value::Bool(b) => Field::Bool(*b),
                Value::String(s) => Field::Text(s.clone()),
                Value::Array(a) if a.len() == 3 => Field::Vec(glam::Vec3::new(a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32)),
                _ => return Err(format!("{name}: use a number, boolean, [x, y, z] or a special's name")),
            };
            for &p in &who { self.gg.world.players[p].weapon.set(name, value.clone()).map_err(|e| format!("{e} (match_state with fields: true lists them)"))?; }
            done.push(format!("{name} = {}", field_json(&self.gg.world.players[who[0]].weapon.get(name).unwrap_or(value))));
        }
        for &p in &who {
            // A bigger magazine is usable at once; health stays within the new maximum.
            let pl = &mut self.gg.world.players[p];
            pl.ammo = pl.ammo.min(pl.mag_size());
            if pl.alive { pl.health = pl.health.min(pl.max_health()); }
            self.gg.vis.grow_gun(p, &self.gg.world.players[p]);
        }
        Ok(vec![Content::Text(format!("{} for player{} {}; lasts until the round ends (weapons are rebuilt from cards)", done.join(", "),
            if who.len() == 1 { "" } else { "s" }, who.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")))])
    }

    fn spawn_bots(&mut self, args: &Value) -> Reply {
        self.authority()?;
        if self.gg.session().remote_players() > 0 {
            return Err("players have joined this host: their views were built for the current player count; spawn bots before anyone joins, or play offline".into());
        }
        if self.gg.world.coop { return Err("coop waves bring their own enemies".into()); }
        let n = args.get("count").and_then(Value::as_u64).unwrap_or(1) as usize;
        let have = self.gg.world.players.len();
        let add = n.min(8usize.saturating_sub(have));
        if add == 0 { return Err(format!("{have} players already (8 at most)")); }
        let w = &mut self.gg.world;
        for k in 0..add {
            let mut p = growing_guns::Player::new(true);
            p.pos = w.arena.spawns.get((have + k) % w.arena.spawns.len().max(1)).copied().unwrap_or_default() + glam::Vec3::Y;
            w.players.push(p);
        }
        crate::play::rebuild_views(self.gg);
        Ok(vec![Content::Text(format!("added {add} bots: {} players now", self.gg.world.players.len()))])
    }

    fn load_mod(&mut self, args: &Value) -> Reply {
        self.authority()?;
        let name = args.get("name").and_then(Value::as_str).ok_or("load_mod needs a name like live/low_gravity")?.trim_end_matches(".luau");
        if name.is_empty() || name.split('/').any(|p| p.is_empty() || p == "." || p == "..") || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_-/".contains(c)) {
            return Err("name: letters, digits, _ - and / only, e.g. live/low_gravity".into());
        }
        let source = args.get("source").and_then(Value::as_str).ok_or("load_mod needs the module's source")?;
        let dir = crate::scripts::find_dir().ok_or("no mods folder (GG_MODS)")?;
        let path = dir.join(format!("{name}.luau"));
        if let Some(d) = path.parent() { std::fs::create_dir_all(d).map_err(|e| e.to_string())?; }
        std::fs::write(&path, source).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(vec![Content::Text(format!("wrote mods/{name}.luau\n{}", self.reload_mod(&format!("{name}.luau"))))])
    }

    /// Reload mods now and report the module's line of `mods`.
    fn reload_mod(&mut self, rel: &str) -> String {
        let Some(s) = self.gg.scripts.clone() else { return "no mods loaded".into() };
        let lines = s.poll(&mut self.gg.world);
        for l in &lines { eprintln!("mods: {l}"); }
        let status = s.status();
        let mine: Vec<&str> = status.lines().filter(|l| l.starts_with(rel)).collect();
        let mut out = if mine.is_empty() { format!("{rel}: not loaded (is it under mods/?)") } else { mine.join("\n") };
        for l in lines { if !out.contains(&l) { out += &format!("\n{l}"); } }
        out
    }
}

fn field_json(v: &Field) -> Value {
    match v { Field::Num(x) => json!(x), Field::Bool(b) => json!(b), Field::Vec(v) => json!(v.to_array()), Field::Text(t) => json!(t) }
}

impl LiveGame for Ctx<'_> {
    fn state(&mut self) -> Result<Value, String> {
        let text = app::Game::state(self.gg).ok_or("no state")?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    fn call(&mut self, tool: &str, args: &Value) -> Reply {
        match tool {
            "match_state" => self.match_state(args),
            "list_cards" => Ok(vec![Content::Json(json!({ "cards": self.gg.world.cards.enabled().map(|(_, d)| json!({
                "id": d.id, "name": d.name, "desc": d.desc, "rare": d.rare, "firepower": d.firepower })).collect::<Vec<_>>() }))]),
            "give_card" => {
                self.authority()?;
                let p = self.player(args)?.ok_or("give_card needs a player")?;
                let card = args.get("card").and_then(Value::as_str).ok_or("give_card needs a card id")?;
                self.command(&format!("give {p} {card}"))
            }
            "set_weapon" => self.set_weapon(args),
            "spawn_bots" => self.spawn_bots(args),
            "next_round" => { self.authority()?; self.command("round") }
            "restart_match" => { self.authority()?; self.command("rematch") }
            "load_mod" => self.load_mod(args),
            "knobs" => Ok(vec![Content::Text(crate::knobs::list())]),
            "set_knob" => {
                self.authority()?;
                let name = args.get("name").and_then(Value::as_str).ok_or("set_knob needs a name (see knobs)")?;
                let value = args.get("value").map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).ok_or("set_knob needs a value")?;
                let rel = crate::knobs::set(name, &value)?;
                if let Some(p) = self.gg.party.as_mut() { p.knob_changed(name, &value); }
                Ok(vec![Content::Text(format!("{name} = {value} in mods/{rel}\n{}", self.reload_mod(&rel)))])
            }
            "command" => {
                let line = args.get("line").and_then(Value::as_str).ok_or("command needs a line (try help)")?;
                if line.trim() != "help" && line.trim() != "cards" && line.trim() != "mods" { self.authority()?; }
                self.command(line)
            }
            other => Err(format!("no tool {other}")),
        }
    }

    fn luau(&mut self, code: &str) -> Result<String, String> {
        let note = if self.gg.session().is_client() { "(client: the host's next snapshot overwrites changes)\n" } else { "" };
        let s = self.gg.scripts.clone().ok_or("no mods loaded")?;
        s.console(&mut self.gg.world, code).map(|r| format!("{note}{r}"))
    }

    fn file_written(&mut self, root: &str, path: &str) -> Option<String> {
        match root {
            "mods" if path.ends_with(".luau") => Some(self.reload_mod(path)),
            "sounds" => {
                let lines = self.bank().scan();
                for l in &lines { eprintln!("sfx: {l}"); }
                let name = path.trim_end_matches(".luau");
                let failed = lines.iter().find(|l| l.contains("keeping the last good version"));
                Some(match (failed, self.bank().find(name)) {
                    (Some(e), _) => format!("{name}: load error, the game keeps the previous version: {e}"),
                    (None, Some(p)) => format!("{name}: loaded, params {:?}; the game's own triggers use it now; play_sound or render_sound to check it", p.params),
                    (None, None) => format!("{path}: loaded, but no sound is named {name} (a file's sounds are named by sound(name, ...)): {}", lines.join("; ")),
                })
            }
            _ => None,
        }
    }

    fn sound(&mut self) -> Option<&mut dyn SoundHook> { Some(self) }
}

/// The patches for live sound tools: the game's own hot-reloaded bank when
/// it has speakers, else a bank of our own (headless, silent runs).
struct Synth { fallback: Option<Bank>, dir: Option<PathBuf>, plays: u32 }

impl Ctx<'_> {
    fn bank(&mut self) -> &mut Bank {
        match self.gg.sounds.as_mut() {
            Some(s) => &mut s.bank,
            None => { let dir = self.synth.dir.clone(); self.synth.fallback.get_or_insert_with(|| Bank::load(dir)) }
        }
    }

    /// A sound by name, after picking up edited files.
    fn patch(&mut self, name: &str) -> Result<Arc<Patch>, String> {
        let bank = self.bank();
        for l in bank.scan() { eprintln!("sfx: {l}"); }
        bank.find(name).ok_or_else(|| {
            let mut names: Vec<&str> = bank.patches().map(|p| p.name.as_str()).collect();
            names.sort();
            if names.is_empty() { "no sound patches (--sfx DIR or GG_SFX, games/growing-guns/sounds)".to_string() } else { format!("no sound {name}; sounds: {}", names.join(", ")) }
        })
    }
}

fn check_params(patch: &Patch, params: &[(String, f32)]) -> Result<(), String> {
    for (n, _) in params {
        if !patch.params.iter().any(|p| &p.0 == n) {
            return Err(format!("{} has no parameter {n} (has {})", patch.name, patch.params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")));
        }
    }
    Ok(())
}

impl SoundHook for Ctx<'_> {
    fn sounds(&mut self) -> Vec<SoundInfo> {
        let bank = self.bank();
        for l in bank.scan() { eprintln!("sfx: {l}"); }
        let mut out: Vec<SoundInfo> = bank.patches().map(|p| SoundInfo {
            name: p.name.clone(), params: p.params.clone(),
            about: format!("{}{}", if p.max_length > 0.0 { format!("up to {:.2} s", p.max_length) } else { "no fixed length".into() }, if p.looping { ", loops" } else { "" }),
        }).collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    fn play(&mut self, name: &str, params: &[(String, f32)], at: Option<[f32; 3]>) -> Result<String, String> {
        let patch = self.patch(name)?;
        check_params(&patch, params)?;
        self.synth.plays += 1;
        let refs: Vec<(&str, f32)> = params.iter().map(|(n, v)| (n.as_str(), *v)).collect();
        let Some(sounds) = self.gg.sounds.as_ref() else { return Err(format!("{name}: this run has no audio output; render_sound measures it offline")) };
        let play = match at { Some(p) => Play::at(p), None => Play::default() }.seed(self.synth.plays);
        sounds.sender().play(&patch, &refs, play);
        let used: Vec<String> = patch.params.iter().map(|(n, d)| format!("{n}={}", params.iter().find(|p| &p.0 == n).map_or(*d, |p| p.1))).collect();
        Ok(format!("playing {name}{} ({})", at.map_or(" at the listener".to_string(), |p| format!(" at {p:?}")), used.join(" ")))
    }

    fn set_param(&mut self, sound: &str, param: &str, value: f32) -> Result<String, String> {
        self.patch(sound)?;
        let old = self.bank().set_default(sound, param, value)?;
        Ok(format!("{sound}.{param}: {old} -> {value} for every later play (values a trigger passes still win; editing the file resets it)"))
    }

    fn render(&mut self, name: &str, params: &[(String, f32)], seconds: Option<f32>) -> Result<Work, String> {
        let patch = self.patch(name)?;
        check_params(&patch, params)?;
        let params = params.to_vec();
        Ok(Box::new(move || Ok(vec![Content::Text(render_report(&patch, &params, seconds))])))
    }
}

/// What `warp-audio render` prints for a patch: mono, at the listener, seed 1.
fn render_report(patch: &Arc<Patch>, params: &[(String, f32)], seconds: Option<f32>) -> String {
    const SR: f32 = audio::SAMPLE_RATE;
    let seconds = seconds.unwrap_or(if patch.max_length > 0.0 { patch.max_length } else { 4.0 }).min(30.0);
    let t0 = std::time::Instant::now();
    let (tx, mut mix) = audio::synth::mixer(audio::synth::Layout::mono(), 16, SR);
    tx.ears(vec![audio::synth::Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
    let refs: Vec<(&str, f32)> = params.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    tx.play(patch, &refs, Play::default().seed(1));
    let mut out = Vec::with_capacity((seconds * SR) as usize);
    let mut t = 0.0;
    while t < seconds { out.extend(mix.render_seconds(0.01)); t += 0.01; }
    audio::analysis::trim_silence(&mut out, 1, SR);
    let l = audio::analysis::levels(&out, 1, SR);
    let used: Vec<String> = patch.params.iter().map(|(n, d)| format!("{n}={}", params.iter().find(|p| &p.0 == n).map_or(*d, |p| p.1))).collect();
    format!("{} ({} nodes; {}): {:.3} s, rendered in {:.1} ms\npeak {:.1} dBFS, rms {:.1} dB, loudest 50 ms {:.1} dB, centroid {:.0} Hz\n{}",
        patch.name, patch.nodes.len(), used.join(" "), l.seconds, t0.elapsed().as_secs_f32() * 1000.0, l.peak_db, l.rms_db, l.punch_db, l.centroid_hz,
        audio::analysis::text_spectrogram(&out, SR))
}
