//! SpaceRacer on Warp: the desktop game (`run`) and the native VR app (`xr`).
mod crash_fx;
mod integration_probe;
mod jets;
mod menu_view;
mod missile_mesh;
mod ocean_life;
mod party;
mod party_settings;
mod pilot_faces;
mod race_hud;
mod replay;
mod respawn;
mod stage;
mod stage_mesh;
mod stage_terrain;
mod visuals;
mod weapon_art;
pub mod web;
pub mod xr;
use app::{Buttons, Controls, Device, Draw, Frame, Game};
use glam::{Mat3, Quat};
use render::{
    Camera,
    hud::{HudDrawList, TextAlign, TextRun},
};
use serde_json::json;
use spaceracer::{
    chase::Chase,
    handling::Input,
    race::{Controls as Drive, Race, Weapon},
    race_rules::Pose,
    rules::Rules,
};
use std::path::PathBuf;

struct Racer {
    handling:app::editor::SceneTuning,
    race: Race,
    visuals: visuals::Visuals,
    root: PathBuf,
    players: usize,
    seed: i64,
    difficulty: usize,
    biome: usize,
    menu: Option<usize>,
    settings_dirty: bool,
    race_started: bool,
    random_biome: bool,
    random_difficulty: bool,
    random_seed: bool,
    menu_axis: [i32; 2],
    cameras: Vec<Chase>,
    replay: Vec<replay::Replay>,
    respawn: Vec<respawn::Camera>,
    previous: Vec<Pose>,
    devices: Vec<Device>,
    party: Option<party::Party>,
    preferences: party_settings::Settings,
    live_settings: party_settings::Live,
    fatal: Option<String>,
    ready: bool,
    demo: bool,
    blur_enabled: bool,
    reload_ticks: u32,
    portrait: bool,
    weapon_view: bool,
    missile_view: bool,
    /// A pointer click on a menu row this tick: the side clicked (VR pointer).
    menu_click: Option<i32>,
    /// Build new races on a worker thread behind a loading screen (VR: a
    /// frozen frame in a headset is not acceptable).
    background_loading: bool,
    loading: Option<Loading>,
    /// The next race, built in the background while the results show.
    next: Option<Next>,
    /// Ticks since the race ended; A continues only after a short beat.
    over_ticks: u32,
    /// The menu's Back to lobby, for a host in this program (a browser page).
    back: bool,
}
/// Results stay on screen this long before A starts the next race (120 Hz).
const CONTINUE_AFTER_TICKS: u32 = 240;
/// A next race preloaded behind the results, with the grid it was built for.
struct Next {
    worker: Worker,
    grid: (usize, usize),
}
/// A race being built on a worker thread.
struct Loading {
    worker: Worker,
    started: Instant,
}
/// `std::time::Instant` panics in a browser.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;
type Built = Result<(Race, visuals::Visuals), String>;
/// A race build: a worker thread natively; in a browser (one thread, and
/// the GPU handles are not `Send`) built at once.
#[cfg(not(target_arch = "wasm32"))]
struct Worker(std::thread::JoinHandle<Built>);
#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    fn spawn(spec: RaceSpec) -> Self { Self(std::thread::spawn(move || spec.build())) }
    fn is_finished(&self) -> bool { self.0.is_finished() }
    fn join(self) -> Built { self.0.join().map_err(|_| "race build panicked".to_string())? }
}
#[cfg(target_arch = "wasm32")]
struct Worker(Built);
#[cfg(target_arch = "wasm32")]
impl Worker {
    fn spawn(spec: RaceSpec) -> Self { Self(spec.build()) }
    fn is_finished(&self) -> bool { true }
    fn join(self) -> Built { self.0 }
}
/// What a new race is built from; all owned, so it can move to a worker.
struct RaceSpec {
    root: PathBuf,
    seed: i64,
    difficulty: usize,
    biome: usize,
    players: usize,
    racers: usize,
    laps: u32,
    seats: Vec<(String, bool)>,
    pickups: bool,
}
impl RaceSpec {
    fn build(self) -> Result<(Race, visuals::Visuals), String> {
        visuals::progress(20);
        let rules = Rules::new(Some(&self.root.join("rules")))?;
        let mut race = Race::with_rules(self.seed, DIFFICULTIES[self.difficulty], BIOMES[self.biome], self.players, self.racers, self.laps, rules)?;
        visuals::progress(150);
        for (driver, (name, bot)) in race.drivers.iter_mut().zip(self.seats) {
            driver.name = name;
            driver.bot = bot;
        }
        if !self.pickups {
            race.pickups.clear();
        }
        let visuals = visuals::Visuals::new(&race, &self.root.join("assets"))?;
        visuals::progress(1000);
        Ok((race, visuals))
    }
}
const BIOMES: [&str; 5] = ["city", "forest", "cell", "desert", "ocean"];
const DIFFICULTIES: [&str; 3] = ["easy", "normal", "hard"];
impl Racer {
    fn new(
        root: PathBuf,
        players: usize,
        seed: i64,
        biome: usize,
        difficulty: usize,
        demo: bool,
    ) -> Result<Self, String> {
        let rules = Rules::new(Some(&root.join("rules")))?;
        let mut race = Race::with_rules(
            seed,
            DIFFICULTIES[difficulty],
            BIOMES[biome],
            players,
            if players > 1 { players } else { spaceracer::race::MAX_RACERS },
            3,
            rules,
        )?;
        let visuals = visuals::Visuals::new(&race, &root.join("assets"))?;
        race.obstacles = visuals.obstacles.clone();
        race.environment.landscape = Some(visuals.stage.landscape.clone());
        let previous = (0..race.pilots.len()).map(|i| race.pose(i)).collect();
        let preferences = party_settings::Settings::new(&root)?;
        let live_settings = preferences.live()?;
        let party = party::Party::connect(preferences.specs.clone())?;
        Ok(Self {
            race,
            visuals,
            root,
            players,
            seed,
            difficulty,
            biome,
            menu: if demo || party.is_some() {
                None
            } else {
                Some(1)
            },
            menu_axis: [0; 2],
            settings_dirty: false,
            race_started: demo || party.is_some(),
            random_biome: false,
            random_difficulty: false,
            random_seed: false,
            cameras: vec![Chase::default(); 4],
            replay: vec![replay::Replay::default(); 4],
            respawn: vec![respawn::Camera::default(); 4],
            previous,
            devices: vec![],
            party,
            preferences,
            handling:app::editor::SceneTuning::new("spaceracer","Handling",&[
                app::editor::SceneControl{label:"Steering gain",default:1.,min:0.25,max:6.,step:0.05},
                app::editor::SceneControl{label:"Airbrake gain",default:1.,min:0.25,max:2.,step:0.05},
                app::editor::SceneControl{label:"Ground thrust",default:1.,min:0.25,max:3.,step:0.05},
                app::editor::SceneControl{label:"Lateral grip",default:1.,min:0.1,max:3.,step:0.05},
                app::editor::SceneControl{label:"Auto-straighten",default:1.,min:0.,max:3.,step:0.05},
                app::editor::SceneControl{label:"Ground strafe speed",default:1.,min:0.25,max:3.,step:0.05},
            ]),
            live_settings,
            fatal: None,
            ready: false,
            demo,
            blur_enabled: true,
            reload_ticks: 0,
            portrait: false,
            weapon_view: false,
            missile_view: false,
            menu_click: None,
            background_loading: false,
            loading: None,
            next: None,
            over_ticks: 0,
            back: false,
        })
    }
    /// The grid: the people here plus the AI racers. Auto is a full grid for
    /// someone alone and no AI with several players.
    fn racers(&self) -> usize {
        let seated = self.party.as_ref().map_or(self.players, |p| p.seats.len()).max(self.players);
        let ai = self.preferences.values["ai_players"].as_str().and_then(|s| s.parse::<usize>().ok());
        match ai {
            Some(n) => seated + n,
            None if self.players > 1 => seated,
            None => spaceracer::race::MAX_RACERS,
        }
    }
    fn race_spec(&self) -> RaceSpec {
        RaceSpec {
            root: self.root.clone(),
            seed: self.seed,
            difficulty: self.difficulty,
            biome: self.biome,
            players: self.players,
            racers: self.racers(),
            laps: self.preferences.values["laps"].as_u64().unwrap_or(3) as u32,
            seats: self.party.as_ref().map_or(vec![], |p| p.seats.iter().map(|s| (s.name.clone(), s.ai)).collect()),
            pickups: self.preferences.values["weapon_pickups"] != false,
        }
    }
    fn restart(&mut self) -> Result<(), String> {
        // Whatever was preloaded was built for the old settings.
        self.next = None;
        let spec = self.race_spec();
        if self.background_loading {
            visuals::progress(0);
            self.menu = None;
            self.loading = Some(Loading { worker: Worker::spawn(spec), started: Instant::now() });
            return Ok(());
        }
        let (race, visuals) = spec.build()?;
        self.finish_restart(race, visuals)
    }
    /// The loading screen's bar, 0..1, while a race builds in the background.
    fn loading_progress(&self) -> Option<f32> {
        self.loading.as_ref().map(|_| visuals::PROGRESS.load(std::sync::atomic::Ordering::Relaxed) as f32 / 1000.)
    }
    fn finish_restart(&mut self, mut race: Race, visuals: visuals::Visuals) -> Result<(), String> {
        self.visuals = visuals;
        race.obstacles = self.visuals.obstacles.clone();
        race.environment.landscape = Some(self.visuals.stage.landscape.clone());
        self.previous = (0..race.pilots.len()).map(|i| race.pose(i)).collect();
        self.race = race;
        self.apply_live_settings();
        self.sync_profiles();
        if let Some(p) = &mut self.party {
            p.new_race();
        }
        self.cameras = vec![Chase::default(); 4];
        self.replay = vec![replay::Replay::default(); 4];
        self.respawn = vec![respawn::Camera::default(); 4];
        self.menu = None;
        self.settings_dirty = false;
        self.race_started = true;
        self.over_ticks = 0;
        Ok(())
    }
    /// Roll the next race's Random choices and build it behind the results.
    fn preload_next(&mut self) -> Result<(), String> {
        self.roll_random()?;
        let spec = self.race_spec();
        let grid = (spec.players, spec.racers);
        self.next = Some(Next { worker: Worker::spawn(spec), grid });
        Ok(())
    }
    /// Results hint: None while racing, else whether the next race is ready.
    fn next_ready(&self) -> Option<bool> {
        if !self.race.over || self.demo { return None; }
        Some(self.over_ticks >= CONTINUE_AFTER_TICKS && self.next.as_ref().is_some_and(|n| n.worker.is_finished()))
    }
    /// Pick anew whatever is set to Random; the rest stays as chosen.
    fn roll_random(&mut self) -> Result<(), String> {
        let defaults: serde_json::Value = self.race.rules.call("menu_defaults", json!({"entropy":menu_entropy()}))?;
        if self.random_seed { self.seed=defaults["seed"].as_i64().ok_or("random seed")?.clamp(1,99999); }
        if self.random_biome { self.biome=defaults["biome"].as_u64().ok_or("random world")?.min(4) as usize; }
        if self.random_difficulty { self.difficulty=defaults["difficulty"].as_u64().ok_or("random difficulty")?.min(2) as usize; }
        Ok(())
    }
    fn start_new_race(&mut self) -> Result<(), String> {
        // The preloaded race already has its Random choices rolled; it shows
        // a loading screen only if it is still building.
        if let Some(next) = self.next.take() {
            if !self.settings_dirty && next.grid == (self.players, self.racers()) {
                if next.worker.is_finished() {
                    let (race, visuals) = next.worker.join()?;
                    return self.finish_restart(race, visuals);
                }
                visuals::progress(500);
                self.menu = None;
                self.loading = Some(Loading { worker: next.worker, started: Instant::now() });
                return Ok(());
            }
        }
        self.roll_random()?;
        self.restart()
    }
    fn random_race(&mut self) -> Result<(), String> {
        self.seed = (self.seed * 48271) % 99999 + 1;
        if self.party.is_none() {
            self.biome = (self.seed as usize / 7) % 5;
        }
        self.restart()
    }
    fn apply_live_settings(&mut self) {
        for p in &mut self.race.pilots {
            p.boost_cost = self.live_settings.cost;
            p.boost_duration = self.live_settings.duration;
        }
        self.race.balance.refill_rate = self.live_settings.refill;
        self.race.balance.respawn_seconds = self.live_settings.recovery;
    }
    fn sync_profiles(&mut self) {
        if let Some(p) = &self.party {
            for (i, seat) in p.seats.iter().enumerate().take(self.race.drivers.len()) {
                self.race.drivers[i].name = seat.name.clone();
                self.race.drivers[i].bot = seat.ai;
                if let Some(profile) = p.host.client.state.player(&seat.player_id) {
                    self.visuals
                        .faces
                        .update(&mut self.visuals.scene, i, profile);
                }
            }
        }
    }
    fn setting(&mut self, key: &str, value: serde_json::Value) -> Result<(), String> {
        if !self.preferences.set(key, value.clone())? {
            return Ok(());
        }
        match key {
            "seed" => {
                let n = value.as_i64().unwrap();
                self.random_seed = n == 0;
                if n > 0 {
                    self.seed = n;
                }
            }
            "difficulty" => {
                self.difficulty = DIFFICULTIES
                    .iter()
                    .position(|s| Some(*s) == value.as_str())
                    .unwrap()
            }
            "biome" => {
                let world = BIOMES.iter().position(|s| Some(*s) == value.as_str());
                self.random_biome = world.is_none();
                self.biome = world.unwrap_or(self.biome);
            }
            _ => {}
        }
        self.live_settings = self.preferences.live()?;
        self.apply_live_settings();
        Ok(())
    }
    fn controls(c: Controls, pad: bool) -> Drive {
        Drive {
            drive: Input {
                throttle: if pad {
                    c.triggers[1] as f64
                } else {
                    c.mv[1].max(0.) as f64
                },
                brake: if pad {
                    c.triggers[0] as f64
                } else {
                    (-c.mv[1]).max(0.) as f64
                },
                steer: c.mv[0] as f64,
                strafe: c.aim[0] as f64,
                trim: if pad {
                    (c.aim[1] + c.mv[1]).clamp(-1., 1.) as f64
                } else {
                    (c.held.has(Buttons::UP) as i32 - c.held.has(Buttons::DOWN) as i32) as f64
                },
                boost: c.held.has(Buttons::EAST) || (!pad && c.held.has(Buttons::SOUTH)),
            },
            fire: c.pressed.has(Buttons::WEST),
            reset: c.pressed.has(Buttons::NORTH),
            left: c.held.has(Buttons::LEFT_SHOULDER),
            right: c.held.has(Buttons::RIGHT_SHOULDER),
            rail_aim:None,
        }
    }
    /// A pointer over the menu at `at` (HUD pixels of a `size` HUD): hovering
    /// selects a row, a click accepts it or steps its value left or right.
    fn menu_pointer(&mut self, at: [f32; 2], size: [u32; 2], click: bool) -> bool {
        if self.menu.is_none() { return false; }
        let ui_scale = (size[1] as f32 / 900.).max(1.);
        let Some((row, side)) = menu_view::hit(size.map(|x| x as f32 / ui_scale), self.race_started, self.lobby(), at.map(|v| v / ui_scale)) else { return false };
        self.menu = Some(row);
        if click { self.menu_click = Some(side); }
        true
    }
    fn menu_tick(&mut self, c: Controls) {
        let Some(row) = self.menu else {
            return;
        };
        let axis = [
            if c.mv[0].abs() > 0.6 {
                c.mv[0].signum() as i32
            } else {
                0
            },
            if c.mv[1].abs() > 0.6 {
                -(c.mv[1].signum() as i32)
            } else {
                0
            },
        ];
        let click = self.menu_click.take();
        let horizontal = c.pressed.has(Buttons::RIGHT) as i32 - c.pressed.has(Buttons::LEFT) as i32
            + if (2..menu_view::LOBBY).contains(&row) { click.unwrap_or(0) } else { 0 }
            + if axis[0] != self.menu_axis[0] {
                axis[0]
            } else {
                0
            };
        let vertical = c.pressed.has(Buttons::DOWN) as i32 - c.pressed.has(Buttons::UP) as i32
            + if axis[1] != self.menu_axis[1] {
                axis[1]
            } else {
                0
            };
        self.menu_axis = axis;
        let out=self.race.rules.call::<serde_json::Value>("menu",json!({"players":self.players,"biome":if self.random_biome {-1}else{self.biome as i64},"difficulty":if self.random_difficulty {-1}else{self.difficulty as i64},"seed":if self.random_seed {0}else{self.seed},"can_resume":self.race_started,"row":row,"vertical":vertical,"horizontal":horizontal,"accept":c.pressed.has(Buttons::SOUTH) || ((row < 2 || row == menu_view::LOBBY) && click.is_some()),"start":c.pressed.has(Buttons::START),"lobby":self.lobby()}));
        let result = (|| -> Result<(), String> {
            let out = out?;
            let row = out["row"].as_u64().ok_or("menu row")? as usize;
            self.menu = Some(row);
            // Apply the displayed selections before starting, including a
            // simultaneous direction + Start input on the same controller tick.
            if out["rebuild"].as_bool() == Some(true) {
                self.players=out["players"].as_u64().ok_or("menu player count")?.clamp(1,4) as usize;
                let biome=out["biome"].as_i64().ok_or("menu world")?;
                let difficulty=out["difficulty"].as_i64().ok_or("menu difficulty")?;
                let seed=out["seed"].as_i64().ok_or("menu seed")?;
                self.random_biome=biome<0; self.random_difficulty=difficulty<0; self.random_seed=seed==0;
                if !self.random_biome {self.biome=biome.clamp(0,4) as usize;}
                if !self.random_difficulty {self.difficulty=difficulty.clamp(0,2) as usize;}
                if !self.random_seed {self.seed=seed.clamp(1,99999);}
                self.settings_dirty=true;
            }
            match out["action"].as_str().unwrap_or("none") {
                "new" => self.start_new_race()?,
                "play" => {
                    if self.race.over || self.settings_dirty {self.start_new_race()?;}
                    else {self.menu=None; self.race_started=true;}
                }
                // The host pauses the race and shows its room.
                "lobby" => {
                    self.menu=None;
                    if let Some(p)=&self.party {p.overlay();} else {self.back=true;}
                }
                _=>{}
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.fatal = Some(e);
        }
    }
}
impl Racer {
    /// A race for a browser page (web.rs): straight into the race as under
    /// Splitscreen Club, the world drawn from `seed`, later races on Random.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn for_page(root: PathBuf, players: usize, seed: u64) -> Result<Self, String> {
        let mut game = Racer::new(root, players, (seed % 99999) as i64 + 1, (seed / 7 % BIOMES.len() as u64) as usize, 1, false)?;
        game.random_biome = true;
        game.random_difficulty = true;
        game.random_seed = true;
        game.menu = None;
        game.race_started = true;
        Ok(game)
    }
    /// A host takes Back to lobby: Splitscreen Club natively, the page in a browser.
    fn lobby(&self) -> bool { self.party.is_some() || app::host_takes_back() }
}
impl Game for Racer {
    fn take_back(&mut self) -> bool { std::mem::take(&mut self.back) }
    fn scene_tuning(&self)->Option<&app::editor::SceneTuning>{Some(&self.handling)}
    fn scene_tuning_mut(&mut self)->Option<&mut app::editor::SceneTuning>{Some(&mut self.handling)}
    fn local_players(&self) -> usize {
        self.players
    }
    fn devices(&mut self, d: &[(Device, Controls)]) {
        self.devices = d
            .iter()
            .filter(|(d, _)| d.is_pad())
            .map(|(d, _)| *d)
            .collect();
    }
    fn seats(&self) -> Option<Vec<Vec<Device>>> {
        Some(
            (0..self.players)
                .map(|i| {
                    if self.devices.is_empty() && i == 0 {
                        vec![Device::Keyboard]
                    } else {
                        self.devices.get(i).copied().into_iter().collect()
                    }
                })
                .collect(),
        )
    }
    fn mouse_capture(&self) -> Option<bool> {
        Some(false)
    }
    /// Escape works like Start: it opens and closes Space Racer's own menu,
    /// also under the host. Only Back goes to the lobby.
    fn escape(&mut self) -> bool {
        if self.party.is_none() && !self.race_started { return true; }
        if self.menu.is_some() && self.settings_dirty {
            if let Err(e) = self.start_new_race() {
                self.fatal = Some(e);
            }
        } else {
            self.menu = if self.menu.is_some() { self.race_started=true; None } else { Some(0) };
        }
        true
    }
    fn screen(&self) -> app::Screen {
        self.party
            .as_ref()
            .map_or(app::Screen::Normal, |p| p.screen())
    }
    fn loading(&mut self, percent: u32, step: &str) {
        if let Some(p) = &mut self.party {
            p.host.loading(percent, step);
        }
    }
    fn tick(&mut self, controls: &[Controls]) {
        if self.fatal.is_some() {
            return;
        }
        if let Some(loading) = self.loading.take() {
            if !loading.worker.is_finished() {
                self.loading = Some(loading);
                return;
            }
            let built = loading.worker.join();
            let started = loading.started;
            match built.and_then(|(race, visuals)| self.finish_restart(race, visuals)) {
                Ok(()) => println!("race built in the background in {} ms", started.elapsed().as_millis()),
                Err(e) => { self.fatal = Some(e); return; }
            }
        }
        self.reload_ticks += 1;
        if self.reload_ticks % 60 == 0 {
            self.race.reload();
            self.apply_live_settings();
        }
        if let Some(mut p) = self.party.take() {
            let events = p.poll();
            self.party = Some(p);
            let mut prepare = false;
            for event in events {
                match event {
                    party::Event::Prepare => {
                        self.players = self
                            .party
                            .as_ref()
                            .unwrap()
                            .seats
                            .iter()
                            .filter(|s| !s.ai)
                            .count()
                            .clamp(1, 4);
                        let seed = self.preferences.values["seed"].as_i64().unwrap_or(0);
                        self.random_seed = seed == 0;
                        self.random_biome = self.preferences.values["biome"] == "random";
                        if seed != 0 {
                            self.seed = seed;
                        }
                        if let Err(e) = self.roll_random() {
                            self.fatal = Some(e);
                        }
                        self.ready = true;
                        prepare = true;
                    }
                    party::Event::Roster => {
                        self.sync_profiles();
                    }
                    party::Event::NewRace => {
                        if let Err(e) = self.start_new_race() {
                            self.fatal = Some(e);
                        }
                    }
                    party::Event::Setting(key, value) => {
                        if self.ready
                            && self.preferences.values[&key] != value
                            && matches!(
                                key.as_str(),
                                "seed" | "difficulty" | "biome" | "laps" | "weapon_pickups" | "ai_players"
                            )
                        {
                            prepare = true;
                        }
                        if let Err(e) = self.setting(&key, value) {
                            self.fatal = Some(e);
                        }
                    }
                }
            }
            // Apply the complete host settings batch before constructing its race.
            if prepare {
                if let Err(e) = self.restart() {
                    self.fatal = Some(e);
                }
            }
            if self.ready && self.visuals.uploaded && self.fatal.is_none() {
                self.party.as_mut().unwrap().ready();
                self.ready = false;
            }
            if !self.party.as_ref().unwrap().running() {
                return;
            }
        }
        // Start opens the race menu with the track settings; only Back
        // (kept by the host) goes to the lobby.
        let mut party_inputs = None;
        if let Some(p) = &mut self.party {
            let (drive, menu) = p.controls();
            if self.menu.is_some() {
                let seated = self.players;
                self.menu_tick(menu);
                // The roster decides the player count under the host.
                self.players = seated;
                return;
            }
            if menu.pressed.has(Buttons::START) {
                self.menu = Some(0);
                return;
            }
            if self.next_ready() == Some(true) && menu.pressed.has(Buttons::SOUTH) {
                if let Err(e) = self.start_new_race() { self.fatal = Some(e); }
                return;
            }
            party_inputs = Some(drive);
        } else {
            let c = controls.first().copied().unwrap_or_default();
            if self.menu.is_some() {
                self.menu_tick(c);
                return;
            }
            if c.pressed.has(Buttons::START) {
                self.menu = Some(0);
                return;
            }
            if self.next_ready() == Some(true) && controls.iter().any(|c| c.pressed.has(Buttons::SOUTH)) {
                if let Err(e) = self.start_new_race() { self.fatal = Some(e); }
                return;
            }
        }
        self.previous = (0..self.race.pilots.len())
            .map(|i| self.race.pose(i))
            .collect();
        let mut inputs = if let Some(drive) = party_inputs {
            drive
        } else {
            controls
                .iter()
                .enumerate()
                .map(|(i, c)| Self::controls(*c, self.devices.get(i).is_some()))
                .collect()
        };
        if self.demo {
            for d in &mut self.race.drivers {
                d.bot = true;
            }
        }
        self.race.player_handling = spaceracer::handling::HandlingGains {
            steering: self.handling.value(0),
            thrust: self.handling.value(2),
            lateral_grip: self.handling.value(3),
            yaw_assist: self.handling.value(4),
            strafe: self.handling.value(5),
        };
        for (i,input) in inputs.iter_mut().enumerate() {
            input.drive.brake=(input.drive.brake*self.handling.value(1)).clamp(0.,1.);
            if let Some(c)=self.cameras.get(i) {
                input.rail_aim=Some(spaceracer::railgun::Aim::camera(c.position,c.forward(),c.up(),(c.fov as f32).to_radians(),self.race.combat.rail_guide_radius));
            }
        }
        self.race.step(&inputs);
        for i in 0..self.previous.len() {
            if self.race.pilots[i].recovery > 0. {
                self.previous[i] = self.race.pose(i);
            }
        }
        if self.race.over {
            if let Some(p) = &mut self.party {
                p.finished();
            }
            self.over_ticks = self.over_ticks.saturating_add(1);
            if self.next.is_none() && !self.demo {
                if let Err(e) = self.preload_next() { self.fatal = Some(e); }
            }
        }
    }
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let frozen = self.menu.is_some() || self.party.as_ref().is_some_and(|p| !p.running());
        let alpha = if frozen { 1. } else { frame.alpha };
        self.visuals.update(&self.race, &self.previous, alpha)?;
        let mut light = self.visuals.light(&self.race);
        light.lod_pixels *= self.live_settings.lod;
        if !self.live_settings.near_shadows {
            light.near_shadows = 0.;
        }
        light.time = (self.race.tick.saturating_sub(1) as f32 + alpha) / 120.;
        if !self.visuals.uploaded {
            frame.set_scene(&self.visuals.scene, &light);
            self.visuals.uploaded = true;
        }
        let renderer = frame.renderer().ok_or("renderer unavailable")?;
        self.visuals
            .slots
            .flush(frame.gpu, renderer, &self.visuals.scene)?;
        self.visuals
            .faces
            .flush(frame.gpu, renderer, &self.visuals.scene)?;
        self.visuals.fx.flush(frame.gpu, renderer, light.time)?;
        self.visuals.jets.flush(frame.gpu, renderer)?;
        self.visuals.stage.flush(frame.gpu, renderer)?;
        renderer.update_aux_words(frame.gpu,self.visuals.vehicle_aux,&self.visuals.scene.aux[self.visuals.vehicle_aux..self.visuals.vehicle_aux+145])?;
        let effect = &self.visuals.crash_fx.config;
        let blur: Vec<_> = self
            .race
            .pilots
            .iter()
            .take(self.players)
            .map(|p| {
                let speed = ((p.speed as f32 - effect.blur_start_speed)
                    / (effect.blur_full_speed - effect.blur_start_speed))
                    .clamp(0., 1.);
                render::motion_blur::Settings {
                    strength: if !self.blur_enabled
                        || !self.live_settings.blur
                        || frozen
                        || p.crashed
                        || p.recovery > 0.
                    {
                        0.
                    } else {
                        speed * effect.blur_strength
                            + if p.boost > 0. { effect.boost_blur } else { 0. }
                    },
                    shutter_seconds: effect.shutter_seconds,
                    delta_seconds: self.visuals.presentation_dt,
                    max_pixels: effect.max_blur_pixels,
                }
            })
            .collect();
        renderer.set_motion_blur(frame.gpu, &blur);

