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
    pub hud: hud::Hud,
    /// Start, pause and settings screens, the match-over buttons.
    pub ui: hud::menus::Menus,
    /// Name tags over the other players.
    plates: hud::plates::Plates,
    /// Seat of each local view while the start screen's background fight runs.
    start_seats: Vec<usize>,
    session: Session,
    /// World player index per local view.
    pub(crate) slots: Vec<usize>,
    since_snap: f32,
    since_poll: u32,
    rebuild: bool,
    pub(crate) sounds: Option<crate::sounds::Sounds>,
    /// The soundtrack and game.gd's music logic.
    music: Option<(crate::music::Shared, crate::music::Director)>,
    /// Card menu per local player.
    menus: Vec<app::ui::Menu>,
    pointer: (app::Pointer, [u32; 2]),
    quit: bool,
    /// Keyboard and mouse off: every local player is on a pad.
    pub no_kbm: bool,
    /// The mods (cards, rules), hot-reloaded.
    pub(crate) scripts: Option<std::sync::Arc<crate::scripts::Scripts>>,
    /// A GameNight party host, when it launched us.
    pub party: Option<crate::party::Party>,
    /// The music volume last applied.
    music_db: Option<f32>,
    /// Where the settings persist (`play` only; scripts keep the defaults).
    pub settings_file: Option<std::path::PathBuf>,
    /// `menu gamenight`: the GameNight pause menu without a party (scripts).
    gn_preview: Option<crate::party::Settings>,
    /// Party names for the seated players, in slot order.
    pub names: Vec<String>,
    /// Live control (`--live`, live.rs).
    pub(crate) live: Option<Box<crate::live::GgLive>>,
    /// Rendering only: Godot's sky or a procedural one (crate::sky).
    sky: crate::sky::Sky,
    /// A baked sky waiting for the renderer (`Renderer::update_sky`).
    sky_upload: Option<render::sky::PreparedSky>,
}

impl Gg {
    pub fn new(mut world: World, session: Session, slots: Vec<usize>) -> Self {
        let scripts = crate::scripts::attach(&mut world).map_err(|e| eprintln!("mods: {e}")).ok();
        let mut vis = view::Visuals::build(&world);
        vis.hide_own_bodies(&slots);
        Gg { prev: world.players.iter().map(|p| p.pos).collect(), vis, light: view::lighting(),
            hud: hud::Hud::new(&world), ui: Default::default(), plates: Default::default(), start_seats: Vec::new(), world, session, slots, since_snap: 0.0, since_poll: 0, rebuild: true, sounds: None, music: None, menus: Vec::new(), pointer: Default::default(), quit: false, no_kbm: false, scripts, party: None, music_db: None, settings_file: None, gn_preview: None, names: Vec::new(), live: None,
            sky: crate::sky::Sky::new(crate::sky::SkyChoice::Godot), sky_upload: None }
    }

    pub fn light_mut(&mut self) -> &mut Lighting { &mut self.light }

    /// A fresh match in place (a party's prepare): new world, views and
    /// mods; the renderer, sound and party stay.
    pub fn reset(&mut self, mut world: World, slots: Vec<usize>) {
        self.scripts = crate::scripts::attach(&mut world).map_err(|e| eprintln!("mods: {e}")).ok();
        self.vis = view::Visuals::build(&world);
        self.vis.hide_own_bodies(&slots);
        self.prev = world.players.iter().map(|p| p.pos).collect();
        self.hud = hud::Hud::new(&world);
        self.ui.open(hud::menus::Screen::Play);
        self.start_seats.clear();
        self.world = world;
        self.slots = slots;
        self.menus.clear();
        self.rebuild = true;
        // The fresh visuals carry Godot's sky: apply the chosen one again.
        self.sky.applied = None;
        let choice = self.sky.choice;
        if let Err(e) = self.set_sky(choice) { eprintln!("sky: {e}"); }
        // A new match for the music too.
        if let Some((_, d)) = &mut self.music { *d = Default::default(); }
    }

