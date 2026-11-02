//! Fly over the procedural terrain: a debug game for looking at the world
//! the way a player would, at any height and speed.
//!
//!   cargo run --release -p app --example terrain_fly
//!   cargo run --release -p app --example terrain_fly -- --style alpine --seed 3
//!   cargo run --release -p app --example terrain_fly -- --track   (a gravel road the land bends to)
//!   cargo run --release -p app --example terrain_fly -- --shot fly.png
//!   cargo run --release -p app --example terrain_fly -- --fullscreen --frames 600   # fps benchmark
//!   ... --no-trees --grass --recipe N   (N nearest trees in full, default 60; then card trees, then voxel proxies)
//!   ... --firs   (the old forest: Sideways' fir cards, for A/B runs)   --proxy-px N   (proxy cell pixels, default 4)
//!   cargo run --release -p app --example terrain_fly -- --bench 120 [--size 2560x1440]   # headless GPU pass times
//!
//! Mouse or right stick looks, WASD or left stick flies, Space or A climbs,
//! Q or LB sinks, Shift/E or RB goes fast, the wheel or arrows up/down set
//! the speed. F1 default, F2 lowland, F4 alpine (F10 opens the debug
//! overlay), F5 the next seed, F6 jumps somewhere new, F7 hugs the ground,
//! F12 saves a screenshot with its position and terrain stats for feedback,
//! F10 opens the debug menu (engine render/light switches plus trees, grass
//! and terrain LOD under "Terrain fly").
//!
//! Grass (render's meadow) and trees (one recipe spruce, three seeds) cover a
//! disc around the camera; the scene is rebuilt on another thread as it moves.
//! The nearest spruces are full assemblies (branch wind, per-branch source or
//! voxels); every other one is the same cooked plant as one voxel proxy.
use app::{Buttons, Controls, Draw, Frame, Game, Pointer};
use glam::{Affine3A, Quat, Vec2, Vec3};
use render::cluster::{ClusterMesh, Part, SimplificationPolicy};
use render::debug_ui::{DebugAction, DebugControl, DebugMenu, DebugValue};
use render::scene::{GeometryCategory, ALPHA_TEST, TWO_SIDED};
use render::recipe_forest;
use render::vegetation_asset::CookedPlant;
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::procedural_terrain::{self, ProceduralTerrain, Settings, Stats, TerrainView};
use render::scene::{Material, SURFACE, TERRAIN};
use render::terrain_field::{self, Style};
use render::world_constraints::{Constraints as WorldConstraints, Track, World as Constraints};
use std::sync::Arc;
use render::foliage_assembly::MeadowBladeBudget;
use render::meadow_data::{MeadowField, MeadowGeometry};
use render::vegetation_style::VegetationStyle;
use render::{Camera, Lighting, Scene};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

const POOL: u32 = 16384;
const TICK_HZ: f32 = 60.0;
/// The biomes (render::biome), as F1/F2/F4 and F8 step through them.
const STYLES: [&str; 6] = ["default", "lowland", "alpine", "polder", "tundra", "desert"];

/// The season and hour from --season (spring|summer|autumn|winter|0..1)
/// and --hour (0..24); without --hour the sun stands as it always did.
fn environment(style: usize) -> render::biome::Environment {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let mut env = render::biome::Environment::new(render::biome::Biome::named(STYLES[style]).unwrap());
    if let Some(s) = arg("--season").and_then(|s| render::biome::Season::named(&s)) { env.season = s; }
    if let Some(h) = arg("--hour").and_then(|h| h.parse().ok()) { env.hour = h; }
    env
}

/// The sun (elevation, azimuth degrees): from the environment with --hour.
fn sun_degrees(env: &render::biome::Environment) -> (f32, f32) {
    if std::env::args().any(|a| a == "--hour") { env.sun_degrees() } else { (28.0, 135.0) }
}
/// Grass is a field of roots around one point; it is rebuilt around the
/// camera once the camera is this far (metres, horizontally) from it.
const GRASS_RECENTRE: f32 = 150.0;
/// Without grass in view, trees follow the camera in bigger steps.
const TREES_RECENTRE: f32 = 300.0;
/// Recipe spruces only matter up close: low over the ground (below
/// `RECIPE_CEILING`) the scene follows every `RECIPE_RECENTRE` metres.
const RECIPE_RECENTRE: f32 = 100.0;
const RECIPE_CEILING: f32 = 30.0;
/// Mountain shadows past the shadow maps come from a heightfield this many
/// metres square at this spacing, rebuilt once the camera has moved
/// `SHADOW_RECENTRE` from its centre.
const SHADOW_SIZE: f32 = 20000.0;
const SHADOW_CELL: f32 = 20.0;
const SHADOW_RECENTRE: f32 = 4000.0;
/// Above this height over the ground the grass is too small to matter and
/// is not followed.
const GRASS_CEILING: f32 = 120.0;
/// Grass blade radius. Meadow cost grows with radius times screen cover (at
/// 1440p on an M5 Pro: 250 m ~550 ms, 120 m ~51-73 ms a frame); past the
/// blades the terrain shader draws the field as a canopy.
const GRASS_RADIUS: f32 = 45.0;

type Built = Result<(Scene, Lighting, procedural_terrain::TerrainSpec, Option<Vec3>, Option<recipe_forest::Pool>), String>;
/// A scene from the builder thread, with its renderer when the thread had
/// the GPU (every rebuild after the first): the swap then costs nothing on
/// the frame instead of ~300 ms of uploads.
type Ready = Result<(Option<(render::Renderer, ProceduralTerrain)>, Scene, Lighting, procedural_terrain::TerrainSpec, Option<Vec3>, Option<recipe_forest::Pool>), String>;

/// What goes into a scene besides the terrain; changing it rebuilds.
#[derive(Clone, Copy, PartialEq)]
struct World {
    grass: bool,
    trees: bool,
    /// Share of suitable ground that grows a tree, 0..1.
    tree_density: f32,
    /// Trees are placed this far around the camera (metres).
    tree_radius: f32,
    /// The nearest this many spruces are full assemblies; the rest are
    /// whole-plant voxel proxies of the same cooked spruce.
    recipe_trees: u32,
    /// The old forest instead: Sideways' fir cards past the recipe spruces.
    /// On the Mac they were 39M camera and 67M near shadow triangles.
    firs: bool,
}

