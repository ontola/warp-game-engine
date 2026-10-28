//! Growing Guns as a Warp `app::Game`: local, host or client sessions with
//! 1 to 4 local players in split screen, the rest bots. Window, tick, input
//! and presentation come from the engine's app shell.
use crate::session::Session;
use crate::{hud, view, Args};
use app::{Buttons, Controls, Draw, Frame};
use glam::Vec3;
use growing_guns::{Event, Input, World, DT, TICK_HZ};
use render::hud::HudDrawList;
use render::Lighting;

/// Hit markers, damage flash and gun bob from this tick's events.
pub fn feedback(fb: &mut [hud::Feedback], world: &World, dt: f32) {
    for (i, f) in fb.iter_mut().enumerate() {
        f.hit = (f.hit - dt).max(0.0);
        f.hurt = (f.hurt - dt).max(0.0);
        if let Some(p) = world.players.get(i) {
            let speed = (p.vel.x * p.vel.x + p.vel.z * p.vel.z).sqrt();
            f.bob += speed * dt * 0.75;
        }
    }
    for e in &world.events {
        if let Event::Hit { victim, attacker, head, .. } = *e {
            if let Some(f) = fb.get_mut(attacker) { f.hit = 0.25; f.headshot = head; }
            if let Some(f) = fb.get_mut(victim) { f.hurt = 0.35; }
        }
    }
}

/// Engine controls to this game's input. Look x is right-positive in the
/// shell and left-positive (yaw) in the simulation.
fn input(c: &Controls) -> Input {
    Input {
        mv: c.mv,
        look: [-c.look[0], c.look[1]],
        // Godot's bindings: A or LB jump, Y or RB (Shift) dash, X reload,
        // RT (LMB) fire, LT or B (RMB) special.
        jump: c.pressed.has(Buttons::SOUTH | Buttons::LEFT_SHOULDER),
        dash: c.pressed.has(Buttons::NORTH | Buttons::RIGHT_SHOULDER),
        reload: c.pressed.has(Buttons::WEST),
        fire: c.held.has(Buttons::RIGHT_TRIGGER),
        special: c.held.has(Buttons::LEFT_TRIGGER | Buttons::EAST),
        pick: None,
    }
}

pub struct Gg {
    pub world: World,
    prev: Vec<Vec3>,
    pub vis: view::Visuals,
    light: Lighting,
    fb: Vec<hud::Feedback>,
    session: Session,
    /// World player index per local view.
    slots: Vec<usize>,
    since_snap: f32,
    since_poll: u32,
    rebuild: bool,
    sounds: Option<crate::sounds::Sounds>,
    /// Card menu per local player.
    menus: Vec<app::ui::Menu>,
    /// The mods (cards, rules), hot-reloaded.
    scripts: Option<std::sync::Arc<crate::scripts::Scripts>>,
}

impl Gg {
    pub fn new(mut world: World, session: Session, slots: Vec<usize>) -> Self {
        let scripts = crate::scripts::attach(&mut world).map_err(|e| eprintln!("mods: {e}")).ok();
        let n = world.players.len();
        let mut vis = view::Visuals::build(&world);
        vis.hide_own_bodies(&slots);
        Gg { prev: world.players.iter().map(|p| p.pos).collect(), vis, light: view::lighting(),
            fb: vec![hud::Feedback::default(); n], world, session, slots, since_snap: 0.0, since_poll: 0, rebuild: true, sounds: None, menus: Vec::new(), scripts }
    }

    pub fn light_mut(&mut self) -> &mut Lighting { &mut self.light }

    /// Client: apply what the host sent; returns the interpolation alpha.
    pub fn receive(&mut self, dt: f32) -> f32 {
        let before: Vec<Vec3> = self.world.players.iter().map(|p| p.pos).collect();
        let (vis, fb, sounds, slots) = (&mut self.vis, &mut self.fb, &mut self.sounds, &self.slots);
        let (snaps, arena) = self.session.receive(&mut self.world, slots, |w| {
            vis.absorb(w, &crate::sounds::ears(w, slots));
            feedback(fb, w, DT);
            if let Some(s) = sounds.as_mut() { s.tick(w, slots, DT); }
        });
        if snaps > 0 { self.prev = before; self.since_snap = 0.0; } else { self.since_snap += dt; }
        if arena { self.new_arena(); }
        (self.since_snap / DT).clamp(0.0, 1.0)
    }

