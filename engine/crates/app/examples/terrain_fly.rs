//! Fly over the procedural terrain: a debug game for looking at the world
//! the way a player would, at any height and speed.
//!
//!   cargo run --release -p app --example terrain_fly
//!   cargo run --release -p app --example terrain_fly -- --style alpine --seed 3
//!   cargo run --release -p app --example terrain_fly -- --shot fly.png
//!
//! Mouse or right stick looks, WASD or left stick flies, Space or A climbs,
//! Q or LB sinks, Shift/E or RB goes fast, the wheel or arrows up/down set
//! the speed. F1 default, F2 lowland, F4 alpine (F3 stays the debug
//! overlay), F5 the next seed, F6 jumps somewhere new, F7 hugs the ground,
//! F12 saves a screenshot with its position and terrain stats for feedback.
use app::{Buttons, Controls, Draw, Frame, Game, Pointer};
use glam::Vec3;
use render::atmosphere::{Atmosphere, AtmosphereCache, BakeQuality, SolarState};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::procedural_terrain::{self, ProceduralTerrain, Settings, Stats, TerrainView};
use render::scene::{Material, SURFACE, TERRAIN};
use render::terrain_field::{self, Style};
use render::{Camera, Lighting, Scene};
use std::path::PathBuf;

const POOL: u32 = 16384;
const TICK_HZ: f32 = 60.0;
const STYLES: [&str; 3] = ["default", "lowland", "alpine"];

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
    /// Scene, light and patch pool waiting for upload after a style or seed change.
    pending: Option<(Scene, Lighting, procedural_terrain::TerrainSpec)>,
    terrain: Option<ProceduralTerrain>,
    light: Lighting,
    stats: Stats,
    jumps: u32,
    fps: f32,
}

impl Fly {
    fn new(seed: u32, style: usize) -> Result<Self, String> {
        let mut fly = Fly { seed, style, pos: Vec3::ZERO, prev: Vec3::ZERO, yaw: 0.6, pitch: -0.15, speed: 40.0, hug: None, wheel: 0.0,
            pending: None, terrain: None, light: Lighting::default(), stats: Stats::default(), jumps: 0, fps: 0.0 };
        fly.rebuild()?;
        fly.pos = Vec3::new(0.0, fly.ground(0.0, 0.0) + 60.0, 0.0);
        fly.prev = fly.pos;
        Ok(fly)
    }

    fn style(&self) -> Style { Style::named(STYLES[self.style]).unwrap() }
    fn ground(&self, x: f32, z: f32) -> f32 { terrain_field::height(x, z, self.seed, &self.style()) }

    /// A new scene for the current seed and style; uploaded on the next frame.
    fn rebuild(&mut self) -> Result<(), String> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../games/sideways/assets/textures");
        let mut scene = Scene::default();
        // Layers in the order terrain_surface.wgsl reads them.
        let t0 = scene.add_texture(&root.join("grass_ground_albedo.jpg"))?;
        scene.add_texture(&root.join("forest_leaves_02_albedo.jpg"))?;
        scene.add_texture(&root.join("forest_ground_04_albedo.jpg"))?;
        scene.add_texture(&root.join("rock_face_03_albedo.jpg"))?;
        scene.add_texture_with_normal_encoding(&root.join("gravel_ground_01_albedo.jpg"), render::scene::NormalEncoding::SrgbTransferred)?;
        scene.add_texture(&root.join("mossy_rock_albedo.jpg"))?;
        let material = scene.add_material(Material { tint: [1.0; 3], layer: t0, roughness: 0.9, flags: TERRAIN | SURFACE, scale: 4.0, metal: 0.0 });
        let pool = procedural_terrain::reserve(&mut scene, POOL, material, self.seed, self.style());
        let mut light = Lighting::default();
        let sky = AtmosphereCache::default().get_or_bake(&Atmosphere::default(), SolarState::from_degrees(28.0, 135.0)?, BakeQuality::default())?;
        sky.apply(&mut scene, &mut light);
        light.fog_density = 0.00003;
        light.fog_end = 60000.0;
        self.pending = Some((scene, light, pool));
        Ok(())
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
        let style = match n { 1 => Some(0), 2 => Some(1), 4 => Some(2), _ => None };
        let result = match n {
            1 | 2 | 4 => { self.style = style.unwrap(); self.rebuild() }
            5 => { self.seed += 1; self.rebuild() }
            6 => { self.jump(); Ok(()) }
            7 => { self.hug = match self.hug { Some(_) => None, None => Some(1.7) }; Ok(()) }
            _ => return false,
        };
        if let Err(e) = result { eprintln!("terrain fly: {e}"); }
        if matches!(n, 1 | 2 | 4 | 5) {
            // Stay at the same height above the new ground.
            let floor = self.ground(self.pos.x, self.pos.z);
            self.pos.y = self.pos.y.max(floor + 30.0);
            self.prev = self.pos;
        }
        true
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if let Some((scene, light, pool)) = self.pending.take() {
            frame.set_scene(&scene, &light);
            self.terrain = Some(ProceduralTerrain::new(pool, Settings::default()));
            self.light = light;
        }
        let pos = self.prev.lerp(self.pos, frame.alpha);
        let cam = Camera { pos, target: pos + self.forward(), fov_y: 65f32.to_radians(), near: 0.05 };
        let [w, h] = frame.size;
        if let (Some(terrain), Some(renderer)) = (self.terrain.as_mut(), frame.renderer()) {
            self.stats = terrain.update(frame.gpu, renderer, &[TerrainView::new(&cam, w, h)])?;
        }
        if frame.dt > 0.0 { self.fps += (1.0 / frame.dt - self.fps) * 0.05; }

