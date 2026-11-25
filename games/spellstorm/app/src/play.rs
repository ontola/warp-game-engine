//! The game in a window: local pads and the keyboard drive the human
//! wizards (pad.gd's buttons), bots fill the other seats, and the view
//! follows the sim. The same `Spellstorm` renders headless for shots.
// The web has no std clock, and web_time is only a dependency on the web and Android.
#[cfg(any(target_arch = "wasm32", target_os = "android"))]
use web_time::Instant;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use std::time::Instant;
use crate::kits::Kits;
use crate::view::View;
use app::{Buttons, Controls, Draw, Frame};
use glam::Vec2;
use render::hud::HudDrawList;
use spellstorm::wizard::Pad;
use crate::party::{Ev as PartyEv, Party};
use spellstorm::world::NAMES;
use spellstorm::{Seat, Settings, World};

pub struct Spellstorm {
    pub world: World,
    pub view: View,
    pub hud: crate::hud::Hud,
    pub menu: crate::menu::Menu,
    humans: usize,
    /// The wizard each player's controls drive, in controls order.
    drive: Vec<usize>,
    /// Under a party: its seat for each entry of `drive`.
    pub party: Option<Party>,
    party_seats: Vec<usize>,
    quit: bool,
    built: bool,
    /// The scene was built ahead ([`Spellstorm::prepare`]): the first draw
    /// only hands it to the renderer.
    prebuilt: bool,
    /// The renderer for the prebuilt scene, made on a helper thread while
    /// the host's loading card moves ([`Spellstorm::set_renderer_ahead`]).
    renderer_ahead: Option<render::Renderer>,
    /// Prepared: tell the host ready once the run's scene is built, so the
    /// build never stalls a started game.
    ready_after_build: bool,
    interp: crate::interp::Interp,
    pub tuning: crate::live::Tuning,
    pub shader: crate::live::Shader,
    pub live: Option<Box<crate::live::SsLive>>,
    pub scan_t: u64,
}

/// The shell's buttons as pad.gd's.
const MAP: [(u32, u32); 16] = [
    (Buttons::SOUTH, Pad::A),
    (Buttons::EAST, Pad::B),
    (Buttons::WEST, Pad::X),
    (Buttons::NORTH, Pad::Y),
    (Buttons::LEFT_SHOULDER, Pad::LB),
    (Buttons::RIGHT_SHOULDER, Pad::RB),
    (Buttons::LEFT_TRIGGER, Pad::LT),
    (Buttons::RIGHT_TRIGGER, Pad::RT),
    (Buttons::SELECT, Pad::BACK),
    (Buttons::START, Pad::START),
    (Buttons::LEFT_STICK, Pad::LS),
    (Buttons::RIGHT_STICK, Pad::RS),
    (Buttons::UP, Pad::UP),
    (Buttons::DOWN, Pad::DOWN),
    (Buttons::LEFT, Pad::LEFT),
    (Buttons::RIGHT, Pad::RIGHT),
];

/// One tick's controls as a wizard's pad (Pad._read_joypad): the shell's
/// stick y is up, the game's is down the screen (towards the camera).
pub fn pad(c: &Controls) -> Pad {
    let mut p = Pad::default();
    for (b, to) in MAP {
        if c.held.has(b) || c.pressed.has(b) {
            p.set(to, true);
        }
    }
    p.mv = Pad::dead(Vec2::new(c.mv[0], -c.mv[1]));
    p.aim = Pad::dead(Vec2::new(c.aim[0], -c.aim[1]));
    p.fire = p.aim.length() > 0.5;
    p
}

impl Spellstorm {
    /// Build the scene before the first frame (while the host's loading
    /// bar shows), so that frame only uploads it.
    /// The renderer made ahead for the scene [`Spellstorm::prepare`] built:
    /// the first frame only takes it (on a tablet it took 6.6 s in there).
    pub fn set_renderer_ahead(&mut self, renderer: render::Renderer) {
        if self.prebuilt { self.renderer_ahead = Some(renderer); }
    }

