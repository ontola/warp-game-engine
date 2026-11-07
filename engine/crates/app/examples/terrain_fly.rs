//! Fly over the procedural terrain: a debug game for looking at the world
//! the way a player would, at any height and speed.
//!
//!   cargo run --release -p app --example terrain_fly
//!   cargo run --release -p app --example terrain_fly -- --style alpine --seed 3
//!   cargo run --release -p app --example terrain_fly -- --track   (a gravel road the land bends to)
//!   cargo run --release -p app --example terrain_fly -- --style alpine --regions polder   (alpine west of x = 0, polder east)
//!   cargo run --release -p app --example terrain_fly -- --profile low   (Quest-class settings)
//!   cargo run --release -p app --example terrain_fly -- --town   (a zone with buildings of 1 to 16 storeys on levelled plots)
//!   cargo run --release -p app --example terrain_fly -- --town --no-buildings   (the bare plots)
//!   cargo run --release -p app --example terrain_fly -- --town --at 40,120,1.7,0.6,0   (start at x,z, metres above ground, yaw, pitch)
//!   cargo run --release -p app --example terrain_fly -- --shot fly.png
//!   cargo run --release -p app --example terrain_fly -- --fullscreen --frames 600   # fps benchmark
//!   ... --validate-world --grass   (normal CPU scene build, no GPU/window)
//!   ... --grass --branch-slots 4096   (bounded GPU vegetation residency)
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
//! Grass cells and biome trees share GPU selection, poses and paging across a
//! disc around the camera; root/ground/sun data stream without renderer rebuilds.
//! GPU visibility and detail selection cover every root with shared recipes,
//! articulated wind and bounded source/voxel residency.
use app::{Buttons, Controls, Draw, Frame, Game, Pointer};
use glam::{Vec2, Vec3};
use render::debug_ui::{DebugAction, DebugControl, DebugMenu, DebugValue};
use render::recipe_forest;
use render::vegetation_asset::CookedPlant;
use render::foliage_population::WorldRoot;
use render::foliage_runtime::RootPlacement;
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::procedural_terrain::{self, ProceduralTerrain, Settings, Stats, TerrainView};
use render::scene::{GeometryCategory, Material, SURFACE, TERRAIN};
use render::terrain_field::{self, Style};
use render::world_constraints::{Constraints as WorldConstraints, Track, World as Constraints};
use std::sync::Arc;
use render::vegetation_style::VegetationStyle;
use render::{Camera, Lighting, Scene};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

// Room beside the shown patches for the ones a turn builds ahead
// (procedural_terrain build_budget).
const POOL: u32 = 24576;
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
/// camera once the camera is this far (metres, horizontally) from it. At 60 m
/// a whole strip of cells appeared at once every 60 m of travel (pop-in, Joep);
/// the rebuild runs on a background thread, so small steps cost no frame time.
const GRASS_RECENTRE: f32 = 15.0;
/// Loaded forest cells recenter independently of ground and shadow streaming.
const TREES_RECENTRE: f32 = 300.0;
/// Mountain shadows past the shadow maps come from a heightfield this many
/// metres square at this spacing, rebuilt once the camera has moved
/// `SHADOW_RECENTRE` from its centre.
const SHADOW_SIZE: f32 = 20000.0;
const SHADOW_CELL: f32 = 20.0;
const SHADOW_RECENTRE: f32 = 4000.0;
/// Loaded grass-cell domain, not source LOD distance. GPU chooses all cuts.
const GRASS_RADIUS: f32 = 240.0;
/// Grass cells tile their 1.5 m step exactly. At 1.15 (against bare seams
/// of the old 3x3 lattice) the overlap doubled the blades along every seam:
/// lines from the air. Jittered patches (cook_meadow_cells) cover the seams.
const GRASS_CELL_OVERLAP: f32 = 1.0;

type Built = Result<(Scene, Lighting, procedural_terrain::TerrainSpec, Option<Vec3>), String>;
/// A scene from the builder thread, with its renderer when the thread had
/// the GPU (every rebuild after the first): the swap then costs nothing on
/// the frame instead of ~300 ms of uploads.
type Ready = Result<(Option<(render::Renderer, ProceduralTerrain)>, Scene, Lighting, procedural_terrain::TerrainSpec, Option<Vec3>), String>;

/// What goes into a scene besides the terrain; changing it rebuilds.
#[derive(Clone, Copy, PartialEq)]
struct World {
    grass: bool,
    trees: bool,
    /// Share of suitable ground that grows a tree, 0..1.
    tree_density: f32,
    /// Trees are placed this far around the camera (metres).
    tree_radius: f32,
    /// Bounded GPU branch residency, shared by every camera and shadow consumer.
    branch_slots: u32,
}

// Debug menu ids (app::debug::GAME_IDS and up).
const MENU: u32 = 1000;
const TREES: u32 = 1001;
const GRASS: u32 = 1002;
const TREE_DENSITY: u32 = 1003;
const TREE_RADIUS: u32 = 1004;
const CELL_PX: u32 = 1005;
const ERROR_PX: u32 = 1006;
const BRANCH_SLOTS: u32 = 1007;

struct Fly {
    seed: u32,
    style: usize,
    pos: Vec3,
    prev: Vec3,
    yaw: f32,
    pitch: f32,
    /// The last tick's turn (yaw, pitch): drawn between ticks with
    /// Frame::alpha, so turns are smooth on a 120 Hz screen.
    turn: (f32, f32),
    /// Cruise speed, metres per second.
    speed: f32,
    /// Immediate feedback while a biome rebuild runs asynchronously.
    last_action: String,
    /// Keep a fixed height above the ground, like walking with a drone.
    hug: Option<f32>,
    wheel: f32,
    /// A scene being built on another thread (style, seed or grass moved).
    building: Option<Receiver<Ready>>,
    /// Seed and style the scene in `building` is for, and when it started.
    building_for: (u32, usize), building_started: web_time::Instant,
    /// Seed and style of the scene on screen. The camera follows its ground
    /// until a new scene swaps in (F1-F8 take 30-50 s to build).
    shown: (u32, usize),
    /// Lift the camera above the new ground once the requested scene shows.
    lift_on_swap: bool,
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
    forest_at:Option<Vec3>,
    forest_building:Option<Receiver<(Vec3,Result<render::foliage_runtime::PreparedPopulation,String>)>>,
    forest_material:Option<(usize,Material)>,
    sun_building:Option<Receiver<(Vec3,render::scene::Heightfield,Vec<f32>)>>,
    /// Terrain LOD from the debug menu, kept across rebuilds.
    terrain_settings: Settings,
    terrain: Option<ProceduralTerrain>,
    light: Lighting,
    stats: Stats,
    last_updates: [f64;2],
    jumps: u32,
    fps: f32,
    worst_ms: f32,
    /// --track: a winding gravel road the land is bent to (render::world_constraints),
    /// baked for the seed and style it was made for.
    track: bool,
    /// --regions NAME: the world turns into biome NAME east of x = 0
    /// (render::regions), over about 1.5 km along a wandering border.
    regions: Option<usize>,
    /// --town: a "town" zone round the origin (render::zones) with a block
    /// of buildings on plots the land is levelled to, and no forest in it.
    town: bool,
    constraints: Option<(u32, usize, Arc<Constraints>)>,
}