// Debug menu ids (app::debug::GAME_IDS and up).
const MENU: u32 = 1000;
const TREES: u32 = 1001;
const GRASS: u32 = 1002;
const TREE_DENSITY: u32 = 1003;
const TREE_RADIUS: u32 = 1004;
const CELL_PX: u32 = 1005;
const ERROR_PX: u32 = 1006;
const RECIPE_TREES: u32 = 1007;
const FIRS: u32 = 1008;

struct Fly {
    seed: u32,
    style: usize,
    pos: Vec3,
    prev: Vec3,
    yaw: f32,
    pitch: f32,
    /// Cruise speed, metres per second.
    speed: f32,
    /// Keep a fixed height above the ground, like walking with a drone.
    hug: Option<f32>,
    wheel: f32,
    /// A scene being built on another thread (style, seed or grass moved).
    building: Option<Receiver<Ready>>,
    /// The GPU, cloned from the first frame, for building renderers off the
    /// frame thread.
    gpu: Option<render::Gpu>,
    /// Another rebuild was asked for while one was building.
    stale: bool,
    world: World,
    /// What the current (or building) scene was made with.
    built_world: World,
    /// Centre of the grass and trees in the current scene.
    world_at: Option<Vec3>,
    /// Terrain LOD from the debug menu, kept across rebuilds.
    terrain_settings: Settings,
    terrain: Option<ProceduralTerrain>,
    trees: Option<recipe_forest::Pool>,
    light: Lighting,
    stats: Stats,
    jumps: u32,
    fps: f32,
    /// --track: a winding gravel road the land is bent to (render::world_constraints),
    /// baked for the seed and style it was made for.
    track: bool,
    constraints: Option<(u32, usize, Arc<Constraints>)>,
}

impl Fly {
    fn new(seed: u32, style: usize, world: World) -> Result<Self, String> {
        let mut fly = Fly { seed, style, pos: Vec3::ZERO, prev: Vec3::ZERO, yaw: 0.6, pitch: -0.15, speed: 40.0, hug: None, wheel: 0.0,
            building: None, gpu: None, stale: false, world, built_world: world, world_at: None, terrain_settings: Settings::default(), terrain: None, trees: None, light: Lighting::default(), stats: Stats::default(), jumps: 0, fps: 0.0,
            track: std::env::args().any(|a| a == "--track"), constraints: None };
        fly.bake_track();
        if let Some((start, yaw)) = fly.track_start() { fly.pos = start; fly.yaw = yaw; fly.pitch = -0.05; }
        if fly.track_start().is_none() { fly.pos = Vec3::new(0.0, fly.ground(0.0, 0.0) + 60.0, 0.0); }
        fly.prev = fly.pos;
        fly.rebuild()?;
        Ok(fly)
    }

    fn style(&self) -> Style { environment(self.style).biome.terrain }
    fn ground(&self, x: f32, z: f32) -> f32 {
        match self.current_track() {
            Some(w) => w.height(x, z, self.seed, &self.style()),
            None => terrain_field::height(x, z, self.seed, &self.style()),
        }
    }

    /// The track baked for the current seed and style, if any.
    fn current_track(&self) -> Option<&Arc<Constraints>> {
        self.constraints.as_ref().filter(|c| c.0 == self.seed && c.1 == self.style).map(|c| &c.2)
    }

    /// Bake the --track road for the current seed and style: a winding loop
    /// about 700 m round the origin, laid on the land at grades up to 10 %.
    fn bake_track(&mut self) {
        if !self.track || self.current_track().is_some() { return; }
        let (seed, style) = (self.seed, self.style());
        let mut track = Track::wander_loop(Vec2::ZERO, 700.0, 3.5, seed);
        track.fit_heights(|x, z| terrain_field::height(x, z, seed, &style), 0.10, 120.0);
        let world = WorldConstraints { tracks: vec![track], pads: vec![] }.bake(seed, &style);
        self.constraints = Some((self.seed, self.style, Arc::new(world)));
    }

    /// Above the start of the track, and the yaw looking along it.
    fn track_start(&self) -> Option<(Vec3, f32)> {
        let w = self.current_track()?;
        let (s, ahead) = (w.segments.first()?, w.segments.get(10)?);
        let (dx, dz) = (ahead.a[0] - s.a[0], ahead.a[2] - s.a[2]);
        Some((Vec3::new(s.a[0], s.a[1] + 4.0, s.a[2]), (-dx).atan2(-dz)))
    }

    /// The ground at (x, z) at full detail, the track included.
    fn sample(&self, x: f32, z: f32) -> terrain_field::Sample {
        match self.current_track() {
            Some(w) => w.eval(x, z, 0.0, self.seed, &self.style()),
            None => terrain_field::eval(x, z, 0.0, self.seed, &self.style()),
        }
    }

    /// Build a new scene for the current seed, style and position on another
    /// thread; [`Game::draw`] uploads it when it is done.
    fn rebuild(&mut self) -> Result<(), String> {
        if self.building.is_some() { self.stale = true; return Ok(()); }
        self.bake_track();
        let track = self.current_track().cloned();
        let (seed, style, pos) = (self.seed, self.style(), self.pos);
        // Each biome has its palette, turned by the season (render::biome).
        let env = environment(self.style);
        let palette = env.vegetation();
        // The density knob (60 % by default) scales the biome's own.
        let world = World { grass: self.grass_wanted(), tree_density: self.world.tree_density / 0.6 * environment(self.style).biome.tree_density, ..self.world };
        self.built_world = world;
        let (tx, rx) = channel();
        let (gpu, settings) = (self.gpu.clone(), self.terrain_settings);
        std::thread::spawn(move || {
            let biome = env.biome;
            let ready = build_scene(seed, style, &biome, &palette, sun_degrees(&env), pos, world, track).map(|(scene, light, pool, at, trees)| {
                let built = gpu.map(|gpu| renderer_for(&gpu, &scene, &light, pool.clone(), settings));
                (built, scene, light, pool, at, trees)
            });
            let _ = tx.send(ready);
        });
        self.building = Some(rx);
        Ok(())
    }

