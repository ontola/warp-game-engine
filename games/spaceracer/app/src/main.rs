mod party;
mod visuals;
use app::{Buttons, Controls, Device, Draw, Frame, Game};
use glam::{Mat3, Quat, Vec3};
use render::{
    Camera,
    headlights::LocalLight,
    hud::{FontFace, HudDrawList, TextAlign, TextRun},
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
    race: Race,
    visuals: visuals::Visuals,
    root: PathBuf,
    players: usize,
    seed: i64,
    difficulty: usize,
    biome: usize,
    menu: Option<usize>,
    settings_dirty: bool,
    menu_axis: [i32; 2],
    cameras: Vec<Chase>,
    previous: Vec<Pose>,
    devices: Vec<Device>,
    party: Option<party::Party>,
    fatal: Option<String>,
    ready: bool,
    demo: bool,
    reload_ticks: u32,
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
            3,
            rules,
        )?;
        let visuals = visuals::Visuals::new(&race, &root.join("assets"))?;
        race.obstacles = visuals.obstacles.clone();
        let previous = (0..race.pilots.len()).map(|i| race.pose(i)).collect();
        let party = party::Party::connect()?;
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
                Some(0)
            },
            menu_axis: [0; 2],
            settings_dirty: false,
            cameras: vec![Chase::default(); 4],
            previous,
            devices: vec![],
            party,
            fatal: None,
            ready: false,
            demo,
            reload_ticks: 0,
        })
    }
    fn restart(&mut self) -> Result<(), String> {
        let rules = Rules::new(Some(&self.root.join("rules")))?;
        let mut race = Race::with_rules(
            self.seed,
            DIFFICULTIES[self.difficulty],
            BIOMES[self.biome],
            self.players,
            3,
            rules,
        )?;
        if let Some(p) = &self.party {
            for (i, seat) in p.seats.iter().enumerate().take(6) {
                race.drivers[i].name = seat.name.clone();
                race.drivers[i].bot = seat.ai;
            }
        }
        self.visuals = visuals::Visuals::new(&race, &self.root.join("assets"))?;
        race.obstacles = self.visuals.obstacles.clone();
        self.previous = (0..race.pilots.len()).map(|i| race.pose(i)).collect();
        self.race = race;
        self.cameras = vec![Chase::default(); 4];
        self.menu = None;
        self.settings_dirty = false;
        Ok(())
    }
    fn random_race(&mut self) -> Result<(), String> {
        self.seed = (self.seed * 48271) % 99999 + 1;
        self.biome = (self.seed as usize / 7) % 5;
        self.restart()
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
        }
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
        let horizontal = c.pressed.has(Buttons::RIGHT) as i32 - c.pressed.has(Buttons::LEFT) as i32
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
        let out=self.race.rules.call::<serde_json::Value>("menu",json!({"players":self.players,"biome":self.biome,"difficulty":self.difficulty,"seed":self.seed,"row":row,"vertical":vertical,"horizontal":horizontal,"accept":c.pressed.has(Buttons::SOUTH),"start":c.pressed.has(Buttons::START)}));
        let result = (|| -> Result<(), String> {
            let out = out?;
            let row = out["row"].as_u64().ok_or("menu row")? as usize;
            self.menu = Some(row);
            match out["action"].as_str().unwrap_or("none") {
                "play" => {
                    if self.race.over || self.settings_dirty {
                        self.restart()?;
                    } else {
                        self.menu = None;
                    }
                }
                "random" => self.random_race()?,
                _ => {}
            }
            if out["rebuild"].as_bool() == Some(true) {
                self.players = out["players"]
                    .as_u64()
                    .ok_or("menu player count")?
                    .clamp(1, 4) as usize;
                self.biome = out["biome"].as_u64().ok_or("menu biome")?.min(4) as usize;
                self.difficulty =
                    out["difficulty"].as_u64().ok_or("menu difficulty")?.min(2) as usize;
                self.seed = out["seed"].as_i64().ok_or("menu seed")?.clamp(1, 99999);
                self.settings_dirty = true;
                self.menu = Some(row);
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.fatal = Some(e);
        }
    }
}
impl Game for Racer {
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
    fn escape(&mut self) -> bool {
        if let Some(p) = &self.party {
            p.overlay();
        } else {
            if self.menu.is_some() && self.settings_dirty {
                if let Err(e) = self.restart() {
                    self.fatal = Some(e);
                }
            } else {
                self.menu = if self.menu.is_some() { None } else { Some(0) };
            }
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
            p.client.loading(percent, step);
        }
    }
    fn tick(&mut self, controls: &[Controls]) {
        if self.fatal.is_some() {
            return;
        }
        self.reload_ticks += 1;
        if self.reload_ticks % 60 == 0 {
            self.race.reload();
        }
        if self.ready && self.visuals.uploaded {
            if let Some(p) = &mut self.party {
                p.ready();
            }
            self.ready = false;
        }
        if let Some(mut p) = self.party.take() {
            let events = p.poll();
            self.party = Some(p);
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
                        self.ready = true;
                        if let Err(e) = self.restart() {
                            self.fatal = Some(e);
                        }
                    }
                    party::Event::Roster => {
                        if let Some(p) = &self.party {
                            for (i, seat) in p.seats.iter().enumerate().take(6) {
                                self.race.drivers[i].name = seat.name.clone();
                                self.race.drivers[i].bot = seat.ai;
                            }
                        }
                    }
                    party::Event::Random => {
                        if let Err(e) = self.random_race() {
                            self.fatal = Some(e);
                        }
                    }
                    party::Event::Setting(key, value) => {
                        if key == "seed" {
                            if let Some(v) = value.as_i64() {
                                self.seed = v.clamp(1, 99999);
                            }
                        }
                        if key == "difficulty" {
                            if let Some(i) =
                                DIFFICULTIES.iter().position(|s| Some(*s) == value.as_str())
                            {
                                self.difficulty = i;
                            }
                        }
                        if key == "biome" {
                            if let Some(i) = BIOMES.iter().position(|s| Some(*s) == value.as_str())
                            {
                                self.biome = i;
                            }
                        }
                    }
                }
            }
            if !self.party.as_ref().unwrap().running {
                return;
            }
        }
        if self.party.is_none() {
            let c = controls.first().copied().unwrap_or_default();
            if self.menu.is_some() {
                self.menu_tick(c);
                return;
            }
            if c.pressed.has(Buttons::START) {
                self.menu = Some(0);
                return;
            }
        }
        self.previous = (0..self.race.pilots.len())
            .map(|i| self.race.pose(i))
            .collect();
        let inputs = if let Some(p) = &mut self.party {
            p.controls()
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
        }
    }
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let frozen = self.menu.is_some()
            || self.race.over
            || self.party.as_ref().is_some_and(|p| !p.running);
        let alpha = if frozen { 1. } else { frame.alpha };
        self.visuals.update(&self.race, &self.previous, alpha);
        let light = self.visuals.light(&self.race);
        if !self.visuals.uploaded {
            frame.set_scene(&self.visuals.scene, &light);
            self.visuals.uploaded = true;
        }
        let renderer = frame.renderer().ok_or("renderer unavailable")?;
        self.visuals
            .slots
            .flush(frame.gpu, renderer, &self.visuals.scene)?;
        self.visuals
            .fx
            .flush(frame.gpu, renderer, self.race.tick as f32 / 120.)?;
        let mut lights = self.visuals.fx.lights(self.race.tick as f32 / 120.);
        for i in 0..self.race.pilots.len() {
            let p = &self.race.pilots[i];
            if p.crashed || p.emp_time > 0. {
                continue;
            }
            let current = self.race.pose(i);
            let previous = &self.previous[i];
            let pose = Pose {
                origin: previous.origin.lerp(current.origin, alpha),
                basis: Mat3::from_quat(
                    Quat::from_mat3(&previous.basis).slerp(Quat::from_mat3(&current.basis), alpha),
                ),
            };
            let power = self.race.drivers[i].input.drive.throttle as f32;
            for side in [-1., 1.] {
                lights.push(LocalLight {
                    position: pose.origin + pose.basis * Vec3::new(side * 2.45, 0.3, -4.),
                    direction: -pose.basis.z_axis,
                    intensity_cd: Vec3::new(0.18, 0.5, 1.)
                        * (80. + power * 280. + if p.boost > 0. || p.on_pad { 1400. } else { 0. }),
                    range_m: 18.,
                    inner_cos: 1.,
                    outer_cos: -1.,
                    source_radius_m: 0.6,
                    falloff: 2.,
                });
            }
        }
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
        renderer.set_local_lights(&lights)?;
        let rects = frame.rects(self.players);
        let mut cams = vec![];
        let mut ups = vec![];
        let mut hud = HudDrawList::new(frame.size);
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
            let rect = rects[i];
            let dt = if frozen { 0. } else { frame.dt as f64 };
            let camera_pose = if p.crashed {
                let node = self.race.world.track.sample(
                    self.race
                        .world
                        .safe_respawn(self.race.drivers[i].safe_distance),
                );
                let t = (self.race.drivers[i].crash_time / 1.8).clamp(0., 1.) as f32;
                Pose {
                    origin: pose.origin.lerp(node.point(0., 1.3), t * t * (3. - 2. * t)),
                    basis: node.geometry.basis(),
                }
            } else {
                pose
            };
            self.cameras[i].update(
                camera_pose,
                p.speed,
                p.boost > 0. || p.on_pad,
                p.acceleration,
                dt,
                false,
                self.race.countdown <= 0.,
                rect[2] as f64 / rect[3] as f64,
            );
            let c = &self.cameras[i];
            cams.push(Camera {
                pos: c.position,
                target: c.position + c.forward() * 30.,
                fov_y: (c.fov as f32).to_radians(),
                near: 0.15,
            });
            ups.push(c.up());
            if p.emp_time <= 0. && self.menu.is_none() {
                text(
                    &mut hud,
                    100 + i as u64 * 20,
                    &self.race.drivers[i].name,
                    [rect[0] + 24., rect[1] + 20.],
                    22.,
                    rect,
                    [175, 233, 255, 220],
                );
                text(
                    &mut hud,
                    101 + i as u64 * 20,
                    &format!("LAP {}/{}", p.lap.min(self.race.laps), self.race.laps),
                    [rect[0] + 24., rect[1] + 50.],
                    16.,
                    rect,
                    [180, 205, 220, 200],
                );
                text(
                    &mut hud,
                    102 + i as u64 * 20,
                    &format!("{:04} km/h", (p.speed * 3.6) as i32),
                    [rect[0] + rect[2] - 235., rect[1] + rect[3] - 85.],
                    27.,
                    rect,
                    [225, 245, 255, 230],
                );
                text(
                    &mut hud,
                    103 + i as u64 * 20,
                    &format!("{} / 6", self.race.drivers[i].rank),
                    [rect[0] + rect[2] - 125., rect[1] + 25.],
                    30.,
                    rect,
                    [230, 240, 255, 230],
                );
                hud.rect(
                    [rect[0] + rect[2] - 235., rect[1] + rect[3] - 40., 210., 5.],
                    [0.08, 0.2, 0.3, 0.7],
                    rect,
                );
                hud.rect(
                    [
                        rect[0] + rect[2] - 235.,
                        rect[1] + rect[3] - 40.,
                        210. * p.energy as f32 / 100.,
                        5.,
                    ],
                    [0.25, 0.9, 1., 0.85],
                    rect,
                );
                if let Some(item) = self.race.drivers[i].item {
                    text(
                        &mut hud,
                        104 + i as u64 * 20,
                        &format!("(X) {}", item.name()),
                        [rect[0] + rect[2] * 0.32, rect[1] + rect[3] - 65.],
                        18.,
                        rect,
                        [230, 220, 255, 230],
                    );
                }
                if let Some(item) = self.race.drivers[i].active {
                    text(
                        &mut hud,
                        105 + i as u64 * 20,
                        &format!("{} ACTIVE", item.name()),
                        [rect[0] + 24., rect[1] + rect[3] - 50.],
                        15.,
                        rect,
                        [150, 225, 255, 210],
                    );
                }
                if self.race.drivers[i].checkpoint.missed {
                    text(
                        &mut hud,
                        106 + i as u64 * 20,
                        "CHECKPOINT MISSED   (Y)",
                        [rect[0] + rect[2] * 0.2, rect[1] + rect[3] * 0.58],
                        26.,
                        rect,
                        [255, 130, 85, 240],
                    );
                } else if p.airborne {
                    text(
                        &mut hud,
                        106 + i as u64 * 20,
                        "(Y) RESET",
                        [rect[0] + rect[2] * 0.4, rect[1] + rect[3] * 0.75],
                        18.,
                        rect,
                        [200, 225, 245, 220],
                    );
                }
                if self.race.countdown > 0. {
                    text(
                        &mut hud,
                        107 + i as u64 * 20,
                        &format!("{}", self.race.countdown.ceil() as i32),
                        [rect[0] + rect[2] * 0.48, rect[1] + rect[3] * 0.35],
                        72.,
                        rect,
                        [220, 250, 255, 220],
                    );
                }
                if let Some((owner, target, at)) = self.race.destroyed {
                    if owner == i && self.race.clock - at < 0.5 {
                        text(
                            &mut hud,
                            111 + i as u64 * 20,
                            &format!("DESTROYED {}", self.race.drivers[target].name),
                            [rect[0] + rect[2] * 0.35, rect[1] + rect[3] * 0.4],
                            24.,
                            rect,
                            [255, 195, 105, 240],
                        );
                    }
                }
                if let Some(item) = self.race.drivers[i].item {
                    let target = match item {
                        Weapon::Railgun => self.race.target(i, 0.975, 700.),
                        Weapon::GlideBomb if p.airborne => self.race.target(i, 0.7, 1000.),
                        _ => None,
                    };
                    if let Some(j) = target {
                        let vp = glam::Mat4::perspective_rh(
                            (c.fov as f32).to_radians(),
                            rect[2] / rect[3],
                            0.15,
                            10000.,
                        ) * glam::Mat4::look_at_rh(
                            c.position,
                            c.position + c.forward(),
                            c.up(),
                        );
                        let h = vp * self.race.pose(j).origin.extend(1.);
                        if h.w > 0. {
                            let ndc = h.truncate() / h.w;
                            let x = rect[0] + (ndc.x + 1.) * 0.5 * rect[2];
                            let y = rect[1] + (1. - ndc.y) * 0.5 * rect[3];
                            for sx in [-1., 1.] {
                                for sy in [-1., 1.] {
                                    let start = [x + sx * 25., y + sy * 20.];
                                    hud.line(
                                        start,
                                        [start[0] - sx * 10., start[1]],
                                        2.,
                                        [1., 0.4, 0.15, 0.85],
                                        rect,
                                    );
                                    hud.line(
                                        start,
                                        [start[0], start[1] - sy * 8.],
                                        2.,
                                        [1., 0.4, 0.15, 0.85],
                                        rect,
                                    );
                                }
                            }
                        }
                    }
                }
                if p.finished {
                    text(
                        &mut hud,
                        108 + i as u64 * 20,
                        if self.race.drivers[i].rank == 1 {
                            "VICTORY"
                        } else {
                            "FINISHED"
                        },
                        [rect[0] + rect[2] * 0.32, rect[1] + rect[3] * 0.32],
                        48.,
                        rect,
                        [220, 245, 255, 240],
                    );
                }
            }
        }
        renderer.set_camera_up_axes(&ups);
        if let Some(row) = self.menu {
            let [w, h] = frame.size.map(|x| x as f32);
            let clip = [0., 0., w, h];
            hud.rect(clip, [0.015, 0.025, 0.05, 0.8], clip);
            text(
                &mut hud,
                1,
                "SPACERACER",
                [w * 0.12, h * 0.12],
                50.,
                clip,
                [210, 240, 255, 240],
            );
            text(
                &mut hud,
                2,
                "WARP ENGINE  /  DEVELOPMENT BUILD",
                [w * 0.12, h * 0.12 + 65.],
                15.,
                clip,
                [95, 170, 200, 220],
            );
            let rows = [
                if self.race.clock > 0. {
                    "RESUME RACE".into()
                } else {
                    "START RACE".into()
                },
                "NEW RANDOM RACE".into(),
                format!("PLAYERS                 {}", self.players),
                format!(
                    "WORLD                    {}",
                    BIOMES[self.biome].to_uppercase()
                ),
                format!(
                    "DIFFICULTY             {}",
                    DIFFICULTIES[self.difficulty].to_uppercase()
                ),
                format!("TRACK SEED           {:05}", self.seed),
            ];
            for (i, label) in rows.iter().enumerate() {
                let y = h * 0.35 + i as f32 * 42.;
                if i == row {
                    hud.rect(
                        [w * 0.12 - 12., y - 5., w * 0.68, 38.],
                        [0.1, 0.35, 0.45, 0.45],
                        clip,
                    );
                }
                text(
                    &mut hud,
                    10 + i as u64,
                    label,
                    [w * 0.12, y],
                    22.,
                    clip,
                    if i == row {
                        [210, 250, 255, 255]
                    } else {
                        [140, 165, 185, 210]
                    },
                );
            }
            text(
                &mut hud,
                30,
                "UP / DOWN SELECT    LEFT / RIGHT CHANGE    START TO RACE",
                [w * 0.12, h - 46.],
                14.,
                clip,
                [120, 160, 180, 210],
            );
        }
        if let Some(error) = self.fatal.as_ref().or(self.race.script_error.as_ref()) {
            let [w, h] = frame.size.map(|x| x as f32);
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
        Ok(Draw {
            cameras: cams,
            light,
            hud,
        })
    }
    fn state(&self) -> Option<String> {
        Some(json!({"seed":self.seed,"biome":BIOMES[self.biome],"difficulty":DIFFICULTIES[self.difficulty],"players":self.players,"clock":self.race.clock,"countdown":self.race.countdown,"paused":self.menu.is_some(),"pilots":self.race.pilots,"inputs":self.race.drivers.iter().map(|d|d.input).collect::<Vec<_>>(),"script_error":self.race.script_error,"error":self.fatal}).to_string())
    }
    fn command(&mut self, cmd: &str) -> Result<String, String> {
        let args: Vec<_> = cmd.split_whitespace().collect();
        match args.as_slice(){
        ["play"]=>self.menu=None,["pause"]=>self.menu=Some(0),["random"]=>self.random_race()?,
        ["give",who,weapon]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let d=self.race.drivers.get_mut(i).ok_or("pilot index")?;d.item=Some(Weapon::parse(weapon).ok_or("unknown weapon")?);},
        ["boost",who]=>{let i:usize=who.parse().map_err(|_|"pilot index")?;let p=self.race.pilots.get_mut(i).ok_or("pilot index")?;p.lap=2;p.boost=3.;},
        ["demo"]=>{self.demo=true;self.menu=None;},["help"]=>return Ok("play; pause; random; demo; give INDEX missile|warp|sentry|emp|railgun|glidebomb; boost INDEX".into()),_=>return Err("unknown command".into())}
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
    hud.text.push(TextRun {
        id,
        text: s.into(),
        position,
        bounds,
        font_size,
        color,
        font: FontFace::Body,
        align: TextAlign::Left,
        width: bounds[2],
    });
}
fn main() -> Result<(), String> {
    render::set_game_surface(include_str!("../shaders/surfaces.wgsl"));
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str| args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone());
    let root = arg("--game-dir").map(PathBuf::from).unwrap_or_else(|| {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join("spaceracer")));
        beside
            .filter(|p| p.join("rules/game.luau").is_file())
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
    });
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
        let (fps, passes) = app::bench(&mut game, &gpu, size, frames, 120.)?;
        let report = json!({"gpu":{"name":gpu.summary().name,"backend":gpu.summary().backend,"driver":gpu.summary().driver},"size":size,"players":players,"frames":frames,"readback_fps":fps,"gpu_pass_ms_smoothed":passes,"scene":game.visuals.scene.stats(),"note":"Headless readback throughput is not interactive FPS; GPU times are smoothed pass samples, not latency percentiles."});
        let report = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        println!("{report}");
        if let Some(path) = arg("--out") {
            std::fs::write(path, report).map_err(|e| e.to_string())?;
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
            fullscreen: args.iter().any(|s| s == "--fullscreen"),
            ..Default::default()
        },
        &mut game,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