impl Fly {
    fn new(seed: u32, style: usize, world: World) -> Result<Self, String> {
        let mut fly = Fly { seed, style, pos: Vec3::ZERO, prev: Vec3::ZERO, yaw: 0.6, pitch: -0.15, turn: (0.0, 0.0), speed: 40.0, last_action: "Ready".into(), hug: None, wheel: 0.0,
            building: None, building_for: (seed, style), building_started: web_time::Instant::now(), shown: (seed, style), lift_on_swap: false, gpu: None, stale: false, world, built_world: world, world_at: None, sun_building:None, forest_at:None, forest_building:None, forest_material:None, terrain_settings: Settings::default(), terrain: None, light: Lighting::default(), stats: Stats::default(), last_updates: [0.0;2], jumps: 0, fps: 0.0, worst_ms: 0.0,
            track: std::env::args().any(|a| a == "--track"), regions: None, town: std::env::args().any(|a| a == "--town"), constraints: None };
        if let Some(name) = std::env::args().collect::<Vec<_>>().windows(2).find(|w| w[0] == "--regions").map(|w| w[1].clone()) {
            fly.regions = Some(STYLES.iter().position(|n| *n == name).ok_or_else(|| format!("unknown biome {name}; one of {}", STYLES.join(", ")))?);
        }
        fly.bake_track();
        if let Some((start, yaw)) = fly.track_start() { fly.pos = start; fly.yaw = yaw; fly.pitch = -0.05; }
        if fly.track_start().is_none() { fly.pos = Vec3::new(0.0, fly.ground(0.0, 0.0) + 60.0, 0.0); }
        fly.prev = fly.pos;
        // --from: a JSON written by F12 (or just its "state"), to stand where the playtester stood.
        if let Some(path) = std::env::args().collect::<Vec<_>>().windows(2).find(|w| w[0] == "--from").map(|w| w[1].clone()) {
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))?;
            let st = v.get("state").unwrap_or(&v);
            let f = |k: &str| st.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
            if let Some(name) = st.get("style").and_then(|x| x.as_str()) { fly.style = STYLES.iter().position(|n| *n == name).ok_or("unknown style")?; }
            if let Some(seed) = st.get("seed").and_then(|x| x.as_u64()) { fly.seed = seed as u32; }
            if let Some(p) = st.get("pos").and_then(|x| x.as_array()) {
                let c: Vec<f32> = p.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
                if c.len() == 3 { fly.pos = Vec3::new(c[0], c[1], c[2]); fly.prev = fly.pos; }
            }
            if let Some(y) = f("yaw_deg") { fly.yaw = y.to_radians(); }
            if let Some(p) = f("pitch_deg") { fly.pitch = p.to_radians(); }
            if let Some(s) = f("speed") { fly.speed = s; }
        }
        fly.rebuild()?;
        Ok(fly)
    }

    fn style(&self) -> Style { environment(self.style).biome.terrain }

    /// The world's biomes as region kinds: the style's, then --regions'.
    /// Their tree densities follow the density knob (60 % by default).
    fn lands(&self) -> render::biome::Lands {
        let biomes = std::iter::once(self.style).chain(self.regions).map(|k| {
            let b = environment(k).biome;
            render::biome::Biome { tree_density: self.world.tree_density / 0.6 * b.tree_density, ..b }
        }).collect();
        render::biome::Lands { biomes }
    }
    fn ground(&self, x: f32, z: f32) -> f32 {
        // Until a requested world swaps in, the ground on screen is the old one.
        if self.shown != (self.seed, self.style) {
            return terrain_field::height(x, z, self.shown.0, &environment(self.shown.1).biome.terrain);
        }
        match self.current_track() {
            Some(w) => w.height(x, z, self.seed, &self.style()),
            None => terrain_field::height(x, z, self.seed, &self.style()),
        }
    }

    /// The track baked for the current seed and style, if any.
    fn current_track(&self) -> Option<&Arc<Constraints>> {
        self.constraints.as_ref().filter(|c| c.0 == self.seed && c.1 == self.style).map(|c| &c.2)
    }

    /// Bake the --track road and --regions for the current seed and style:
    /// a winding loop about 700 m round the origin, laid on the land at
    /// grades up to 10 %, and the second biome east of x = 0.
    fn bake_track(&mut self) {
        if (!self.track && self.regions.is_none() && !self.town) || self.current_track().is_some() { return; }
        let (seed, style) = (self.seed, self.style());
        let regions = self.regions.map(|other| render::regions::Regions::split([style, environment(other).biome.terrain], Vec2::ZERO, 16_000.0,
            Vec2::X, 1_500.0, 600.0, seed));
        let mut tracks = Vec::new();
        if self.track {
            let mut track = Track::wander_loop(Vec2::ZERO, 700.0, 3.5, seed);
            let land = |x: f32, z: f32| regions.as_ref().map_or(style, |r| r.style_at(x, z));
            track.fit_heights(|x, z| terrain_field::height(x, z, seed, &land(x, z)), 0.10, 120.0);
            tracks.push(track);
        }
        let (mut zones, mut footprints) = (Vec::new(), Vec::new());
        if self.town {
            let outline = (0..7).map(|k| {
                let a = k as f32 / 7.0 * std::f32::consts::TAU;
                Vec2::from_angle(a) * (420.0 + 90.0 * (2.0 * a + 1.0).sin())
            }).collect();
            zones.push(render::zones::Zone::new("town", outline, 120.0).density(0.6).age(0.4));
            // Plots on a turned grid, each levelled at its own land height.
            let turn = Vec2::from_angle(0.35);
            for j in -2..=2 {
                for i in -3..=3 {
                    let c = turn.rotate(Vec2::new(i as f32 * 70.0, j as f32 * 55.0));
                    footprints.push(render::world_constraints::Footprint::rect(c, Vec2::new(30.0, 18.0), 0.35, 6.0, None).bare(1.0));
                }
            }
        }
        let world = WorldConstraints { tracks, regions, footprints, zones, ..Default::default() }.bake(seed, &style);
        self.constraints = Some((self.seed, self.style, Arc::new(world)));
    }

    /// Above the start of the track, and the yaw looking along it.
    fn track_start(&self) -> Option<(Vec3, f32)> {
        let w = self.current_track().filter(|_| self.track)?;
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
        self.building_for = (self.seed, self.style);
        self.building_started = web_time::Instant::now();
        let track = self.current_track().cloned();
        let (seed, style, pos, lands) = (self.seed, self.style(), self.pos, self.lands());
        // Each biome has its palette, turned by the season (render::biome).
        let env = environment(self.style);
        let palette = env.vegetation();
        // The density knob (60 % by default) scales the biome's own.
        let world = World { tree_density: self.world.tree_density / 0.6 * environment(self.style).biome.tree_density, ..self.world };
        self.built_world = World { grass: world.grass, ..self.world };
        self.forest_building=None;self.sun_building=None;
        let (tx, rx) = channel();
        let (gpu, settings) = (self.gpu.clone(), self.terrain_settings);
        background(move || {
            let started = web_time::Instant::now();
            let ready = build_scene(seed, style, &lands, &env, &palette, sun_degrees(&env), pos, world, track).map(|(scene, light, pool, at)| {
                let scene_ms = started.elapsed().as_secs_f64() * 1000.0;
                let renderer_started = web_time::Instant::now();
                let built = gpu.map(|gpu| renderer_for(&gpu, &scene, &light, pool.clone(), settings));
                eprintln!("terrain stream: scene_cpu_ms={scene_ms:.2} renderer_build_wall_ms={:.2}", renderer_started.elapsed().as_secs_f64()*1000.0);
                (built, scene, light, pool, at)
            });
            let _ = tx.send(ready);
        });
        self.building = Some(rx);
        Ok(())
    }

    /// The scene's grass or trees no longer fit the settings or no longer
    /// cover what the camera sees.
    fn world_stale(&self) -> bool {self.world!=self.built_world}

    /// Somewhere new on a spiral out from the origin: high ground with a view.
    fn jump(&mut self) {
        self.jumps += 1;
        let a = self.jumps as f32 * 2.399;
        let r = 1500.0 * (self.jumps as f32).sqrt();
        let (x, z) = (r * a.cos(), r * a.sin());
        self.pos = Vec3::new(x, self.ground(x, z) + 120.0, z);
        self.prev = self.pos;
    }

    /// The camera between the last two ticks (`alpha` 0..1).
    fn camera(&self, pos: Vec3, alpha: f32) -> Camera {
        let back = 1.0 - alpha;
        let forward = look(self.yaw - self.turn.0 * back, self.pitch - self.turn.1 * back);
        Camera { pos, target: pos + forward, fov_y: 65f32.to_radians(), near: 0.05 }
    }

    fn forward(&self) -> Vec3 { look(self.yaw, self.pitch) }
}