    pub fn prepare(&mut self) -> Result<(), String> {
        if !self.built && !self.prebuilt {
            self.view.build(&self.world)?;
            self.prebuilt = true;
        }
        Ok(())
    }

    /// `humans` seats for local players first, then `bots`.
    pub fn new(kits: Kits, humans: usize, bots: usize, seed: u64) -> Spellstorm {
        Self::with_party(kits, humans, bots, seed, None)
    }

    /// Under a party the host seats everyone (`prepare`); until then the
    /// bots play hidden.
    pub fn with_party(kits: Kits, humans: usize, bots: usize, seed: u64, party: Option<Party>) -> Spellstorm {
        let humans = if party.is_some() { 0 } else { humans };
        let bots = if party.is_some() { bots.max(1) } else { bots };
        let mut seats: Vec<Seat> = (0..humans).map(|i| Seat::player(i, NAMES[i % NAMES.len()])).collect();
        seats.extend((humans..humans + bots).map(Seat::bot));
        // The balance knobs and the surface shader from the repository.
        let dir = crate::live::game_dir();
        let tuning = crate::live::Tuning::new(dir.as_deref());
        let shader = crate::live::Shader::new(dir.as_deref());
        render::set_game_surface(&crate::live::surface(&shader.good));
        crate::live::install_mask();
        crate::effects::watch(dir.as_deref());
        // The spell rules in Luau, from the repository's mods (watched) or
        // the copy built in.
        match spellstorm::rules::install(spellstorm::rules::find_dir().as_deref()) {
            Ok(_) => {}
            Err(e) => eprintln!("mods: {e} (the built-in Rust rules play)"),
        }
        let world = World::new(&seats, seed, Settings::default());
        let mut hud = crate::hud::Hud::default();
        hud.level(&world);
        let mut menu = crate::menu::Menu::new(party.is_none());
        // Straight into the first level, no title screen (Joep); Start
        // opens the menu.
        let _ = humans;
        Spellstorm { world, view: View::new(kits), hud, menu, humans, drive: (0..humans).collect(), party, party_seats: vec![], quit: false, built: false, prebuilt: false, renderer_ahead: None, ready_after_build: false, interp: Default::default(), tuning, shader, live: None, scan_t: 0 }
    }

    /// The party's seats (main.gd _on_prepared): a wizard for each, couch
    /// seats as players, AI seats as bots, and a fresh run.
    fn seat(&mut self, list: Vec<crate::party::Seated>) {
        let mut seats = vec![];
        self.drive.clear();
        self.party_seats.clear();
        for s in &list {
            let i = seats.len();
            if s.bot {
                seats.push(Seat::bot(i));
                continue;
            }
            let mut seat = Seat::player(i, if s.name.is_empty() { NAMES[i % NAMES.len()] } else { &s.name });
            if let Some(c) = s.color {
                seat.robe = [c[0], c[1], c[2], 1.0];
            }
            seat.player_id = s.player_id.clone();
            self.drive.push(i);
            self.party_seats.push(s.seat);
            seats.push(seat);
        }
        if seats.is_empty() {
            seats.push(Seat::bot(0));
        }
        let settings = self.world.settings.clone();
        let seed = self.world.seed.wrapping_add(1);
        self.world = World::new(&seats, seed, settings);
        self.humans = self.drive.len();
        self.hud = crate::hud::Hud::default();
        self.hud.level(&self.world);
        self.view.fx = crate::fx::Fx::default();
        self.menu.open = false;
        self.built = false;
    }

    /// A player picks up a controller mid-run (Joep): their wizard joins
    /// the run in progress, driven by the next controls. Without a party
    /// host only; under one the host seats people.
    pub fn join_player(&mut self, name: Option<&str>, color: Option<[f32; 3]>) -> bool {
        if self.party.is_some() {
            return false;
        }
        self.add_wizard(name, color, "").is_some()
    }