    /// Grass is on and the camera is low enough to see it.
    fn grass_wanted(&self) -> bool { self.world.grass && self.pos.y - self.ground(self.pos.x, self.pos.z) < GRASS_CEILING }

    /// The scene's grass or trees no longer fit the settings or no longer
    /// cover what the camera sees.
    fn world_stale(&self) -> bool {
        let want = World { grass: self.grass_wanted(), ..self.world };
        if want != self.built_world { return true; }
        let low = self.pos.y - self.ground(self.pos.x, self.pos.z) < RECIPE_CEILING;
        let reach = if want.trees && want.recipe_trees > 0 && low { RECIPE_RECENTRE }
            else if want.grass { GRASS_RECENTRE } else if want.trees { TREES_RECENTRE } else { SHADOW_RECENTRE };
        self.world_at.is_none_or(|c| (c - self.pos).with_y(0.0).length() > reach)
    }

    /// Somewhere new on a spiral out from the origin: high ground with a view.
    fn jump(&mut self) {
        self.jumps += 1;
        let a = self.jumps as f32 * 2.399;
        let r = 1500.0 * (self.jumps as f32).sqrt();
        let (x, z) = (r * a.cos(), r * a.sin());
        self.pos = Vec3::new(x, self.ground(x, z) + 120.0, z);
        self.prev = self.pos;
    }

    fn camera(&self, pos: Vec3) -> Camera { Camera { pos, target: pos + self.forward(), fov_y: 65f32.to_radians(), near: 0.05 } }

    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
}

impl Game for Fly {
    fn local_players(&self) -> usize { 1 }

    fn pointer(&mut self, pointer: Pointer, _size: [u32; 2]) { self.wheel += pointer.wheel; }

    fn tick(&mut self, c: &[Controls]) {
        let dt = 1.0 / TICK_HZ;
        let c: Controls = c[0];
        self.prev = self.pos;
        self.yaw -= c.look[0];
        self.pitch = (self.pitch + c.look[1]).clamp(-1.5, 1.5);
        let steps = std::mem::take(&mut self.wheel) + if c.pressed.has(Buttons::UP) { 1.0 } else { 0.0 } - if c.pressed.has(Buttons::DOWN) { 1.0 } else { 0.0 };
        self.speed = (self.speed * 1.25f32.powf(steps)).clamp(1.0, 4000.0);
        let boost = if c.held.has(Buttons::RIGHT_SHOULDER) { 6.0 } else { 1.0 };
        let fwd = self.forward();
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let lift = (c.held.has(Buttons::SOUTH) as i32 - c.held.has(Buttons::LEFT_SHOULDER) as i32) as f32;
        self.pos += (fwd * c.mv[1] + right * c.mv[0] + Vec3::Y * lift) * self.speed * boost * dt;
        let floor = self.ground(self.pos.x, self.pos.z);
        match self.hug {
            Some(h) => self.pos.y = floor + h,
            None => self.pos.y = self.pos.y.max(floor + 1.7),
        }
    }

    fn function_key(&mut self, n: u8) -> bool {
        let style = match n { 1 => Some(0), 2 => Some(1), 4 => Some(2), 8 => Some((self.style + 1) % STYLES.len()), _ => None };
        let result = match n {
            1 | 2 | 4 | 8 => { self.style = style.unwrap(); self.rebuild() }
            5 => { self.seed += 1; self.rebuild() }
            6 => { self.jump(); Ok(()) }
            7 => { self.hug = match self.hug { Some(_) => None, None => Some(1.7) }; Ok(()) }
            _ => return false,
        };
        if let Err(e) = result { eprintln!("terrain fly: {e}"); }
        if matches!(n, 1 | 2 | 4 | 5 | 8) {
            // Stay at the same height above the new ground.
            let floor = self.ground(self.pos.x, self.pos.z);
            self.pos.y = self.pos.y.max(floor + 30.0);
            self.prev = self.pos;
        }
        true
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if self.gpu.is_none() { self.gpu = Some(frame.gpu.clone()); }
        // The first scene is waited for; later ones swap in when ready.
        let ready = match &self.building {
            Some(rx) if self.terrain.is_none() => Some(rx.recv().map_err(|e| e.to_string())),
            Some(rx) => rx.try_recv().ok().map(Ok),
            None => None,
        };
        if let Some(built) = ready {
            self.building = None;
            let (prebuilt, scene, light, pool, world_at, trees) = built??;
            self.trees = trees;
            match prebuilt {
                Some((renderer, terrain)) => { frame.set_renderer(renderer); self.terrain = Some(terrain); }
                None => { frame.set_scene(&scene, &light); self.terrain = Some(ProceduralTerrain::new(pool, self.terrain_settings)); }
            }
            self.light = light;
            self.world_at = world_at;
            if std::mem::take(&mut self.stale) { self.rebuild()?; }
        }
        if self.building.is_none() && self.world_stale() { self.rebuild()?; }
        let pos = self.prev.lerp(self.pos, frame.alpha);
        let cam = self.camera(pos);
        let [w, h] = frame.size;
        if let (Some(terrain), Some(renderer)) = (self.terrain.as_mut(), frame.renderer()) {
            self.stats = terrain.update(frame.gpu, renderer, &[TerrainView::new(&cam, w, h)])?;
            // The full spruces follow the camera (recipe_forest::Pool).
            if let Some(trees) = &mut self.trees { trees.update(renderer, frame.gpu, &[pos])?; }
        }
        if frame.dt > 0.0 { self.fps += (1.0 / frame.dt - self.fps) * 0.05; }

        let s = self.sample(pos.x, pos.z);
        let above = pos.y - s.pos.y;
        let lines = [
            format!("{}  seed {}  {:.0} fps", STYLES[self.style], self.seed, self.fps),
            format!("x {:.0}  z {:.0}  height {:.0} m  {:.0} m above ground", pos.x, pos.z, pos.y, above),
            format!("speed {:.0} m/s{}", self.speed, if self.hug.is_some() { "  ground hug" } else { "" }),
            format!("patches {}  overflow {}  finest {}  lod x{:.2}", self.stats.patches, self.stats.overflow, self.stats.finest_level, self.stats.lod_scale),
            format!("ground: rock {:.2}  scree {:.2}  snow {:.2}  water {:.2}  slope {:.0}°", s.rock, s.scree, s.snow, s.water, s.up.clamp(0.0, 1.0).acos().to_degrees()),
            "F1 default  F2 lowland  F4 alpine  F8 next biome  F5 seed  F6 jump  F7 hug  wheel speed  F12 screenshot".to_string(),
        ];
        let mut hud = HudDrawList::new(frame.size);
        for (i, text) in lines.into_iter().enumerate() {
            hud.text.push(TextRun { id: i as u64 + 1, text, position: [20.0, 16.0 + 26.0 * i as f32], bounds: [0.0, 0.0, w as f32, h as f32],
                font_size: 20.0, color: [255, 255, 255, 230], font: FontFace::Body, align: TextAlign::Left, width: w as f32 });
        }
        Ok(Draw { cameras: vec![cam], light: self.light.clone(), hud })
    }