/// Work off the frame: on its own thread natively. A browser build has no
/// threads (and GPU objects cannot cross them there), so it runs at once.
#[cfg(not(target_arch = "wasm32"))]
fn background(work: impl FnOnce() + Send + 'static) { std::thread::spawn(work); }
#[cfg(target_arch = "wasm32")]
fn background(work: impl FnOnce() + 'static) { work() }

fn look(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

impl Game for Fly {
    fn local_players(&self) -> usize { 1 }

    // This is a renderer inspection demo: changing resolution or density while
    // judging LOD hides failures. Keep the requested quality stable by default.
    fn fixed_quality(&self) -> bool { true }

    fn pointer(&mut self, pointer: Pointer, _size: [u32; 2]) { self.wheel += pointer.wheel; }

    fn tick(&mut self, c: &[Controls]) {
        let dt = 1.0 / TICK_HZ;
        let c: Controls = c[0];
        self.prev = self.pos;
        let pitch = (self.pitch + c.look[1]).clamp(-1.5, 1.5);
        self.turn = (-c.look[0], pitch - self.pitch);
        self.yaw -= c.look[0];
        self.pitch = pitch;
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
        self.last_action = match n {
            1 | 2 | 4 | 8 => format!("F{n}: {} requested", STYLES[self.style]),
            5 => format!("F5: seed {} requested", self.seed),
            6 => format!("F6: jumped to x {:.0}, z {:.0}", self.pos.x, self.pos.z),
            7 => format!("F7: ground hug {}", if self.hug.is_some() { "on" } else { "off" }),
            _ => unreachable!(),
        };
        if let Err(e) = result { self.last_action = format!("F{n}: {e}"); }
        println!("terrain input: {}", self.last_action);
        // Rise above the new ground only once it shows (lift_on_swap).
        if matches!(n, 1 | 2 | 4 | 5 | 8) { self.lift_on_swap = true; }
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
            let (prebuilt, scene, light, pool, world_at) = built??;
            self.forest_at=world_at;self.forest_building=None;self.sun_building=None;
            self.forest_material=scene.materials.iter().find(|m|m.flags&TERRAIN!=0 && m.scale<0.)
                .map(|m|{let i=(-m.scale-1.).round()as usize+2;(i,scene.materials[i])});
            match prebuilt {
                Some((renderer, terrain)) => { frame.set_renderer(renderer); self.terrain = Some(terrain); }
                None => { frame.set_scene(&scene, &light); self.terrain = Some(ProceduralTerrain::new(pool, self.terrain_settings)); }
            }
            self.light = light;
            self.world_at = world_at;
            self.shown = self.building_for;
            if self.lift_on_swap && self.shown == (self.seed, self.style) {
                self.lift_on_swap = false;
                let floor = self.ground(self.pos.x, self.pos.z);
                self.pos.y = self.pos.y.max(floor + 30.0);
                self.prev = self.pos;
            }
            if std::mem::take(&mut self.stale) { self.rebuild()?; }
        }
        if self.building.is_none() && self.world_stale() { self.rebuild()?; }
        if self.building.is_none() && (self.built_world.trees || self.built_world.grass) {
            if let Some((centre,roots))=self.forest_building.as_ref().and_then(|rx|rx.try_recv().ok()) {
                if let Some(renderer)=frame.renderer() {
                    let started=web_time::Instant::now();
                    let stats=renderer.commit_gpu_forest_roots(frame.gpu,roots?)?;
                    if let Some((index,mut material))=self.forest_material {material.tint[2]=centre.x;material.roughness=centre.z;renderer.update_materials(frame.gpu,index,&[material])?;self.forest_material=Some((index,material));}
                    eprintln!("forest stream: {} enqueue_cpu_ms={:.2}",serde_json::to_string(&stats).unwrap(),started.elapsed().as_secs_f64()*1000.);
                    self.forest_at=Some(centre);self.forest_building=None;
                }
            }
            if self.forest_building.is_none() && self.forest_at.is_none_or(|p|(p-self.pos).with_y(0.).length()>if self.built_world.grass {GRASS_RECENTRE} else {TREES_RECENTRE}) {
                let Some(renderer)=frame.renderer() else {return Err("forest streaming has no renderer".into());};
                let preparation=renderer.gpu_forest_population_seed()?;
                let (tx,rx)=channel();let (seed,style,centre)=(self.seed,self.style(),self.pos);
                let biome=environment(self.style).biome;let world=World {tree_density:self.world.tree_density/0.6*biome.tree_density,..self.world};let track=self.current_track().cloned();let lands=self.lands();
                background(move||{let roots=forest(seed,&style,&lands,centre,world,track.as_deref());let _=tx.send((centre,preparation.prepare(&roots)));});
                self.forest_building=Some(rx);
            }
        }
        if self.building.is_none() {
            if let Some((centre,heightfield,sun))=self.sun_building.as_ref().and_then(|rx|rx.try_recv().ok()) {
                let gpu=frame.gpu.clone();
                if let Some(renderer)=frame.renderer_mut(){renderer.update_baked_terrain_sun(&gpu,heightfield,&sun)?;}
                self.world_at=Some(centre);self.sun_building=None;
            }
            if self.sun_building.is_none() && self.world_at.is_none_or(|p|(p-self.pos).with_y(0.).length()>SHADOW_RECENTRE) {
                let (tx,rx)=channel();let (seed,style,centre,sun)=(self.seed,self.style(),self.pos,self.light.sun_dir);let track=self.current_track().cloned();
                background(move||{let (field,visibility)=procedural_terrain::sun_heightfield(seed,&style,track.as_deref(),[centre.x,centre.z],SHADOW_SIZE,SHADOW_CELL,sun);let _=tx.send((centre,field,visibility));});
                self.sun_building=Some(rx);
            }
        }
        let pos = self.prev.lerp(self.pos, frame.alpha);
        let cam = self.camera(pos, frame.alpha);
        let [w, h] = frame.size;
        // Sharp sun shadow reaches about four times the height above the
        // ground, in steps of sqrt(2) (each step resizes the map), at most out
        // to the trees: from 600 m up its edge read as a ring in the forest.
        if let Some(renderer)=frame.renderer() {
            let above=(pos.y-self.ground(pos.x,pos.z)).max(0.0);
            let steps=(2.0*(4.0*above/180.0).max(1.0).log2()).ceil();
            renderer.set_shadow_reach((180.0*2f32.powf(steps/2.0)).min(self.world.tree_radius.min(SHADOW_REACH_MAX).max(180.0)));
            // Centre the sun maps on the ground under the camera, not on the
            // camera: seen from the sun, a map round the eye lands height /
            // tan(sun elevation) downsun on the ground, so 16 m up the sharp
            // near map missed the ground below and the closest shadows blurred.
            let foot=Vec3::new(pos.x,pos.y-above,pos.z);
            let _=renderer.set_lighting_focus(&[foot]);
        }
        if let (Some(terrain), Some(renderer)) = (self.terrain.as_mut(), frame.renderer()) {
            let started=web_time::Instant::now();
            self.stats = terrain.update(frame.gpu, renderer, &[TerrainView::new(&cam, w, h)])?;
            self.last_updates=[started.elapsed().as_secs_f64()*1000.0,0.0];
        }
        // Scene frames only: with the debug menu open the HUD redraws faster.
        self.fps = frame.scene_fps;
        self.worst_ms = frame.scene_worst_ms;

        let s = self.sample(pos.x, pos.z);
        let above = pos.y - s.pos.y;
        let lines = [
            format!("{}  seed {}  {:.0} fps  worst {:.0} ms", STYLES[self.style], self.seed, self.fps, self.worst_ms),
            format!("x {:.0}  z {:.0}  height {:.0} m  {:.0} m above ground", pos.x, pos.z, pos.y, above),
            format!("speed {:.0} m/s{}", self.speed, if self.hug.is_some() { "  ground hug" } else { "" }),
            format!("patches {}  overflow {}  finest {}  lod x{:.2}", self.stats.patches, self.stats.overflow, self.stats.finest_level, self.stats.lod_scale),
            format!("ground: rock {:.2}  scree {:.2}  snow {:.2}  water {:.2}  slope {:.0}°", s.rock, s.scree, s.snow, s.water, s.up.clamp(0.0, 1.0).acos().to_degrees()),
            format!("{}{}", self.last_action, if self.building.is_some() {
                format!(" - building {} seed {}: {:.0} s (takes 30-50 s){}", STYLES[self.building_for.1], self.building_for.0,
                    self.building_started.elapsed().as_secs_f32(), if self.stale { ", then the latest request" } else { "" })
            } else { String::new() }),
            "F1 default  F2 lowland  F3 stats  F4 alpine  F8 next biome  F5 seed  F6 jump  F7 hug  wheel speed  F12 screenshot".to_string(),
        ];
        let mut hud = HudDrawList::new(frame.size);
        // --no-hud: captures that are compared pixel by pixel.
        let lines = if std::env::args().any(|a| a == "--no-hud") { Vec::new() } else { lines.to_vec() };
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
            number(BRANCH_SLOTS, "GPU branch budget", f64::from(w.branch_slots), (256.0, 131072.0, 1024.0, 0), "", "Shared resident branch capacity; the GPU selects detail for all trees."),
            number(TREE_RADIUS, "Tree radius", f64::from(w.tree_radius), (200.0, 3000.0, 100.0, 0), "m", "Trees are placed this far around the camera."),
            toggle(GRASS, "Grass in scene", w.grass, "Streams shared articulated grass cells; GPU selects source or voxel detail. The Grass menu only hides it."),
            number(CELL_PX, "Terrain cell size", f64::from(t.cell_pixels), (4.0, 96.0, 4.0, 0), "px", "Largest projected terrain cell before a patch splits."),
            number(ERROR_PX, "Terrain error", f64::from(t.error_pixels), (0.25, 12.0, 0.25, 2), "px", "Largest projected height error before a patch splits."),
        ])
    }

    fn debug_action(&mut self, action: DebugAction) {
        match action {
            DebugAction::ResetDefaults => { self.world = World::default(); self.terrain_settings = Settings::default(); }
            DebugAction::SetToggle { id: TREES, value } => self.world.trees = value,
            DebugAction::SetToggle { id: GRASS, value } => self.world.grass = value,
            DebugAction::SetNumber { id: TREE_DENSITY, value } => self.world.tree_density = (value / 100.0).clamp(0.05, 1.0) as f32,
            DebugAction::SetNumber { id: BRANCH_SLOTS, value } => self.world.branch_slots = value.clamp(256.0, 131072.0) as u32,
            DebugAction::SetNumber { id: TREE_RADIUS, value } => self.world.tree_radius = value.clamp(200.0, 3000.0) as f32,
            DebugAction::SetNumber { id: CELL_PX, value } => self.terrain_settings.cell_pixels = value.clamp(4.0, 96.0) as f32,
            DebugAction::SetNumber { id: ERROR_PX, value } => self.terrain_settings.error_pixels = value.clamp(0.25, 12.0) as f32,
            _ => return,
        }
        if let Some(t) = &mut self.terrain { t.settings = self.terrain_settings; }
    }

    fn title(&self) -> Option<String> { Some(format!("Terrain fly: {} seed {}", STYLES[self.style], self.seed)) }

    fn debug_stats(&self) -> Vec<(String, String)> {
        let t = &self.stats;
        let above = self.pos.y - self.ground(self.pos.x, self.pos.z);
        let mut lines = vec![
            ("World".into(), format!("{} seed {}", STYLES[self.style], self.seed)),
            ("Camera".into(), format!("x {:.0} z {:.0}, {:.0} m up, {:.0} m/s", self.pos.x, self.pos.z, above, self.speed)),
            ("Terrain".into(), format!("{} patches, finest level {}, lod x{:.2}", t.patches, t.finest_level, t.lod_scale)),
            ("Streaming".into(), format!("{} built, {} freed, {} pending, {} overflow", t.built, t.freed, t.pending, t.overflow)),
            ("Terrain CPU".into(), format!("{:.2} ms quadtree update", self.last_updates[0])),
        ];
        if self.building.is_some() {
            lines.push(("Rebuilding".into(), format!("{} seed {}, {:.0} s", STYLES[self.building_for.1], self.building_for.0, self.building_started.elapsed().as_secs_f32())));
        }
        lines
    }

    fn state(&self) -> Option<String> {
        let g = self.sample(self.pos.x, self.pos.z);
        Some(serde_json::json!({
            "style": STYLES[self.style], "seed": self.seed, "pos": self.pos.to_array(),
            "trees": self.built_world.trees, "grass": self.built_world.grass, "tree_density": self.world.tree_density, "tree_radius_m": self.world.tree_radius, "branch_slots": self.world.branch_slots, "vegetation_backend": "gpu",
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
    // 32768 slots (6 November, settled pages): forest 25.2 -> 22.2 ms, near 21.0 -> 17.3,
    // mostly voxel splat walk and occluder cull; 541 of 2.2M splats over 2 px.
    fn default() -> Self { World { grass: false, trees: true, tree_density: 0.6, tree_radius: 2500.0, branch_slots: 32768 } }
}

/// Terrain textures, the patch pool, the sky, and grass and trees around
/// `centre` as `world` asks, and the shadow heightfield around it. Returns the centre.
/// Metres from floor to floor.
const STOREY: f32 = 3.2;

/// How many storeys the building on plot `k` has: mostly low, some mid-rise,
/// a few towers, so that they hide different amounts of what stands behind.
fn storeys(k: usize, seed: u32) -> u32 {
    let cell = render::placement::Cell::new(k as i32, 0, seed);
    let (kind, pick) = (cell.hash(91), cell.hash(92));
    let (low, high) = if kind < 0.45 { (1, 3) } else if kind < 0.8 { (4, 8) } else { (10, 16) };
    low + (pick * (high - low + 1) as f32) as u32
}

/// A plain block with a floor slab and a ribbon of windows on each storey on
/// every --town plot. The solid body is one box, so it hides what is behind it.
fn buildings(scene: &mut Scene, world: &Constraints, seed: u32, style: &Style) {
    let mut paint = |tint: [f32; 3], roughness: f32| {
        let material = scene.add_material(Material { tint, layer: -1, roughness, flags: 0, scale: 1.0, metal: 0.0 });
        scene.add_mesh(&render::primitives::cube(material).mesh())
    };
    let walls = [paint([0.55, 0.33, 0.24], 0.9), paint([0.62, 0.58, 0.5], 0.9), paint([0.34, 0.36, 0.4], 0.8)];
    let (slab, glass, roof) = (paint([0.7, 0.7, 0.68], 0.8), paint([0.06, 0.09, 0.12], 0.15), paint([0.2, 0.2, 0.21], 0.95));
    // Opaque buildings hide forest before it is selected, like the land and
    // any other rigid opaque geometry (frame.rs, "foliage seed"). The category
    // is only a label; --no-building-occlusion (in main) is the A/B switch.
    let category = GeometryCategory::Structures;
    let mut block = |mesh: usize, at: Vec2, yaw: f32, bottom: f32, size: Vec3| {
        let centre = Vec3::new(at.x, bottom + size.y * 0.5, at.y);
        scene.add_classified_instance(mesh, glam::Affine3A::from_scale_rotation_translation(size, glam::Quat::from_rotation_y(-yaw), centre), category);
    };
    for (k, plot) in world.shapes.chunks_exact(4).enumerate() {
        let at = plot.iter().sum::<Vec2>() / 4.0;
        let (along, across) = (plot[1] - plot[0], plot[3] - plot[0]);
        let yaw = along.y.atan2(along.x);
        let (wide, deep) = (along.length() - 6.0, across.length() - 5.0);
        let floors = storeys(k, seed);
        let ground = world.height(at.x, at.y, seed, style);
        let top = ground + floors as f32 * STOREY;
        block(walls[k % walls.len()], at, yaw, ground - 1.0, Vec3::new(wide, top - ground + 1.0, deep));
        for floor in 0..floors {
            let level = ground + floor as f32 * STOREY;
            block(glass, at, yaw, level + 1.0, Vec3::new(wide + 0.1, 1.4, deep + 0.1));
            block(slab, at, yaw, level + 2.9, Vec3::new(wide + 0.5, 0.3, deep + 0.5));
        }
        block(roof, at, yaw, top, Vec3::new(wide + 0.5, 0.5, deep + 0.5));
    }
}

fn build_scene(seed: u32, style: Style, lands: &render::biome::Lands, env: &render::biome::Environment, palette: &VegetationStyle, sun: (f32, f32), centre: Vec3, world: World, track: Option<Arc<Constraints>>) -> Built {
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets");
    let root = assets.join("textures");
    let mut scene = Scene::default();
    // Layers in the order terrain_surface.wgsl reads them.
    let t0 = scene.add_texture(&root.join("grass_ground_albedo.jpg"))?;
    scene.add_texture(&root.join("forest_leaves_02_albedo.jpg"))?;
    scene.add_texture(&root.join("forest_ground_04_albedo.jpg"))?;
    scene.add_texture(&root.join("rock_face_03_albedo.jpg"))?;
    scene.add_texture_with_normal_encoding(&root.join("gravel_ground_01_albedo.jpg"), render::scene::NormalEncoding::SrgbTransferred)?;
    scene.add_texture(&root.join("mossy_rock_albedo.jpg"))?;
    let material = scene.add_material(Material { tint: [1.0; 3], layer: t0, roughness: 0.9, flags: TERRAIN | SURFACE, scale: 4.0, metal: 0.0 });
    if world.trees || world.grass {
        let roots = forest(seed, &style, lands, centre, world, track.as_deref());
        recipe_spruces(&mut scene, lands, palette.meadow_dry_season, world, &roots, world.branch_slots)?;
    }
    let mut pool = procedural_terrain::reserve(&mut scene, POOL, material, seed, style);
    // Beyond the trees the terrain draws the same forest as colour, out to
    // the horizon.
    procedural_terrain::vegetation_style(&mut scene, material, palette);
    // --regions: each region's ground in its own biome's colours.
    if let Some(regions) = track.as_ref().and_then(|w| w.regions.as_ref()) {
        let grounds: Vec<_> = lands.biomes.iter().map(|b| render::biome::Environment { biome: *b, ..*env }.vegetation().ground).collect();
        procedural_terrain::region_ground(&mut scene, material, regions, &grounds);
    }
    // Drawn trees hand over to the painted forest in the outer 15 % of their
    // radius, tree by tree (forest_field::handover_keep), so the forest share
    // holds across the ring.
    if world.trees {
        let (start, end) = handover_band(world);
        procedural_terrain::far_forest_handover(&mut scene, material, [centre.x, centre.z], start, end, world.tree_density, &palette.forest);
    }
    // Past the grass disc the ground takes the blades' mean colour (measured
    // against the bare ground at the disc edge; KERN_GRASS_PROXY=r,g,b to tune).
    if world.grass {
        let tint=std::env::var("KERN_GRASS_PROXY").ok().and_then(|v|{
            let p:Vec<f32>=v.split(',').filter_map(|x|x.trim().parse().ok()).collect();
            (p.len()==3).then(||[p[0],p[1],p[2]])}).unwrap_or([0.6,0.85,0.5]);
        procedural_terrain::grass_proxy(&mut scene, material, GRASS_RADIUS, tint);
    }
    // The --track road: the land bends to it, and its pixels draw gravel.
    if let Some(w) = track.clone() {
        if !std::env::args().any(|a| a == "--no-buildings") { buildings(&mut scene, &w, seed, &style); }
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
    (scene.heightfield, scene.terrain_sun) = procedural_terrain::sun_heightfield(seed, &style, track.as_deref(), [centre.x, centre.z], SHADOW_SIZE, SHADOW_CELL, light.sun_dir);
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
    let low = std::env::args().collect::<Vec<_>>().windows(2).any(|w| w[0] == "--profile" && w[1] == "low");
    light.taa = std::env::args().collect::<Vec<_>>().windows(2).find(|w| w[0] == "--taa").and_then(|w| w[1].parse().ok()).unwrap_or(if low { 0.0 } else { 1.0 });
    light.taa_blend = 0.04;
    light.sharpen = 0.2;
    // --light key=value[,key=value]: Lighting::set, for look and TAA tuning.
    for kv in std::env::args().collect::<Vec<_>>().windows(2).filter(|w| w[0] == "--light").flat_map(|w| w[1].split(',').map(String::from).collect::<Vec<_>>()) {
        if let Err(e) = light.set(&kv) { eprintln!("--light {kv}: {e}"); }
    }
    Ok((scene, light, pool, Some(centre)))
}

/// The renderer for `scene`, built on the scene thread: the swap then
/// costs the frame nothing instead of ~300 ms of uploads. (Also warming it
/// up with a first terrain update and a small frame did not measurably help
/// and competed with the live frames for the GPU.)
fn renderer_for(gpu: &render::Gpu, scene: &Scene, light: &Lighting, pool: procedural_terrain::TerrainSpec, settings: Settings) -> (render::Renderer, ProceduralTerrain) {
    (render::Renderer::new(gpu, scene, light), ProceduralTerrain::new(pool, settings))
}

/// Shared recipes are cooked once per set of biomes, independently of world population.
/// --pine-ages: the forest is replaced by one Jeffrey pine per age, in a row
/// north of the start, grown by render::plant_growth.
const PINE_AGES: [u32; 9] = [3, 6, 12, 25, 50, 80, 120, 200, 300];
fn pine_ages() -> bool { std::env::args().any(|a| a == "--pine-ages") }

/// How many tree and shrub variants the forest uses (grass follows them).
fn flora_count(lands: &render::biome::Lands) -> usize { if pine_ages() { PINE_AGES.len() } else { lands.species().len() } }

fn cooked_flora(lands: &render::biome::Lands) -> Result<&'static Vec<CookedPlant>, String> {
    if pine_ages() {
        static GROWN: std::sync::OnceLock<Result<Vec<CookedPlant>, String>> = std::sync::OnceLock::new();
        let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
        return GROWN.get_or_init(|| {
            let species = render::plant_growth::Species::jeffrey_pine();
            let variants: Vec<_> = PINE_AGES.iter().map(|&age| (species, age, 1)).collect();
            let cooked = recipe_forest::cook_grown(&variants, &render::plant_growth_assembly::Parts::long_needle_pine(),
                render::plant_growth_assembly::Look::jeffrey_pine(), Some(&cache))?;
            for (c, age) in cooked.iter().zip(PINE_AGES) {
                eprintln!("jeffrey pine age {age}: {} in {:.0} ms", if c.loaded { "loaded" } else { "cooked" }, c.millis);
            }
            Ok(cooked.into_iter().map(|c| c.plant).collect())
        }).as_ref().map_err(|e| e.clone());
    }
    type Flora = Vec<CookedPlant>;
    static COOKED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, &'static Flora>>> = std::sync::OnceLock::new();
    let mut map = COOKED.get_or_init(Default::default).lock().unwrap();
    let key = lands.biomes.iter().map(|b| b.name).collect::<Vec<_>>().join("+");
    if let Some(&flora) = map.get(&key) { return Ok(flora); }
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
    let species = lands.species();
    let recipes: Vec<_> = species.iter().map(|(_, r)| (*r, 1)).collect();
    let cooked = recipe_forest::cook_variants(&recipes, Some(&cache))?;
    for (c, (name, _)) in cooked.iter().zip(&species) {
        eprintln!("{name}: {} in {:.0} ms", if c.loaded { "loaded" } else { "cooked" }, c.millis);
    }
    let flora: &'static Flora = Box::leak(Box::new(cooked.into_iter().map(|c| c.plant).collect()));
    map.insert(key, flora);
    Ok(flora)
}

// Quality is a rendering requirement, not a performance pressure valve.
fn vegetation_quality_policy()->render::aggregate_lod::Policy {
    render::aggregate_lod::Policy {max_error_pixels:1.,allow_sampled_optics:true,
        measured_source_cuts:false,proxy_pixels:1.,proxy_pixels_from:0.,..Default::default()}
}

fn recipe_spruces(scene:&mut Scene,lands:&render::biome::Lands,season:f32,world:World,roots:&[WorldRoot],branch_slots:u32)->Result<(),String> {
    let cooked=if world.trees {Some(cooked_flora(lands)?)} else {None};
    let cache=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
    let grass=if world.grass {recipe_forest::cook_meadow_cells(&GRASS_VARIANT_SEEDS.map(|seed|(seed,season)),Some(&cache))?} else {Vec::new()};
    for (i,c) in grass.iter().enumerate() {
        eprintln!("grass cell {i}: {} in {:.1} ms, {} placements, {} branches, {} mesh bindings",if c.loaded {"loaded"} else {"cooked"},c.millis,c.plant.stats().placements,c.plant.stats().branches,c.plant.stats().primitive_meshes);
    }
    let species=cooked.into_iter().flat_map(|c|c.iter()).map(|plant|render::foliage_runtime::Species {plant,category:render::scene::GeometryCategory::Trees,deformation:render::foliage_runtime::Deformation::Rigid})
        .chain(grass.iter().map(|c|render::foliage_runtime::Species {plant:&c.plant,category:render::scene::GeometryCategory::Grass,deformation:render::foliage_runtime::Deformation::BasalShear})).collect::<Vec<_>>();
    let policy=vegetation_quality_policy();
    if std::env::args().any(|a|a=="--validate-world") {
        for (index,species) in species.iter().enumerate() {
            println!("vegetation selection evidence {index}: {}",serde_json::to_string(&species.plant.selection_evidence(policy)).map_err(|e|e.to_string())?);
        }
    }
    // Reserve the complete grass domain even when the initial location is
    // bare. Entering a meadow must not overflow an initially tiny population.
    let grass_capacity=if world.grass {((2.*GRASS_RADIUS/recipe_forest::MEADOW_CELL_SIZE).ceil()as usize+3).pow(2)} else {0};
    let capacity=(roots.len()*2).max(grass_capacity).max(16384);
    render::foliage_runtime::plant_species_population(scene,&species,roots,capacity,branch_slots,policy)?;
    // Budget override for reproducible pressure tests; the same cache path is
    // used at every size, including normal gameplay.
    let args=std::env::args().collect::<Vec<_>>();
    if let Some(value)=args.windows(2).find(|a|a[0]=="--splat-budget") {
        let budget=value[1].parse::<u32>().ok().filter(|n|*n>0).ok_or("--splat-budget needs a positive u32")?;
        scene.gpu_forest.as_mut().unwrap().raster_budget=budget;
        println!("Requested vegetation splat budget per consumer class: {budget}; bounded by GPU record capacity");
    }
    if let Some(value)=args.windows(2).find(|a|a[0]=="--voxel-cache-mib") {
        let mib=value[1].parse::<u32>().map_err(|_|"--voxel-cache-mib needs a positive integer")?;
        let bytes=mib.checked_mul(1024*1024).filter(|n|*n>0).ok_or("voxel cache budget overflow/zero")?;
        // Cells are 128 bytes packed; grow the index and splat arenas with them
        // (the defaults are sized for 256 MiB of cells).
        let budget=&mut scene.gpu_forest.as_mut().unwrap().page_budget;
        let scale=|n:u32|(u64::from(n)*u64::from(mib)/256).clamp(1,u64::from(u32::MAX))as u32;
        budget.cells=bytes/128;budget.index_words=scale(budget.index_words);budget.splats=scale(budget.splats);
    }
    if let Some(value)=args.windows(2).find(|a|a[0]=="--source-cache-mib") {
        let mib=value[1].parse::<u32>().map_err(|_|"--source-cache-mib needs a positive integer")?;
        let bytes=mib.checked_mul(1024*1024).filter(|n|*n>0).ok_or("source cache budget overflow/zero")?;
        scene.gpu_forest.as_mut().unwrap().source_budget.vertices=bytes/32;
    }
    println!("GPU vegetation: {} world roots, {capacity} reserved roots, {branch_slots} shared branch slots",roots.len());Ok(())
}

/// Trees within `world.tree_radius` of `centre`, from the engine's forest
/// field (the distant terrain draws the same forest). Position, yaw, scale
/// and variant.
fn forest(seed: u32, style: &Style, lands: &render::biome::Lands, centre: Vec3, world: World, track: Option<&Constraints>) -> Vec<WorldRoot> {
    use render::placement::{self, TerrainWorld};
    let (start, end) = handover_band(world);
    let c = Vec2::new(centre.x, centre.z);
    // Bent to the --track road; the rules keep off it (placement::off_road).
    let land = TerrainWorld { seed, style, constraints: track };
    // Each region grows its own biome's flora (render::biome::Lands).
    // No forest in a town (--town: render::zones).
    let (trees, shrubs): (Vec<_>, Vec<_>) = placement::thinned(lands.rules(), "zone:town").into_iter().partition(|r| placement::base_rule(r.id) == placement::TREE);
    // Trees out to the painted forest, handed over to it tree by tree;
    // shrubs only near, where a knee-high bush is more than a pixel.
    // --overrides FILE (render::overrides): removed, moved and added objects
    // by id. Pinned trees stand whatever the handover to the far forest says.
    // --write-overrides FILE: an example edit around the start, written out
    // and used: the 30 trees nearest the camera go, the next one moves 6 m
    // and a row of five trees is added 20 m north.
    if let (None,Some(path))=(OVERRIDES.get(),std::env::args().skip_while(|a|a!="--write-overrides").nth(1)) {
        let mut near=placement::place(&trees,&land,c,120.0);
        near.sort_by(|a,b|a.pos.distance_squared(centre).total_cmp(&b.pos.distance_squared(centre)));
        let mut edits=render::overrides::Overrides::default();
        for t in near.iter().take(30) {edits.remove(t.id);}
        if let Some(t)=near.get(30) {let mut p=render::overrides::Pinned::of(t,&land);p.x+=6.;edits.replace(t.id,p);}
        if let Some(t)=near.first() {
            for k in 0..5 {let mut p=render::overrides::Pinned::of(t,&land);p.x=c.x-10.+5.*k as f32;p.z=c.y-20.;p.lift=0.;edits.add(p);}
        }
        if let Err(e)=std::fs::write(&path,edits.to_text()) {eprintln!("--write-overrides {path}: {e}");}
        else {eprintln!("overrides: wrote {} edits to {path}",edits.edits.len());}
        let _=OVERRIDES.set(edits);
    }
    let edits=OVERRIDES.get().filter(|o|!o.is_empty());
    let edited=|placed:Vec<placement::Instance>,rules:&[placement::Rule],radius:f32|match edits {Some(o)=>o.apply(placed,rules,&land,c,radius),None=>placed};
    let woody=if world.trees {edited(placement::place(&trees, &land, c, world.tree_radius),&trees,world.tree_radius).into_iter()
        .filter(|t| edits.is_some_and(|o|o.edits.contains_key(&t.id)) || render::forest_field::handover_keep(t.pos, c, start, end))
        .chain(edited(placement::place(&shrubs, &land, c, world.tree_radius.min(SHRUB_RADIUS)),&shrubs,world.tree_radius.min(SHRUB_RADIUS)))
        .map(|t|WorldRoot {id:t.id,placement:RootPlacement {up:Vec3::Y,pos:t.pos,yaw:t.yaw,scale:t.scale.y,variant:t.variant as usize}}).collect()} else {Vec::new()};
    // One pine per age in a row 30 m north of the start, spaced by size.
    let woody:Vec<WorldRoot>=if world.trees && pine_ages() {
        // Anchored where the first build put it: the forest centre follows the
        // camera, and a row rebuilt round it walked along with the player.
        static ROW:std::sync::OnceLock<Vec2>=std::sync::OnceLock::new();
        // --pine-row X,Z places it in the world instead.
        let c=*ROW.get_or_init(||std::env::args().skip_while(|a|a!="--pine-row").nth(1)
            .and_then(|s|s.split_once(',').and_then(|(x,z)|Some(Vec2::new(x.parse().ok()?,z.parse::<f32>().ok()?-30.))))
            .unwrap_or(c));
        let mut x=c.x-60.;
        PINE_AGES.iter().enumerate().map(|(i,&age)|{
            let gap=2.+0.06*age as f32;x+=gap;let at=Vec2::new(x,c.y+30.);x+=gap;
            WorldRoot {id:render::placement::instance_id(2000,i as i32,0),placement:RootPlacement {up:Vec3::Y,pos:placement::World::ground(&land,at.x,at.y).pos,yaw:0.,scale:1.,variant:i}}
        }).collect()
    } else {woody};
    if !world.grass {return woody;}
    let offset=if world.trees {flora_count(lands)} else {0};
    let mut roots=woody;
    let started=web_time::Instant::now();
    let cells=grass_cells(seed,&land,|x,z|land.style_at(x,z).grass,c,offset,GRASS_RADIUS);
    eprintln!("grass world cells: count={} generation_cpu_ms={:.2} radius_m={GRASS_RADIUS}",cells.len(),started.elapsed().as_secs_f64()*1000.);
    roots.extend(cells);roots
}

/// Artist edits from --overrides FILE, read once at start.
static OVERRIDES: std::sync::OnceLock<render::overrides::Overrides> = std::sync::OnceLock::new();

/// Farthest sharp sun shadow from high up (metres). At 2500 m the default
/// style's 170k shadow-only roots overflowed the shadow splat budget even at
/// their coarsest level (scale pegged, long streaks on slopes).
const SHADOW_REACH_MAX: f32 = 1500.0;

/// Shrubs are placed this far around the camera (metres).
const SHRUB_RADIUS: f32 = 400.0;

/// Where drawn trees hand over to the painted forest (metres from the centre).
fn handover_band(world: World) -> (f32, f32) {
    (world.tree_radius * 0.85, world.tree_radius)
}

/// Stable world cells share two cooked assemblies. Terrain sampling happens
/// only on world streaming; no CPU visibility or near-source shortlist.
/// Seeds of the cooked meadow cell variants (recipe_forest::cook_meadow_cells).
const GRASS_VARIANT_SEEDS: [u64; 4] = [47, 83, 131, 197];

fn grass_cells(seed:u32,land:&impl render::placement::World,grass:impl Fn(f32,f32)->f32,centre:Vec2,variant_offset:usize,radius:f32)->Vec<WorldRoot> {
    use render::placement::Cell;
    let cell=recipe_forest::MEADOW_CELL_SIZE;
    let lo=((centre-Vec2::splat(radius))/cell).floor().as_ivec2();
    let hi=((centre+Vec2::splat(radius))/cell).ceil().as_ivec2();
    let mut roots=Vec::new();
    for j in lo.y..=hi.y {
    // Each row slides along x by its own fraction of a cell: cells still tile
    // the row, but the seams and the 0.5 m patch lattice of one row no longer
    // line up with the next, so the field shows no square grid from above.
    let shift=Cell::new(0,j,seed).hash(79)*cell;
    for i in lo.x-1..=hi.x {
        let x=(i as f32+0.5)*cell+shift;let z=(j as f32+0.5)*cell;
        let d=Vec2::new(x,z).distance(centre);
        if d>radius {continue;}
        // The outer 30 % thins out cell by cell while the ground proxy
        // (terrain_surface.wgsl grass_proxy) fades in over the same band:
        // no line where the blades end.
        if Cell::new(i,j,seed).hash(81)<((d-0.7*radius)/(0.3*radius)).clamp(0.,1.) {continue;}
        let s=land.ground(x,z);
        let sparse=0.5+0.8*terrain_field::noised(Vec2::new(x,z)/40.,seed.wrapping_add(31)).x>grass(x,z);
        if sparse || s.scree>0.8 || s.water>0.02 || land.clearance(x,z)<cell {continue;}
        let dx=land.ground(x+0.5,z).pos.y-land.ground(x-0.5,z).pos.y;
        let dz=land.ground(x,z+0.5).pos.y-land.ground(x,z-0.5).pos.y;
        // The cell tilts with the ground, so along the fall line it covers
        // only cos(slope) of the grid step: a bare strip per row on a slope
        // (Mancave, 28 degrees: 0.18 m). Scaled by 1/up.y it meets the next row.
        let up=Vec3::new(-dx,1.,-dz).normalize();
        // Rock and snow thin the grass out by a per-cell hash, not at one value.
        // (Letting grass onto mossy, up-facing rock put it over outcrops.)
        let smooth=|v:f32,a:f32,b:f32|((v-a)/(b-a)).clamp(0.,1.);
        let cut=Cell::new(i,j,seed).hash(78);
        if cut<smooth(s.rock,0.2,0.6) || cut<smooth(s.snow,0.15,0.45) {continue;}
        let hash=Cell::new(i,j,seed).hash(77);
        // A cell is a 3x3 block of patches that tiles the 1.5 m square: turn
        // it by quarter turns only. Any yaw left its corners bare, a dark hole
        // at every grid corner (Joep's regular black circles).
        let quarter=(hash*8.).floor() as u32%4;
        // Cells tile exactly: no growth, shift or free turn. Any overlap
        // doubled the blades along every seam, which read as lines from the
        // air (Joep, playtest 4); the cell's own patches are jittered at cook
        // time (recipe_forest::cook_meadow_cells), so seams look like the
        // inside of a cell.
        let turn=0.0;
        let shift=Vec2::ZERO;
        if land.clearance(x+shift.x,z+shift.y)<cell {continue;}
        // Four cooked variants, picked independently of the turn: two
        // repeated every few cells and read as a grid from above.
        let variant=((Cell::new(i,j,seed).hash(80)*GRASS_VARIANT_SEEDS.len() as f32) as usize).min(GRASS_VARIANT_SEEDS.len()-1);
        roots.push(WorldRoot {id:render::placement::instance_id(1000,i,j),placement:RootPlacement {
            pos:Vec3::new(x+shift.x,land.ground(x+shift.x,z+shift.y).pos.y-0.015,z+shift.y),up,yaw:quarter as f32*std::f32::consts::FRAC_PI_2+turn,
            scale:GRASS_CELL_OVERLAP/up.y.max(0.7),variant:variant_offset+variant}});
    }}
    roots
}

/// --diff-images A B HEAT: how far two captures differ, as JSON on stdout,
/// and a heat map of the difference (black is equal, 32 levels is full red).
/// Used by engine/tools/render-suite.py; needs no GPU or world.
fn diff_images(a:&str,b:&str,heat:&str)->Result<(),String> {
    let load=|path:&str|image::open(path).map(|i|i.to_rgb8()).map_err(|e|format!("{path}: {e}"));
    let (a,b)=(load(a)?,load(b)?);
    if a.dimensions()!=b.dimensions() {return Err(format!("sizes differ: {:?} and {:?}",a.dimensions(),b.dimensions()));}
    let (w,h)=a.dimensions();
    let (mut over4,mut over16,mut max,mut sum,mut square)=(0u64,0u64,0u8,0u64,0u64);
    let mut map=image::RgbImage::new(w,h);
    for ((pa,pb),out) in a.pixels().zip(b.pixels()).zip(map.pixels_mut()) {
        let d=(0..3).map(|c|pa[c].abs_diff(pb[c])).max().unwrap_or(0);
        over4+=u64::from(d>4);over16+=u64::from(d>16);max=max.max(d);sum+=u64::from(d);square+=u64::from(d)*u64::from(d);
        let v=(u32::from(d)*8).min(255) as u8;
        *out=image::Rgb([v,v/4,0]);
    }
    map.save(heat).map_err(|e|format!("{heat}: {e}"))?;
    // Mean SSIM on luma over 8x8 blocks.
    let luma=|p:&image::Rgb<u8>|0.2126*f64::from(p[0])+0.7152*f64::from(p[1])+0.0722*f64::from(p[2]);
    let (mut ssim,mut blocks)=(0.0,0u32);
    for y in (0..h.saturating_sub(7)).step_by(8) {for x in (0..w.saturating_sub(7)).step_by(8) {
        let (mut ma,mut mb,mut aa,mut bb,mut ab)=(0.0,0.0,0.0,0.0,0.0);
        for j in 0..8 {for i in 0..8 {
            let (u,v)=(luma(a.get_pixel(x+i,y+j)),luma(b.get_pixel(x+i,y+j)));
            ma+=u;mb+=v;aa+=u*u;bb+=v*v;ab+=u*v;
        }}
        let n=64.0;(ma,mb)=(ma/n,mb/n);
        let (va,vb,cov)=(aa/n-ma*ma,bb/n-mb*mb,ab/n-ma*mb);
        ssim+=((2.0*ma*mb+6.5025)*(2.0*cov+58.5225))/((ma*ma+mb*mb+6.5025)*(va+vb+58.5225));blocks+=1;
    }}
    let pixels=f64::from(w)*f64::from(h);
    println!("{}",serde_json::json!({"over4_pct":over4 as f64/pixels*100.,"over16_pct":over16 as f64/pixels*100.,"max":max,"mean":sum as f64/pixels,
        "rmse":(square as f64/pixels).sqrt(),"ssim":ssim/f64::from(blocks.max(1))}));
    Ok(())
}

/// Ground textures `build_scene` reads (albedo and normal maps): a browser
/// build fetches them from `assets/` next to the page before it starts.
#[cfg(target_arch = "wasm32")]
const WEB_TEXTURES: [&str; 6] = ["grass_ground", "forest_leaves_02", "forest_ground_04", "rock_face_03", "gravel_ground_01", "mossy_rock"];

/// In a browser: `?style=alpine&seed=2` picks the world, `?trees=1` adds
/// the trees (or `#alpine-trees`); the low profile.
#[cfg(target_arch = "wasm32")]
fn main() {
    app::web::install_panic_hook();
    wasm_bindgen_futures::spawn_local(async {
        let names: Vec<String> = WEB_TEXTURES.iter().flat_map(|t| [format!("{t}_albedo.jpg"), format!("{t}_normal.jpg")]).collect();
        for (i, name) in names.iter().enumerate() {
            app::web::progress(Some(0.3 + 0.3 * i as f32 / names.len() as f32), &format!("Downloading ground textures ({}/{})", i + 1, names.len()));
            match app::web::fetch(&format!("assets/{name}")).await {
                Ok(bytes) => render::files::preload(name, bytes),
                Err(e) => app::web::log(&format!("asset {e}")),
            }
        }
        let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
        let param = |k: &str| query.trim_start_matches('?').split('&').find_map(|kv| kv.split_once('=').filter(|(key, _)| *key == k).map(|(_, v)| v.to_owned()));
        // Hosts that pass only a `#name` get the same as words: `#alpine-trees`.
        let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
        let words: Vec<&str> = hash.trim_start_matches('#').split('-').collect();
        let seed = param("seed").and_then(|s| s.parse().ok()).unwrap_or(1);
        let style = param("style").or_else(|| words.iter().find(|w| STYLES.contains(w)).map(|w| w.to_string()))
            .and_then(|s| STYLES.iter().position(|n| *n == s)).unwrap_or(0);
        // Trees are opt-in here: without threads or the native cook cache,
        // preparing them blocks the page for minutes.
        let trees = param("trees").as_deref() == Some("1") || words.contains(&"trees");
        let world = World { trees, branch_slots: 512, ..World::default() };
        app::web::log(&format!("terrain fly: seed {seed}, style {}, trees {trees}", STYLES[style]));
        app::web::progress(Some(0.6), if trees { "Building the world and cooking trees (minutes, the page freezes meanwhile)" } else { "Building the world" });
        app::web::yield_to_browser().await;
        match Fly::new(seed, style, world) {
            Ok(game) => app::web::run(app::Config { title: "Terrain fly".into(), tick_hz: TICK_HZ, max_render_height: Some(720), ..Default::default() }, Box::new(game)),
            Err(e) => app::web::log(&format!("terrain fly: {e}")),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    // Normal vegetation renderer. These diagnostic overrides can disable a
    // culling stage, but never select a different CPU population backend.
    for name in ["KERN_TREE_OCCLUSION", "KERN_CLUSTER_TRAVERSAL"] {
        if std::env::var_os(name).is_none() {unsafe {std::env::set_var(name,"1");}}
    }
    if let Some(path) = std::env::args().skip_while(|a| a != "--overrides").nth(1) {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("--overrides {path}: {e}"))?;
        let edits = render::overrides::Overrides::parse(&text)?;
        eprintln!("overrides: {} edits from {path}", edits.edits.len());
        let _ = OVERRIDES.set(edits);
    }
    // A/B: only the land hides forest, as if no other geometry were there.
    if std::env::args().any(|a| a == "--no-building-occlusion") { unsafe { std::env::set_var("KERN_FOLIAGE_OCCLUDERS", "terrain"); } }
    let args: Vec<String> = std::env::args().collect();
    if let Some(w) = args.windows(4).find(|w| w[0] == "--diff-images") { return diff_images(&w[1],&w[2],&w[3]); }
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    for retired in ["--recipe", "--firs", "--cards", "--forest-layer-only"] {
        if args.iter().any(|a| a == retired) {
            return Err(format!("{retired} selected a retired CPU forest mode; use --branch-slots N for the GPU residency budget"));
        }
    }
    // Low profile changes lighting/resolution and the shared branch budget,
    // never the population backend. Historical CPU-pool timings do not apply.
    let low = arg("--profile").as_deref() == Some("low");
    if low {
        // Single-threaded here: nothing has started yet.
        if std::env::var_os("KERN_SIMPLE_LIGHTING").is_none() { unsafe { std::env::set_var("KERN_SIMPLE_LIGHTING", "1"); } }
        if std::env::var_os("WARP_RENDER_HEIGHT").is_none() { unsafe { std::env::set_var("WARP_RENDER_HEIGHT", "720"); } }
    }
    let seed = arg("--seed").map(|s| s.parse().map_err(|_| "--seed N")).transpose()?.unwrap_or(1);
    let style = arg("--style").map(|s| STYLES.iter().position(|n| *n == s).ok_or("--style default|lowland|alpine|polder|tundra|desert")).transpose()?.unwrap_or(0);
    let world = World { trees: !args.iter().any(|a| a == "--no-trees"), grass: args.iter().any(|a| a == "--grass"),
        tree_radius: arg("--tree-radius").map(|n| n.parse().map_err(|_| "--tree-radius M")).transpose()?.unwrap_or(World::default().tree_radius),
        branch_slots: arg("--branch-slots").map(|n|n.parse().map_err(|_|"--branch-slots N")).transpose()?.unwrap_or(if low {512}else{World::default().branch_slots}), ..World::default() };
    let mut game = Fly::new(seed, style, world)?;
    if let Some(at) = arg("--at") {
        let at = at.split(',').map(|n| n.parse::<f32>().map_err(|_| "--at X,Z,ABOVE,YAW,PITCH")).collect::<Result<Vec<_>, _>>()?;
        let &[x, z, above, yaw, pitch] = at.as_slice() else { return Err("--at X,Z,ABOVE,YAW,PITCH".into()); };
        game.pos = Vec3::new(x, game.ground(x, z) + above, z);
        game.prev = game.pos;
        (game.yaw, game.pitch) = (yaw, pitch);
    }
    // Explicit speed overrides captured camera state in interactive runs too.
    if let Some(speed) = arg("--speed") {
        let speed = speed.parse::<f32>().map_err(|_| "--speed needs metres per second")?;
        if !speed.is_finite() || !(1.0..=4000.0).contains(&speed) { return Err("--speed must be between 1 and 4000 m/s".into()); }
        game.speed = speed;
    }
    // Build the exact normal world/catalog without acquiring an adapter or
    // opening a window. Useful while another game owns the GPU.
    if args.iter().any(|a|a=="--validate-world") {
        let (_,scene,_,_,_)=game.building.take().ok_or("no scene build pending")?.recv().map_err(|e|e.to_string())??;
        let forest=scene.gpu_forest.as_ref();
        if world.grass && scene.meadow.is_some() {return Err("terrain grass used legacy meadow".into());}
        println!("{}",serde_json::json!({"validation":"normal_scene_cpu_only","grass":world.grass,"trees":world.trees,
            "legacy_meadow":scene.meadow.is_some(),"roots_reserved":forest.map(|f|f.roots.len()).unwrap_or(0),
            "roots_active":forest.map(|f|f.roots.iter().filter(|r|r.info[3]==0).count()).unwrap_or(0),
            "branch_slots":forest.map(|f|f.slots).unwrap_or(0),"hierarchy_nodes":forest.map(|f|f.nodes.len()).unwrap_or(0),
            "meshes":scene.meshes.len(),"source_vertices":scene.vertices.len(),"gpu_rendered":false}));
        return Ok(());
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
    // default 80), or --stationary for fixed-camera streaming convergence.
    // One tick per frame; prints frame-time percentiles, so
    // hitches from scene rebuilds show without a window.
    if let Some(n) = arg("--fly") {
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1280, 720]);
        game.speed = arg("--speed").and_then(|s| s.parse().ok()).unwrap_or(80.0);
        let upscale=arg("--upscale").unwrap_or_else(||"native".into()).parse::<render::upscale::UpscaleMode>()?;
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], TICK_HZ)?;
        let mut times = Vec::new();
        let mut cpu_times = Vec::new();
        let mut gpu_samples = Vec::new();
        let mut page_samples = Vec::new();
        let stationary=std::env::args().any(|a|a=="--stationary");
        println!("Headless camera stationary={stationary}; wind and asset streaming continue");
        let forward = Controls { mv: [0.0, if stationary {0.0}else{1.0}], ..Default::default() };
        let frames = n.parse::<u32>().map_err(|_| "--fly N")?;
        if frames <= 5 { return Err("--fly needs at least six frames".into()); }
        for i in 0..frames {
            let t = web_time::Instant::now();
            game.tick(&[forward]);
            let rgba = headless.frame(&mut game)?;
            if i >= 5 {
                times.push(t.elapsed().as_secs_f64() * 1e3);
                cpu_times.push(serde_json::json!({"frame":i,"draw":headless.last_draw_timings(),"terrain_ms":game.last_updates[0],"tree_pool_ms":game.last_updates[1],
                    "renderer_spans":headless.renderer().and_then(|r|r.cpu_spans())}));
            }
            if let Some(renderer)=headless.renderer() {
                renderer.set_upscale(&gpu,upscale)?;
                // Rebuilds may replace a Renderer at the same address. Enable
                // profiling on the actual instance, not just pointer changes.
                if let Some(stats)=renderer.foliage_page_stats(){page_samples.push(serde_json::json!({"frame":i,"cache":stats}));}
                renderer.set_async_gpu_profiling(&gpu,true)?;
                // WARP_STATS: the overlay in the shot takes the samples.
                let samples=if headless.stats.open(){Vec::new()}else{renderer.drain_gpu_profiles(&gpu).samples};
                for sample in samples {
                    gpu_samples.push(serde_json::json!({"frame":sample.render_sequence,"output":sample.output_size,"views":sample.views,
                        "covered_ms":sample.covered_interval_ms,"passes":sample.gpu_ms,
                        "upscale":renderer.upscale_status().mode.as_str(),
                        "reconstruction_views":renderer.upscale_status().views.iter().map(|v|serde_json::json!({"input":v.input,"output":v.output})).collect::<Vec<_>>()}));
                }
            }
            if i+1 == frames { if let Some(path) = arg("--fly-shot") { render::write_png(std::path::Path::new(&path), w, h, &rgba)?; } }
        }
        // --fly-stats PATH: counters and memory of the settled view, for
        // engine/tools/render-suite.py. Measured after the timed frames.
        if let Some(path) = arg("--fly-stats") {
            let stats=headless.measure(&mut game)?;
            let renderer=headless.renderer().ok_or("no renderer to measure")?;
            let voxels=renderer.voxel_diagnostics(&gpu)?.unwrap_or_default().iter().map(|s|serde_json::json!({"splats":s.raster_splats,"vertices":s.raster_vertices,"capacity":s.raster_capacity})).collect::<Vec<_>>();
            let cache=renderer.resource_cache_stats();
            let memory=gpu.device.generate_allocator_report().map(|report| {
                let mut named=std::collections::BTreeMap::<String,(u64,u32)>::new();
                for a in &report.allocations {let e=named.entry(a.name.clone()).or_default();e.0+=a.size;e.1+=1;}
                let mut top=named.into_iter().collect::<Vec<_>>();
                top.sort_by_key(|(_,(bytes,_))|std::cmp::Reverse(*bytes));
                serde_json::json!({"allocated_bytes":report.total_allocated_bytes,"reserved_bytes":report.total_reserved_bytes,"allocations":report.allocations.len(),
                    "top":top.iter().take(40).map(|(name,(bytes,count))|serde_json::json!({"name":name,"bytes":bytes,"count":count})).collect::<Vec<_>>()})
            });
            let report=serde_json::json!({"adapter":gpu.info.name,"backend":format!("{:?}",gpu.info.backend),"driver":gpu.info.driver_info,"size":[w,h],"frames":frames,
                "camera":{"clusters":stats.clusters,"triangles":stats.triangles,"submitted_vertices":stats.submitted_vertices},
                "shadow":{"clusters":stats.shadow_clusters,"triangles":stats.shadow_triangles,"submitted_vertices":stats.shadow_submitted_vertices,
                    "near_clusters":stats.near_shadow_clusters,"near_triangles":stats.near_shadow_triangles,
                    "far_clusters":stats.far_sun_clusters,"far_triangles":stats.far_sun_triangles,"far_updated":stats.far_sun_cache_updated},
                "voxel_views":voxels,"forest":renderer.gpu_forest_diagnostics(&gpu)?,"pages":renderer.foliage_page_stats(),
                "resource_cache":{"hits":cache.hits,"misses":cache.misses,"entries":cache.entries},"memory":memory});
            std::fs::write(&path,serde_json::to_vec_pretty(&report).map_err(|e|e.to_string())?).map_err(|e|format!("{path}: {e}"))?;
        }
        if let Some(renderer)=headless.renderer() {
            if args.iter().any(|a|a=="--voxel-raster-profile") {println!("Voxel raster profile: {}",serde_json::to_string(&renderer.voxel_raster_profile(&gpu)?).map_err(|e|e.to_string())?);}
            if let Some(stats)=renderer.voxel_diagnostics(&gpu)? {for (view,s) in stats.iter().enumerate(){println!("Voxel raster final: {}",serde_json::json!({"view":view,"splats":s.raster_splats,"vertices":s.raster_vertices,"capacity":s.raster_capacity}));}}
            if let Some(stats)=renderer.foliage_page_stats(){println!("GPU voxel page cache: {}",serde_json::to_string(&stats).map_err(|e|e.to_string())?);}
            if let Some(stats)=renderer.gpu_forest_diagnostics(&gpu)? {println!("GPU forest final: {}",serde_json::to_string(&stats).map_err(|e|e.to_string())?);
            let d=renderer.debug_tree_occlusion(&gpu,0,None)?;
            println!("GPU tree recovery final: {}",serde_json::json!({"history_first":d.history_first,"phase1_clusters":d.phase1_clusters,"phase2_clusters":d.phase2_clusters,"deferred_clusters":d.deferred_clusters,"deferred_capacity":d.deferred_capacity,"deferred_overflow":d.deferred_overflow,"instances_tested":d.instances_tested,"instances_rejected":d.instances_rejected,"clusters_tested":d.clusters_tested,"clusters_rejected":d.clusters_rejected}));
            }
        }
        if let Some(path) = arg("--fly-times") {
            std::fs::write(format!("{path}.gpu.json"),serde_json::to_vec(&gpu_samples).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            std::fs::write(format!("{path}.pages.json"),serde_json::to_vec(&page_samples).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            std::fs::write(format!("{path}.cpu.json"),serde_json::to_vec(&cpu_times).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            std::fs::write(path, serde_json::to_vec(&times).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        }
        times.sort_by(f64::total_cmp);
        let p = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
        let slow = times.iter().filter(|&&t| t > 3.0 * p(0.5)).count();
        println!("{w}x{h} {} frames at {} m/s: p50 {:.1} ms  p99 {:.1} ms  max {:.1} ms  {} frames over 3x p50", times.len(), game.speed, p(0.5), p(0.99), p(1.0), slow);
        if args.iter().any(|a|a=="--require-pixel-vegetation") {
            let renderer=headless.renderer().ok_or("missing renderer for pixel validation")?;
            let profile=renderer.voxel_raster_profile(&gpu)?;
            let mut violations=Vec::new();
            for (view,p) in profile.iter().enumerate() {
                let largest=p["largest_submitted_cells"].as_array().and_then(|c|c.first()).and_then(|c|c["diameter_px"].as_f64()).unwrap_or(0.);
                let missing=p["missing_records"].as_u64().unwrap_or(0);
                let crossings=p["near_plane_crossings"].as_u64().unwrap_or(0);
                if largest>1.0 || missing>0 || crossings>0 {violations.push(format!("view {view}: largest submitted cell {largest:.3}px, missing {missing}, near-plane crossings {crossings}"));}
            }
            if let Some(d)=renderer.gpu_forest_diagnostics(&gpu)? {
                if d.selected_over_target[1]>0 || d.refinement_faults!=0 || d.camera_fallback_roots>0 {
                    violations.push(format!("camera over-target {}, refinement faults {}, camera fallbacks {}",d.selected_over_target[1],d.refinement_faults,d.camera_fallback_roots));
                }
            }
            if !violations.is_empty(){return Err(format!("PIXEL VEGETATION QUALITY FAILED: {}",violations.join("; ")));}
            println!("PIXEL VEGETATION QUALITY PASS (final-frame submitted-cell check only)");
        }
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
        // Clips: --dump-warm N still frames first (pages stream in), then fly
        // forward (--dump-forward) keeping every --dump-every th frame.
        let warm = arg("--dump-warm").and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
        let every = arg("--dump-every").and_then(|n| n.parse::<u32>().ok()).unwrap_or(1).max(1);
        let forward = if args.iter().any(|a| a == "--dump-forward") { 1.0 } else { 0.0 };
        for _ in 0..warm { game.tick(&[Controls::default()]); headless.frame(&mut game)?; }
        for i in 0..n.parse::<u32>().map_err(|_| "--dump N DIR")? {
            game.tick(&[Controls { look: [pan, 0.0], mv: [0.0, forward], ..Default::default() }]);
            let rgba = headless.frame(&mut game)?;
            if i % every == 0 { render::write_png(&PathBuf::from(&dir).join(format!("{:03}.png", i / every)), w, h, &rgba)?; }
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

#[cfg(test)]
mod vegetation_tests {
    #[test]
    fn vegetation_policy_does_not_relax_pixel_error_with_distance() {
        let p=super::vegetation_quality_policy();
        assert_eq!(p.max_error_pixels,1.);assert_eq!(p.proxy_pixels,1.);assert_eq!(p.proxy_pixels_from,0.);
    }

    use super::*;
    struct Slope;
    impl render::placement::World for Slope {
        fn seed(&self)->u32 {1}
        fn ground(&self,x:f32,z:f32)->terrain_field::Sample {
            let mut s=terrain_field::eval(x,z,0.,1,&Style::default());
            s.pos=Vec3::new(x,0.2*x-0.1*z,z);s.rock=0.;s.scree=0.;s.water=0.;s.snow=0.;s
        }
        fn clearance(&self,x:f32,_z:f32)->f32 {if (2.0..4.0).contains(&x) {0.} else {100.}}
    }
    #[test]
    fn streamed_grass_cells_keep_ids_terrain_orientation_and_catalog_offset() {
        let style=Style {grass:1.,..Style::default()};
        let a=grass_cells(1,&Slope,|_,_|style.grass,Vec2::ZERO,7,12.);
        let b=grass_cells(1,&Slope,|_,_|style.grass,Vec2::new(4.,0.),7,12.);
        assert!(!a.is_empty());let mut overlap=0;
        for root in &a {
            let p=root.placement;
            assert!((p.pos.y-(0.2*p.pos.x-0.1*p.pos.z-0.015)).abs()<1e-5);
            assert!(p.up.abs_diff_eq(Vec3::new(-0.2,1.,0.1).normalize(),1e-5));
            assert!(!(2.0..4.0).contains(&p.pos.x));assert!((7..7+GRASS_VARIANT_SEEDS.len()).contains(&p.variant));
            if let Some(next)=b.iter().find(|r|r.id==root.id) {assert_eq!(root,next);overlap+=1;}
        }
        assert!(overlap>a.len()/3);
    }
}