    pub fn session(&self) -> &Session { &self.session }
    pub fn set_sounds(&mut self, s: crate::sounds::Sounds) { self.sounds = Some(s); }

    /// The crowd's recordings (`--crowd DIR`, GG_CROWD, or found beside the
    /// sound folder); without them the crowd still reacts, silently.
    pub fn set_crowd(&mut self, arg: Option<&str>, sounds: &std::path::Path, tx: &audio::sfx::SfxSender) {
        let Some(dir) = crate::crowd_sound::find_dir(arg, Some(sounds)) else { eprintln!("silent crowd: no crowd_sound samples (--crowd DIR or GG_CROWD)"); return };
        match crate::crowd_sound::Samples::load(&dir) {
            Ok(s) => { let mut c = crate::crowd_sound::CrowdSound::new(Some((tx.clone(), s))); c.kickoff(); self.vis.stadium.set_sound(c); }
            Err(e) => eprintln!("silent crowd: {e}"),
        }
    }

    /// A new round: refill the arena slots; the renderer is kept.
    fn new_arena(&mut self) {
        self.vis.set_arena(&self.world);
        self.prev = self.world.players.iter().map(|p| p.pos).collect();
    }
}

impl app::Game for Gg {
    fn local_players(&self) -> usize { self.slots.len() }

    fn tick(&mut self, controls: &[Controls]) {
        // Edited mods take effect a quarter second later, between ticks.
        self.since_poll += 1;
        if self.since_poll >= TICK_HZ / 4 {
            self.since_poll = 0;
            if let Some(s) = &self.scripts { for line in s.poll(&mut self.world) { eprintln!("mods: {line}"); } }
        }
        let mut local: Vec<Input> = controls.iter().map(input).collect();
        // Sniper zoom slows the look (Godot x0.4).
        for (k, &s) in self.slots.iter().enumerate() {
            if self.world.players.get(s).is_some_and(|p| p.zooming) && let Some(i) = local.get_mut(k) { i.look = i.look.map(|v| v * 0.4); }
        }
        // Card picks: the menu takes this player's controls while an offer is open.
        self.menus.resize_with(self.slots.len(), Default::default);
        for (k, &s) in self.slots.iter().enumerate() {
            let Some(n) = self.world.players.get(s).and_then(|p| p.offer.as_ref()).map(|o| o.len()) else { continue };
            if let (Some(c), Some(i)) = (controls.get(k), local.get_mut(k)) {
                if let app::ui::MenuEvent::Select(n) = self.menus[k].update(c, n, n, DT) { i.pick = Some(n as u8); }
                i.fire = false;
                i.jump = false;
            }
        }
        if self.session.is_client() { self.session.send(local); return; }
        let mut inputs = vec![Input::default(); self.world.players.len()];
        for (k, &s) in self.slots.iter().enumerate() { if let Some(i) = local.get(k) { inputs[s] = *i; } }
        self.session.poll(&mut self.world);
        self.session.remote_inputs(&mut inputs);
        self.prev = self.world.players.iter().map(|p| p.pos).collect();
        self.world.step(&inputs);
        if self.world.events.contains(&Event::NewArena) { self.new_arena(); }
        self.vis.absorb(&self.world, &crate::sounds::ears(&self.world, &self.slots));
        feedback(&mut self.fb, &self.world, DT);
        if let Some(s) = &mut self.sounds { s.tick(&self.world, &self.slots, DT); }
        self.session.publish(&self.world);
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let alpha = if self.session.is_client() { self.receive(frame.dt) } else { frame.alpha };
        self.vis.pose(&self.world, &self.prev, alpha, frame.dt);
        if std::mem::take(&mut self.rebuild) || frame.renderer().is_none() { frame.set_scene(&self.vis.scene, &self.light); }
        let r = frame.renderer().expect("scene set above");
        self.vis.flush(frame.gpu, r)?;
        r.update_instances(frame.gpu, self.vis.first_dynamic, self.vis.dynamic())?;
        r.set_local_lights(&self.vis.local_lights())?;
        let rects = frame.rects(self.slots.len());
        let cameras = self.slots.iter().map(|&s| self.vis.punched(&self.world, s, view::camera(&self.world, s, &self.prev, alpha))).collect();
        let mut list = HudDrawList::new(frame.size);
        for (k, (rect, &s)) in rects.iter().zip(&self.slots).enumerate() {
            let flash = self.vis.ion_flash(self.world.players[s].eye());
            if flash > 0.0 { list.rect(*rect, [0.96, 0.99, 1.0, flash], *rect); }
            hud::view(&mut list, &self.world, s, *rect, &self.fb[s], self.menus.get(k).map_or(0, |m| m.focus));
        }
        let status = self.session.status(&self.slots);
        hud::global(&mut list, &self.world, status.as_deref());
        let mut light = self.light.clone();
        // The lava flows on the game clock.
        light.time = (self.world.tick as f32 - 1.0 + alpha) * DT;
        Ok(Draw { cameras, light, hud: list })
    }

