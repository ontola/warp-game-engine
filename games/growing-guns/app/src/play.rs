//! Growing Guns as a Warp `app::Game`: local, host or client sessions with
//! 1 to 4 local players in split screen, the rest bots. Window, tick, input
//! and presentation come from the engine's app shell.
//!
//! Splitscreen hot-join (Godot `splitscreen_manager.gd`): with splitscreen
//! on (the pause menu's SPLITSCREEN, or more than one local seat), a pad no
//! seat uses joins by pressing X: the new local player takes over a bot's
//! seat, or (offline, versus, nobody remote) a new seat is added. A joined
//! seat's Start opens its PLAYER MENU (RESUME / LEAVE); leaving hands the
//! seat back to a bot. Seats change between ticks on the host only, so the
//! simulation stays deterministic and clients just follow the snapshots.
//! Under GameNight the party owns the seats and there is no hot-join.
use crate::session::Session;
use crate::{hud, view, Args};
use app::{Buttons, Controls, Device, Draw, Frame};
use glam::Vec3;
use growing_guns::{Event, Input, World, DT, TICK_HZ};
use render::hud::HudDrawList;
use render::Lighting;

/// The floodlight banks' size for their soft shadows (metres).
const FLOOD_SOFTNESS: f32 = 0.6;

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
    pub(crate) recorder: crate::replay::Recorder,
    replay: Option<Box<crate::replay::Viewer>>,
    replay_size: [u32; 2],
    replay_alpha: f32,
    prev: Vec<view::Pose>,
    /// Whether the last tick advanced the world; a frozen match (paused,
    /// warming) draws its current state instead of interpolating.
    stepped: bool,
    pub(crate) xr_gun: Option<glam::Affine3A>,
    pub(crate) xr_aim: Option<growing_guns::TrackedAim>,
    pub(crate) xr_stereo: bool,
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
    /// The atmosphere shown is the island arenas' (Godot's sky only).
    inferno: bool,
    pub(crate) sounds: Option<crate::sounds::Sounds>,
    /// The crowd's mixer, muted while a host keeps the match off screen
    /// (warming, paused), as the Godot SDK mutes the master bus.
    crowd_tx: Option<audio::sfx::SfxSender>,
    crowd_audible: bool,
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
    /// `ui.aim` as last sent to or taken from GameNight (players' own setting).
    aim_synced: Vec<(String, f32)>,
    /// Live control (`--live`, live.rs).
    pub(crate) live: Option<Box<crate::live::GgLive>>,
    /// Rendering only: Godot's sky or a procedural one (crate::sky).
    sky: crate::sky::Sky,
    /// A baked sky waiting for the renderer (`Renderer::update_sky`).
    sky_upload: Option<render::sky::PreparedSky>,
    /// The light a blackout round replaced.
    before_blackout: Option<Lighting>,
    /// `light KEY=VALUE` settings, applied again over every sky change.
    light_overrides: Vec<String>,
    /// Scatter weights of the stadium's lights (`light flood_scatter=..`).
    scatter: view::Scatter,
    /// Splitscreen: spare pads may join (see the module notes).
    pub split: bool,
    /// The devices driving each local seat while split (`Game::seats`).
    binding: Vec<Vec<Device>>,
    /// Devices connected now; the last pad used, and whether the last input
    /// came from a pad (Godot `_last_input_was_controller`).
    connected: Vec<Device>,
    last_pad: Option<Device>,
    pad_last: bool,
    /// Local seats that joined this tick: their join press is not play input.
    joined_now: Vec<usize>,
    /// A joined seat's PLAYER MENU: local seat and its focus.
    player_menu: Option<(usize, app::ui::Menu)>,
    /// A join from the menus' field, connecting on its own thread.
    joining: Option<std::sync::mpsc::Receiver<Result<(Session, World, Vec<usize>), String>>>,
    /// The Player name as last saved.
    saved_name: String,
    /// Menu joins skip relays (`--lan`, tests).
    pub lan: bool,
    /// The F3 debug overlay (hud/debug.rs): it runs the commands below.
    pub debug: hud::debug::DebugMenu,
}

impl Gg {
    pub fn new(mut world: World, session: Session, slots: Vec<usize>) -> Self {
        let scripts = crate::scripts::attach(&mut world).map_err(|e| eprintln!("mods: {e}")).ok();
        let mut vis = view::Visuals::build(&world);
        vis.hide_own_bodies(&slots);
        Gg { recorder: Default::default(), replay: None, replay_size: [1600, 900], replay_alpha: 1.0, prev: view::Pose::all(&world), stepped: true, xr_gun: None, xr_aim: None, xr_stereo: false, vis, light: view::lighting(),
            hud: hud::Hud::new(&world), ui: Default::default(), plates: Default::default(), start_seats: Vec::new(), world, session, slots, since_snap: 0.0, since_poll: 0, rebuild: true, inferno: false, sounds: None, crowd_tx: None, crowd_audible: true, music: None, menus: Vec::new(), pointer: Default::default(), quit: false, no_kbm: false, scripts, party: None, music_db: None, settings_file: None, gn_preview: None, names: Vec::new(), aim_synced: Vec::new(), live: None,
            sky: crate::sky::Sky::new(crate::sky::SkyChoice::Godot), sky_upload: None, before_blackout: None, light_overrides: Vec::new(), scatter: Default::default(),
            split: false, binding: Vec::new(), connected: Vec::new(), last_pad: None, pad_last: false, joined_now: Vec::new(), player_menu: None, joining: None, saved_name: String::new(), lan: false, debug: Default::default() }
            .with_split()
    }

    fn with_split(mut self) -> Self { self.split = self.slots.len() > 1; self }

    pub(crate) fn xr_eye(&self, alpha: f32) -> Vec3 {
        view::pose(&self.world, self.slots[0], &self.prev, if self.stepped { alpha } else { 1.0 }).pos + Vec3::Y * growing_guns::EYE
    }

    pub fn light_mut(&mut self) -> &mut Lighting { &mut self.light }

    /// Apply recorded gameplay without executing current simulation rules/mods.
    pub(crate) fn replay_apply(&mut self, sample: crate::replay::Sample, arena: Option<growing_guns::arena::Arena>) -> Result<(), String> {
        self.prev = view::Pose::all(&self.world);
        let new_arena = arena.is_some();
        if let Some(arena) = arena { self.world.arena = arena; }
        self.world.arena.apply_breakage(&sample.broken);
        self.world.breakage = sample.broken;
        self.world.apply(sample.state);
        self.slots = sample.slots;
        self.ui.tilt = sample.tilt;
        if new_arena { self.new_arena(); }
        if self.vis.player_capacity() < self.world.players.len() { rebuild_views(self); }
        self.vis.hide_own_bodies(&self.slots);
        self.hud.locals.clone_from(&self.slots);
        self.hud.set_names(&self.world, &self.slots, &[]);
        let sky = crate::sky::SkyChoice::parse(&sample.sky)?;
        if self.sky.choice != sky { self.set_sky(sky)?; }
        // Shot feedback needs this frame's controller pose, including after
        // rebuilding the arena, before the render-time gun posing runs.
        if self.xr_stereo {self.vis.track_gun(self.slots[0],self.xr_gun);}
        self.vis.absorb(&self.world, &crate::sounds::ears(&self.world, &self.slots));
        self.hud.tick(&self.world, DT);
        // Canonical 60 Hz presentation state, independent of review framerate.
        self.vis.pose(&self.world, &self.prev, 1.0, DT);
        self.stepped = true;
        Ok(())
    }

    pub(crate) fn replay_gpu_tick(&mut self, frame: &mut Frame) -> Result<(), String> {
        if self.rebuild || frame.renderer().is_none() {
            self.vis.scene.live_screen_feeds.clear();
            app::Game::draw(self, frame)?;
        }
        let renderer = frame.renderer().ok_or("replay renderer unavailable")?;
        self.vis.flush(frame.gpu, renderer, &self.light)?;
        renderer.advance_replay_debris(frame.gpu);
        Ok(())
    }

