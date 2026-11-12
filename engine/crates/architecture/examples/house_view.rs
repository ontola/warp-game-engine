//! Walk through a house, natively or in a browser. Left/Right arrows jump
//! between the outside views and one view per room; WASD and the mouse move
//! and look (click to capture the mouse, Esc releases it), Up/Down change
//! height, Shift moves faster.
//!   cargo run --release -p architecture --example house_view -- HOUSE.glb [--shot out.png]
//! In a browser (engine/tools/build-web.py --example house_view): the page
//! fetches `assets/<name>.json` (glTF with an embedded buffer) or
//! `<name>.glb` for `?house=<name>` or `#<name>` (default fzk-haus).
use app::{Buttons, Controls, Draw, Frame, Game};
use architecture::{sun::Site, House};
use glam::Vec3;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::{Grid, Probe};
use render::{primitives, Camera, Lighting, Scene};
use std::path::Path;

const TICK_HZ: f32 = 60.0;

fn log(s: &str) {
    #[cfg(target_arch = "wasm32")]
    app::web::log(s);
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{s}");
}

struct View {
    scene: Scene,
    shots: Vec<(String, Camera)>,
    shot: usize,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    light: Lighting,
    /// The room bake, made before the window opens, uploaded on the first frame.
    bake: Option<(Vec<Probe>, Vec<[f32; 4]>)>,
    uploaded: bool,
    name: String,
}

impl View {
    /// Load the house and its light: a shipped bake when one matches,
    /// else baked here (seconds on a many-core CPU; a browser tab bakes on
    /// one thread and the page waits meanwhile).
    fn new(bytes: &[u8], name: &str, rays: usize, shipped: Option<&[u8]>) -> Result<Self, String> {
        let mut scene = Scene::default();
        scene.sky = Some(primitives::gradient_sky([0.3, 0.45, 0.7], [0.75, 0.8, 0.85], [0.35, 0.33, 0.3]));
        let house = House::from_glb(&mut scene, bytes, Path::new(name), None, 0.0)?;
        house.add_ground(&mut scene);
        let grid: Grid = house.probe_grid();
        scene.reserve_irradiance_volume(grid)?;
        let mut light = Lighting::default();
        light.fog_start = 1000.0;
        light.fog_end = 2000.0;
        light.fog_density = 0.0;
        light.sun_dir = Site { latitude: 52.37, longitude: 4.9, north_deg: 180.0 }.sun_dir(105, 9.5);
        light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, ..Default::default() });
        let occluders = render::room_bake::Occluders::from_scene(&scene, |_| true);
        let shipped = shipped.map(|b| architecture::baked::from_bytes(b, &grid, light.sun_dir));
        if let Some(Err(e)) = &shipped { log(&format!("{name}: shipped light not used ({e}), baking")); }
        let (probes, planes) = match shipped {
            Some(Ok(b)) => (b.probes, b.planes),
            _ => (render::room_bake::bake(&grid, &occluders, light.sun_dir, &render::room_bake::Settings { sky_rays: rays, ..Default::default() })?,
                  render::room_bake::separation_planes(&grid, &occluders)),
        };
        let mut shots = house.exterior_shots();
        shots.extend(house.room_shots());
        let mut view = View { scene, shots, shot: 0, pos: Vec3::ZERO, yaw: 0.0, pitch: 0.0, light, bake: Some((probes, planes)), uploaded: false, name: name.to_string() };
        view.go(0);
        Ok(view)
    }

    fn go(&mut self, shot: usize) {
        self.shot = shot % self.shots.len();
        let cam = self.shots[self.shot].1;
        self.look(cam.pos, cam.target);
    }

    fn look(&mut self, pos: Vec3, target: Vec3) {
        let cam = Camera { pos, target, ..self.shots[self.shot].1 };
        let d = (cam.target - cam.pos).normalize_or(Vec3::NEG_Z);
        self.pos = cam.pos;
        self.yaw = (-d.x).atan2(-d.z);
        self.pitch = d.y.asin();
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
}

impl Game for View {
    fn local_players(&self) -> usize { 1 }