    fn debug(&self) -> (Vec<DebugMenu>, Vec<DebugControl>) {
        let menu = vec![DebugMenu { id: MENU, parent: None, label: "Terrain fly".into(), help: "Trees, grass and terrain LOD. Trees and grass rebuild the scene in the background.".into() }];
        let toggle = |id, label: &str, value, help: &str| DebugControl { id, label: label.into(), value: DebugValue::Toggle(value), enabled: true, menu: Some(MENU), advanced: false, help: help.into() };
        let number = |id, label: &str, value: f64, (min, max, step, decimals): (f64, f64, f64, u8), unit: &str, help: &str| DebugControl {
            id, label: label.into(), value: DebugValue::Number { value, min, max, step, decimals, unit: unit.into() }, enabled: true, menu: Some(MENU), advanced: false, help: help.into() };
        let w = &self.world;
        let t = &self.terrain_settings;
        (menu, vec![
            toggle(TREES, "Trees", w.trees, "Spruce forest around the camera."),
            number(TREE_DENSITY, "Tree density", f64::from(w.tree_density) * 100.0, (5.0, 100.0, 5.0, 0), "%", "Share of forest ground that grows a tree."),
            number(RECIPE_TREES, "Full spruces near camera", f64::from(w.recipe_trees), (0.0, 120.0, 6.0, 0), "", "The nearest spruces are full assemblies (detailed sprays, branch wind); the rest are voxel proxies of the same spruce."),
            toggle(FIRS, "Old fir cards", w.firs, "The old forest past the full spruces: Sideways' fir meshes instead of voxel proxies."),
            number(TREE_RADIUS, "Tree radius", f64::from(w.tree_radius), (200.0, 3000.0, 100.0, 0), "m", "Trees are placed this far around the camera."),
            toggle(GRASS, "Grass in scene", w.grass, "Builds the meadow (45 m of blades, canopy shading beyond) below 120 m above ground. Expensive. The Grass menu only hides it."),
            number(CELL_PX, "Terrain cell size", f64::from(t.cell_pixels), (4.0, 96.0, 4.0, 0), "px", "Largest projected terrain cell before a patch splits."),
            number(ERROR_PX, "Terrain error", f64::from(t.error_pixels), (0.25, 12.0, 0.25, 2), "px", "Largest projected height error before a patch splits."),
        ])
    }

    fn debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::ResetDefaults => { self.world = World::default(); self.terrain_settings = Settings::default(); }
            DebugAction::SetToggle { id: TREES, value } => self.world.trees = value,
            DebugAction::SetToggle { id: GRASS, value } => self.world.grass = value,
            DebugAction::SetToggle { id: FIRS, value } => self.world.firs = value,
            DebugAction::SetNumber { id: TREE_DENSITY, value } => self.world.tree_density = (value / 100.0).clamp(0.05, 1.0) as f32,
            DebugAction::SetNumber { id: RECIPE_TREES, value } => self.world.recipe_trees = value.clamp(0.0, 120.0) as u32,
            DebugAction::SetNumber { id: TREE_RADIUS, value } => self.world.tree_radius = value.clamp(200.0, 3000.0) as f32,
            DebugAction::SetNumber { id: CELL_PX, value } => self.terrain_settings.cell_pixels = value.clamp(4.0, 96.0) as f32,
            DebugAction::SetNumber { id: ERROR_PX, value } => self.terrain_settings.error_pixels = value.clamp(0.25, 12.0) as f32,
            _ => return,
        }
        if let Some(t) = &mut self.terrain { t.settings = self.terrain_settings; }
    }

    fn title(&self) -> Option<String> { Some(format!("Terrain fly: {} seed {}", STYLES[self.style], self.seed)) }

    fn state(&self) -> Option<String> {
        let g = self.sample(self.pos.x, self.pos.z);
        Some(serde_json::json!({
            "style": STYLES[self.style], "seed": self.seed, "pos": self.pos.to_array(),
            "trees": self.built_world.trees, "grass": self.built_world.grass, "tree_density": self.world.tree_density, "tree_radius_m": self.world.tree_radius, "recipe_trees": self.world.recipe_trees, "firs": self.world.firs,
            "yaw_deg": self.yaw.to_degrees(), "pitch_deg": self.pitch.to_degrees(),
            "above_ground_m": self.pos.y - g.pos.y, "speed": self.speed, "ground_hug": self.hug, "fps": self.fps,
            "terrain": {"patches": self.stats.patches, "overflow": self.stats.overflow, "finest_level": self.stats.finest_level, "lod_scale": self.stats.lod_scale},
            "ground": {"rock": g.rock, "scree": g.scree, "snow": g.snow, "water": g.water, "slope_deg": g.up.clamp(0.0, 1.0).acos().to_degrees()},
            "repro": format!("teleport {:.1} {:.1} {:.1} | look {:.1} {:.1}", self.pos.x, self.pos.z, self.pos.y - g.pos.y, self.yaw.to_degrees(), self.pitch.to_degrees()),
        }).to_string())
    }

    fn command(&mut self, cmd: &str) -> Result<String, String> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        match parts.as_slice() {
            ["help"] => Ok("teleport X Z [HEIGHT_ABOVE] | look YAW PITCH | style default|lowland|alpine | seed N".into()),
            ["teleport", x, z, rest @ ..] => {
                let (x, z): (f32, f32) = (x.parse().map_err(|_| "x")?, z.parse().map_err(|_| "z")?);
                let above: f32 = rest.first().map(|h| h.parse().map_err(|_| "height")).transpose()?.unwrap_or(60.0);
                self.pos = Vec3::new(x, self.ground(x, z) + above, z);
                self.prev = self.pos;
                Ok(format!("at {:?}", self.pos))
            }
            ["look", yaw, pitch] => {
                self.yaw = yaw.parse::<f32>().map_err(|_| "yaw")?.to_radians();
                self.pitch = pitch.parse::<f32>().map_err(|_| "pitch")?.to_radians();
                Ok("ok".into())
            }
            ["style", name] => {
                self.style = STYLES.iter().position(|s| s == name).ok_or("no such style")?;
                self.rebuild()?;
                Ok("ok".into())
            }
            ["seed", n] => { self.seed = n.parse().map_err(|_| "seed")?; self.rebuild()?; Ok("ok".into()) }
            _ => Err(format!("unknown command {cmd}; try help")),
        }
    }
}