    /// A fresh match in place (a party's prepare): new world, views and
    /// mods; the renderer, sound and party stay.
    pub fn reset(&mut self, mut world: World, slots: Vec<usize>) {
        self.recorder.barrier();
        self.scripts = crate::scripts::attach(&mut world).map_err(|e| eprintln!("mods: {e}")).ok();
        let bake = self.vis.stadium.bake_in_background;
        // The crowd's voices outlive the stadium they cheer in.
        let crowd = std::mem::replace(&mut self.vis.stadium.sound, crate::crowd_sound::CrowdSound::new(None));
        self.vis = view::Visuals::build(&world);
        self.vis.stadium.bake_in_background = bake;
        self.vis.stadium.set_sound(crowd);
        self.vis.stadium.sound.kickoff();
        self.vis.hide_own_bodies(&slots);
        self.prev = view::Pose::all(&world);
        self.hud = hud::Hud::new(&world);
        self.ui.open(hud::menus::Screen::Play);
        self.start_seats.clear();
        self.world = world;
        self.slots = slots;
        self.menus.clear();
        self.split = self.slots.len() > 1;
        self.binding.clear();
        self.player_menu = None;
        self.rebuild = true;
        // The fresh visuals carry Godot's sky: apply the chosen one again.
        self.sky.applied = None;
        self.inferno = false;
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
                    world.next_modifier = party.next_modifier;
                    self.reset(world, (0..humans.len()).collect());
                    self.names = humans;
                    self.party.as_mut().expect("party").ready();
                }
                crate::party::Ev::Names(n) => self.names = n,
                crate::party::Ev::NextModifier(m) => self.world.next_modifier = m,
                crate::party::Ev::Sticks(sticks) => {
                    for (k, v) in sticks { if let Some(n) = self.names.get(k).cloned() { self.ui.set_aim(&n, v); } }
                    self.aim_synced.clone_from(&self.ui.aim);
                }
                crate::party::Ev::Dispose => {}
            }
        }
        let party = self.party.as_mut()?;
        if self.world.events.iter().any(|e| matches!(e, Event::MatchOver { .. })) { party.finished(); }
        if self.world.events.contains(&Event::NewArena) {
            party.settings.apply_round(&mut self.world);
            // A pick for this round was taken (World::start_round).
            if self.world.next_modifier.is_none() { party.next_modifier_used(); }
        }
        Some(party.running().then(|| party.controls(self.slots.len(), DT)))
    }

    /// Client: apply what the host sent; returns the interpolation alpha.
    pub fn receive(&mut self, dt: f32) -> f32 {
        self.hud.locals.clone_from(&self.slots);
        let names = self.seat_names();
        self.hud.set_names(&self.world, &self.slots, &names);
        let (vis, hud, sounds, music, slots, prev, since_snap) = (
            &mut self.vis,
            &mut self.hud,
            &mut self.sounds,
            &mut self.music,
            &self.slots,
            &mut self.prev,
            &mut self.since_snap,
        );
        let (_snaps, arena) = self.session.receive(&mut self.world, prev, since_snap, dt, slots, |w| {
            vis.absorb(w, &crate::sounds::ears(w, slots));
            hud.tick(w, DT);
            if let Some(s) = sounds.as_mut() { s.tick(w, slots, DT); vis.stadium.sound.loudness_db = s.duck_db(); }
            follow_music(music, w, slots);
        });
        let needed = if self.world.coop { self.world.players.len().max(view::COOP_VIEW_PLAYERS) } else { self.world.players.len() };
        if self.vis.player_capacity() < needed {
            rebuild_views(self);
        } else if arena {
            self.new_arena();
        }
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
    pub fn set_crowd(&mut self, arg: Option<&str>, sounds: Option<&std::path::Path>, tx: &audio::sfx::SfxSender) {
        let Some(dir) = crate::crowd_sound::find_dir(arg, sounds) else { eprintln!("silent crowd: no crowd_sound samples (--crowd DIR or GG_CROWD)"); return };
        eprintln!("crowd: samples from {}", dir.display());
        match crate::crowd_sound::Samples::load(&dir) {
            Ok(s) => {
                let mut c = crate::crowd_sound::CrowdSound::new(Some((tx.clone(), s)));
                c.kickoff();
                self.vis.stadium.set_sound(c);
                self.crowd_tx = Some(tx.clone());
                self.crowd_audible = true;
            }
            Err(e) => eprintln!("silent crowd: {e}"),
        }
    }

    /// A new round: refill the arena slots; the renderer is kept.
    fn new_arena(&mut self) {
        self.recorder.barrier();
        // Also between ticks (debug commands, live tools): clients need it.
        self.session.mark_arena();
        self.vis.set_arena(&self.world);
        self.prev = view::Pose::all(&self.world);
    }

    /// Choose the sky (`--sky`, the `sky` command). Godot's is the default.
    pub fn set_sky(&mut self, choice: crate::sky::SkyChoice) -> Result<(), String> {
        self.sky.choice = choice;
        self.sky.live = None;
        self.apply_sky()?;
        if let Some(s) = self.sky.applied.filter(|_| matches!(choice, crate::sky::SkyChoice::Cycle { .. } | crate::sky::SkyChoice::Rounds(_))) {
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
                self.vis.scene.sky = Some(view::sky_for(self.inferno));
                self.vis.scene.sky_physical = false;
                let taa = self.light.taa;
                self.light = Lighting { taa, ..view::lighting_for(self.inferno) };
                self.vis.set_floodlights(1.0);
                self.apply_light_overrides();
                self.keep_blackout();
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

    /// Godot's island arenas have their own atmosphere (`_inferno_palette`):
    /// under Godot's sky, swap the sky and sun when a round changes between
    /// an island arena and another (a renderer rebuild, only then). The
    /// procedural skies keep theirs.
    fn sync_atmosphere(&mut self) {
        let inferno = self.world.arena.variant == growing_guns::arena::Variant::Island;
        if inferno == self.inferno { return; }
        self.inferno = inferno;
        if self.sky.applied.is_some() { return; }
        self.vis.scene.sky = Some(view::sky_for(inferno));
        let taa = self.light.taa;
        self.light = Lighting { taa, ..view::lighting_for(inferno) };
        self.apply_light_overrides();
        self.keep_blackout();
        self.rebuild = true;
    }

    /// A blackout round darkens the scene's light (the sky's light is baked
    /// into the scene, so it rebuilds; a new round rebuilds anyway).
    fn sync_blackout(&mut self) {
        let on = self.world.modifier == Some(growing_guns::Modifier::Blackout);
        if on == self.vis.blackout { return; }
        self.vis.blackout = on;
        if on {
            self.before_blackout = Some(self.light.clone());
            blackout_light(&mut self.light);
        } else if let Some(l) = self.before_blackout.take() {
            self.light = Lighting { taa: self.light.taa, ..l };
        }
        self.rebuild = true;
    }

    /// The light was replaced during a blackout: that is the light to come back to.
    fn keep_blackout(&mut self) {
        if !self.vis.blackout { return; }
        self.before_blackout = Some(self.light.clone());
        blackout_light(&mut self.light);
    }

    fn show_sky(&mut self, prepared: render::sky::PreparedSky) {
        prepared.apply_scene(&mut self.vis.scene, &mut self.light);
        self.vis.set_floodlights(crate::sky::floodlights(prepared.settings.solar().elevation_degrees));
        self.light.volumetric = crate::sky::volumetric(&prepared.settings);
        self.apply_light_overrides();
        self.keep_blackout();
        self.sky.applied = Some(prepared.settings);
        if let Some(live) = &mut self.sky.live { live.accepted(prepared.clone()); }
        self.sky_upload = Some(prepared);
    }

    /// Re-apply the `light` command's settings after the sky replaced them.
    fn apply_light_overrides(&mut self) {
        for kv in self.light_overrides.clone() { if let Err(e) = self.set_light(&kv) { eprintln!("light {kv}: {e}"); } }
    }

    /// One `light` setting: `Lighting::set`, or the stadium's scatter weights
    /// `flood_scatter`, `beam_scatter`, `lava_scatter`, `beacon_scatter`,
    /// `pickup_scatter` (drop beam haze and ground lights).
    fn set_light(&mut self, kv: &str) -> Result<(), String> {
        let weight = |v: &str| v.parse::<f32>().ok().filter(|w| w.is_finite() && (0.0..=100.0).contains(w)).ok_or(format!("{kv}: weight 0..100"));
        match kv.split_once('=') {
            Some(("flood_scatter", v)) => self.scatter.flood = weight(v)?,
            Some(("beam_scatter", v)) => self.scatter.beam = weight(v)?,
            Some(("lava_scatter", v)) => self.scatter.lava = weight(v)?,
            Some(("beacon_scatter", v)) => self.scatter.beacon = weight(v)?,
            Some(("pickup_scatter", v)) => self.scatter.pickup = weight(v)?,
            _ => {
                let mut probe = self.light.clone();
                probe.set(kv)?;
                self.light = probe;
            }
        }
        Ok(())
    }

    /// `light` (show), `light KEY=VALUE..` (set and keep over sky changes;
    /// see `Lighting::set`, e.g. `volumetric=0|1`, `vol_density`,
    /// `vol_intensity`, `vol_anisotropy`, `vol_r/g/b`), `light reset`.
    fn light_command(&mut self, args: &[&str]) -> Result<String, String> {
        if args == ["reset"] {
            self.light_overrides.clear();
            self.scatter = Default::default();
            let taa = self.light.taa;
            self.light = Lighting { taa, ..view::lighting() };
            self.sky.applied = None;
            let choice = self.sky.choice;
            self.set_sky(choice)?;
        }
        for kv in args.iter().filter(|a| **a != "reset") {
            self.set_light(kv)?;
            let key = kv.split_once('=').map_or(*kv, |(k, _)| k);
            self.light_overrides.retain(|o| o.split_once('=').map_or(o.as_str(), |(k, _)| k) != key);
            self.light_overrides.push(kv.to_string());
        }
        Ok(format!("light overrides {:?}, volumetric {:?}, {:?}", self.light_overrides, self.light.volumetric, self.scatter))
    }

    /// Rounds and the cycle: request the sky for this round and the game
    /// clock off the frame loop (a bake is ~0.2 s), take finished bakes.
    fn cycle_sky(&mut self) {
        let Some(want) = self.sky.choice.settings(self.world.round, self.world.tick as f32 * DT) else { return };
        let now = std::time::Instant::now();
        let Some(live) = &mut self.sky.live else { return };
        if live.desired() != want && let Err(e) = live.request(want, now) { eprintln!("sky: {e}"); }
        if let Some(prepared) = live.poll(now) { self.show_sky(prepared); }
    }
}

/// `GG_HITCH=MS` (default 12 under GameNight, else off): print any step of
/// a tick or frame slower than that, to find what freezes the screen.
pub(crate) fn hitch(what: &str, since: std::time::Instant) {
    static LIMIT: std::sync::LazyLock<Option<f32>> = std::sync::LazyLock::new(|| {
        std::env::var("GG_HITCH").ok().and_then(|v| v.parse().ok()).or_else(|| std::env::var("GAMENIGHT").is_ok().then_some(12.0)).filter(|&l: &f32| l > 0.0)
    });
    let Some(limit) = *LIMIT else { return };
    let ms = since.elapsed().as_secs_f32() * 1000.0;
    if ms >= limit { eprintln!("hitch: {what} {ms:.0} ms"); }
}

/// A blackout round (Godot `game.gd`: ambient, sun and fill to zero, a
/// near-black sky, no fog): only the flashlights and the emissive accents
/// light the arena, exposed for them.
fn blackout_light(light: &mut Lighting) {
    light.sun_lux = 0.0;
    light.sky_lux = 0.0;
    light.fog_density = 0.0;
    light.volumetric = None;
    light.ev100 = *BLACKOUT_EV100;
    light.fx_white_floor = light.fx_white_floor.max(1.0);
}

/// The exposure of a blackout round (`GG_BLACKOUT_EV` overrides).
const BLACKOUT_EV: f32 = 8.0;
static BLACKOUT_EV100: std::sync::LazyLock<f32> = std::sync::LazyLock::new(|| std::env::var("GG_BLACKOUT_EV").ok().and_then(|v| v.parse().ok()).unwrap_or(BLACKOUT_EV));

/// What the menus show of the session; `party`: GameNight's settings.
fn menu_ctx<'a>(session: &Session, world: &World, start_seats: &[usize], ticket: Option<&'a str>, party: Option<&crate::party::Settings>, names: &[String], xr: bool) -> hud::menus::Ctx<'a> {
    let status = if world.coop { format!("WAVE SURVIVAL  /  WAVE {}", world.waves.wave) } else { format!("VERSUS  /  ROUND {}  /  FIRST TO {}", world.round, world.rounds_to_win) };
    let bots = (0..world.players.len()).filter(|i| world.players[*i].bot && !start_seats.contains(i)).count();
    hud::menus::Ctx {
        xr, ticket, client: session.is_client(), seed: world.arena.seed, bots,
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
        self.vis.hide_own_bodies(&[]);
        self.ui.open(hud::menus::Screen::Start);
    }

    /// Names for the local seats: the party's (or `names`), else the Player
    /// name for the first seat.
    fn seat_names(&self) -> Vec<String> {
        if !self.names.is_empty() || self.ui.player_name.is_empty() { self.names.clone() } else { vec![self.ui.player_name.clone()] }
    }

    fn ctx_parts(&self) -> (Option<String>, bool) { (self.session.ticket().map(str::to_string), self.party.is_some()) }

    /// Hot-join is possible here: splitscreen on, the host's (or an offline)
    /// match, no party, in play.
    fn can_join(&self) -> bool {
        self.split && self.party.is_none() && !self.session.is_client() && self.start_seats.is_empty() && !matches!(self.ui.screen, hud::menus::Screen::Start)
    }

    /// Godot `_should_show_splitscreen_hint`: a spare pad is connected and
    /// splitscreen is off (one pad only counts while the keyboard is in use).
    fn hint_visible(&self) -> bool {
        let pads = self.connected.iter().filter(|d| d.is_pad()).count();
        self.party.is_none() && !self.split && !self.session.is_client() && pads > 0 && (pads >= 2 || !self.pad_last)
    }

    /// Godot `_join_player`: device `dev` gets a local seat. It takes over a
    /// bot's seat (not a remote's, not a wave enemy); offline in versus with
    /// none free, a new seat is added.
    fn join_local(&mut self, dev: Device) -> Result<usize, String> {
        if self.session.is_client() { return Err("SPLITSCREEN IS HOST-ONLY".into()); }
        if self.slots.len() >= 4 { return Err("ALL 4 SEATS ARE TAKEN".into()); }
        let taken = self.session.remote_slots();
        let w = &self.world;
        let seats = if w.coop { w.waves.seats.min(w.players.len()) } else { w.players.len() };
        let free = (0..seats).find(|i| w.players[*i].bot && !self.slots.contains(i) && !taken.contains(i));
        let s = match free {
            Some(s) => s,
            None if self.session.remote_players() == 0 && !w.coop && w.players.len() < 8 => {
                self.world.add_bot()
            }
            None => return Err("GAME FULL".into()),
        };
        self.world.join_human(s);
        self.slots.push(s);
        self.binding.push(vec![dev]);
        self.joined_now.push(self.slots.len() - 1);
        rebuild_views(self);
        let names = self.seat_names();
        self.hud.locals.clone_from(&self.slots);
        self.hud.set_names(&self.world, &self.slots, &names);
        self.hud.sync(&self.world);
        let n = self.hud.name(s).to_string();
        self.hud.announce(format!("{n} joined"), 2.0, 0.0);
        println!("{dev} joined as player {}", s + 1);
        Ok(s)
    }

    /// Godot `_leave_player`: local seat `k` (not the first) goes back to a bot.
    fn leave_local(&mut self, k: usize) {
        if k == 0 || k >= self.slots.len() { return; }
        let s = self.slots.remove(k);
        if k < self.binding.len() { self.binding.remove(k); }
        if k < self.menus.len() { self.menus.remove(k); }
        self.player_menu = None;
        self.world.set_bot(s, true);
        let n = self.hud.name(s).to_string();
        rebuild_views(self);
        self.hud.locals.clone_from(&self.slots);
        self.hud.sync(&self.world);
        self.hud.announce(format!("{n} left"), 2.0, 0.0);
    }

    /// The shared menus' context.
    fn ctx<'a>(&'a self, ticket: Option<&'a str>) -> hud::menus::Ctx<'a> {
        menu_ctx(&self.session, &self.world, &self.start_seats, ticket, self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names, self.xr_stereo)
    }

    /// A finished join from the menu: the new session replaces this one.
    fn poll_join(&mut self) {
        let Some(rx) = &self.joining else { return };
        let got = match rx.try_recv() { Ok(r) => r, Err(std::sync::mpsc::TryRecvError::Empty) => return, Err(_) => Err("the join stopped".into()) };
        self.joining = None;
        self.ui.joining = false;
        match got {
            Ok((session, world, slots)) => {
                println!("joined: playing as {:?}", slots.iter().map(|s| s + 1).collect::<Vec<_>>());
                self.session = session;
                self.reset(world, slots);
                self.ui.join.set("");
            }
            Err(e) => { self.ui.notice = Some(format!("Could not join: {e}")); }
        }
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
                let needed = if self.world.coop { self.world.players.len().max(view::COOP_VIEW_PLAYERS) } else { self.world.players.len() };
                if self.vis.player_capacity() < needed {
                    rebuild_views(self);
                } else {
                    self.vis.hide_own_bodies(&self.slots);
                    self.new_arena();
                }
                self.hud = hud::Hud::new(&self.world);
                // A match starts on the round flash, as every round does.
                self.hud.round_age = 0.0;
                self.ui.open(Screen::Play);
            }
            Action::Host if matches!(self.session, Session::Local) => match Session::host(false) {
                // The field shows the match ID; COPY puts it on the clipboard.
                Ok(s) => {
                    if let Some(t) = s.ticket() { println!("hosting. Others join with:\n  growing-guns play --join {t}"); }
                    self.session = s;
                }
                Err(e) => { self.ui.notice = Some(format!("could not host: {e}")); self.ui.error = Some(format!("Couldn't start the online host: {e}")); }
            },
            // Godot `_pause_copy_game_id`: the clipboard, and COPIED! for a moment.
            Action::Copy => if let Some(t) = self.session.ticket() {
                app::clipboard::set(t);
                println!("ticket: {t}");
                self.ui.copied = 1.5;
            },
            // Godot `_pause_join`: leave this game and join that one.
            Action::Join if self.joining.is_none() && !self.session.is_client() && self.party.is_none() => {
                let ticket = self.ui.join_ticket();
                let (players, lan) = (self.slots.len().max(1) as u8, self.lan);
                let (tx, rx) = std::sync::mpsc::channel();
                std::thread::spawn(move || { let _ = tx.send(Session::join(&ticket, players, lan)); });
                self.joining = Some(rx);
                self.ui.joining = true;
                println!("joining {}...", self.ui.join_ticket());
            }
            // Godot `_pause_toggle_splitscreen`.
            Action::Splitscreen if host && self.party.is_none() => {
                if self.split {
                    while self.slots.len() > 1 { let k = self.slots.len() - 1; self.leave_local(k); }
                    self.split = false;
                    self.binding.clear();
                    self.hud.announce("SPLITSCREEN OFF", 2.0, 0.0);
                } else {
                    self.split = true;
                    // The first seat keeps the keyboard, and the pad last used.
                    let mut first = Vec::new();
                    if !self.no_kbm { first.push(Device::Keyboard); }
                    if (self.pad_last || self.no_kbm) && let Some(p) = self.last_pad { first.push(p); }
                    self.binding = vec![first];
                }
                self.ui.open(Screen::Play);
            }
            Action::Rematch if host => { self.world.restart_match(); self.new_arena(); self.hud.round_age = 0.0; self.ui.open(Screen::Play); }
            Action::Advanced => self.ui.open(Screen::Advanced),
            Action::Party(k, v) => {
                if let Some(p) = &mut self.party { p.set(k, v); } else if let Some(s) = &mut self.gn_preview && let Some(key) = s.key(k) { s.change(&key, &serde_json::json!(v)); }
            }
            // Godot `_spawn_bots` / `_despawn_one_bot` from the start or pause menu.
            Action::AddBot if host && !self.world.coop && self.world.players.len() < 8 => {
                self.world.add_bot();
                if self.vis.player_capacity() < self.world.players.len() {
                    rebuild_views(self);
                } else {
                    self.prev = view::Pose::all(&self.world);
                    self.hud.sync(&self.world);
                }
            }
            Action::RemoveBot if host && !self.world.coop => {
                let mut keep = self.slots.clone();
                keep.extend_from_slice(&self.start_seats);
                keep.extend(self.session.remote_slots());
                if self.world.remove_bot(&keep) {
                    self.prev = view::Pose::all(&self.world);
                    self.hud.sync(&self.world);
                }
            }
            Action::Extend if host => self.world.extend_match(),
            _ => {}
        }
    }
}