    fn tick(&mut self, controls: &[Controls]) {
        let Some(c) = controls.first() else { return };
        let n = self.shots.len();
        if c.pressed.has(Buttons::RIGHT) { self.go(self.shot + 1); }
        if c.pressed.has(Buttons::LEFT) { self.go(self.shot + n - 1); }
        self.yaw -= c.look[0];
        self.pitch = (self.pitch + c.look[1]).clamp(-1.5, 1.5);
        let dt = 1.0 / TICK_HZ;
        let speed = if c.held.has(Buttons::RIGHT_SHOULDER) { 8.0 } else { 2.5 };
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let up = if c.held.has(Buttons::UP) { 1.0 } else if c.held.has(Buttons::DOWN) { -1.0 } else { 0.0 };
        self.pos += (right * c.mv[0] + fwd * c.mv[1] + Vec3::Y * up) * speed * dt;
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !std::mem::replace(&mut self.uploaded, true) {
            frame.set_scene(&self.scene, &self.light);
            if let Some((probes, planes)) = self.bake.take() {
                frame.renderer().ok_or("no renderer")?.set_sky_sun_separated(frame.gpu, &probes, &planes)?;
            }
        }
        let cam = Camera { pos: self.pos, target: self.pos + self.forward(), fov_y: 60f32.to_radians(), near: 0.05 };
        let mut hud = HudDrawList::new(frame.size);
        let [w, h] = frame.size.map(|v| v as f32);
        let text = format!("{}: {} ({}/{})    Left/Right: views    WASD + mouse: move    Up/Down: height", self.name, self.shots[self.shot].0, self.shot + 1, self.shots.len());
        hud.text.push(TextRun { id: 1, text, position: [16.0, 12.0], bounds: [0.0, 0.0, w, h], font_size: 20.0, color: [255, 255, 255, 220], font: FontFace::Body, align: TextAlign::Left, width: w - 32.0 });
        Ok(Draw { cameras: vec![cam], light: self.light.clone(), hud })
    }

    fn state(&self) -> Option<String> {
        Some(format!(r#"{{"shot":{},"pos":[{},{},{}],"yaw":{},"pitch":{}}}"#, self.shot, self.pos.x, self.pos.y, self.pos.z, self.yaw, self.pitch))
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    app::web::install_panic_hook();
    wasm_bindgen_futures::spawn_local(async {
        let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
        let hash = web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default();
        let name = query.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("house=").map(str::to_owned))
            .or_else(|| Some(hash.trim_start_matches('#').to_owned()).filter(|h| !h.is_empty()))
            .unwrap_or_else(|| "fzk-haus".into());
        app::web::progress(Some(0.35), &format!("Downloading {name}"));
        // glTF JSON with an embedded buffer (hosts that serve .json but
        // not .glb), else a GLB.
        let bytes = match app::web::fetch(&format!("assets/{name}.json")).await {
            Ok(b) => b,
            Err(_) => match app::web::fetch(&format!("assets/{name}.glb")).await {
                Ok(b) => b,
                Err(e) => { app::web::progress(Some(0.35), &format!("Could not load {name}: {e}")); return; }
            },
        };
        // The light baked ahead on a many-core machine (house_view
        // --bake-out); without it the tab bakes on one thread. Hosts that
        // serve no binary files get it as base64 text (.light.txt).
        let shipped = match app::web::fetch(&format!("assets/{name}.light.bin")).await {
            Ok(b) => Some(b),
            Err(_) => app::web::fetch(&format!("assets/{name}.light.txt")).await.ok()
                .and_then(|t| assets::gltf::base64_decode(String::from_utf8_lossy(&t).trim()).ok()),
        };
        let step = if shipped.is_some() { "Setting up the rooms" } else { "Baking the light in the rooms (a minute, the page waits)" };
        app::web::progress(Some(0.5), step);
        app::web::yield_to_browser().await;
        match View::new(&bytes, &name, 1024, shipped.as_deref()) {
            Ok(view) => app::web::run(app::Config { title: "Warp house".into(), tick_hz: TICK_HZ, max_render_height: Some(900), ..Default::default() }, Box::new(view)),
            Err(e) => { app::web::progress(Some(0.5), &format!("{name}: {e}")); app::web::log(&e) }
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let path = args.get(1).filter(|a| !a.starts_with("--")).ok_or("house_view HOUSE.glb [--shot out.png] [--view N] [--bake-out light.bin] [--bake-in light.bin]")?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let name = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("house").to_string();
    let shipped = arg("--bake-in").map(|p| std::fs::read(&p).map_err(|e| format!("{p}: {e}"))).transpose()?;
    let mut view = View::new(&bytes, &name, 2048, shipped.as_deref())?;
    if let Some(out) = arg("--bake-out") {
        let (probes, planes) = view.bake.as_ref().ok_or("no bake")?;
        let grid = view.scene.irradiance_volume.map(|(_, g)| g).ok_or("no grid")?;
        std::fs::write(&out, architecture::baked::to_bytes(&grid, view.light.sun_dir, probes, planes)).map_err(|e| format!("{out}: {e}"))?;
        if arg("--shot").is_none() { return Ok(()); }
    }
    if let Some(n) = arg("--view").and_then(|v| v.parse().ok()) { view.go(n); }
    // --cam px,py,pz,tx,ty,tz: a fixed spot (regression stills).
    if let Some(c) = arg("--cam") {
        let v: Vec<f32> = c.split(',').map(|x| x.parse().map_err(|_| format!("--cam {c}"))).collect::<Result<_, _>>()?;
        if v.len() != 6 { return Err("--cam px,py,pz,tx,ty,tz".into()); }
        view.look(Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]));
    }
    if let Some(out) = arg("--shot") {
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut view, &gpu, [960, 540], 30, TICK_HZ)?;
        return render::write_png(Path::new(&out), 960, 540, &rgba);
    }
    app::run(app::Config { title: "Warp house".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut view)
}