        let s = terrain_field::eval(pos.x, pos.z, 0.0, self.seed, &self.style());
        let above = pos.y - s.pos.y;
        let lines = [
            format!("{}  seed {}  {:.0} fps", STYLES[self.style], self.seed, self.fps),
            format!("x {:.0}  z {:.0}  height {:.0} m  {:.0} m above ground", pos.x, pos.z, pos.y, above),
            format!("speed {:.0} m/s{}", self.speed, if self.hug.is_some() { "  ground hug" } else { "" }),
            format!("patches {}  overflow {}  finest {}  lod x{:.2}", self.stats.patches, self.stats.overflow, self.stats.finest_level, self.stats.lod_scale),
            format!("ground: rock {:.2}  scree {:.2}  snow {:.2}  water {:.2}  slope {:.0}°", s.rock, s.scree, s.snow, s.water, s.up.clamp(0.0, 1.0).acos().to_degrees()),
            "F1 default  F2 lowland  F4 alpine  F5 seed  F6 jump  F7 hug  wheel speed  F12 screenshot".to_string(),
        ];
        let mut hud = HudDrawList::new(frame.size);
        for (i, text) in lines.into_iter().enumerate() {
            hud.text.push(TextRun { id: i as u64 + 1, text, position: [20.0, 16.0 + 26.0 * i as f32], bounds: [0.0, 0.0, w as f32, h as f32],
                font_size: 20.0, color: [255, 255, 255, 230], font: FontFace::Body, align: TextAlign::Left, width: w as f32 });
        }
        Ok(Draw { cameras: vec![cam], light: self.light.clone(), hud })
    }

    fn title(&self) -> Option<String> { Some(format!("Terrain fly: {} seed {}", STYLES[self.style], self.seed)) }

    fn state(&self) -> Option<String> {
        let g = terrain_field::eval(self.pos.x, self.pos.z, 0.0, self.seed, &self.style());
        Some(serde_json::json!({
            "style": STYLES[self.style], "seed": self.seed, "pos": self.pos.to_array(),
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

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let seed = arg("--seed").map(|s| s.parse().map_err(|_| "--seed N")).transpose()?.unwrap_or(1);
    let style = arg("--style").map(|s| STYLES.iter().position(|n| *n == s).ok_or("--style default|lowland|alpine")).transpose()?.unwrap_or(0);
    let mut game = Fly::new(seed, style)?;
    // --from: a JSON written by F12 (or just its "state"), to stand where the playtester stood.
    if let Some(path) = arg("--from") {
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?).map_err(|e| format!("{path}: {e}"))?;
        let st = v.get("state").unwrap_or(&v);
        let f = |k: &str| st.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
        if let Some(name) = st.get("style").and_then(|x| x.as_str()) { game.style = STYLES.iter().position(|n| *n == name).ok_or("unknown style")?; }
        if let Some(seed) = st.get("seed").and_then(|x| x.as_u64()) { game.seed = seed as u32; }
        game.rebuild()?;
        if let Some(p) = st.get("pos").and_then(|x| x.as_array()) {
            let c: Vec<f32> = p.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect();
            if c.len() == 3 { game.pos = Vec3::new(c[0], c[1], c[2]); game.prev = game.pos; }
        }
        if let Some(y) = f("yaw_deg") { game.yaw = y.to_radians(); }
        if let Some(p) = f("pitch_deg") { game.pitch = p.to_radians(); }
        if let Some(s) = f("speed") { game.speed = s; }
    }
    if let Some(out) = arg("--shot") {
        let [w, h] = arg("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1280, 720]);
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut game, &gpu, [w, h], 10, TICK_HZ)?;
        return render::write_png(std::path::Path::new(&out), w, h, &rgba);
    }
    app::run(app::Config { title: "Terrain fly".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut game)
}