    /// GameNight's lifecycle; returns the party's controls while it runs,
    /// or None when the match is frozen (warming, paused, between sessions).
    fn party_tick(&mut self) -> Option<Option<Vec<Controls>>> {
        let party = self.party.as_mut()?;
        let evs = party.poll();
        for ev in evs {
            match ev {
                crate::party::Ev::Prepare { humans, bots } => {
                    let seat_flags: Vec<bool> = (0..humans.len() + bots).map(|i| i >= humans.len()).collect();
                    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs());
                    let mut world = World::new(seed, &seat_flags);
                    let party = self.party.as_mut().expect("party");
                    party.settings.apply_match(&mut world);
                    self.reset(world, (0..humans.len()).collect());
                    self.names = humans;
                    self.party.as_mut().expect("party").ready();
                }
                crate::party::Ev::Names(n) => self.names = n,
                crate::party::Ev::Dispose => {}
            }
        }
        let party = self.party.as_mut()?;
        if self.world.events.iter().any(|e| matches!(e, Event::MatchOver { .. })) { party.finished(); }
        if self.world.events.contains(&Event::NewArena) { party.settings.apply_round(&mut self.world); }
        Some(party.running().then(|| party.controls(self.slots.len(), DT)))
    }

    /// Client: apply what the host sent; returns the interpolation alpha.
    pub fn receive(&mut self, dt: f32) -> f32 {
        let before: Vec<Vec3> = self.world.players.iter().map(|p| p.pos).collect();
        self.hud.locals.clone_from(&self.slots);
        self.hud.set_names(&self.world, &self.slots, &self.names);
        let (vis, hud, sounds, music, slots) = (&mut self.vis, &mut self.hud, &mut self.sounds, &mut self.music, &self.slots);
        let (snaps, arena) = self.session.receive(&mut self.world, slots, |w| {
            vis.absorb(w, &crate::sounds::ears(w, slots));
            hud.tick(w, DT);
            if let Some(s) = sounds.as_mut() { s.tick(w, slots, DT); vis.stadium.sound.loudness_db = s.duck_db(); }
            follow_music(music, w, slots);
        });
        if snaps > 0 { self.prev = before; self.since_snap = 0.0; } else { self.since_snap += dt; }
        if arena { self.new_arena(); }
        (self.since_snap / DT).clamp(0.0, 1.0)
    }

    pub fn session(&self) -> &Session { &self.session }
    pub fn set_sounds(&mut self, s: crate::sounds::Sounds) { self.sounds = Some(s); }

    /// The soundtrack: shots and explosions duck it, round phases drive it.
    pub fn set_music(&mut self, m: crate::music::Shared) {
        if let Some(s) = &mut self.sounds { s.music = Some(m.clone()); }
        self.music = Some((m, Default::default()));
    }

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
        // Also between ticks (debug commands, live tools): clients need it.
        self.session.mark_arena();
        self.vis.set_arena(&self.world);
        self.prev = self.world.players.iter().map(|p| p.pos).collect();
        if matches!(self.sky.choice, crate::sky::SkyChoice::Rounds(_)) && let Err(e) = self.apply_sky() { eprintln!("sky: {e}"); }
    }

    /// Choose the sky (`--sky`, the `sky` command). Godot's is the default.
    pub fn set_sky(&mut self, choice: crate::sky::SkyChoice) -> Result<(), String> {
        self.sky.choice = choice;
        self.sky.live = None;
        self.apply_sky()?;
        if let Some(s) = self.sky.applied.filter(|_| matches!(choice, crate::sky::SkyChoice::Cycle { .. })) {
            self.sky.live = Some(render::sky::LiveSky::starting(s, &serde_json::Value::Null));
        }
        Ok(())
    }

    /// Bake and apply the sky for this round now (synchronous; see
    /// engine/docs/live-sky-controls.md for the cost). Godot's sky needs a
    /// renderer rebuild; a procedural one updates the renderer in place.
    fn apply_sky(&mut self) -> Result<(), String> {
        match self.sky.choice.settings(self.world.round, self.world.tick as f32 * DT) {
            None if self.sky.applied.is_some() => {
                self.vis.scene.sky = Some(view::sky());
                self.vis.scene.sky_physical = false;
                let taa = self.light.taa;
                self.light = Lighting { taa, ..view::lighting() };
                self.vis.set_floodlights(1.0);
                self.sky.applied = None;
                self.sky_upload = None;
                self.rebuild = true;
            }
            None => {}
            Some(s) if self.sky.applied != Some(s) => {
                let prepared = crate::sky::bake(s)?;
                self.show_sky(prepared);
            }
            Some(_) => {}
        }
        Ok(())
    }

    fn show_sky(&mut self, prepared: render::sky::PreparedSky) {
        prepared.apply_scene(&mut self.vis.scene, &mut self.light);
        self.vis.set_floodlights(crate::sky::floodlights(prepared.settings.solar().elevation_degrees));
        self.sky.applied = Some(prepared.settings);
        if let Some(live) = &mut self.sky.live { live.accepted(prepared.clone()); }
        self.sky_upload = Some(prepared);
    }

    /// The cycle: request the sun for the game clock, take finished bakes.
    fn cycle_sky(&mut self) {
        let Some(want) = self.sky.choice.settings(self.world.round, self.world.tick as f32 * DT) else { return };
        let now = std::time::Instant::now();
        let Some(live) = &mut self.sky.live else { return };
        if live.desired() != want && let Err(e) = live.request(want, now) { eprintln!("sky: {e}"); }
        if let Some(prepared) = live.poll(now) { self.show_sky(prepared); }
    }
}