impl Gg {
    /// What the debug overlay shows of the match.
    fn debug_ctx(&self) -> hud::debug::Ctx {
        let w = &self.world;
        let p = self.debug.player.min(w.players.len().saturating_sub(1));
        let target = w.players.get(p);
        hud::debug::Ctx {
            cards: w.cards.enabled().map(|(_, d)| (d.id.clone(), d.name.clone(), hud::style::rgb(d.color), d.rare)).collect(),
            players: w.players.iter().enumerate().map(|(i, q)| (self.hud.name(i).to_string(), q.pos.to_array(), q.bot)).collect(),
            held: target.map_or(Vec::new(), |t| t.cards.iter().map(|&c| card_id(w, c)).collect()),
            special: target.map_or("grenade", |t| t.weapon.special.name()),
            max_health: target.map_or(100, |t| t.max_health()),
            mag: target.map_or(1, |t| t.mag_size()),
            sky: self.sky.choice.name(),
            modifier: w.modifier.map(|m| m.id()),
            forced: w.forced_modifier.map(|m| m.id()),
            bots_paused: w.bots_paused,
            round: w.round,
            seed: w.arena.seed,
            lava: w.lava.active,
            island: w.arena.variant == growing_guns::arena::Variant::Island,
        }
    }

    /// Run the debug overlay's commands; the last reply shows in it.
    fn run_debug(&mut self, cmds: Vec<String>) {
        for c in cmds {
            let r = app::Game::command(self, &c);
            match &r { Ok(t) => println!("debug: {c}: {t}"), Err(e) => eprintln!("debug: {c}: {e}") }
            self.debug.reply(r.map(|t| format!("{c}: {t}")).map_err(|e| format!("{c}: {e}")));
        }
    }