        let mut lights = self.visuals.fx.lights(light.time);
        lights.extend_from_slice(&self.visuals.jets.lights);
        let mut selected = std::collections::HashSet::new();
        for i in 0..self.players {
            let at = self.race.pose(i).origin;
            let mut near: Vec<_> = self
                .visuals
                .lamps
                .iter()
                .enumerate()
                .filter(|(_, l)| l.position.distance_squared(at) < 180. * 180.)
                .collect();
            near.sort_by(|a, b| {
                a.1.position
                    .distance_squared(at)
                    .total_cmp(&b.1.position.distance_squared(at))
            });
            for (id, lamp) in near.into_iter().take(4) {
                if selected.insert(id) {
                    lights.push(*lamp);
                }
            }
        }
        lights.truncate(32);
        renderer
            .set_local_lights(&lights)
            .map_err(|e| format!("{e}: {:?}", lights.iter().find(|l| l.validated().is_err())))?;
        let rects = frame.rects(self.players);
        let mut cams = vec![];
        let mut ups = vec![];
        let mut hud = HudDrawList::new(frame.size);
        let ui_scale = (frame.size[1] as f32 / 900.).max(1.);
        let next_ready = self.next_ready();
        for i in 0..self.players {
            let p = &self.race.pilots[i];
            let current = self.race.pose(i);
            let prev = &self.previous[i];
            let pose = Pose {
                origin: prev.origin.lerp(current.origin, alpha),
                basis: Mat3::from_quat(
                    Quat::from_mat3(&prev.basis).slerp(Quat::from_mat3(&current.basis), alpha),
                ),
            };
            let rect = rects[i].map(|v| v / ui_scale);
            let dt = if frozen { 0. } else { frame.dt as f64 };
            let aspect=rect[2] as f64/rect[3] as f64;
            if !self.respawn[i].update(&self.race,i,alpha,&mut self.cameras[i],aspect,effect.camera) {
                self.cameras[i].update_tuned(pose,p.speed,p.boost>0.||p.on_pad,p.acceleration,dt,false,self.race.countdown<=0.,aspect,effect.camera);
            }
            let c = &self.cameras[i];
            cams.push(Camera {
                pos: c.position,
                target: c.position + c.forward() * 30.,
                fov_y: (c.fov as f32).to_radians(),
                near: 0.15,
            });
            if self.portrait {
                let pose = self.race.pose(i);
                cams[i] = Camera {
                    pos: pose.origin + pose.basis * glam::Vec3::new(11., 3., 2.),
                    target: pose.origin + pose.basis * glam::Vec3::new(0., 0.2, 0.),
                    fov_y: 45f32.to_radians(),
                    near: 0.1,
                };
            }
            if self.weapon_view {
                cams[i]=Camera {pos:pose.origin+pose.basis*glam::Vec3::new(4.2,3.6,-6.2),target:pose.origin+pose.basis*glam::Vec3::new(0.,1.9,-0.8),fov_y:50f32.to_radians(),near:0.1};
            }
            if self.missile_view {
                if let Some(missile) = self.race.projectiles.first() {
                    let node = self.race.world.track.sample(missile.distance);
                    let forward = missile
                        .velocity
                        .try_normalize()
                        .unwrap_or(node.frame(missile.x).z_axis);
                    let right = node.frame(missile.x).y_axis.cross(forward).normalize();
                    let at = missile.previous.lerp(missile.position, alpha);
                    cams[i] = Camera {
                        pos: at + right * 9. + node.frame(missile.x).y_axis * 3. - forward * 7.,
                        target: at,
                        fov_y: 45f32.to_radians(),
                        near: 0.1,
                    };
                }
            }
            let (replay_camera, replay_up) =
                self.replay[i].update(&self.race, i, pose, &cams[i], c.up(), rect[2] / rect[3]);
            cams[i] = replay_camera;
            ups.push(replay_up);
            if p.emp_time <= 0. && self.menu.is_none() {
                race_hud::draw(
                    &mut hud,
                    &self.race,
                    next_ready,
                    i,
                    rect,
                    &cams[i],
                    replay_up,
                    if self.players > 1 { 0.010 } else { 0.035 },
                    self.visuals.jets.config.speed_kmh_per_unit,
                );
                if !p.finished && !self.race.over {
                    self.visuals.faces.draw_hud(&mut hud, i, rect);
                }
            }
        }
        renderer.set_camera_up_axes(&ups);
        if let Some(done) = self.loading_progress() {
            menu_view::loading(&mut hud, frame.size.map(|x| x as f32 / ui_scale), done);
        } else if let Some(row) = self.menu {
            menu_view::draw(&mut hud, frame.size.map(|x| x as f32 / ui_scale), row, self);
        }
        if let Some(error) = self.fatal.as_ref().or(self.race.script_error.as_ref()) {
            let [w, h] = frame.size.map(|x| x as f32 / ui_scale);
            text(
                &mut hud,
                999,
                &format!("PAUSED: {error}"),
                [20., h * 0.5],
                22.,
                [0., 0., w, h],
                [255, 130, 100, 255],
            );
        }
        scale_hud(&mut hud, ui_scale);
        if self.menu.is_none() && self.fatal.is_none() && self.race.script_error.is_none() {
            for i in 0..self.players {
                if self.race.pilots[i].emp_time <= 0. {
                    hud.optics.push(render::hud::Optics {
                        rect: rects[i],
                        curvature: if self.players > 1 { 0.010 } else { 0.035 },
                        glow: 1.,
                        quiet: self.race.pilots[i].finished,
                    });
                }
            }
        }
        Ok(Draw {
            cameras: cams,
            light,
            hud,
        })
    }
    fn state(&self) -> Option<String> {
        Some(json!({"menu_random":{"world":self.random_biome,"difficulty":self.random_difficulty,"seed":self.random_seed},"track_type":spaceracer::generation::Recipe::name_for_seed(self.seed),"active_track_type":self.race.world.centreline.recipe.name,"stage_shadow_cache":self.visuals.scene.stage_sun,"flight_state":self.race.drivers.iter().map(|d|json!({"position":d.motion.air.position,"velocity":d.motion.air.velocity,"frame":d.motion.air.frame.to_cols_array(),"time":d.motion.air.time})).collect::<Vec<_>>(),"weapon_state":self.race.drivers.iter().map(|d|json!({"item":d.item,"active":d.active,"remaining":d.active_time,"shots":d.shot_count,"target":d.sentry_target,"warp_fx":d.warp_fx})).collect::<Vec<_>>(),"projectiles":self.race.projectiles.iter().map(|p|json!({"target":p.target,"locked":p.locked,"terminal":p.terminal,"position":p.position,"age":p.time})).collect::<Vec<_>>(),"victory_age":self.race.drivers.iter().map(|d|d.victory.as_ref().map(|v|v.age)).collect::<Vec<_>>(),"seed":self.seed,"biome":BIOMES[self.biome],"difficulty":DIFFICULTIES[self.difficulty],"players":self.players,"clock":self.race.clock,"countdown":self.race.countdown,"paused":self.menu.is_some(),"pilots":self.race.pilots,"inputs":self.race.drivers.iter().map(|d|d.input).collect::<Vec<_>>(),"stage":self.visuals.stage.counts,"debris":self.visuals.crash_fx.active_pieces(),"face_revisions":self.visuals.faces.revisions,"settings":self.preferences.values,"active_laps":self.race.laps,"pickup_count":self.race.pickups.len(),"pilot_names":self.race.drivers.iter().map(|d|&d.name).collect::<Vec<_>>(),"host_seats":self.party.as_ref().map(|p|p.seats.iter().map(|s|s.index).collect::<Vec<_>>()),"bots":self.race.drivers.iter().map(|d|d.bot).collect::<Vec<_>>(),"live_settings":{"refill":self.race.balance.refill_rate,"recovery":self.race.balance.respawn_seconds},"host_phase":self.party.as_ref().map(|p|p.host.client.state.phase.as_str()),"script_error":self.race.script_error,"error":self.fatal}).to_string())
    }
    fn command(&mut self, cmd: &str) -> Result<String, String> {
        let args: Vec<_> = cmd.split_whitespace().collect();
        match args.as_slice(){
        ["setting",key,value]=>{self.setting(key,serde_json::from_str(value).map_err(|_|"setting JSON")?)?;},
        ["play"]=>{self.menu=None;self.race_started=true;},["pause"]=>self.menu=Some(0),["random"]=>self.random_race()?,
        ["finish",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;if i>=self.race.pilots.len(){return Err("pilot index".into());}self.race.begin_victory(i);},
        ["fire",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;if i>=self.race.pilots.len(){return Err("pilot index".into());}let mut c=vec![Drive::default();self.race.pilots.len()];c[i].fire=true;if let Some(cam)=self.cameras.get(i){c[i].rail_aim=Some(spaceracer::railgun::Aim::camera(cam.position,cam.forward(),cam.up(),(cam.fov as f32).to_radians(),self.race.combat.rail_guide_radius));}let bot=self.race.drivers[i].bot;self.race.drivers[i].bot=false;self.previous=(0..self.race.pilots.len()).map(|i|self.race.pose(i)).collect();self.race.step(&c);self.race.drivers[i].bot=bot;},
        ["give",who,weapon]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let d=self.race.drivers.get_mut(i).ok_or("pilot index")?;d.item=Some(Weapon::parse(weapon).ok_or("unknown weapon")?);},
        ["weapon-review",kind]=>{
            let weapon=Weapon::parse(kind).ok_or("weapon kind")?;self.menu=None;self.race_started=true;self.demo=false;self.race.countdown=0.;self.race.projectiles.clear();self.race.beams.clear();
            for i in 0..self.race.pilots.len(){let p=&mut self.race.pilots[i];p.distance=if i==0{0.}else if i==1{if weapon==Weapon::GlideBomb{230.}else{90.}}else{-500.-i as f64*30.};p.x=if i==1 && weapon==Weapon::GlideBomb {18.}else{0.};p.speed=if i==0{220.}else{130.};p.airborne=false;p.crashed=false;p.finished=false;p.recovery=0.;p.energy=100.;p.emp_time=0.;p.warp_time=0.;let d=&mut self.race.drivers[i];d.bot=false;d.item=None;d.active=None;d.active_time=0.;d.warp_fx=0.;d.victory=None;d.motion=Default::default();d.finish_time=None;d.rail_preview=None;d.rail_preview_at=-1.;}
            self.race.drivers[0].item=Some(weapon);
            if weapon==Weapon::GlideBomb {let node=self.race.world.track.sample(0.);let pose=Pose{origin:node.point(0.,65.),basis:node.geometry.basis()};self.race.drivers[0].motion.ground_velocity=pose.basis.z_axis*220.;self.race.drivers[0].motion.launch(&mut self.race.pilots[0],pose);}
            self.previous=(0..self.race.pilots.len()).map(|i|self.race.pose(i)).collect();self.cameras=vec![Chase::default();4];
        },
        ["weapon-view","on"]=>self.weapon_view=true,["weapon-view","off"]=>self.weapon_view=false,
        ["missile-view","on"]=>self.missile_view=true,["missile-view","off"]=>self.missile_view=false,
        ["energy",who,value]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let v:f64=value.parse().map_err(|_|"energy")?;if !v.is_finite() {return Err("invalid energy".into());}self.race.pilots.get_mut(i).ok_or("pilot index")?.energy=v.clamp(0.,100.);},
        ["reset",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;if i>=self.race.pilots.len(){return Err("pilot index".into());}self.race.reset(i);self.previous[i]=self.race.pose(i);},
        ["place",who,distance]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let distance:f64=distance.parse().map_err(|_|"distance")?;if !distance.is_finite() || distance.abs()>100000. {return Err("invalid distance".into());}let p=self.race.pilots.get_mut(i).ok_or("pilot index")?;p.distance=distance;p.x=0.;self.previous[i]=self.race.pose(i);},
        ["place-recharge",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let at=self.race.world.recharge.first().ok_or("no recharge strip")?[0]+8.;let p=self.race.pilots.get_mut(i).ok_or("pilot index")?;p.distance=at;p.x=-4.;p.speed=60.;self.previous[i]=self.race.pose(i);},
        ["portrait","on"]=>self.portrait=true,["portrait","off"]=>self.portrait=false,
        ["blur","off"]=>self.blur_enabled=false,["blur","on"]=>self.blur_enabled=true,
        ["crash",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;if i>=self.race.pilots.len(){return Err("pilot index".into());}let pose=self.race.pose(i);self.race.drivers[i].motion.air.position=pose.origin;self.race.drivers[i].motion.air.frame=pose.basis;self.race.drivers[i].motion.crash(&mut self.race.pilots[i],glam::Vec3::Y);},
        ["boost",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let p=self.race.pilots.get_mut(i).ok_or("pilot index")?;p.lap=2;p.boost=3.;},
        ["boost",who,seconds]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let duration:f64=seconds.parse().map_err(|_|"boost seconds")?;if !duration.is_finite() || !(0.0..=10.).contains(&duration) {return Err("boost seconds must be 0..10".into());}let p=self.race.pilots.get_mut(i).ok_or("pilot index")?;p.lap=2;p.boost=duration;},
        ["demo"]=>{self.demo=true;self.menu=None;},["help"]=>return Ok("play; pause; random; demo; give INDEX missile|warp|sentry|emp|railgun|glidebomb; boost INDEX; crash INDEX; blur on|off".into()),_=>return Err("unknown command".into())}
        Ok("ok".into())
    }
}
fn text(
    hud: &mut HudDrawList,
    id: u64,
    s: &str,
    position: [f32; 2],
    font_size: f32,
    bounds: [f32; 4],
    color: [u8; 4],
) {
    hud.outline(id, 0.7, [3, 9, 17, 180]);
    hud.text.push(TextRun {
        id,
        text: s.into(),
        position,
        bounds,
        font_size,
        color,
        font: race_hud::font(),
        align: TextAlign::Left,
        width: bounds[2],
    });
}
fn scale_hud(hud: &mut HudDrawList, scale: f32) {
    for v in &mut hud.vertices {
        v.position = v.position.map(|x| x * scale);
        v.clip = v.clip.map(|x| x * scale);
        if v.shape[3] == 1. {
            for x in &mut v.shape[..3] {
                *x *= scale;
            }
        }
        if v.shape[3] == 2. {
            for x in &mut v.shape[..2] {
                *x *= scale;
            }
        }
        // Rings: centre, radius and the half width packed above 4.
        if v.shape[3] >= 4. {
            for x in &mut v.shape[..3] {
                *x *= scale;
            }
            v.shape[3] = 4. + (v.shape[3] - 4.) * scale;
        }
    }
    for t in &mut hud.text {
        t.position = t.position.map(|x| x * scale);
        t.bounds = t.bounds.map(|x| x * scale);
        t.font_size *= scale;
        t.width *= scale;
    }
    for o in hud.outlines.values_mut() {
        o.width *= scale;
    }
}
fn menu_entropy() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish() as u32
}
/// SpaceRacer's vehicle surface and mask shaders, before any renderer exists.
pub fn install_shaders(no_reflections: bool) {
    let mut surface = include_str!("../shaders/surfaces.wgsl").to_owned();
    // Diagnostic omission only; never persisted as a player quality setting.
    if no_reflections {
        surface = surface.replace("if aux_len()<8u", "if true || aux_len()<8u");
    }
    render::set_game_surface(&format!("{}\n{}",include_str!("../shaders/vehicle_data.wgsl"),surface));
    render::set_game_mask(&format!("{}\n{}",include_str!("../shaders/vehicle_data.wgsl"),include_str!("../shaders/vehicle_mask.wgsl")));
}
/// The game folder (rules/, assets/): `--game-dir`, else beside the exe, else the source tree.
fn game_root(dir: Option<String>) -> PathBuf {
    dir.map(PathBuf::from).unwrap_or_else(|| {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("spaceracer")));
        beside
            .filter(|p| p.join("rules/game.luau").is_file())
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
    })
}
/// The desktop game and its command-line tools.
pub fn run() -> Result<(), String> {
    app::startup::init("SpaceRacer", Some(include_bytes!("../../assets/source/assets/gamenight/icon.png")));
    install_shaders(std::env::args().any(|a| a == "--profile-no-reflections"));
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str| args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone());
    if args.get(1).is_some_and(|a| a == "xr-shot") {
        return xr::shot(&args);
    }
    let root = game_root(arg("--game-dir"));
    let players = arg("--players")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 4);
    let biome = arg("--biome")
        .and_then(|s| BIOMES.iter().position(|b| *b == s))
        .unwrap_or(0);
    let difficulty = arg("--difficulty")
        .and_then(|s| DIFFICULTIES.iter().position(|b| *b == s))
        .unwrap_or(1);
    let demo = args.iter().any(|s| s == "--demo");
    let mut game = Racer::new(
        root,
        players,
        arg("--seed").and_then(|s| s.parse().ok()).unwrap_or(35),
        biome,
        difficulty,
        demo,
    )?;
    // Explicit launch parameters and scripted fixtures remain reproducible.
    // A regular local launch defaults to Random; player count is never random.
    let random_defaults=args.iter().any(|s|s=="--random-settings") ||
        !["--script","--shot","--bench","--integration-probe"].iter().any(|key|arg(key).is_some());
    if game.party.is_none() && !demo && random_defaults {
        game.random_biome=arg("--biome").is_none();
        game.random_difficulty=arg("--difficulty").is_none();
        game.random_seed=arg("--seed").is_none();
    }
    game.blur_enabled = !args.iter().any(|s| s == "--no-motion-blur");
    // Diagnostic omission, never a persisted player quality setting.
    game.visuals.jets.volume_plumes = !args.iter().any(|s|s=="--profile-no-plumes");
    if let Some(dir) = arg("--integration-probe") {
        return integration_probe::run(&mut game, std::path::Path::new(&dir));
    }
    if let Some(frames) = arg("--bench") {
        let frames: u32 = frames.parse().map_err(|_| "--bench needs a frame count")?;
        if frames < 30 || frames > 10000 {
            return Err("--bench frames must be 30..10000".into());
        }
        let ticks = arg("--ticks")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(750);
        for _ in 0..ticks {
            game.tick(&vec![Controls::default(); players]);
        }
        let size = [
            arg("--width").and_then(|s| s.parse().ok()).unwrap_or(1920),
            arg("--height").and_then(|s| s.parse().ok()).unwrap_or(1080),
        ];
        let gpu = render::Gpu::new()?;
        let warmup = arg("--warmup").and_then(|s| s.parse().ok()).unwrap_or(60);
        let measurements = app::profile::bench(&mut game, &gpu, size, warmup, frames, 120.)?;
        let environment: std::collections::BTreeMap<_,_> = ["WGPU_BACKEND","WGPU_DEBUG","WGPU_VALIDATION","WGPU_VALIDATION_INDIRECT_CALL","VK_LOADER_LAYERS_DISABLE","KERN_CPU_SPANS","KERN_SHADE_FEATURES"].into_iter().map(|name| (name,std::env::var(name).ok())).collect();
        let report = json!({"gpu":{"name":gpu.summary().name,"backend":gpu.summary().backend,"driver":gpu.summary().driver},"size":size,"players":players,"motion_blur":game.blur_enabled,"frames":frames,"profile":measurements,"scene":game.visuals.scene.stats(),
            "scenario":{"seed":game.seed,"biome":BIOMES[game.biome],"difficulty":DIFFICULTIES[game.difficulty],"players":players,"ticks":ticks,"warmup":warmup,"frames":frames,"size":size},
            "diagnostics":{"no_reflections":args.iter().any(|a|a=="--profile-no-reflections"),"no_plumes":!game.visuals.jets.volume_plumes,"debug_assertions":cfg!(debug_assertions),"environment":environment}});
        let report = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        if let Some(path) = arg("--out") {
            println!("{}", serde_json::to_string_pretty(&measurements["summary"]).unwrap());
            std::fs::write(path, report).map_err(|e| e.to_string())?;
        } else {
            println!("{report}");
        }
        return Ok(());
    }
    if let Some(path) = arg("--shot") {
        let ticks = arg("--ticks")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(600);
        for _ in 0..ticks {
            game.tick(&vec![Controls::default(); players]);
        }
        let size = [
            arg("--width").and_then(|s| s.parse().ok()).unwrap_or(1280),
            arg("--height").and_then(|s| s.parse().ok()).unwrap_or(720),
        ];
        let gpu = render::Gpu::new()?;
        let mut h = app::Headless::new(&gpu, size, 120.)?;
        let mut rgba = vec![];
        for _ in 0..8 {
            rgba = h.frame(&mut game)?;
        }
        render::write_png(std::path::Path::new(&path), size[0], size[1], &rgba)?;
        std::fs::write(format!("{path}.json"), game.state().unwrap()).map_err(|e| e.to_string())?;
        return Ok(());
    }
    if let Some(script) = arg("--script") {
        let gpu = render::Gpu::new()?;
        let out = arg("--out").unwrap_or("build/spaceracer-script".into());
        let failed =
            app::script::run_file(&mut game, &gpu, 120., script.as_ref(), out.as_ref(), None)?;
        if failed > 0 {
            return Err(format!("{failed} script assertions failed"));
        }
        return Ok(());
    }
    app::run(
        app::Config {
            title: "SpaceRacer / Warp".into(),
            tick_hz: 120.,
            fullscreen: app::startup::fullscreen(&args),
            ..Default::default()
        },
        &mut game,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_race_uses_selection_and_applies_same_tick_seed_change() {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut game=Racer::new(root,2,31,0,0,false).unwrap();
        assert_eq!(game.menu,Some(1));
        game.menu=Some(5);
        game.menu_tick(Controls{pressed:Buttons(Buttons::RIGHT|Buttons::START),..Default::default()});
        assert!(game.fatal.is_none(),"{:?}",game.fatal);
        assert_eq!(game.seed,32);assert_eq!(game.players,2);assert_eq!(game.biome,0);assert_eq!(game.difficulty,0);
        assert_eq!(game.race.world.centreline.recipe.name,"Underpass");assert!(game.menu.is_none());
        game.race.clock=20.;game.menu=Some(1);
        game.menu_tick(Controls{pressed:Buttons(Buttons::SOUTH),..Default::default()});
        assert!(game.fatal.is_none(),"{:?}",game.fatal);
        assert_eq!(game.seed,32);assert_eq!(game.race.clock,0.);assert_eq!(game.race.world.centreline.recipe.name,"Underpass");
        game.random_seed=true;game.random_difficulty=true;
        game.race.clock=8.;game.menu=Some(0);
        game.menu_tick(Controls{pressed:Buttons(Buttons::START),..Default::default()});
        assert_eq!(game.race.clock,8.);assert_eq!(game.seed,32);assert!(game.menu.is_none());
        game.menu=Some(1);
        game.menu_tick(Controls{pressed:Buttons(Buttons::SOUTH),..Default::default()});
        assert!(game.fatal.is_none(),"{:?}",game.fatal);
        assert!((1..=99999).contains(&game.seed));assert!(game.difficulty<3);
        assert_eq!(game.biome,0);assert_eq!(game.players,2);
        assert!(game.random_seed && game.random_difficulty && !game.random_biome);
        assert_eq!(game.race.world.centreline.difficulty,DIFFICULTIES[game.difficulty]);
    }
    #[test]
    fn a_background_race_shows_progress_then_starts() {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut game=Racer::new(root,1,31,0,0,false).unwrap();
        game.background_loading=true;
        let old_seed=game.seed;
        game.menu=Some(5);
        game.menu_tick(Controls{pressed:Buttons(Buttons::RIGHT|Buttons::START),..Default::default()});
        assert!(game.loading_progress().is_some(),"loading screen while building");
        assert!(game.menu.is_none());
        let start=std::time::Instant::now();
        while game.loading.is_some() {
            assert!(start.elapsed().as_secs()<120,"race build finished");
            std::thread::sleep(std::time::Duration::from_millis(20));
            game.tick(&[Controls::default()]);
        }
        assert!(game.fatal.is_none(),"{:?}",game.fatal);
        assert_eq!(game.seed,old_seed+1);
        assert!(game.race_started && !game.visuals.uploaded);
    }
    #[test]
    fn the_next_race_preloads_behind_the_results_and_a_starts_it() {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut game=Racer::new(root,1,31,0,0,false).unwrap();
        game.menu=None;game.race_started=true;game.random_biome=true;
        game.race.over=true;
        let south=Controls{pressed:Buttons(Buttons::SOUTH),..Default::default()};
        game.tick(&[south]);
        assert!(game.next.is_some(),"next race builds as soon as the race ends");
        assert_eq!(game.next_ready(),Some(false),"A waits for a short beat on the results");
        assert!(game.race.over,"an early A does not skip the results");
        let start=std::time::Instant::now();
        while game.next_ready()!=Some(true) {
            assert!(start.elapsed().as_secs()<120,"next race built");
            game.tick(&[Controls::default()]);
        }
        let biome=game.biome;
        game.tick(&[south]);
        assert!(game.fatal.is_none(),"{:?}",game.fatal);
        assert!(!game.race.over && game.loading.is_none() && game.next.is_none(),"A starts the preloaded race at once");
        assert_eq!(game.race.biome,BIOMES[biome]);
    }
    #[test]
    fn the_pointer_hovers_and_clicks_menu_rows() {
        let root=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut game=Racer::new(root,1,31,0,0,false).unwrap();
        // Row 3 (World) sits at y 180+2*66 before a race; its right third steps right.
        assert!(game.menu_pointer([420.,340.],[1280,720],false));
        assert_eq!(game.menu,Some(3));
        let biome=game.biome;
        game.random_biome=false;
        game.menu_pointer([420.,340.],[1280,720],true);
        game.menu_tick(Controls::default());
        assert_eq!(game.biome,(biome+1)%5);
        assert!(!game.menu_pointer([1200.,700.],[1280,720],true),"outside the panel");
    }
    #[test]
    fn city_lights_remain_valid_during_race() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut game = Racer::new(root, 1, 35, 0, 1, true).unwrap();
        for tick in 0..2400 {
            game.tick(&[]);
            game.visuals
                .update(&game.race, &game.previous, 0.37)
                .unwrap();
            let time = (game.race.tick.saturating_sub(1) as f32 + 0.37) / 120.;
            for l in game
                .visuals
                .fx
                .lights(time)
                .iter()
                .chain(&game.visuals.jets.lights)
                .chain(&game.visuals.lamps)
            {
                assert!(l.validated().is_ok(), "invalid light at {tick}: {l:?}");
            }
        }
    }
    #[test]
    fn analog_flight_bindings_preserve_axes_and_trigger_pressure() {
        let c = Controls {
            mv: [0.7, -0.2],
            aim: [-0.3, -0.5],
            triggers: [0.25, 0.75],
            held: Buttons(Buttons::LEFT_SHOULDER),
            ..Default::default()
        };
        let d = Racer::controls(c, true);
        assert!((d.drive.throttle - 0.75).abs() < 1e-6);
        assert_eq!(d.drive.brake, 0.25);
        assert!((d.drive.trim + 0.7).abs() < 1e-6);
        assert!((d.drive.strafe + 0.3).abs() < 1e-6);
        assert!(d.left);
        assert!(!d.fire);
    }
}