/// What the menus show of the session; `party`: GameNight's settings.
fn menu_ctx<'a>(session: &Session, world: &World, ticket: Option<&'a str>, party: Option<&crate::party::Settings>, names: &[String]) -> hud::menus::Ctx<'a> {
    let status = if world.coop { format!("WAVE SURVIVAL  /  WAVE {}", world.waves.wave) } else { format!("VERSUS  /  ROUND {}  /  FIRST TO {}", world.round, world.rounds_to_win) };
    hud::menus::Ctx {
        ticket, client: session.is_client(), seed: world.arena.seed, bots: world.players.iter().filter(|p| p.bot).count(),
        players: world.players.len(), coop: world.coop, seats: if party.is_some() { names.to_vec() } else { Vec::new() }, party: party.map(|s| (s.entries(), status)),
    }
}

/// `$XDG_CONFIG_HOME/growing-guns/settings.json` (or ~/.config/...), or
/// `%APPDATA%\growing-guns\settings.json` on Windows.
fn settings_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("growing-guns").join("settings.json"))
}

/// All local players' controls as one, for the shared menus.
fn merge(controls: &[Controls]) -> Controls {
    let mut m = Controls::default();
    for c in controls {
        m.held.0 |= c.held.0;
        m.pressed.0 |= c.pressed.0;
        if m.mv == [0.0; 2] { m.mv = c.mv; }
    }
    m
}

impl Gg {
    /// The start screen: local seats become bots and fight in the background.
    pub fn show_start(&mut self) {
        self.start_seats = self.slots.clone();
        for &s in &self.slots { self.world.set_bot(s, true); }
        self.ui.open(hud::menus::Screen::Start);
    }

    /// Godot `MenuHelpers.save_settings`.
    fn save_settings(&self) {
        let Some(path) = &self.settings_file else { return };
        if let Some(d) = path.parent() { let _ = std::fs::create_dir_all(d); }
        if let Err(e) = std::fs::write(path, serde_json::to_string_pretty(&self.ui.settings_json()).unwrap_or_default()) { eprintln!("settings: {e}"); }
    }