    /// Godot's per-player PLAYER MENU: a joined seat's Start opens it; it
    /// then takes that seat's controls (A or X select, B or Start resume).
    fn player_menus(&mut self, controls: &mut [Controls]) {
        if !self.split || self.party.is_some() { self.player_menu = None; return; }
        let mut leave = None;
        if let Some((k, menu)) = &mut self.player_menu {
            let k = *k;
            let c = controls.get(k).copied().unwrap_or_default();
            if c.pressed.has(Buttons::START | Buttons::EAST) { self.player_menu = None; }
            else {
                let sel = Buttons(if c.pressed.has(Buttons::WEST) { c.pressed.0 | Buttons::SOUTH } else { c.pressed.0 });
                match menu.update(&Controls { pressed: sel, ..c }, hud::menus::PLAYER_MENU.len(), 1, DT) {
                    app::ui::MenuEvent::Select(0) | app::ui::MenuEvent::Back => self.player_menu = None,
                    app::ui::MenuEvent::Select(_) => { leave = Some(k); }
                    _ => {}
                }
            }
            if let Some(c) = controls.get_mut(k) { *c = Controls::default(); }
        } else if !self.ui.is_open() {
            if let Some(k) = (1..controls.len()).find(|&k| controls[k].pressed.has(Buttons::START)) {
                self.player_menu = Some((k, Default::default()));
                controls[k] = Controls::default();
            }
        }
        if let Some(k) = leave { self.leave_local(k); }
    }
}

impl app::Game for Gg {
    fn fixed_quality(&self) -> bool { self.replay.is_some() || self.recorder.disabled }

    fn local_players(&self) -> usize { if self.replay.is_some() { 1 } else { self.slots.len() } }

    fn pointer(&mut self, pointer: app::Pointer, size: [u32; 2]) {
        if let Some(r) = &mut self.replay { app::Game::pointer(&mut **r, pointer, size); return; }
        self.pointer = (if self.no_kbm { app::Pointer::default() } else { pointer }, size);
    }

    fn mouse_capture(&self) -> Option<bool> {
        if self.replay.is_some() { return Some(false); }
        Some(!self.no_kbm && !self.ui.is_open() && !self.debug.open && self.hud.match_over.is_none()
            && !self.slots.first().and_then(|&s| self.world.players.get(s)).is_some_and(|p| p.offer.is_some()))
    }