    /// A wizard for someone new in the run in progress (random tomes equal
    /// to the others'), driven by the next controls; None when four play.
    fn add_wizard(&mut self, name: Option<&str>, color: Option<[f32; 3]>, player_id: &str) -> Option<usize> {
        if self.drive.len() >= 4 {
            return None;
        }
        let k = self.drive.len();
        let mut seat = Seat::player(k, name.filter(|n| !n.is_empty()).unwrap_or(NAMES[k % NAMES.len()]));
        if let Some(c) = color {
            seat.robe = [c[0], c[1], c[2], 1.0];
        }
        seat.player_id = player_id.to_string();
        let i = self.world.join(&seat);
        self.drive.push(i);
        self.humans = self.drive.len();
        Some(i)
    }

    /// The party's news, then its controllers for this tick.
    fn party_tick(&mut self) -> Option<Vec<Controls>> {
        let evs = self.party.as_mut()?.poll();
        for ev in evs {
            match ev {
                PartyEv::Prepare(list) => {
                    self.seat(list);
                    self.ready_after_build = true;
                }
                PartyEv::Names(names) => {
                    for id in self.world.wizards.clone() {
                        let mut w = self.world.wiz_mut(id);
                        if let Some((_, n)) = names.iter().find(|(pid, n)| !pid.is_empty() && *pid == w.player_id && !n.is_empty()) {
                            w.name = n.clone();
                        }
                    }
                }
                // Someone picked up a spare controller and pressed A (a
                // desktop host): their wizard joins the run in progress.
                PartyEv::Seats(list) => {
                    for s in list.iter().filter(|s| !s.bot) {
                        if self.party_seats.contains(&s.seat) {
                            continue;
                        }
                        if self.add_wizard(Some(&s.name), s.color, &s.player_id).is_some() {
                            self.party_seats.push(s.seat);
                        }
                    }
                }
                PartyEv::Setting(k, v) => self.world.set_option(&k, &v),
                PartyEv::NewRun => self.world.new_run(),
                PartyEv::Dispose => self.menu.open = false,
            }
        }
        let takes = self.drive.len() < 4;
        let p = self.party.as_mut()?;
        p.watch_joins(takes);
        Some(p.controls(&self.party_seats, spellstorm::world::DT as f32, self.menu.open))
    }

    /// One tick with these players' controls.
    pub fn step(&mut self, controls: &[Controls]) {
        self.live_tick();
        let from_party = self.party_tick();
        let controls = from_party.as_deref().unwrap_or(controls);
        let host_paused = self.party.as_ref().is_some_and(|p| !p.running());
        let was_open = self.menu.open;
        match self.menu.update(&mut self.world, controls, spellstorm::world::DT as f32) {
            crate::menu::Action::StartAt(b) => self.world.start_at(b),
            crate::menu::Action::Quit => self.quit = true,
            crate::menu::Action::Lobby => {
                if let Some(p) = &self.party {
                    p.overlay();
                }
            }
            _ => {}
        }
        // The game stands still under the menu; the press that closed it
        // does not reach the wizards.
        self.world.paused = self.menu.open || host_paused;
        let idle = self.menu.open || was_open;
        for (k, c) in controls.iter().enumerate() {
            let c = if idle { &Controls::default() } else { c };
            if let Some(&id) = self.drive.get(k).and_then(|&i| self.world.wizards.get(i)) {
                self.world.wiz_mut(id).input = pad(c);
            }
        }
        self.interp.keep(&mut self.world);
        self.world.step();
        if let Some(p) = &mut self.party {
            for e in &self.world.events {
                match e {
                    spellstorm::fx::Ev::RunOver { .. } => p.finished(),
                    spellstorm::fx::Ev::Level { depth: 0, .. } => p.room_log("New run"),
                    _ => {}
                }
            }
        }
        self.view.fx.absorb(&self.world);
        self.hud.absorb(&self.world);
        if !self.world.paused {
            self.hud.update(spellstorm::world::DT as f32);
        }
    }
}

impl app::Game for Spellstorm {
    fn takes_joins(&self) -> bool { self.party.is_none() && self.drive.len() < 4 }
    fn notice(&mut self) -> Option<String> { self.party.as_mut()?.take_notice() }
    fn player_joins(&mut self, name: Option<&str>, color: Option<[f32; 3]>) -> bool {
        self.join_player(name, color)
    }