    fn act(&mut self, a: hud::menus::Action) {
        use hud::menus::{Action, Screen};
        let host = !self.session.is_client();
        match a {
            Action::None => {}
            Action::Resume => self.ui.open(Screen::Play),
            Action::Settings => { let from_start = self.ui.screen == Screen::Start; self.ui.open(Screen::Settings { from_start }); }
            Action::Back => {
                if matches!(self.ui.screen, Screen::Settings { .. }) { self.save_settings(); }
                let a = self.ui.back();
                self.act(a);
            }
            Action::Quit => self.quit = true,
            Action::Versus | Action::Wave if host => {
                for s in std::mem::take(&mut self.start_seats) { self.world.set_bot(s, false); }
                self.world.coop = a == Action::Wave;
                self.world.restart_match();
                self.new_arena();
                self.hud = hud::Hud::new(&self.world);
                // A match starts on the round flash, as every round does.
                self.hud.round_age = 0.0;
                self.ui.open(Screen::Play);
            }
            Action::Host if matches!(self.session, Session::Local) => match Session::host(false) {
                Ok(s) => {
                    if let Some(t) = s.ticket() { println!("hosting. Others join with:\n  growing-guns play --join {t}"); let _ = std::fs::write("growing-guns-ticket.txt", t); }
                    self.session = s;
                    self.ui.notice = Some("Hosting: ticket written to growing-guns-ticket.txt".into());
                }
                Err(e) => { self.ui.notice = Some(format!("could not host: {e}")); self.ui.error = Some(format!("Couldn't start the online host: {e}")); }
            },
            Action::Copy => if let Some(t) = self.session.ticket() {
                let _ = std::fs::write("growing-guns-ticket.txt", t);
                println!("ticket: {t}");
                self.ui.notice = Some("Ticket written to growing-guns-ticket.txt".into());
            },
            Action::Rematch if host => { self.world.restart_match(); self.new_arena(); self.hud.round_age = 0.0; self.ui.open(Screen::Play); }
            Action::Advanced => self.ui.open(Screen::Advanced),
            Action::Party(k, v) => {
                if let Some(p) = &mut self.party { p.set(k, v); } else if let Some(s) = &mut self.gn_preview && let Some(key) = s.key(k) { s.change(&key, &serde_json::json!(v)); }
            }
            // Godot `_spawn_bots` / `_despawn_one_bot` from the pause menu: the
            // views are rebuilt for the new count, so not with anyone joined.
            Action::AddBot if host && self.session.remote_players() == 0 && !self.world.coop && self.world.players.len() < 8 => {
                let w = &mut self.world;
                let mut p = growing_guns::Player::new(true);
                let have = w.players.len();
                p.pos = w.arena.spawns.get(have % w.arena.spawns.len().max(1)).copied().unwrap_or_default() + Vec3::Y;
                w.players.push(p);
                rebuild_views(self);
            }
            Action::RemoveBot if host && self.session.remote_players() == 0 && !self.world.coop => {
                let keep = self.slots.iter().chain(&self.start_seats).max().map_or(1, |&m| m + 1).max(2);
                if self.world.players.len() > keep && self.world.players.last().is_some_and(|p| p.bot) {
                    self.world.players.pop();
                    rebuild_views(self);
                }
            }
            Action::Extend if host => self.world.extend_match(),
            _ => {}
        }
    }
}

impl app::Game for Gg {
    fn local_players(&self) -> usize { self.slots.len() }

    fn pointer(&mut self, pointer: app::Pointer, size: [u32; 2]) {
        self.pointer = (if self.no_kbm { app::Pointer::default() } else { pointer }, size);
    }

    fn mouse_capture(&self) -> Option<bool> {
        Some(!self.no_kbm && !self.ui.is_open() && self.hud.match_over.is_none()
            && !self.slots.first().and_then(|&s| self.world.players.get(s)).is_some_and(|p| p.offer.is_some()))
    }