    fn title(&self) -> Option<String> { Some(format!("round {}", self.world.round)) }

    fn command(&mut self, cmd: &str) -> Result<String, String> {
        let w: Vec<&str> = cmd.split_whitespace().collect();
        let num = |k: usize| -> Result<f32, String> { w.get(k).ok_or(format!("{cmd}: missing argument {k}"))?.parse::<f32>().map_err(|e| format!("{cmd}: {e}")) };
        let player = |k: usize, world: &World| -> Result<usize, String> {
            let p = num(k)? as usize;
            if p < world.players.len() { Ok(p) } else { Err(format!("no player {p}")) }
        };
        match w.first().copied().unwrap_or("") {
            "help" => Ok("give P CARD | teleport P X Y Z | aim P X Y Z | health P N | ammo P N | bots pause|resume | blast X Y Z R DAMAGE | boom X Y Z R [DAMAGE] | arena SEED | cards | lua CODE | mods | special P NAME | modifier ID|none | round | cage | rematch | extend".into()),
            "special" => {
                let p = player(1, &self.world)?;
                let name = w.get(2).ok_or("special P NAME")?;
                let sp = growing_guns::Special::named(name).ok_or(format!("unknown special {name}"))?;
                self.world.players[p].weapon.special = sp;
                Ok(format!("player {p} special {name}"))
            }
            "modifier" => {
                let id = w.get(1).copied().unwrap_or("none");
                self.world.forced_modifier = if id == "none" { None } else { Some(growing_guns::Modifier::named(id).ok_or(format!("unknown modifier {id}"))?) };
                if id == "none" { self.world.modifier_chance = 0.0; }
                Ok(format!("modifier {id} from the next round (round: start it now)"))
            }
            "round" => { self.world.next_round(); self.new_arena(); Ok(format!("round {}", self.world.round)) }
            "cage" => { self.world.release_cage(); Ok("cage open".into()) }
            "rematch" => { self.world.restart_match(); self.new_arena(); Ok("rematch".into()) }
            "extend" => { self.world.extend_match(); Ok(format!("first to {}", self.world.rounds_to_win)) }
            "lua" => {
                let code = cmd.trim_start().strip_prefix("lua").unwrap_or("").trim();
                self.scripts.as_ref().ok_or("no mods loaded")?.console(&mut self.world, code)
            }
            "mods" => {
                let s = self.scripts.as_ref().ok_or("no mods loaded")?;
                for line in s.poll(&mut self.world) { eprintln!("mods: {line}"); }
                Ok(s.status())
            }
            "arena" => {
                let seed = num(1)? as u64;
                self.world.arena = growing_guns::Arena::generate(seed);
                self.new_arena();
                let a = &self.world.arena;
                Ok(format!("arena {seed}: {:?}, {} m, {} chunks", a.variant, a.size, a.chunks.len()))
            }
            "cards" => Ok(self.world.cards.enabled().map(|(_, d)| d.id.clone()).collect::<Vec<_>>().join(" ")),
            "give" => {
                let p = player(1, &self.world)?;
                let name = w.get(2).ok_or("give P CARD")?.to_lowercase();
                let card = self.world.cards.find(&name).ok_or(format!("unknown card {name} (try: cards)"))?;
                self.world.give(p, card);
                let pl = &mut self.world.players[p];
                pl.ammo = pl.mag_size();
                // The gun grows into the new card, as after a pick.
                self.vis.grow_gun(p, &self.world.players[p]);
                let ids: Vec<&str> = self.world.players[p].cards.iter().map(|&c| self.world.cards.get(c).map_or("?", |d| d.id.as_str())).collect();
                Ok(format!("player {p} has {ids:?}"))
            }
            "teleport" => {
                let p = player(1, &self.world)?;
                let to = Vec3::new(num(2)?, num(3)?, num(4)?);
                self.world.players[p].pos = to;
                self.world.players[p].vel = Vec3::ZERO;
                if let Some(q) = self.prev.get_mut(p) { *q = to; }
                Ok(format!("player {p} at {to}"))
            }
            "aim" => {
                let p = player(1, &self.world)?;
                let pl = &mut self.world.players[p];
                let d = (Vec3::new(num(2)?, num(3)?, num(4)?) - pl.eye()).normalize_or(Vec3::NEG_Z);
                pl.yaw = (-d.x).atan2(-d.z);
                pl.pitch = d.y.clamp(-1.0, 1.0).asin().clamp(-1.5, 1.5);
                Ok(format!("player {p} yaw {:.3} pitch {:.3}", pl.yaw, pl.pitch))
            }
            "health" => { let p = player(1, &self.world)?; self.world.players[p].health = num(2)? as i32; Ok(format!("player {p} health {}", self.world.players[p].health)) }
            "ammo" => { let p = player(1, &self.world)?; self.world.players[p].ammo = num(2)? as u32; Ok(format!("player {p} ammo {}", self.world.players[p].ammo)) }
            "bots" => { self.world.bots_paused = w.get(1) == Some(&"pause"); Ok(format!("bots paused: {}", self.world.bots_paused)) }
            "boom" => {
                let (pos, r) = (Vec3::new(num(1)?, num(2)?, num(3)?), num(4)?);
                self.world.queue_explosion(pos, r, w.get(5).and_then(|d| d.parse().ok()).unwrap_or(50.0), 0);
                Ok(format!("explosion of {r} m queued at {pos}"))
            }
            "blast" => {
                let c = growing_guns::Carve { pos: Vec3::new(num(1)?, num(2)?, num(3)?), radius: num(4)?, damage: num(5)? };
                self.world.queue_carve(c);
                Ok("blast queued for the next tick".into())
            }
            other => Err(format!("unknown command {other} (try: help)")),
        }
    }