impl Default for World {
    fn default() -> Self { World { grass: false, trees: true, tree_density: 0.6, tree_radius: 2500.0, recipe_trees: 60, firs: false } }
}

/// Terrain textures, the patch pool, the sky, and grass and trees around
/// `centre` as `world` asks, and the shadow heightfield around it. Returns the centre.
fn build_scene(seed: u32, style: Style, biome: &render::biome::Biome, palette: &VegetationStyle, sun: (f32, f32), centre: Vec3, world: World, track: Option<Arc<Constraints>>) -> Built {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets");
    let root = assets.join("textures");
    let mut scene = Scene::default();
    if world.grass { scene.meadow = Some(meadow(seed, &style, palette.meadow_dry_season, centre)?); }
    // Layers in the order terrain_surface.wgsl reads them.
    let t0 = scene.add_texture(&root.join("grass_ground_albedo.jpg"))?;
    scene.add_texture(&root.join("forest_leaves_02_albedo.jpg"))?;
    scene.add_texture(&root.join("forest_ground_04_albedo.jpg"))?;
    scene.add_texture(&root.join("rock_face_03_albedo.jpg"))?;
    scene.add_texture_with_normal_encoding(&root.join("gravel_ground_01_albedo.jpg"), render::scene::NormalEncoding::SrgbTransferred)?;
    scene.add_texture(&root.join("mossy_rock_albedo.jpg"))?;
    let material = scene.add_material(Material { tint: [1.0; 3], layer: t0, roughness: 0.9, flags: TERRAIN | SURFACE, scale: 4.0, metal: 0.0 });
    let mut tree_pool = None;
    if world.trees {
        let layer_only = std::env::args().any(|a| a == "--forest-layer-only");
        let mut trees = if layer_only { Vec::new() } else { forest(seed, &style, biome, centre, world, track.as_deref()) };
        trees.sort_by(|a, b| (a.0 - centre).length_squared().total_cmp(&(b.0 - centre).length_squared()));
        let near = (world.recipe_trees as usize).min(trees.len());
        if world.firs {
            let firs_only: Vec<_> = trees[near..].iter().map(|&(p, y, s, v)| (p, y, s, v % 3)).collect();
            firs(&mut scene, &assets, &firs_only)?;
        }
        let far = if world.firs { &[][..] } else { &trees[near..] };
        tree_pool = recipe_spruces(&mut scene, biome, &trees[..near], far)?;
    }
    let mut pool = procedural_terrain::reserve(&mut scene, POOL, material, seed, style);
    // Beyond the trees the terrain draws the same forest as colour, out to
    // the horizon.
    procedural_terrain::vegetation_style(&mut scene, material, palette);
    // Drawn trees hand over to the painted forest in the outer 15 % of their
    // radius, tree by tree (forest_field::handover_keep), so the forest share
    // holds across the ring. --forest-layer-only paints it everywhere (the
    // reference for that share).
    if world.trees {
        let (start, end) = if std::env::args().any(|a| a == "--forest-layer-only") { (0.0, 1.0) } else { handover_band(world) };
        procedural_terrain::far_forest_handover(&mut scene, material, [centre.x, centre.z], start, end, world.tree_density, &palette.forest);
    }
    // The --track road: the land bends to it, and its pixels draw gravel.
    if let Some(w) = track {
        procedural_terrain::road_surface(&mut scene, material, &w);
        pool = procedural_terrain::constrain(&mut scene, w);
    }
    let mut light = Lighting::default();
    // Sunlit valleys bounce light onto the slopes facing away from the sun:
    // with the default 0.15 a lee slope 3 km out rendered near black.
    let atmosphere = Atmosphere { ground_albedo: glam::DVec3::splat(0.25), ..Atmosphere::default() };
    let sky = AtmosphereCache::default().get_or_bake(&atmosphere, SolarState::from_degrees(sun.0 as f64, sun.1 as f64)?, BakeQuality::default())?;
    sky.apply(&mut scene, &mut light);
    // Sun visibility baked on a coarse heightfield shades the valleys behind
    // mountains past the reach of the shadow maps.
    (scene.heightfield, scene.terrain_sun) = procedural_terrain::sun_heightfield(seed, &style, [centre.x, centre.z], SHADOW_SIZE, SHADOW_CELL, light.sun_dir);
    scene.terrain_visibility_complete = true;
    scene.world_terrain_visibility = true;
    // A light summer haze (~60 km visibility): distant lee slopes
    // and forests fade towards the sky instead of staying near black.
    light.fog_density = 0.00005;
    light.fog_end = 60000.0;
    // TAA reprojects with per-pixel motion vectors (wind and parts too):
    // turning the camera, foliage shimmers about 75 % less than without it
    // (motion pan test, playtest cameras 3 and 4). Its history converges
    // slowly (blend 0.04) and is barely sharpened, which re-adds shimmer.
    // --taa 0 turns it off.
    light.taa = std::env::args().collect::<Vec<_>>().windows(2).find(|w| w[0] == "--taa").and_then(|w| w[1].parse().ok()).unwrap_or(1.0);
    light.taa_blend = 0.04;
    light.sharpen = 0.2;
    // --light key=value[,key=value]: Lighting::set, for look and TAA tuning.
    for kv in std::env::args().collect::<Vec<_>>().windows(2).filter(|w| w[0] == "--light").flat_map(|w| w[1].split(',').map(String::from).collect::<Vec<_>>()) {
        if let Err(e) = light.set(&kv) { eprintln!("--light {kv}: {e}"); }
    }
    Ok((scene, light, pool, Some(centre), tree_pool))
}