    fn tick(&mut self, controls: &[Controls]) {
        let (pointer, pointer_size) = std::mem::take(&mut self.pointer);
        // Live-control requests, between ticks (also while a party is paused).
        crate::live::tick(self);
        let party_controls = match self.party_tick() {
            None => None,
            Some(None) => return,
            Some(Some(c)) => Some(c),
        };
        // Each GameNight player's own aim sensitivity (Godot `stick_sens_for`).
        let party_controls = party_controls.map(|mut c| {
            for (k, c) in c.iter_mut().enumerate() { let m = self.names.get(k).map_or(self.ui.stick, |n| self.ui.aim_for(n)); c.look = c.look.map(|v| v * m); }
            c
        });
        let controls: &[Controls] = party_controls.as_deref().unwrap_or(controls);
        // The music volume setting.
        if let Some((m, _)) = &self.music && self.music_db != Some(self.ui.music_db) {
            crate::music::lock(m).set_music_db(self.ui.music_db);
            self.music_db = Some(self.ui.music_db);
        }
        // Edited mods take effect a quarter second later, between ticks.
        self.since_poll += 1;
        if self.since_poll >= TICK_HZ / 4 {
            self.since_poll = 0;
            if let Some(s) = &self.scripts { for line in s.poll(&mut self.world) { eprintln!("mods: {line}"); } }
        }
        self.hud.locals.clone_from(&self.slots);
        self.hud.set_names(&self.world, &self.slots, &self.names);
        // Menus take the local players' controls (merged) while open.
        let merged = merge(controls);
        let ctx_ticket = self.session.ticket().map(str::to_string);
        let ctx = menu_ctx(&self.session, &self.world, ctx_ticket.as_deref(), self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names);
        let modal = self.ui.is_open() || self.hud.match_over.is_some();
        let mut action = self.ui.update(&merged, &self.world, self.hud.match_over, &ctx, DT);
        if action == hud::menus::Action::None {
            action = self.ui.pointer(pointer, pointer_size, &self.world, self.hud.match_over, &ctx);
        }
        self.act(action);
        let menu_open = self.ui.is_open();
        let mut local: Vec<Input> = if menu_open { vec![Input::default(); controls.len()] } else { controls.iter().map(input).collect() };
        // Sniper zoom slows the look (Godot x0.4).
        for (k, &s) in self.slots.iter().enumerate() {
            if self.world.players.get(s).is_some_and(|p| p.zooming) && let Some(i) = local.get_mut(k) { i.look = i.look.map(|v| v * 0.4); }
        }
        // Card picks: the menu takes this player's controls while an offer is open.
        self.menus.resize_with(self.slots.len(), Default::default);
        for (k, &s) in self.slots.iter().enumerate() {
            let Some((n, age)) = self.world.players.get(s).and_then(|p| p.offer.as_ref().map(|o| (o.len(), p.offer_age))) else { continue };
            // Godot locks the cards for 0.75 s while they flip open.
            if let (Some(c), Some(i)) = (controls.get(k), local.get_mut(k)) && !menu_open && age >= 0.75 {
                if let app::ui::MenuEvent::Select(n) = self.menus[k].update(c, n, n, DT) { i.pick = Some(n as u8); }
                // The mouse belongs to the keyboard seat; global menus always win.
                if k == 0 && !modal && (pointer.moved || pointer.pressed) && let Some(pos) = pointer.position
                    && let Some(hit) = self.hud.card_hit(&self.world, s, pointer_size, pos) {
                    self.menus[k].focus = hit;
                    if pointer.pressed { i.pick = Some(hit as u8); }
                }
            }
            if let Some(i) = local.get_mut(k) { i.fire = false; i.jump = false; }
            let focus = (age >= 0.75).then(|| self.menus[k].focus);
            if let Some(f) = self.hud.fb.get_mut(s) { hud::cards::ease_highlight(f, n, focus, DT); }
        }
        if self.session.is_client() { self.session.send(local); return; }
        let mut inputs = vec![Input::default(); self.world.players.len()];
        for (k, &s) in self.slots.iter().enumerate() { if let Some(i) = local.get(k) { inputs[s] = *i; } }
        self.session.poll(&mut self.world);
        self.session.remote_inputs(&mut inputs);
        // The pause menu stops a match nobody else is playing (Godot: one human).
        let humans = self.world.players.iter().filter(|p| !p.bot).count();
        if matches!(self.ui.screen, hud::menus::Screen::Pause | hud::menus::Screen::Settings { from_start: false }) && humans <= 1 { return; }
        self.prev = self.world.players.iter().map(|p| p.pos).collect();
        self.world.step(&inputs);
        if self.world.events.contains(&Event::NewArena) { self.new_arena(); }
        self.vis.absorb(&self.world, &crate::sounds::ears(&self.world, &self.slots));
        self.hud.tick(&self.world, DT);
        if let Some(s) = &mut self.sounds { s.tick(&self.world, &self.slots, DT); self.vis.stadium.sound.loudness_db = s.duck_db(); }
        follow_music(&mut self.music, &self.world, &self.slots);
        self.session.publish(&self.world);
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let alpha = if self.session.is_client() { self.receive(frame.dt) } else { frame.alpha };
        self.vis.set_tilt(self.ui.tilt);
        self.vis.pose(&self.world, &self.prev, alpha, frame.dt);
        if self.sky.live.is_some() { self.cycle_sky(); }
        if std::mem::take(&mut self.rebuild) || frame.renderer().is_none() {
            frame.set_scene(&self.vis.scene, &self.light);
            self.plates.invalidate();
            self.sky_upload = None;
        } else if let Some(prepared) = self.sky_upload.take() {
            let gpu = frame.gpu;
            let r = frame.renderer_mut().expect("renderer checked above");
            if let Err(e) = r.update_sky(gpu, &prepared.sky, &mut self.light) { eprintln!("sky unchanged on the GPU: {e}"); }
        }
        crate::live::frame(self, frame);
        let r = frame.renderer().expect("scene set above");
        self.vis.flush(frame.gpu, r)?;
        r.update_instances(frame.gpu, self.vis.first_dynamic, self.vis.dynamic())?;
        r.set_local_lights(&self.vis.local_lights())?;
        let start = matches!(self.ui.screen, hud::menus::Screen::Start | hud::menus::Screen::Settings { from_start: true });
        // Name tags over everyone not seated here (on the start screen, everyone).
        let drawn: Vec<Vec3> = self.world.players.iter().enumerate().map(|(i, p)| self.prev.get(i).map_or(p.pos, |q| q.lerp(p.pos, alpha))).collect();
        self.plates.update(frame.gpu, r, &self.world, &self.hud, if start { &[] } else { &self.slots }, &drawn)?;
        let mut list = HudDrawList::new(frame.size);
        self.hud.begin_pointer_frame(frame.size);
        let cameras = if start {
            // The start screen orbits the arena while bots fight (Godot: 82 m out, 34 up, FOV 66).
            let t = self.ui.age * 0.055;
            let r = self.world.arena.size * 1.14;
            vec![render::Camera { pos: Vec3::new(t.cos() * r, 34.0, t.sin() * r), target: Vec3::new(0.0, 5.5, 0.0), fov_y: 66f32.to_radians(), near: 0.1 }]
        } else {
            let rects = frame.rects(self.slots.len());
            for (k, (rect, &s)) in rects.iter().zip(&self.slots).enumerate() {
                let flash = self.vis.ion_flash(self.world.players[s].eye());
                let pad = k > 0 || self.no_kbm;
                self.hud.view(&mut list, &self.world, s, *rect, self.menus.get(k).map_or(0, |m| m.focus), flash, pad);
            }
            list.begin_overlay();
            let error = self.session.error().or_else(|| self.ui.error.clone());
            self.hud.global(&mut list, &self.world, error.as_deref());
            self.slots.iter().map(|&s| self.vis.punched(&self.world, s, view::camera(&self.world, s, &self.prev, alpha))).collect()
        };
        list.begin_overlay();
        let ticket = self.session.ticket().map(str::to_string);
        let ctx = menu_ctx(&self.session, &self.world, ticket.as_deref(), self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names);
        self.ui.draw(&mut list, &self.world, &self.hud, &ctx);
        self.hud.round_flash(&mut list);
        let mut light = self.light.clone();
        // The lava flows on the game clock.
        light.time = (self.world.tick as f32 - 1.0 + alpha) * DT;
        Ok(Draw { cameras, light, hud: list })
    }