    fn state(&self) -> Option<String> {
        let w = &self.world;
        let players: Vec<_> = w.players.iter().enumerate().map(|(i, p)| serde_json::json!({
            "index": i, "bot": p.bot, "local_view": self.slots.iter().position(|&s| s == i),
            "alive": p.alive, "downed": p.downed, "ghost": p.ghost, "health": p.health, "score": p.score, "ammo": p.ammo, "reloading": p.reloading(),
            "pos": p.pos.to_array(), "vel": p.vel.to_array(), "yaw": p.yaw, "pitch": p.pitch,
            "on_floor": p.on_floor, "on_wall": p.wall_normal.is_some(), "dash_charges": p.dash_charges,
            "cards": p.cards.iter().map(|&c| card_id(w, c)).collect::<Vec<_>>(), "offer": p.offer.as_ref().map(|o| o.iter().map(|&c| card_id(w, c)).collect::<Vec<_>>()), "max_health": p.max_health(), "mag": p.mag_size(),
        })).collect();
        Some(serde_json::json!({
            "tick": w.tick, "round": w.round, "phase": w.phase, "ended": w.between_rounds(), "modifier": w.modifier.map(|m| m.id()), "cage": w.cage_left(),
            "projectiles": w.projectiles.len(), "strikes": w.strikes.len(), "pickups": w.pickups.len(),
            "arena": { "seed": w.arena.seed, "size": w.arena.size, "variant": format!("{:?}", w.arena.variant), "blocks": w.arena.blocks.len(),
                "chunks": w.arena.chunks.len(), "chunks_broken": w.arena.chunks.len() - w.arena.alive_chunks() },
            "players": players, "waves": w.coop.then_some(&w.waves), "bullets": w.bullets.len(), "events": w.events, "ragdolls": self.vis.ragdolls(), "stadium": self.vis.stadium.state(),
        }).to_string())
    }
}