/// The renderer for `scene`, built on the scene thread: the swap then
/// costs the frame nothing instead of ~300 ms of uploads. (Also warming it
/// up with a first terrain update and a small frame did not measurably help
/// and competed with the live frames for the GPU.)
fn renderer_for(gpu: &render::Gpu, scene: &Scene, light: &Lighting, pool: procedural_terrain::TerrainSpec, settings: Settings) -> (render::Renderer, ProceduralTerrain) {
    (render::Renderer::new(gpu, scene, light), ProceduralTerrain::new(pool, settings))
}

/// Sideways' fir cards at these spots, as before the recipe spruces took
/// over. Materials 1..=4, as tree_meshes() expects: call before any other.
fn firs(scene: &mut Scene, assets: &std::path::Path, spots: &[(Vec3, f32, Vec3, usize)]) -> Result<(), String> {
    let m = |layer, flags| Material { tint: [1.0; 3], layer, roughness: 0.9, flags, scale: 1.0, metal: 0.0 };
    for wood in ["trunk_a_albedo.jpg", "trunk_b_albedo.jpg", "bark_albedo.jpg"] {
        let layer = scene.add_texture(&assets.join("trees").join(wood))?;
        scene.add_material(m(layer, 0));
    }
    let crown = scene.add_texture(&assets.join("trees/authored_pine_albedo.png"))?;
    scene.add_material(Material { tint: [0.62, 0.66, 0.55], ..m(crown, ALPHA_TEST | TWO_SIDED) });
    let meshes: Vec<usize> = tree_meshes()?.iter().map(|cm| scene.add_mesh(cm)).collect();
    for &(pos, yaw, scale, variant) in spots {
        scene.add_classified_instance(meshes[variant], Affine3A::from_scale_rotation_translation(scale, Quat::from_rotation_y(yaw), pos), GeometryCategory::Trees);
        scene.mark_foliage();
    }
    Ok(())
}

/// Recipe spruces (render::recipe_forest): three seeds, cooked once and
/// shared. Every spot is the plant as one voxel proxy, and as many full
/// assemblies as `near` has follow the camera (the returned pool). Without
/// `far` (the firs draw those) the `near` spots are full and fixed.
/// The biome's species (render::biome::Biome::species), cooked once per
/// biome in this run (and from the disk cache across runs), and their cards.
fn cooked_flora(biome: &render::biome::Biome) -> Result<&'static (Vec<CookedPlant>, Vec<render::plant_mesh_lod::PlantCards>), String> {
    type Flora = (Vec<CookedPlant>, Vec<render::plant_mesh_lod::PlantCards>);
    static COOKED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, &'static Flora>>> = std::sync::OnceLock::new();
    let mut map = COOKED.get_or_init(Default::default).lock().unwrap();
    if let Some(&flora) = map.get(biome.name) { return Ok(flora); }
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
    let species = biome.species();
    let recipes: Vec<_> = species.iter().map(|(_, r)| (*r, 1)).collect();
    let cooked = recipe_forest::cook_variants(&recipes, Some(&cache))?;
    for (c, (name, _)) in cooked.iter().zip(&species) {
        eprintln!("{name}: {} in {:.0} ms", if c.loaded { "loaded" } else { "cooked" }, c.millis);
    }
    let cards = recipe_forest::cook_cards(&recipes, Some(&cache))?;
    let flora: &'static Flora = Box::leak(Box::new((cooked.into_iter().map(|c| c.plant).collect(), cards)));
    map.insert(biome.name, flora);
    Ok(flora)
}

fn recipe_spruces(scene: &mut Scene, biome: &render::biome::Biome, near: &[(Vec3, f32, Vec3, usize)], far: &[(Vec3, f32, Vec3, usize)]) -> Result<Option<recipe_forest::Pool>, String> {
    if near.is_empty() && far.is_empty() { return Ok(None); }
    let (cooked, baked) = cooked_flora(biome)?;
    // The firs are ~15 m; the recipe is 16 m, scaled like them.
    let trees: Vec<recipe_forest::Tree> = near.iter().chain(far).map(|&(pos, yaw, scale, variant)|
        recipe_forest::Tree { pos, yaw, scale: scale.y, variant }).collect();
    let arg = |k: &str| std::env::args().collect::<Vec<_>>().windows(2).find(|w| w[0] == k).and_then(|w| w[1].parse().ok());
    let defaults = recipe_forest::game_policy();
    let policy = render::aggregate_lod::Policy { proxy_pixels: arg("--proxy-px").unwrap_or(defaults.proxy_pixels),
        proxy_pixels_from: arg("--proxy-from").unwrap_or(defaults.proxy_pixels_from), ..defaults };
    let started = std::time::Instant::now();
    if far.is_empty() {
        recipe_forest::plant(scene, cooked, &trees, near.len(), policy)?;
        println!("{} full trees planted in {} ms", trees.len(), started.elapsed().as_millis());
        return Ok(None);
    }
    // Three variants: the full set is shared out over them.
    let slots = near.len().div_ceil(cooked.len());
    let (_, mut pool) = recipe_forest::plant_streamed(scene, cooked, &trees, slots, policy)?;
    // Past the full ones, card trees (plant_mesh_lod) within --card-radius
    // (150 m), --cards per variant (400; 0: voxel proxies only). Cards hold
    // the source's look to 300 m (forest_ladder), but seen from above out
    // to 300 m they cost 23 ms at 1440p; to 150 m the four playtest cameras
    // run 24-29 fps against 24-29 before at 300 m, the high ones +4.6.
    let (cards, card_radius) = (arg("--cards").unwrap_or(400.0f32) as usize, arg("--card-radius").unwrap_or(150.0f32));
    if cards > 0 { recipe_forest::add_cards(scene, &mut pool, baked, cards, card_radius)?; }
    println!("{} trees ({slots} full per variant following the camera, {cards} card trees per variant within {card_radius} m) planted in {} ms",
        trees.len(), started.elapsed().as_millis());
    Ok(Some(pool))
}