    fn title(&self) -> Option<String> { Some(format!("round {}", self.world.round)) }
    fn wants_capture(&self) -> bool { crate::live::wants_capture(self) }
    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) { crate::live::captured(self, size, rgba) }

    fn screen(&self) -> app::Screen { self.party.as_ref().map_or(app::Screen::Normal, |p| p.screen()) }

    fn split(&self) -> render::SplitLayout { render::SplitLayout::SideBySide }

    fn escape(&mut self) -> bool {
        use hud::menus::Screen;
        match self.ui.screen {
            // Esc on the start screen leaves the game (the shell first frees the mouse).
            Screen::Start => false,
            _ => { let a = self.ui.back(); self.act(a); true }
        }
    }

    fn quit(&self) -> bool { self.quit }

    fn tuning(&self, t: &mut app::Tuning) {
        let d = app::Tuning::default();
        t.mouse = d.mouse * self.ui.mouse;
        t.stick = d.stick * self.ui.stick;
    }

    fn command(&mut self, cmd: &str) -> Result<String, String> {
        let w: Vec<&str> = cmd.split_whitespace().collect();
        let num = |k: usize| -> Result<f32, String> { w.get(k).ok_or(format!("{cmd}: missing argument {k}"))?.parse::<f32>().map_err(|e| format!("{cmd}: {e}")) };
        let player = |k: usize, world: &World| -> Result<usize, String> {
            let p = num(k)? as usize;
            if p < world.players.len() { Ok(p) } else { Err(format!("no player {p}")) }
        };
        match w.first().copied().unwrap_or("") {
            "help" => Ok("menu start|pause|settings|gamenight|advanced|none | names NAME,NAME.. | net-error TEXT|none | give P CARD | teleport P X Y Z | aim P X Y Z | health P N | ammo P N | bots pause|resume | blast X Y Z R DAMAGE | boom X Y Z R [DAMAGE] | arena SEED | cards | lua CODE | mods | special P NAME | modifier ID|none | sky [godot|TIME[:WEATHER]|rounds|cycle] | round | cage | rematch | extend".into()),
            // Show (or clear with `none`) a network error in the red panel.
            "net-error" => {
                let t = cmd.trim_start().strip_prefix("net-error").unwrap_or("").trim();
                self.ui.error = (t != "none" && !t.is_empty()).then(|| t.to_string());
                Ok(format!("net error {:?}", self.ui.error))
            }
            // Party names for the local seats, comma separated (GameNight's `names`).
            "names" => {
                self.names = cmd.trim_start().strip_prefix("names").unwrap_or("").split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
                self.hud.set_names(&self.world, &self.slots, &self.names);
                Ok(format!("names {:?}", self.names))
            }
            "menu" => {
                use hud::menus::Screen;
                match w.get(1).copied().unwrap_or("") {
                    "start" => self.show_start(),
                    "pause" => self.ui.open(Screen::Pause),
                    "settings" => self.ui.open(Screen::Settings { from_start: false }),
                    "none" => self.ui.open(Screen::Play),
                    "gamenight" => { self.gn_preview.get_or_insert_with(Default::default); self.ui.open(Screen::Pause) }
                    "advanced" => { self.gn_preview.get_or_insert_with(Default::default); self.ui.open(Screen::Advanced) }
                    m => return Err(format!("menu start|pause|settings|gamenight|advanced|none, not {m}")),
                }
                Ok(format!("menu {:?}", self.ui.screen))
            }
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
            "sky" => {
                if let Some(name) = w.get(1) { self.set_sky(crate::sky::SkyChoice::parse(name)?)?; }
                Ok(format!("sky {}", self.sky.choice.name()))
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
            "tick": w.tick, "round": w.round, "phase": w.phase, "screen": format!("{:?}", self.ui.screen), "net": self.session.status(&self.slots), "ended": w.between_rounds(), "modifier": w.modifier.map(|m| m.id()), "cage": w.cage_left(),
            "projectiles": w.projectiles.len(), "strikes": w.strikes.len(), "pickups": w.pickups.len(),
            "arena": { "seed": w.arena.seed, "size": w.arena.size, "variant": format!("{:?}", w.arena.variant), "blocks": w.arena.blocks.len(),
                "chunks": w.arena.chunks.len(), "chunks_broken": w.arena.chunks.len() - w.arena.alive_chunks() },
            "players": players, "waves": w.coop.then_some(&w.waves), "bullets": w.bullets.len(), "events": w.events, "ragdolls": self.vis.ragdolls(), "stadium": self.vis.stadium.state(),
            "sky": { "choice": self.sky.choice.name(), "applied": self.sky.applied.map(|s| s.metadata()) },
        }).to_string())
    }
}