    fn tick(&mut self, controls: &[Controls]) {
        if let Some(result) = self.recorder.completion() {
            let message = match result { Ok(_) => "Replay saved. F9 to review.".into(), Err(e) => format!("Replay save failed: {e}") };
            self.hud.announce(message, 4.0, 0.0);
        }
        if let Some(r) = &mut self.replay { app::Game::tick(&mut **r, controls); return; }
        self.stepped = false;
        let (pointer, pointer_size) = std::mem::take(&mut self.pointer);
        // Live-control requests, between ticks (also while a party is paused).
        crate::live::tick(self);
        self.poll_join();
        if self.ui.player_name != self.saved_name { self.saved_name = self.ui.player_name.clone(); self.save_settings(); }
        let audible = self.party.as_ref().is_none_or(|p| p.running());
        if audible != self.crowd_audible {
            self.crowd_audible = audible;
            if let Some(tx) = &self.crowd_tx { tx.volume(if audible { 1.0 } else { 0.0 }); }
        }
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
        let mut controls: Vec<Controls> = party_controls.unwrap_or_else(|| controls.to_vec());
        controls.resize(self.slots.len(), Controls::default());
        // A seat that joined this tick: its X was the join, not a reload.
        for k in std::mem::take(&mut self.joined_now) { if let Some(c) = controls.get_mut(k) { *c = Controls::default(); } }
        self.player_menus(&mut controls);
        // A player's sensitivity changed in the game's menu: into their profile.
        if self.ui.aim != self.aim_synced && let Some(party) = self.party.as_mut() {
            for (n, v) in &self.ui.aim {
                if self.aim_synced.iter().any(|a| a.0 == *n && a.1 == *v) { continue; }
                if let Some(k) = self.names.iter().position(|m| m == n) { party.set_stick(k, *v); }
            }
            self.aim_synced.clone_from(&self.ui.aim);
        }
        let controls: &[Controls] = &controls;
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
        let names = self.seat_names();
        self.hud.set_names(&self.world, &self.slots, &names);
        // Menus take the local players' controls (merged) while open.
        let mut merged = merge(controls);
        // Back + Start on a pad (Tab + Enter) toggles the debug overlay, as F3 does.
        if merged.held.has(Buttons::SELECT) && merged.pressed.has(Buttons::START) {
            self.debug.toggle();
            merged.pressed.0 &= !Buttons::START;
        } else if self.debug.open {
            let dctx = self.debug_ctx();
            let mut cmds = self.debug.update(&merged, &dctx, DT);
            if cmds.is_empty() { cmds = self.debug.pointer(pointer, pointer_size, &dctx); }
            self.run_debug(cmds);
        }
        let debug = self.debug.open;
        let ctx_ticket = self.session.ticket().map(str::to_string);
        let ctx = menu_ctx(&self.session, &self.world, &self.start_seats, ctx_ticket.as_deref(), self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names, self.xr_stereo);
        let modal = self.ui.is_open() || self.hud.match_over.is_some() || debug;
        // The debug overlay takes the controls; the match runs on under it.
        if !debug {
            let mut action = self.ui.update(&merged, &self.world, self.hud.match_over, &ctx, DT);
            if action == hud::menus::Action::None {
                action = self.ui.pointer(pointer, pointer_size, &self.world, self.hud.match_over, &ctx);
            }
            self.act(action);
        }
        let menu_open = self.ui.is_open() || debug;
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
        if self.session.is_client() {
            self.session.send(&mut self.world, &self.slots, local);
            self.stepped = true;
            return;
        }
        let mut inputs = vec![Input::default(); self.world.players.len()];
        for (k, &s) in self.slots.iter().enumerate() { if let Some(i) = local.get(k) { inputs[s] = *i; } }
        self.session.poll(&mut self.world);
        if self.vis.player_capacity() < self.world.players.len() { rebuild_views(self); }
        inputs.resize(self.world.players.len(), Input::default());
        self.session.remote_inputs(&mut inputs);
        // The pause menu stops a match nobody else is playing (Godot: one human).
        let humans = self.world.players.iter().filter(|p| !p.bot).count();
        if matches!(self.ui.screen, hud::menus::Screen::Pause | hud::menus::Screen::Settings { from_start: false }) && humans <= 1 { return; }
        self.prev = view::Pose::all(&self.world);
        let t = std::time::Instant::now();
        if self.xr_stereo {
            let mut aims = vec![None; inputs.len()];
            if let Some(&slot) = self.slots.first() { aims[slot] = self.xr_aim; }
            self.world.step_tracked(&inputs, &aims);
        } else { self.world.step(&inputs); }
        hitch("world.step", t);
        self.stepped = true;
        let t = std::time::Instant::now();
        if self.world.events.contains(&Event::NewArena) { self.new_arena(); }
        hitch("new_arena", t);
        let t = std::time::Instant::now();
        // Shot feedback needs this frame's controller pose, including after
        // rebuilding the arena, before the render-time gun posing runs.
        if self.xr_stereo {self.vis.track_gun(self.slots[0],self.xr_gun);}
        self.vis.absorb(&self.world, &crate::sounds::ears(&self.world, &self.slots));
        hitch("vis.absorb", t);
        let t = std::time::Instant::now();
        self.hud.tick(&self.world, DT);
        if let Some(s) = &mut self.sounds { s.tick(&self.world, &self.slots, DT); self.vis.stadium.sound.loudness_db = s.duck_db(); }
        follow_music(&mut self.music, &self.world, &self.slots);
        hitch("hud+sounds+music", t);
        let t = std::time::Instant::now();
        self.session.publish(&self.world);
        hitch("session.publish", t);
        // Desktop replays do not capture headset/controller poses.
        if !self.xr_stereo { self.recorder.push(&self.world, &self.slots, &self.sky.choice.name(), self.ui.tilt); }
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if let Some(r) = &mut self.replay { return app::Game::draw(&mut **r, frame); }
        self.replay_size = frame.size;
        let draw_t = std::time::Instant::now();
        // Interpolating a frozen match swings between two ticks (a flicker at 120 Hz).
        let alpha = if self.session.is_client() {
            let a = self.receive(frame.dt);
            self.session.pose_locals(&mut self.world, &mut self.prev, &self.slots, frame.alpha, frame.dt);
            a
        } else if self.stepped {
            frame.alpha
        } else {
            1.0
        };
        self.replay_alpha = alpha;
        self.hud.xr = self.xr_stereo;
        if self.xr_stereo {
            self.vis.hide_own_bodies(&[self.slots[0], self.slots[0]]);
            self.vis.track_gun(self.slots[0], self.xr_gun);
        }
        self.vis.set_tilt(self.ui.tilt && !self.xr_stereo);

        self.vis.pose(&self.world, &self.prev, alpha, frame.dt);
        if self.recorder.disabled { self.apply_sky()?; self.sky.live = None; }
        else if self.sky.live.is_some() { self.cycle_sky(); }
        self.sync_atmosphere();
        self.sync_blackout();
        let t = std::time::Instant::now();
        if std::mem::take(&mut self.rebuild) || frame.renderer().is_none() {
            frame.set_scene(&self.vis.scene, &self.light);
            self.plates.invalidate();
            self.vis.invalidate_sprites();
            self.sky_upload = None;
            hitch("set_scene", t);
        } else if let Some(prepared) = self.sky_upload.take() {
            let gpu = frame.gpu;
            let r = frame.renderer_mut().expect("renderer checked above");
            if let Err(e) = r.update_sky(gpu, &prepared.sky, &mut self.light) { eprintln!("sky unchanged on the GPU: {e}"); }
        }
        crate::live::frame(self, frame);
        let r = frame.renderer().expect("scene set above");
        let t = std::time::Instant::now();
        self.vis.flush(frame.gpu, r, &self.light)?;
        hitch("vis.flush", t);
        r.update_instances(frame.gpu, self.vis.first_dynamic, self.vis.dynamic())?;
        let (lights, scatter) = self.vis.local_lights_scattering(self.scatter);
        r.set_local_lights_scattering(&lights, &scatter)?;
        // At night the floodlights are shadowed spots: falloff 1 and the
        // range window of Godot's lights (the sign), as the unshadowed ones.
        let blackout = self.vis.blackout;
        let spots = if blackout { view::Visuals::flashlights(&self.world, &self.prev, alpha) } else { self.vis.flood_spots() };
        r.set_spot_falloff(&vec![-1.0; spots.len()]);
        r.set_spot_softness(if spots.is_empty() { 0.0 } else { FLOOD_SOFTNESS });
        r.set_headlights(&spots)?;
        r.set_volumetric_beams(&if self.light.volumetric.is_some() { self.vis.volumetric_beams(self.scatter) } else { Vec::new() })?;
        let start = matches!(self.ui.screen, hud::menus::Screen::Start | hud::menus::Screen::Settings { from_start: true });
        // Name tags over everyone not seated here (on the start screen, everyone).
        let drawn: Vec<Vec3> = (0..self.world.players.len()).map(|i| view::pose(&self.world, i, &self.prev, alpha).pos).collect();
        self.plates.update(frame.gpu, r, &self.world, &self.hud, if start { &[] } else { &self.slots }, &drawn, self.vis.sprites())?;
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
                // Under the HUD (Godot's flare overlay sits below the crosshair).
                let cam = self.vis.punched(&self.world, s, view::camera(&self.world, s, &self.prev, alpha));
                self.vis.airstrike_flare(&mut list, *rect, &cam, &self.world);
                let flash = self.vis.ion_flash(self.world.players[s].eye());
                let pad = k > 0 || self.no_kbm;
                self.hud.view(&mut list, &self.world, s, *rect, self.menus.get(k).map_or(0, |m| m.focus), flash, pad);
                // An air strike's impact whites out every view for two frames
                // (Godot `flash_air_strike_impact`).
                let impact = self.vis.air_impact_flash();
                if impact > 0.0 { list.begin_overlay(); list.rect(*rect, [1.0, 0.95, 0.86, impact * 0.92], *rect); }
            }
            list.begin_overlay();
            let error = self.session.error().or_else(|| self.ui.error.clone());
            self.hud.global(&mut list, &self.world, error.as_deref());
            // Splitscreen: the join prompt while seats are open, else the hint.
            if self.can_join() && self.slots.len() < 4 { hud::menus::join_prompt(&mut list); } else if self.hint_visible() { hud::menus::splitscreen_hint(&mut list); }
            if let Some((k, m)) = &self.player_menu && let Some(&s) = self.slots.get(*k) { hud::menus::player_menu(&mut list, self.hud.name(s), m.focus); }
            self.slots.iter().map(|&s| self.vis.punched(&self.world, s, view::camera(&self.world, s, &self.prev, alpha))).collect()
        };
        list.begin_overlay();
        let ticket = self.session.ticket().map(str::to_string);
        let ctx = menu_ctx(&self.session, &self.world, &self.start_seats, ticket.as_deref(), self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names, self.xr_stereo);
        self.ui.draw(&mut list, &self.world, &self.hud, &ctx);
        self.hud.round_flash(&mut list);
        if self.debug.open { let dctx = self.debug_ctx(); self.debug.draw(&mut list, &dctx); }
        let mut light = self.light.clone();
        let glow = self.vis.ion_glow();
        if glow > 0.0 {
            light.bloom_boost *= 1.0 + 31.0 * glow;
            light.bloom_rounds = light.bloom_rounds.max(4);
            light.bloom_threshold = light.bloom_threshold.min(2.0 - glow);
        }
        // The lava flows on the game clock.
        light.time = (self.world.tick as f32 - 1.0 + alpha) * DT;
        hitch("draw (game side)", draw_t);
        Ok(Draw { cameras, light, hud: list })
    }

    fn feedback_replay(&mut self, path: &std::path::Path, alpha: f32) -> Result<Option<serde_json::Value>, String> {
        if let Some(r) = &mut self.replay { return app::Game::feedback_replay(&mut **r, path, alpha); }
        if self.session.is_client() { return Err("Replay recording currently requires local or host play".into()); }
        self.recorder.save(path, self.replay_alpha, self.replay_size).map(Some)
    }

    fn title(&self) -> Option<String> { Some(format!("round {}", self.world.round)) }
    fn wants_capture(&self) -> bool { crate::live::wants_capture(self) }
    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) { crate::live::captured(self, size, rgba) }

    fn loading(&mut self, percent: u32, step: &str) { if let Some(p) = self.party.as_mut() { p.loading(percent, step); } }
    fn gpu_reset(&mut self) { self.plates.reset(); self.sky_upload = None; self.rebuild = true; }

    fn screen(&self) -> app::Screen { self.party.as_ref().map_or(app::Screen::Normal, |p| p.screen()) }

    fn split(&self) -> render::SplitLayout { render::SplitLayout::SideBySide }

    fn escape(&mut self) -> bool {
        if self.replay.take().is_some() { self.rebuild = true; return true; }
        use hud::menus::Screen;
        if self.debug.open { self.debug.set_open(false); return true; }
        match self.ui.screen {
            // Esc on the start screen leaves the game (the shell first frees the mouse).
            Screen::Start => false,
            _ => { let a = self.ui.back(); self.act(a); true }
        }
    }

    fn quit(&self) -> bool { self.quit }

    /// F3: the debug overlay.
    fn function_key(&mut self, n: u8) -> bool {
        if n == 9 {
            if self.replay.take().is_some() { self.rebuild = true; }
            else if !matches!(self.session, Session::Local) || self.party.is_some() {
                self.hud.announce("Open the saved replay separately during online play.", 4.0, 0.0);
            } else if let Some((tape, path)) = &self.recorder.last {
                match crate::replay::Viewer::new(tape.clone(), path.clone()) {
                    Ok(r) => self.replay = Some(Box::new(r)),
                    Err(e) => self.hud.announce(e, 4.0, 0.0),
                }
            } else {
                match crate::replay::open_latest() {
                    Ok(r) => self.replay = Some(Box::new(r)),
                    Err(e) => self.hud.announce(e, 4.0, 0.0),
                }
            }
            return true;
        }
        if self.replay.is_some() { return true; }
        if n != 3 { return false; }
        self.debug.toggle();
        true
    }

    fn tuning(&self, t: &mut app::Tuning) {
        let d = app::Tuning::default();
        t.mouse = d.mouse * self.ui.mouse;
        t.stick = d.stick * self.ui.stick;
    }

    fn devices(&mut self, devices: &[(Device, Controls)]) {
        if self.replay.is_some() { return; }
        self.connected = devices.iter().map(|d| d.0).collect();
        for (d, c) in devices {
            if c.pressed.0 != 0 || c.mv != [0.0; 2] || c.look != [0.0; 2] { self.pad_last = d.is_pad(); if d.is_pad() { self.last_pad = Some(*d); } }
        }
        if !self.split || self.party.is_some() { return; }
        // Seats without a device (launched with --players N) take the next
        // free one in order, as the shell's own routing would.
        self.binding.resize(self.slots.len(), Vec::new());
        for k in 0..self.binding.len() {
            if !self.binding[k].is_empty() { continue; }
            if let Some(&d) = self.connected.iter().find(|d| !self.binding.iter().any(|b| b.contains(d))) { self.binding[k].push(d); }
        }
        if !self.can_join() { return; }
        for (d, c) in devices {
            if c.pressed.has(Buttons::WEST) && !self.binding.iter().any(|b| b.contains(d)) {
                if let Err(e) = self.join_local(*d) { self.hud.announce(e, 2.0, 0.0); }
            }
        }
    }

    fn seats(&self) -> Option<Vec<Vec<Device>>> { if self.replay.is_some() { return None; } (self.split && self.party.is_none()).then(|| self.binding.clone()) }

    fn text_input(&self) -> bool {
        if self.replay.is_some() { return false; }
        if self.debug.open { return self.debug.text_focus(&self.debug_ctx()); }
        let (ticket, _) = self.ctx_parts();
        self.ui.text_focus(&self.ctx(ticket.as_deref())).is_some()
    }

    fn text(&mut self, ev: app::TextEvent) {
        if self.debug.open {
            let dctx = self.debug_ctx();
            let cmds = self.debug.text(ev, &dctx);
            self.run_debug(cmds);
            return;
        }
        let (ticket, _) = self.ctx_parts();
        let a = { let ctx = menu_ctx(&self.session, &self.world, &self.start_seats, ticket.as_deref(), self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names, self.xr_stereo); self.ui.text(ev, &ctx) };
        self.act(a);
    }

    fn command(&mut self, cmd: &str) -> Result<String, String> {
        let w: Vec<&str> = cmd.split_whitespace().collect();
        let num = |k: usize| -> Result<f32, String> { w.get(k).ok_or(format!("{cmd}: missing argument {k}"))?.parse::<f32>().map_err(|e| format!("{cmd}: {e}")) };
        let player = |k: usize, world: &World| -> Result<usize, String> {
            let p = num(k)? as usize;
            if p < world.players.len() { Ok(p) } else { Err(format!("no player {p}")) }
        };
        match w.first().copied().unwrap_or("") {
            "help" => Ok("menu start|pause|settings|gamenight|advanced|none | debug [on|off] | debug tab cards|items|sky|modes | debug player P | debug focus search|N | focus join|name | split on|off | leave K | copy | names NAME,NAME.. | net-error TEXT|none | give P CARD | pickup KIND X Z [Y] | teleport P X Y Z | aim P X Y Z | health P N | ammo P N | bots pause|resume | blast X Y Z R DAMAGE | boom X Y Z R [DAMAGE] | arena SEED | arena island [SEED] | lava [SECONDS] | cards | lua CODE | mods | knobs | knob NAME VALUE | special P NAME | modifier ID|none | sky [godot|TIME[:WEATHER]|rounds|cycle] | light [KEY=VALUE..|reset] | round | cage | rematch | extend".into()),
            // The keyboard focus on a text field of the open menu.
            "focus" => {
                use hud::menus::Field;
                let f = match w.get(1).copied() { Some("join") => Field::Join, Some("name") => Field::Name, _ => return Err("focus join|name".into()) };
                let (ticket, _) = self.ctx_parts();
                let ok = { let ctx = menu_ctx(&self.session, &self.world, &self.start_seats, ticket.as_deref(), self.party.as_ref().map(|p| &p.settings).or(self.gn_preview.as_ref()), &self.names, self.xr_stereo); self.ui.focus_field(f, &ctx) };
                if ok { Ok(format!("focus {f:?}")) } else { Err(format!("no {f:?} field on {:?}", self.ui.screen)) }
            }
            // Splitscreen on or off, as the pause menu's SPLITSCREEN.
            "split" => {
                let want = match w.get(1).copied() { Some("on") => true, Some("off") => false, _ => !self.split };
                if want != self.split { self.act(hud::menus::Action::Splitscreen); }
                Ok(format!("splitscreen {}, local seats {:?}", self.split, self.slots))
            }
            // The menus' COPY: the match ID to the clipboard.
            "copy" => { self.act(hud::menus::Action::Copy); Ok(format!("clipboard {:?}", app::clipboard::get())) }
            // A local seat leaves (PLAYER MENU's LEAVE).
            "leave" => { let k = num(1)? as usize; if k == 0 || k >= self.slots.len() { return Err(format!("no joined seat {k}")); } self.leave_local(k); Ok(format!("local seats {:?}", self.slots)) }
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
            // The F3 overlay, for scripts: open or close it, pick its tab,
            // target player and focused line (then press, type as a player would).
            "debug" => {
                let dctx = self.debug_ctx();
                match (w.get(1).copied(), w.get(2).copied()) {
                    (None, _) => self.debug.toggle(),
                    (Some("on"), _) => self.debug.set_open(true),
                    (Some("off"), _) => self.debug.set_open(false),
                    (Some("tab"), Some(t)) => {
                        let k = hud::debug::TABS.iter().position(|n| n.eq_ignore_ascii_case(t)).or_else(|| t.parse().ok()).ok_or(format!("debug tab {}", hud::debug::TABS.join("|").to_lowercase()))?;
                        self.debug.set_tab(k);
                    }
                    (Some("player"), Some(_)) => self.debug.player = player(2, &self.world)?,
                    (Some("focus"), Some("search")) => if !self.debug.focus_search(&dctx) { return Err("no search field on this tab".into()) },
                    (Some("focus"), Some(_)) => if !self.debug.focus(num(2)? as usize, &dctx) { return Err(format!("no line {} on this tab", w[2])) },
                    _ => return Err("debug [on|off] | debug tab NAME | debug player P | debug focus search|N".into()),
                }
                Ok(self.debug.describe(&self.debug_ctx()).to_string())
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
            "light" => self.light_command(&w[1..]),
            "knobs" => Ok(crate::knobs::list()),
            "knob" => {
                let (name, value) = cmd.trim_start().strip_prefix("knob").unwrap_or("").trim().split_once(' ').ok_or("knob NAME VALUE")?;
                let rel = crate::knobs::set(name.trim(), value.trim())?;
                if let Some(s) = &self.scripts { for line in s.poll(&mut self.world) { eprintln!("mods: {line}"); } }
                if let Some(p) = self.party.as_mut() { p.knob_changed(name.trim(), value.trim()); }
                Ok(format!("{} = {} in mods/{rel}", name.trim(), value.trim()))
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
                // `arena SEED`, or `arena island [SEED]`: the island layout
                // whatever the seed rolls (Godot `force_all_floor_lava`).
                let island = w.get(1) == Some(&"island");
                let seed = if island { w.get(2).map_or(Ok(1.0), |_| num(2))? } else { num(1)? } as u64;
                self.world.arena = growing_guns::Arena::generate_with(seed, 55.0, 95.0, island);
                self.world.lava = Default::default();
                // On islands everyone moves onto one (anywhere else is lava).
                if island { self.world.place_on_spawns(); }
                self.new_arena();
                let a = &self.world.arena;
                let hole = a.hole.map(|(c, s)| format!(", hole {s:.1} m at {:.1} {:.1}", c.x, c.y)).unwrap_or_default();
                let islands = if a.islands.is_empty() { String::new() } else { format!(", {} islands", a.islands.len()) };
                Ok(format!("arena {seed}: {:?}, {} m, {} chunks{hole}{islands}", a.variant, a.size, a.chunks.len()))
            }
            "lava" => {
                // Godot's dev key L; `lava S` as if the leak began S seconds ago.
                let s = w.get(1).map_or(Ok(0.0), |_| num(1))?;
                self.world.start_lava_leak_at(s);
                let l = &self.world.lava;
                Ok(format!("lava leak at {s} s: inset {:.1} m, surface {:.2} m", l.inset(), l.surface()))
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
            "pickup" => {
                // A sky drop as the timer spawns it (52 m up unless Y given).
                let name = w.get(1).ok_or("pickup KIND X Z [Y]")?.to_lowercase();
                let kind = *growing_guns::PICKUPS.iter().find(|k| format!("{k:?}").to_lowercase() == name.replace('_', ""))
                    .ok_or(format!("unknown pickup {name} (heart mushroom bomb plus_one laser dice air_strike)"))?;
                let y = if w.len() > 4 { num(4)? } else { 52.0 };
                let pos = Vec3::new(num(2)?, y, num(3)?);
                self.world.pickups.push(growing_guns::Pickup { kind, pos, vy: 0.0, grounded: None });
                Ok(format!("pickup {kind:?} at {pos}"))
            }
            "teleport" => {
                let p = player(1, &self.world)?;
                let to = Vec3::new(num(2)?, num(3)?, num(4)?);
                self.world.players[p].pos = to;
                self.world.players[p].vel = Vec3::ZERO;
                if let Some(q) = self.prev.get_mut(p) { q.pos = to; }
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
        if let Some(r) = &self.replay { return app::Game::state(&**r); }
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
                "chunks": w.arena.chunks.len(), "chunks_broken": w.arena.chunks.len() - w.arena.alive_chunks(),
                "islands": w.arena.islands.len(), "lava_surface": w.arena.lava_surface(), "leak_surface": w.lava.active.then(|| w.lava.surface()) },
            "players": players, "waves": w.coop.then_some(&w.waves), "bullets": w.bullets.len(), "events": w.events, "ragdolls": self.vis.ragdolls(), "stadium": self.vis.stadium.state(),
            "sky": { "choice": self.sky.choice.name(), "applied": self.sky.applied.map(|s| s.metadata()) },
            "local": { "seats": self.slots, "split": self.split, "devices": self.binding.iter().map(|b| b.iter().map(|d| d.to_string()).collect::<Vec<_>>()).collect::<Vec<_>>(),
                "connected": self.connected.iter().map(|d| d.to_string()).collect::<Vec<_>>(), "player_menu": self.player_menu.as_ref().map(|m| m.0),
                "join_prompt": self.can_join() && self.slots.len() < 4, "hint": self.hint_visible() },
            "menu": { "join_field": self.ui.join.text(), "joining": self.ui.joining, "player_name": self.ui.player_name, "name_field": self.ui.name.text(),
                "text_input": app::Game::text_input(self), "picker": self.ui.picker.is_some(), "copied": self.ui.copied > 0.0, "notice": self.ui.notice,
                "clipboard": app::clipboard::get() },
            "debug": self.debug.describe(&self.debug_ctx()),
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

/// Refresh seat visibility after a join/leave. The reserved player slots keep
/// the renderer and effects alive; only exceeding capacity needs a new scene.
pub fn rebuild_views(gg: &mut Gg) {
    let start = matches!(gg.ui.screen, hud::menus::Screen::Start | hud::menus::Screen::Settings { from_start: true });
    let needed = gg.world.players.len().max(if gg.world.coop { view::COOP_VIEW_PLAYERS } else { 8 });
    if gg.vis.player_capacity() >= needed {
        gg.vis.hide_own_bodies(if start { &[] } else { &gg.slots });
        gg.prev = view::Pose::all(&gg.world);
        gg.hud.sync(&gg.world);
        return;
    }
    let bake = gg.vis.stadium.bake_in_background;
    let sound = std::mem::replace(&mut gg.vis.stadium.sound, crate::crowd_sound::CrowdSound::new(None));
    gg.vis = view::Visuals::build(&gg.world);
    gg.vis.stadium.bake_in_background = bake;
    gg.vis.hide_own_bodies(if start { &[] } else { &gg.slots });
    gg.vis.stadium.sound = sound;
    gg.prev = view::Pose::all(&gg.world);
    gg.hud.sync(&gg.world);
    gg.rebuild = true;
}

fn card_id(w: &World, c: growing_guns::cards::Card) -> String { w.cards.get(c).map_or_else(|| format!("#{}", c.0), |d| d.id.clone()) }

/// `--modifier ID` forces a round modifier (`none` for none), `--rounds N`
/// sets the wins that end the match, `--coop` plays wave survival,
/// `--island` makes every arena an island arena.
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
    w.force_island = args.flag("--island");
}

pub fn run(args: &Args) -> Result<(), String> {
    let total = args.num("--total", 4usize).clamp(1, 8);
    let humans = args.num("--players", 1usize).clamp(1, 4).min(total);
    let seed = args.num("--seed", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs()));
    let lan = args.flag("--lan");
    let (session, world, slots) = if let Some(ticket) = args.value("--join") {
        let ticket = hud::menus::extract_ticket(ticket);
        let (session, world, slots) = Session::join(&ticket, humans as u8, lan)?;
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
    // Round changes must not stall the frame (and the audio) on the sky bake.
    game.vis.stadium.bake_in_background = true;
    game.party = crate::party::Party::connect();
    if game.party.is_some() { println!("gamenight: waiting for the party host"); }
    game.live = crate::live::start(args, game.party.is_some())?.map(Box::new);
    if let (Some(party), Some(file)) = (game.party.as_mut(), game.live.as_ref().and_then(|l| l.token_file())) {
        party.offer_live("Growing Guns can be changed live while you play: damage, fire rate, run and bullet speed, explosions, health, cards for a player, bots, rounds, rules and sounds. Send any request to change the game as it is said.".into(), file);
    }
    game.no_kbm = args.flag("--no-kbm");
    game.lan = lan;
    // Settings persist like Godot's user://settings.cfg.
    game.settings_file = settings_path();
    if let Some(v) = game.settings_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) { game.ui.load_settings(&v); }
    // Godot names a new player Player_NNN.
    if game.ui.player_name.is_empty() { game.ui.player_name = format!("Player_{}", seed % 1000); }
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
            game.set_crowd(args.value("--crowd"), samples.as_deref(), &crowd);
            game.set_sounds(crate::sounds::Sounds::new(tx, patches, samples.as_deref()));
            if let Some(m) = music { game.set_music(m); }
            Some(speaker)
        }
        Err(e) => { eprintln!("silent: {e}"); None }
    };
    let target_fps = args.value("--target-fps").and_then(|v| v.parse().ok());
    let cfg = app::Config {
        title: "Growing Guns | Warp".into(),
        tick_hz: TICK_HZ as f32,
        fullscreen: args.flag("--fullscreen"),
        keyboard: !args.flag("--no-kbm"),
        frames: args.num("--frames", 0u64),
        target_fps,
        ..Default::default()
    };
    app::run(cfg, &mut game)
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::Game;

    fn press(b: u32) -> Controls { Controls { held: Buttons(b), pressed: Buttons(b), ..Default::default() } }
    fn idle(n: u32) -> Vec<(Device, Controls)> { std::iter::once(Device::Keyboard).chain((0..n).map(Device::Pad)).map(|d| (d, Controls::default())).collect() }
    /// One shell tick: devices, then routed controls.
    fn step(g: &mut Gg, devices: &[(Device, Controls)]) -> Vec<Controls> {
        let c = app::seat_controls(g, devices);
        g.tick(&c);
        c
    }

    #[test]
    fn spare_pads_join_over_bots_lead_their_own_seat_and_leave() {
        let mut world = World::new(5, &[false, true, true, true]);
        world.release_cage();
        let mut g = Gg::new(world, Session::Local, vec![0]);
        g.rebuild = false; // Initial renderer already installed by the shell.
        let instances = g.vis.scene.instances.as_ptr();
        // Not split: every device drives the one seat; a second pad shows the hint.
        step(&mut g, &idle(2));
        assert!(g.seats().is_none());
        assert!(g.hint_visible());
        g.command("split on").unwrap();
        assert!(g.split && !g.hint_visible());
        assert_eq!(g.seats(), Some(vec![vec![Device::Keyboard]]));
        // pad1 presses X: it takes the first bot's seat.
        let mut d = idle(2);
        d[2].1 = press(Buttons::WEST);
        let c = step(&mut g, &d);
        assert_eq!(g.slots, vec![0, 1]);
        assert!(!g.world.players[1].bot);
        assert_eq!(c.len(), 2, "the seat counts from its join tick");
        assert_eq!(g.seats(), Some(vec![vec![Device::Keyboard], vec![Device::Pad(1)]]));
        // Its sticks move player 2 only.
        let before = (g.world.players[0].pos, g.world.players[1].pos);
        let mut d = idle(2);
        d[2].1 = Controls { mv: [0.0, 1.0], ..Default::default() };
        for _ in 0..40 { step(&mut g, &d); }
        assert!(g.world.players[1].pos.distance(before.1) > 0.5, "the joined pad moves its player");
        assert!(g.world.players[0].pos.distance(before.0) < 0.5, "and not the first");
        // pad0 joins too; the keyboard never takes a second seat.
        let mut d = idle(2);
        d[1].1 = press(Buttons::WEST);
        d[0].1 = press(Buttons::WEST);
        step(&mut g, &d);
        assert_eq!(g.slots, vec![0, 1, 2]);
        // Start on pad1 opens its PLAYER MENU; down and A leaves.
        let mut d = idle(2);
        d[2].1 = press(Buttons::START);
        step(&mut g, &d);
        assert_eq!(g.player_menu.as_ref().map(|m| m.0), Some(1));
        assert!(!g.ui.is_open(), "the match menu stays shut");
        d[2].1 = press(Buttons::DOWN);
        step(&mut g, &d);
        d[2].1 = press(Buttons::SOUTH);
        step(&mut g, &d);
        assert_eq!(g.slots, vec![0, 2]);
        assert!(g.world.players[1].bot, "the seat goes back to a bot");
        assert_eq!(g.binding, vec![vec![Device::Keyboard], vec![Device::Pad(0)]]);
        // SPLITSCREEN off sends everyone joined back to the bots.
        g.command("split off").unwrap();
        assert_eq!(g.slots, vec![0]);
        assert!(g.world.players[2].bot && g.seats().is_none());
        assert!(!g.rebuild, "joining/leaving must not recreate GPU pipelines");
        assert_eq!(instances, g.vis.scene.instances.as_ptr(), "keep the scene and live effects");
    }

    #[test]
    fn bot_count_changes_reuse_all_eight_reserved_seats() {
        let mut g = Gg::new(World::new(5, &[false, true]), Session::Local, vec![0]);
        g.rebuild = false;
        let instances = g.vis.scene.instances.as_ptr();
        for _ in 0..6 { g.act(hud::menus::Action::AddBot); }
        assert_eq!(g.world.players.len(), 8);
        for _ in 0..6 { g.act(hud::menus::Action::RemoveBot); }
        assert_eq!(g.world.players.len(), 2);
        g.vis.pose(&g.world, &g.prev, 1.0, DT);
        assert!(!g.rebuild);
        assert_eq!(instances, g.vis.scene.instances.as_ptr());
    }

    #[test]
    fn a_full_match_adds_a_seat_offline_and_the_start_screen_takes_no_joins() {
        let mut g = Gg::new(World::new(5, &[false, false]), Session::Local, vec![0, 1]);
        assert!(g.split, "two local seats are splitscreen");
        // Launch seats take the devices in order, as the shell would.
        let mut d = idle(1);
        d[1].1 = press(Buttons::WEST);
        step(&mut g, &d);
        assert_eq!(g.binding, vec![vec![Device::Keyboard], vec![Device::Pad(0)]], "pad0 drives seat 2, no join");
        let mut d = idle(2);
        d[2].1 = press(Buttons::WEST);
        step(&mut g, &d);
        assert_eq!(g.slots, vec![0, 1, 2], "no bot free: a third player is added");
        assert_eq!(g.world.players.len(), 3);
        assert!(!g.world.players[2].bot);
        // The determinism of the sim: the same joins at the same ticks give the same world.
        let digest = |g: &Gg| g.world.digest();
        let mut a = Gg::new(World::new(9, &[false, true, true]), Session::Local, vec![0]);
        let mut b = Gg::new(World::new(9, &[false, true, true]), Session::Local, vec![0]);
        for g in [&mut a, &mut b] {
            g.world.release_cage();
            g.command("split on").unwrap();
            for t in 0..90 {
                let mut d = idle(1);
                if t == 30 { d[1].1 = press(Buttons::WEST); }
                if t > 30 { d[1].1 = Controls { mv: [0.3, 1.0], held: Buttons(Buttons::RIGHT_TRIGGER), ..Default::default() }; }
                step(g, &d);
            }
        }
        assert_eq!(digest(&a), digest(&b));
        assert_eq!(a.slots, vec![0, 1]);
        // Behind the start screen the local seats are bots: no joins.
        let mut p = Gg::new(World::new(5, &[false, true]), Session::Local, vec![0]);
        p.split = true;
        assert!(p.can_join());
        let mut d = idle(1);
        d[1].1 = press(Buttons::WEST);
        p.start_seats = vec![0];
        step(&mut p, &d);
        assert_eq!(p.slots, vec![0], "no joins behind the start screen");
    }

    #[test]
    fn copy_puts_the_ticket_on_the_clipboard_and_names_reach_the_hud() {
        app::clipboard::memory_only();
        let mut g = Gg::new(World::new(5, &[false, true]), Session::Local, vec![0]);
        g.ui.player_name = "Joep".into();
        step(&mut g, &idle(0));
        assert_eq!(g.hud.name(0), "Joep");
        g.command("menu pause").unwrap();
        assert!(!app::Game::text_input(&g));
        g.command("focus join").unwrap();
        assert!(app::Game::text_input(&g));
        g.text(app::TextEvent::Insert("xyz".into()));
        assert_eq!(g.ui.join.text(), "xyz");
        let state: serde_json::Value = serde_json::from_str(&g.state().unwrap()).unwrap();
        assert_eq!(state["menu"]["join_field"], "xyz");
        assert_eq!(state["menu"]["text_input"], true);
    }

    #[test]
    fn start_menu_and_pause_adjust_bots_revive_joined_humans_and_size_coop_views() {
        let mut g = Gg::new(World::new(5, &[false, true, true, true]), Session::Local, vec![0]);
        g.show_start();
        assert_eq!(g.ctx(None).bots, 3, "start screen excludes the host's own seat from bot count");
        g.act(hud::menus::Action::RemoveBot);
        assert_eq!(g.ctx(None).bots, 2);
        assert_eq!(g.world.players.len(), 3);
        // Joining into a dead/ghost bot seat revives that player for the round.
        g.world.players[1].alive = false;
        g.world.players[1].ghost = true;
        g.world.join_human(1);
        assert!(g.world.players[1].alive && !g.world.players[1].ghost && !g.world.players[1].bot);
        // With two humans in the match, bots can be lowered all the way to 0 (1v1).
        g.act(hud::menus::Action::RemoveBot);
        assert_eq!(g.ctx(None).bots, 0);
        assert_eq!(g.world.players.len(), 2);
        // Switching to Wave Survival expands Visuals capacity for wave enemies.
        g.act(hud::menus::Action::Wave);
        assert!(g.vis.player_capacity() >= view::COOP_VIEW_PLAYERS);
    }
}