type Surfaces = Vec<(String, Vec<render::cluster::Vertex>, Vec<u32>)>;

/// Sideways' three fir meshes as read from disk, once.
fn fir_surfaces() -> Result<&'static [Surfaces], String> {
    static SURFACES: std::sync::OnceLock<Result<Vec<Surfaces>, String>> = std::sync::OnceLock::new();
    SURFACES.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets/meshes");
        ["a", "b", "c"].iter().map(|v| {
            let path = dir.join(format!("fir_{v}_near.kmesh"));
            render::cluster::read_kmesh(&std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
        }).collect()
    }).as_deref().map_err(Clone::clone)
}

/// Wood uses materials 1..=3 (trunk a, trunk b, bark), needles material 4.
fn fir_material(name: &str) -> u32 {
    if name.contains("trunk_b") { 2 } else if name.contains("trunk") { 1 } else if name.contains("bark") { 3 } else { 4 }
}

/// Sideways' three firs as cluster meshes, built once.
fn tree_meshes() -> Result<&'static [ClusterMesh], String> {
    static MESHES: std::sync::OnceLock<Result<Vec<ClusterMesh>, String>> = std::sync::OnceLock::new();
    let built = MESHES.get_or_init(|| {
        fir_surfaces()?.iter().map(|surfaces| {
            let parts: Vec<Part> = surfaces.iter().map(|(name, vertices, indices)| Part { vertices, indices, material: fir_material(name) }).collect();
            Ok(ClusterMesh::build_with_policy(&parts, |p| if p.material == 4 { SimplificationPolicy::AlphaCards } else { SimplificationPolicy::TexturedGeometry }))
        }).collect()
    });
    built.as_deref().map_err(Clone::clone)
}

/// Trees within `world.tree_radius` of `centre`, from the engine's forest
/// field (the distant terrain draws the same forest). Position, yaw, scale
/// and variant.
fn forest(seed: u32, style: &Style, biome: &render::biome::Biome, centre: Vec3, world: World, track: Option<&Constraints>) -> Vec<(Vec3, f32, Vec3, usize)> {
    use render::placement::{self, TerrainWorld};
    let (start, end) = handover_band(world);
    let c = Vec2::new(centre.x, centre.z);
    let biome = render::biome::Biome { tree_density: world.tree_density, ..*biome };
    // Bent to the --track road; the rules keep off it (placement::off_road).
    let land = TerrainWorld { seed, style, constraints: track };
    let (trees, shrubs): (Vec<_>, Vec<_>) = biome.rules().into_iter().partition(|r| r.id == placement::TREE);
    // Trees out to the painted forest, handed over to it tree by tree;
    // shrubs only near, where a knee-high bush is more than a pixel.
    placement::place(&trees, &land, c, world.tree_radius).into_iter()
        .filter(|t| render::forest_field::handover_keep(t.pos, c, start, end))
        .chain(placement::place(&shrubs, &land, c, SHRUB_RADIUS))
        .map(|t| (t.pos, t.yaw, t.scale, t.variant as usize)).collect()
}

/// Shrubs are placed this far around the camera (metres).
const SHRUB_RADIUS: f32 = 400.0;

/// Where drawn trees hand over to the painted forest (metres from the centre).
fn handover_band(world: World) -> (f32, f32) {
    (world.tree_radius * 0.85, world.tree_radius)
}

/// Windswept meadow roots on a 0.5 m grid of the terrain function, 300 m
/// around `centre` (as in render's procedural_terrain example). Rock,
/// scree, snow and water are bare.
fn meadow(seed: u32, style: &Style, dry_season: f32, centre: Vec3) -> Result<MeadowField, String> {
    let (cell, n) = (0.5f32, 1200u32);
    let origin = [(centre.x / cell).floor() * cell - n as f32 * cell * 0.5, (centre.z / cell).floor() * cell - n as f32 * cell * 0.5];
    let mut samples = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (origin[0] + i as f32 * cell, origin[1] + j as f32 * cell);
            let s = terrain_field::eval(x, z, 0.0, seed, style);
            let sparse = 0.5 + 0.8 * terrain_field::noised(glam::Vec2::new(x, z) / 40.0, seed.wrapping_add(31)).x > style.grass;
            let bare = sparse || s.rock > 0.5 || s.scree > 0.8 || s.water > 0.02 || s.snow > 0.3;
            samples.push(if bare { [0.0; 4] } else { [s.pos.y, 100.0, 1.0, 0.0] });
        }
    }
    let field = MeadowField {
        origin, cell, width: n, height: n, samples, seed, growth: 1.0, dry_season, radius: GRASS_RADIUS, fade_start: GRASS_RADIUS * 0.8,
        geometry: MeadowGeometry::cook_windswept(seed as u64, dry_season, MeadowBladeBudget::Blades120)?,
    };
    field.validate()?;
    Ok(field)
}

