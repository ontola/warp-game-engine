//! Fly over the procedural terrain: a debug game for looking at the world
//! the way a player would, at any height and speed.
//!
//!   cargo run --release -p app --example terrain_fly
//!   cargo run --release -p app --example terrain_fly -- --style alpine --seed 3
//!   cargo run --release -p app --example terrain_fly -- --track   (a gravel road the land bends to)
//!   cargo run --release -p app --example terrain_fly -- --profile low   (Quest-class settings)
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
use render::scene::{Material, SURFACE, TERRAIN};
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
/// camera once the camera is this far (metres, horizontally) from it.
const GRASS_RECENTRE: f32 = 60.0;
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
    constraints: Option<(u32, usize, Arc<Constraints>)>,
}

impl Fly {
    fn new(seed: u32, style: usize, world: World) -> Result<Self, String> {
        let mut fly = Fly { seed, style, pos: Vec3::ZERO, prev: Vec3::ZERO, yaw: 0.6, pitch: -0.15, turn: (0.0, 0.0), speed: 40.0, hug: None, wheel: 0.0,
            building: None, gpu: None, stale: false, world, built_world: world, world_at: None, sun_building:None, forest_at:None, forest_building:None, forest_material:None, terrain_settings: Settings::default(), terrain: None, light: Lighting::default(), stats: Stats::default(), last_updates: [0.0;2], jumps: 0, fps: 0.0, worst_ms: 0.0,
            track: std::env::args().any(|a| a == "--track"), constraints: None };
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
        let world = World { tree_density: self.world.tree_density / 0.6 * environment(self.style).biome.tree_density, ..self.world };
        self.built_world = World { grass: world.grass, ..self.world };
        self.forest_building=None;self.sun_building=None;
        let (tx, rx) = channel();
        let (gpu, settings) = (self.gpu.clone(), self.terrain_settings);
        std::thread::spawn(move || {
            let biome = env.biome;
            let started = std::time::Instant::now();
            let ready = build_scene(seed, style, &biome, &palette, sun_degrees(&env), pos, world, track).map(|(scene, light, pool, at)| {
                let scene_ms = started.elapsed().as_secs_f64() * 1000.0;
                let renderer_started = std::time::Instant::now();
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

fn look(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos())
}

impl Game for Fly {
    fn local_players(&self) -> usize { 1 }

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
            if std::mem::take(&mut self.stale) { self.rebuild()?; }
        }
        if self.building.is_none() && self.world_stale() { self.rebuild()?; }
        if self.building.is_none() && (self.built_world.trees || self.built_world.grass) {
            if let Some((centre,roots))=self.forest_building.as_ref().and_then(|rx|rx.try_recv().ok()) {
                if let Some(renderer)=frame.renderer() {
                    let started=std::time::Instant::now();
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
                let biome=environment(self.style).biome;let world=World {tree_density:self.world.tree_density/0.6*biome.tree_density,..self.world};let track=self.current_track().cloned();
                std::thread::spawn(move||{let roots=forest(seed,&style,&biome,centre,world,track.as_deref());let _=tx.send((centre,preparation.prepare(&roots)));});
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
                let (tx,rx)=channel();let (seed,style,centre,sun)=(self.seed,self.style(),self.pos,self.light.sun_dir);
                std::thread::spawn(move||{let (field,visibility)=procedural_terrain::sun_heightfield(seed,&style,[centre.x,centre.z],SHADOW_SIZE,SHADOW_CELL,sun);let _=tx.send((centre,field,visibility));});
                self.sun_building=Some(rx);
            }
        }
        let pos = self.prev.lerp(self.pos, frame.alpha);
        let cam = self.camera(pos, frame.alpha);
        let [w, h] = frame.size;
        if let (Some(terrain), Some(renderer)) = (self.terrain.as_mut(), frame.renderer()) {
            let started=std::time::Instant::now();
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
            number(BRANCH_SLOTS, "GPU branch budget", f64::from(w.branch_slots), (256.0, 16384.0, 256.0, 0), "", "Shared resident branch capacity; the GPU selects detail for all trees."),
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
            DebugAction::SetNumber { id: BRANCH_SLOTS, value } => self.world.branch_slots = value.clamp(256.0, 16384.0) as u32,
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
    fn default() -> Self { World { grass: false, trees: true, tree_density: 0.6, tree_radius: 2500.0, branch_slots: 4096 } }
}

/// Terrain textures, the patch pool, the sky, and grass and trees around
/// `centre` as `world` asks, and the shadow heightfield around it. Returns the centre.
fn build_scene(seed: u32, style: Style, biome: &render::biome::Biome, palette: &VegetationStyle, sun: (f32, f32), centre: Vec3, world: World, track: Option<Arc<Constraints>>) -> Built {
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
        let roots = forest(seed, &style, biome, centre, world, track.as_deref());
        recipe_spruces(&mut scene, biome, palette.meadow_dry_season, world, &roots, world.branch_slots)?;
    }
    let mut pool = procedural_terrain::reserve(&mut scene, POOL, material, seed, style);
    // Beyond the trees the terrain draws the same forest as colour, out to
    // the horizon.
    procedural_terrain::vegetation_style(&mut scene, material, palette);
    // Drawn trees hand over to the painted forest in the outer 15 % of their
    // radius, tree by tree (forest_field::handover_keep), so the forest share
    // holds across the ring.
    if world.trees {
        let (start, end) = handover_band(world);
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

/// Shared recipes are cooked once per biome, independently of world population.
fn cooked_flora(biome: &render::biome::Biome) -> Result<&'static Vec<CookedPlant>, String> {
    type Flora = Vec<CookedPlant>;
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
    let flora: &'static Flora = Box::leak(Box::new(cooked.into_iter().map(|c| c.plant).collect()));
    map.insert(biome.name, flora);
    Ok(flora)
}

fn recipe_spruces(scene:&mut Scene,biome:&render::biome::Biome,season:f32,world:World,roots:&[WorldRoot],branch_slots:u32)->Result<(),String> {
    let cooked=if world.trees {Some(cooked_flora(biome)?)} else {None};
    let cache=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../build/cook-cache");
    let grass=if world.grass {recipe_forest::cook_meadow_cells(&[(47,season),(83,season)],Some(&cache))?} else {Vec::new()};
    for (i,c) in grass.iter().enumerate() {
        eprintln!("grass cell {i}: {} in {:.1} ms, {} placements, {} branches, {} mesh bindings",if c.loaded {"loaded"} else {"cooked"},c.millis,c.plant.stats().placements,c.plant.stats().branches,c.plant.stats().primitive_meshes);
    }
    let species=cooked.into_iter().flat_map(|c|c.iter()).map(|plant|render::foliage_runtime::Species {plant,category:render::scene::GeometryCategory::Trees,deformation:render::foliage_runtime::Deformation::Rigid})
        .chain(grass.iter().map(|c|render::foliage_runtime::Species {plant:&c.plant,category:render::scene::GeometryCategory::Grass,deformation:render::foliage_runtime::Deformation::BasalShear})).collect::<Vec<_>>();
    let policy=render::aggregate_lod::Policy {max_error_pixels:32.,allow_sampled_optics:true,measured_source_cuts:false,proxy_pixels:6.,proxy_pixels_from:50.,..Default::default()};
    // Reserve the complete grass domain even when the initial location is
    // bare. Entering a meadow must not overflow an initially tiny population.
    let grass_capacity=if world.grass {((2.*GRASS_RADIUS/recipe_forest::MEADOW_CELL_SIZE).ceil()as usize+3).pow(2)} else {0};
    let capacity=(roots.len()*2).max(grass_capacity).max(16384);
    render::foliage_runtime::plant_species_population(scene,&species,roots,capacity,branch_slots,policy)?;
    // Budget override for reproducible pressure tests; the same cache path is
    // used at every size, including normal gameplay.
    let args=std::env::args().collect::<Vec<_>>();
    if let Some(value)=args.windows(2).find(|a|a[0]=="--voxel-cache-mib") {
        let mib=value[1].parse::<u32>().map_err(|_|"--voxel-cache-mib needs a positive integer")?;
        let bytes=mib.checked_mul(1024*1024).filter(|n|*n>0).ok_or("voxel cache budget overflow/zero")?;
        scene.gpu_forest.as_mut().unwrap().page_budget.cells=bytes/176;
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
fn forest(seed: u32, style: &Style, biome: &render::biome::Biome, centre: Vec3, world: World, track: Option<&Constraints>) -> Vec<WorldRoot> {
    use render::placement::{self, TerrainWorld};
    let (start, end) = handover_band(world);
    let c = Vec2::new(centre.x, centre.z);
    let biome = render::biome::Biome { tree_density: world.tree_density, ..*biome };
    // Bent to the --track road; the rules keep off it (placement::off_road).
    let land = TerrainWorld { seed, style, constraints: track };
    let (trees, shrubs): (Vec<_>, Vec<_>) = biome.rules().into_iter().partition(|r| r.id == placement::TREE);
    // Trees out to the painted forest, handed over to it tree by tree;
    // shrubs only near, where a knee-high bush is more than a pixel.
    let woody=if world.trees {placement::place(&trees, &land, c, world.tree_radius).into_iter()
        .filter(|t| render::forest_field::handover_keep(t.pos, c, start, end))
        .chain(placement::place(&shrubs, &land, c, SHRUB_RADIUS))
        .map(|t|WorldRoot {id:t.id,placement:RootPlacement {up:Vec3::Y,pos:t.pos,yaw:t.yaw,scale:t.scale.y,variant:t.variant as usize}}).collect()} else {Vec::new()};
    if !world.grass {return woody;}
    let offset=if world.trees {biome.species().len()} else {0};
    let mut roots=woody;
    let started=std::time::Instant::now();
    let cells=grass_cells(seed,style,&land,c,offset,GRASS_RADIUS);
    eprintln!("grass world cells: count={} generation_cpu_ms={:.2} radius_m={GRASS_RADIUS}",cells.len(),started.elapsed().as_secs_f64()*1000.);
    roots.extend(cells);roots
}

/// Shrubs are placed this far around the camera (metres).
const SHRUB_RADIUS: f32 = 400.0;

/// Where drawn trees hand over to the painted forest (metres from the centre).
fn handover_band(world: World) -> (f32, f32) {
    (world.tree_radius * 0.85, world.tree_radius)
}

/// Stable world cells share two cooked assemblies. Terrain sampling happens
/// only on world streaming; no CPU visibility or near-source shortlist.
fn grass_cells(seed:u32,style:&Style,land:&dyn render::placement::World,centre:Vec2,variant_offset:usize,radius:f32)->Vec<WorldRoot> {
    let cell=recipe_forest::MEADOW_CELL_SIZE;
    let lo=((centre-Vec2::splat(radius))/cell).floor().as_ivec2();
    let hi=((centre+Vec2::splat(radius))/cell).ceil().as_ivec2();
    let mut roots=Vec::new();
    for j in lo.y..=hi.y {for i in lo.x..=hi.x {
        let x=(i as f32+0.5)*cell;let z=(j as f32+0.5)*cell;
        if Vec2::new(x,z).distance_squared(centre)>radius*radius {continue;}
        let s=land.ground(x,z);
        let sparse=0.5+0.8*terrain_field::noised(Vec2::new(x,z)/40.,seed.wrapping_add(31)).x>style.grass;
        if sparse || s.rock>0.5 || s.scree>0.8 || s.water>0.02 || s.snow>0.3 || land.clearance(x,z)<cell {continue;}
        let dx=land.ground(x+0.5,z).pos.y-land.ground(x-0.5,z).pos.y;
        let dz=land.ground(x,z+0.5).pos.y-land.ground(x,z-0.5).pos.y;
        let up=Vec3::new(-dx,1.,-dz).normalize();
        let hash=render::placement::Cell::new(i,j,seed).hash(77);
        roots.push(WorldRoot {id:render::placement::instance_id(1000,i,j),placement:RootPlacement {
            pos:Vec3::new(x,s.pos.y-0.015,z),up,yaw:hash*std::f32::consts::TAU,scale:1.,variant:variant_offset+usize::from(hash>=0.5)}});
    }}
    roots
}

fn main() -> Result<(), String> {
    // Normal vegetation renderer. These diagnostic overrides can disable a
    // culling stage, but never select a different CPU population backend.
    for name in ["KERN_TREE_OCCLUSION", "KERN_CLUSTER_TRAVERSAL"] {
        if std::env::var_os(name).is_none() {unsafe {std::env::set_var(name,"1");}}
    }
    let args: Vec<String> = std::env::args().collect();
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
        branch_slots: arg("--branch-slots").map(|n|n.parse().map_err(|_|"--branch-slots N")).transpose()?.unwrap_or(if low {512}else{4096}), ..World::default() };
    let mut game = Fly::new(seed, style, world)?;
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
    // default 80), one tick per frame; prints frame-time percentiles, so
    // hitches from scene rebuilds show without a window.
    if let Some(n) = arg("--fly") {
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1280, 720]);
        game.speed = arg("--speed").and_then(|s| s.parse().ok()).unwrap_or(80.0);
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], TICK_HZ)?;
        let mut times = Vec::new();
        let mut cpu_times = Vec::new();
        let mut gpu_samples = Vec::new();
        let mut page_samples = Vec::new();
        let forward = Controls { mv: [0.0, 1.0], ..Default::default() };
        let frames = n.parse::<u32>().map_err(|_| "--fly N")?;
        if frames <= 5 { return Err("--fly needs at least six frames".into()); }
        for i in 0..frames {
            let t = std::time::Instant::now();
            game.tick(&[forward]);
            let rgba = headless.frame(&mut game)?;
            if i >= 5 {
                times.push(t.elapsed().as_secs_f64() * 1e3);
                cpu_times.push(serde_json::json!({"frame":i,"draw":headless.last_draw_timings(),"terrain_ms":game.last_updates[0],"tree_pool_ms":game.last_updates[1],
                    "renderer_spans":headless.renderer().and_then(|r|r.cpu_spans())}));
            }
            if let Some(renderer)=headless.renderer() {
                // Rebuilds may replace a Renderer at the same address. Enable
                // profiling on the actual instance, not just pointer changes.
                if let Some(stats)=renderer.foliage_page_stats(){page_samples.push(serde_json::json!({"frame":i,"cache":stats}));}
                renderer.set_async_gpu_profiling(&gpu,true)?;
                for sample in renderer.drain_gpu_profiles(&gpu).samples {
                    gpu_samples.push(serde_json::json!({"frame":sample.render_sequence,"output":sample.output_size,"views":sample.views,
                        "covered_ms":sample.covered_interval_ms,"passes":sample.gpu_ms}));
                }
            }
            if i+1 == frames { if let Some(path) = arg("--fly-shot") { render::write_png(std::path::Path::new(&path), w, h, &rgba)?; } }
        }
        if let Some(renderer)=headless.renderer() {
            if let Some(stats)=renderer.foliage_page_stats(){println!("GPU voxel page cache: {}",serde_json::to_string(&stats).map_err(|e|e.to_string())?);}
            if let Some(stats)=renderer.gpu_forest_diagnostics(&gpu)? {println!("GPU forest final: {}",serde_json::to_string(&stats).map_err(|e|e.to_string())?);
            let d=renderer.debug_tree_occlusion(&gpu,0,None)?;
            println!("GPU tree recovery final: {}",serde_json::json!({"history_first":d.history_first,"phase1_clusters":d.phase1_clusters,"phase2_clusters":d.phase2_clusters,"deferred_clusters":d.deferred_clusters,"deferred_capacity":d.deferred_capacity,"deferred_overflow":d.deferred_overflow,"clusters_tested":d.clusters_tested,"clusters_rejected":d.clusters_rejected}));
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

#[cfg(test)]
mod vegetation_tests {
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
        let a=grass_cells(1,&style,&Slope,Vec2::ZERO,7,12.);
        let b=grass_cells(1,&style,&Slope,Vec2::new(4.,0.),7,12.);
        assert!(!a.is_empty());let mut overlap=0;
        for root in &a {
            let p=root.placement;
            assert!((p.pos.y-(0.2*p.pos.x-0.1*p.pos.z-0.015)).abs()<1e-5);
            assert!(p.up.abs_diff_eq(Vec3::new(-0.2,1.,0.1).normalize(),1e-5));
            assert!(!(2.0..4.0).contains(&p.pos.x));assert!((7..9).contains(&p.variant));
            if let Some(next)=b.iter().find(|r|r.id==root.id) {assert_eq!(root,next);overlap+=1;}
        }
        assert!(overlap>a.len()/3);
    }
}