fn follow_music(music: &mut Option<(crate::music::Shared, crate::music::Director)>, world: &World, slots: &[usize]) {
    let Some((m, director)) = music else { return };
    let mut m = crate::music::lock(m);
    director.follow(&mut *m, world, slots);
    m.update(DT);
}

/// The soundtrack unless `--no-music`, its stems from `--music DIR` (see
/// music::find_dir); loaded on a thread when `background`.
pub fn music(args: &Args, sounds: Option<&std::path::Path>, seed: u64, background: bool) -> Option<crate::music::Shared> {
    if args.flag("--no-music") { return None; }
    let Some(dir) = crate::music::find_dir(args.value("--music"), sounds) else {
        eprintln!("no music: no stems (--music DIR or GG_MUSIC, the Godot project's assets/music)");
        return None;
    };
    let m = crate::music::shared(crate::music::Music::new(audio::SAMPLE_RATE, seed));
    crate::music::load_into(&m, dir, background);
    Some(m)
}

/// After the player count changed (live `spawn_bots`): new views and a new
/// renderer scene; the crowd keeps its voice.
pub fn rebuild_views(gg: &mut Gg) {
    let sound = std::mem::replace(&mut gg.vis.stadium.sound, crate::crowd_sound::CrowdSound::new(None));
    gg.vis = view::Visuals::build(&gg.world);
    gg.vis.hide_own_bodies(&gg.slots);
    gg.vis.stadium.sound = sound;
    gg.prev = gg.world.players.iter().map(|p| p.pos).collect();
    gg.hud.sync(&gg.world);
    gg.rebuild = true;
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
    game.party = crate::party::Party::connect();
    if game.party.is_some() { println!("gamenight: waiting for the party host"); }
    game.live = crate::live::start(args)?.map(Box::new);
    game.no_kbm = args.flag("--no-kbm");
    // Settings persist like Godot's user://settings.cfg.
    game.settings_file = settings_path();
    if let Some(v) = game.settings_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) { game.ui.load_settings(&v); }
    // Godot opens on the start screen; `--skip-menu`, joining and a party go straight in.
    if !args.flag("--skip-menu") && !game.session.is_client() && game.party.is_none() { game.show_start(); }
    game.set_sky(crate::sky::Sky::from_args(args.value("--sky"))?)?;
    // `--give explosive --give damage`: start local player 1 with these cards.
    for card in args.list.windows(2).filter(|w| w[0] == "--give").map(|w| w[1].clone()) {
        println!("{}", app::Game::command(&mut game, &format!("give 0 {card}"))?);
    }
    // Keep the speaker alive for the whole run.
    let samples = crate::sounds::find_dir(args.value("--sounds"));
    let patches = crate::sounds::find_patches(args.value("--sfx"));
    if samples.is_none() { eprintln!("no recordings: footsteps, hurts and deaths are silent (--sounds DIR or GG_SOUNDS, the Godot project's assets/audio)"); }
    if patches.is_none() { eprintln!("no sound patches (--sfx DIR or GG_SFX, games/growing-guns/sounds)"); }
    let music = music(args, samples.as_deref(), seed, true);
    let _speaker = match crate::sounds::speakers(args.value("--speakers"), music.as_ref()) {
        Ok((speaker, tx, crowd)) => {
            if let Some(dir) = &samples { game.set_crowd(args.value("--crowd"), dir, &crowd); }
            game.set_sounds(crate::sounds::Sounds::new(tx, patches, samples.as_deref()));
            if let Some(m) = music { game.set_music(m); }
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