fn main() -> Result<(), String> {
    // Full spruces share their sprays per species (virtual members), so
    // dozens fit where 12 did (docs/foliage-assemblies-plan.md).
    recipe_forest::set_virtual_members(true);
    // Distant fir cards cast shadows from cached foliage sun pages; without
    // them tree shadows popped in at the edge of the near shadow map. Headless
    // 1440p: the same GPU time, and unlike KERN_FAR_SUN_SHADOW no striping
    // on the terrain. An explicit KERN_PAGED_FOLIAGE_SUN=0 still wins. Voxel
    // proxies cast into the sun maps themselves.
    if std::env::var_os("KERN_PAGED_FOLIAGE_SUN").is_none() && std::env::args().any(|a| a == "--firs") {
        // Single-threaded here: nothing has started yet.
        unsafe { std::env::set_var("KERN_PAGED_FOLIAGE_SUN", "1"); }
    }
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let seed = arg("--seed").map(|s| s.parse().map_err(|_| "--seed N")).transpose()?.unwrap_or(1);
    let style = arg("--style").map(|s| STYLES.iter().position(|n| *n == s).ok_or("--style default|lowland|alpine|polder|tundra|desert")).transpose()?.unwrap_or(0);
    let world = World { trees: !args.iter().any(|a| a == "--no-trees"), grass: args.iter().any(|a| a == "--grass"),
        firs: args.iter().any(|a| a == "--firs"),
        tree_radius: arg("--tree-radius").map(|n| n.parse().map_err(|_| "--tree-radius M")).transpose()?.unwrap_or(World::default().tree_radius),
        recipe_trees: arg("--recipe").map(|n| n.parse().map_err(|_| "--recipe N")).transpose()?.unwrap_or(World::default().recipe_trees), ..World::default() };
    let mut game = Fly::new(seed, style, world)?;
    // --from: a JSON written by F12 (or just its "state"), to stand where the playtester stood.
    if let Some(path) = arg("--from") {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))?;
        let st = v.get("state").unwrap_or(&v);
        let f = |k: &str| st.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
        if let Some(name) = st.get("style").and_then(|x| x.as_str()) { game.style = STYLES.iter().position(|n| *n == name).ok_or("unknown style")?; }
        if let Some(seed) = st.get("seed").and_then(|x| x.as_u64()) { game.seed = seed as u32; }
        if let Some(p) = st.get("pos").and_then(|x| x.as_array()) {
            let c: Vec<f32> = p.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
            if c.len() == 3 { game.pos = Vec3::new(c[0], c[1], c[2]); game.prev = game.pos; }
        }
        if let Some(y) = f("yaw_deg") { game.yaw = y.to_radians(); }
        if let Some(p) = f("pitch_deg") { game.pitch = p.to_radians(); }
        if let Some(s) = f("speed") { game.speed = s; }
        game.rebuild()?;
    }
    // --bench N: N headless frames at --size (default 2560x1440), GPU pass times.
    if let Some(n) = arg("--bench") {
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([2560, 1440]);
        let gpu = render::Gpu::new()?;
        let (fps, passes) = app::bench(&mut game, &gpu, [w, h], n.parse().map_err(|_| "--bench N")?, TICK_HZ)?;
        println!("{w}x{h}: {fps:.1} fps");
        for (pass, ms) in passes.iter().filter(|p| p.1 >= 0.05) { println!("  {pass:>28} {ms:7.2} ms"); }
        return Ok(());
    }
    // --fly N: N headless frames flying straight ahead at --speed (m/s,
    // default 80), one tick per frame; prints frame-time percentiles, so
    // hitches from scene rebuilds show without a window.
    if let Some(n) = arg("--fly") {
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1280, 720]);
        game.speed = arg("--speed").and_then(|s| s.parse().ok()).unwrap_or(80.0);
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], TICK_HZ)?;
        let mut times = Vec::new();
        let forward = Controls { mv: [0.0, 1.0], ..Default::default() };
        for i in 0..n.parse::<u32>().map_err(|_| "--fly N")? {
            let t = std::time::Instant::now();
            game.tick(&[forward]);
            headless.frame(&mut game)?;
            if i >= 5 { times.push(t.elapsed().as_secs_f64() * 1e3); }
        }
        times.sort_by(f64::total_cmp);
        let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        let slow = times.iter().filter(|&&t| t > 3.0 * p(0.5)).count();
        println!("{w}x{h} {} frames at {} m/s: p50 {:.1} ms  p99 {:.1} ms  max {:.1} ms  {} frames over 3x p50", times.len(), game.speed, p(0.5), p(0.99), p(1.0), slow);
        return Ok(());
    }
    // --dump N DIR: N consecutive headless frames as PNGs (flicker checks);
    // --pan DEG turns the camera that far right each frame (motion flicker).
    if let Some(n) = arg("--dump") {
        let dir = args.windows(3).find(|w| w[0] == "--dump").map(|w| w[2].clone()).ok_or("--dump N DIR")?;
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1280, 720]);
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], TICK_HZ)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let pan = arg("--pan").and_then(|d| d.parse::<f32>().ok()).unwrap_or(0.0).to_radians();
        for i in 0..n.parse::<u32>().map_err(|_| "--dump N DIR")? {
            game.tick(&[Controls { look: [pan, 0.0], ..Default::default() }]);
            let rgba = headless.frame(&mut game)?;
            render::write_png(&PathBuf::from(&dir).join(format!("{i:03}.png")), w, h, &rgba)?;
        }
        return Ok(());
    }
    if let Some(out) = arg("--shot") {
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1280, 720]);
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut game, &gpu, [w, h], 10, TICK_HZ)?;
        return render::write_png(std::path::Path::new(&out), w, h, &rgba);
    }
    let frames = arg("--frames").map(|s| s.parse().map_err(|_| "--frames N")).transpose()?.unwrap_or(0);
    app::run(app::Config { title: "Terrain fly".into(), tick_hz: TICK_HZ, fullscreen: args.iter().any(|a| a == "--fullscreen"), frames,
        // Shading dominates at 5K; WARP_RENDER_HEIGHT=0 renders native.
        max_render_height: Some(1440), ..Default::default() }, &mut game)
}