fn card_id(w: &World, c: growing_guns::cards::Card) -> String { w.cards.get(c).map_or_else(|| format!("#{}", c.0), |d| d.id.clone()) }

/// `--modifier ID` forces a round modifier (`none` for none), `--rounds N`
/// sets the wins that end the match, `--coop` plays wave survival.
/// Headless tools roll no modifiers unless asked.
pub fn match_options(args: &Args, w: &mut World, play: bool) {
    if !play { w.modifier_chance = 0.0; }
    match args.value("--modifier") {
        Some("none") => w.modifier_chance = 0.0,
        Some(id) => match growing_guns::Modifier::named(id) {
            Some(m) => w.forced_modifier = Some(m),
            None => eprintln!("unknown modifier {id}; one of {}", growing_guns::MODIFIERS.map(|m| m.1).join(", ")),
        },
        None => {}
    }
    w.rounds_to_win = args.num("--rounds", w.rounds_to_win);
    w.coop = args.flag("--coop");
}

pub fn run(args: &Args) -> Result<(), String> {
    let total = args.num("--total", 4usize).clamp(1, 8);
    let humans = args.num("--players", 1usize).clamp(1, 4).min(total);
    let seed = args.num("--seed", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs()));
    let lan = args.flag("--lan");
    let (session, world, slots) = if let Some(ticket) = args.value("--join") {
        let (session, world, slots) = Session::join(ticket, humans as u8, lan)?;
        println!("joined: playing as {:?}", slots.iter().map(|s| s + 1).collect::<Vec<_>>());
        (session, world, slots)
    } else {
        let bots: Vec<bool> = (0..total).map(|i| i >= humans).collect();
        let session = if args.flag("--host") { Session::host(lan)? } else { Session::Local };
        if let Some(t) = session.ticket() {
            println!("hosting. Others join with:\n  growing-guns play --join {t}{}", if lan { " --lan" } else { "" });
            let _ = std::fs::write("growing-guns-ticket.txt", t);
        }
        (session, World::new_with(seed, &bots, |w| match_options(args, w, true)), (0..humans).collect())
    };
    println!("seed {seed}, {} local, {} players", slots.len(), world.players.len());
    let mut game = Gg::new(world, session, slots);
    // `--give explosive --give damage`: start local player 1 with these cards.
    for card in args.list.windows(2).filter(|w| w[0] == "--give").map(|w| w[1].clone()) {
        println!("{}", app::Game::command(&mut game, &format!("give 0 {card}"))?);
    }
    // Keep the speaker alive for the whole run.
    let samples = crate::sounds::find_dir(args.value("--sounds"));
    let patches = crate::sounds::find_patches(args.value("--sfx"));
    if samples.is_none() { eprintln!("no recordings: footsteps, hurts and deaths are silent (--sounds DIR or GG_SOUNDS, the Godot project's assets/audio)"); }
    if patches.is_none() { eprintln!("no sound patches (--sfx DIR or GG_SFX, games/growing-guns/sounds)"); }
    let _speaker = match crate::sounds::speakers(args.value("--speakers")) {
        Ok((speaker, tx, crowd)) => {
            if let Some(dir) = &samples { game.set_crowd(args.value("--crowd"), dir, &crowd); }
            game.set_sounds(crate::sounds::Sounds::new(tx, patches, samples.as_deref()));
            Some(speaker)
        }
        Err(e) => { eprintln!("silent: {e}"); None }
    };
    let cfg = app::Config {
        title: "Growing Guns | Warp".into(),
        tick_hz: TICK_HZ as f32,
        fullscreen: args.flag("--fullscreen"),
        keyboard: !args.flag("--no-kbm"),
        frames: args.num("--frames", 0u64),
        ..Default::default()
    };
    app::run(cfg, &mut game)
}