    fn scene_tuning(&self)->Option<&app::editor::SceneTuning>{Some(&self.view.spell_effects)}
    fn scene_tuning_mut(&mut self)->Option<&mut app::editor::SceneTuning>{Some(&mut self.view.spell_effects)}
    fn local_players(&self) -> usize {
        if self.party.is_some() { 0 } else { self.humans }
    }

    fn loading(&mut self, percent: u32, step: &str) { if let Some(p) = self.party.as_mut() { p.loading(percent, step); } }

    fn screen(&self) -> app::Screen {
        self.party.as_ref().map_or(app::Screen::Normal, |p| p.screen())
    }

    fn tick(&mut self, controls: &[Controls]) {
        self.step(controls);
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        self.live_frame(frame);
        if !self.built || self.view.stale(&self.world) {
            let t0 = Instant::now();
            if !std::mem::take(&mut self.prebuilt) || self.view.stale(&self.world) {
                self.renderer_ahead = None;
                self.view.build(&self.world)?;
            }
            let t1 = Instant::now();
            match self.renderer_ahead.take() {
                Some(r) => frame.set_renderer(r),
                None => frame.set_scene(&self.view.scene, &self.view.light),
            }
            let t2 = Instant::now();
            let r = frame.renderer().ok_or("no renderer")?;
            self.view.upload(frame.gpu, r)?;
            self.built = true;
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
            let line = format!("spellstorm: scene {:.0} ms, renderer {:.0} ms, upload {:.0} ms", ms(t0, t1), ms(t1, t2), ms(t2, Instant::now()));
            #[cfg(any(target_arch = "wasm32", target_os = "android"))]
            app::web::log(&line);
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            eprintln!("{line}");
        }
        if self.ready_after_build {
            self.ready_after_build = false;
            if let Some(p) = &mut self.party {
                p.ready();
            }
        }
        let r = frame.renderer().ok_or("no renderer")?;
        // Drawn between the last two ticks, so a fast screen moves smoothly.
        self.interp.ease(&mut self.world, frame.alpha);
        // Paused, nothing moves: no animation, particle or effect clock runs.
        let dt = if self.world.paused { 0.0 } else { frame.dt };
        let posed = self.view.pose(frame.gpu, r, &self.world, dt);
        let mut hud = HudDrawList::new(frame.size);
        // SPELLSTORM_NO_HUD=1: the picture alone (store videos), no HUD,
        // names, hints, banners or level cards.
        if std::env::var_os("SPELLSTORM_NO_HUD").is_none() {
            self.hud.draw(&mut hud, &self.world, &self.view.camera, frame.size);
        }
        self.interp.restore(&mut self.world);
        posed?;
        self.menu.draw(&mut hud, &self.world, frame.size);
        Ok(Draw { cameras: vec![self.view.camera], light: self.view.light.clone(), hud })
    }

    /// Esc: the host's overlay in a party, else the menu.
    fn escape(&mut self) -> bool {
        match &self.party {
            Some(p) if !p.session().is_empty() => p.overlay(),
            _ => self.menu.escape(&self.world),
        }
        true
    }

    fn command(&mut self, line: &str) -> Result<String, String> {
        self.console(line)
    }

    fn wants_capture(&self) -> bool {
        self.live_wants_capture()
    }

    fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) {
        self.live_captured(size, rgba)
    }

    fn quit(&self) -> bool {
        self.quit
    }

    fn title(&self) -> Option<String> {
        Some("Spellstorm".into())
    }

    fn state(&self) -> Option<String> {
        Some(self.world.snapshot().to_string())
    }

    /// Twin-stick aiming reads the raw sticks; Pad::dead shapes them.
    /// Pads and keys only: the mouse is never captured, nor asked for.
    fn mouse_capture(&self) -> Option<bool> {
        Some(false)
    }

    fn tuning(&self, t: &mut app::Tuning) {
        t.deadzone = 0.0;
        t.curve = 1.0;
    }
}
