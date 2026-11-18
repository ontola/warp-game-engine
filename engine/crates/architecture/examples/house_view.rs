//! Walk through a house, natively or in a browser. Left/Right arrows jump
//! between the outside views and one view per room; WASD and the mouse move
//! and look (click to capture the mouse, Esc releases it), Up/Down change
//! height, Shift moves faster. On a touch screen: a thumb on the left
//! third walks (a stick from where it lands), dragging elsewhere looks, and
//! tapping a floor or wall walks there.
//!   cargo run --release -p architecture --example house_view -- HOUSE.glb [--shot out.png]
//! In a browser (engine/tools/build-web.py --example house_view): the page
//! fetches `assets/<name>.json` (glTF with an embedded buffer) or
//! `<name>.glb` for `?house=<name>` or `#<name>` (default fzk-haus).
use app::{Buttons, Controls, Draw, Frame, Game};
use architecture::{sun::Site, House};
use glam::Vec3;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::Grid;
use render::{primitives, Camera, Lighting, Scene};
use std::path::Path;

const TICK_HZ: f32 = 60.0;
/// Vertical field of view, degrees.
const FOV_DEG: f32 = 60.0;
/// Eye height over the floor, metres.
const EYE: f32 = 1.6;

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
    bake: Option<architecture::baked::Baked>,
    uploaded: bool,
    name: String,
    /// One reflection box per room; the nearest the renderer can hold are set each frame.
    rooms: Vec<render::reflection_probes::ProbeBox>,
    /// What a tap hits (the house's opaque triangles).
    solid: render::room_bake::Occluders,
    /// A tap waiting for the next tick: position and screen size, in pixels.
    tap: Option<([f32; 2], [u32; 2])>,
    /// Where a tap sent the eye; it glides there.
    walk_to: Option<Vec3>,
    /// The file's lamps (KHR_lights_punctual), set on the renderer at upload.
    lamps: Vec<render::clustered_lights::ClusteredLight>,
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
        scene.graphics.clustered_lights = !house.lights.is_empty();
        let grid: Grid = house.probe_grid();
        scene.reserve_irradiance_volume(grid)?;
        // DDGI visibility, probes moved out of walls and per-probe reach:
        // without them light leaks through thin IFC walls and slabs.
        scene.reserve_probe_visibility()?;
        let mut light = Lighting::default();
        light.fog_start = 1000.0;
        light.fog_end = 2000.0;
        light.fog_density = 0.0;
        light.sun_dir = Site { latitude: 52.37, longitude: 4.9, north_deg: 180.0 }.sun_dir(105, 9.5);
        light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, ..Default::default() });
        let occluders = render::room_bake::Occluders::from_scene(&scene, |_| true);
        let shipped = shipped.map(|b| architecture::baked::Baked::from_bytes(b, &grid, light.sun_dir));
        if let Some(Err(e)) = &shipped { log(&format!("{name}: shipped light not used ({e}), baking")); }
        let baked = match shipped {
            Some(Ok(b)) => b,
            _ => architecture::baked::Baked::bake(&grid, &occluders, light.sun_dir, &render::room_bake::Settings { sky_rays: rays, ..Default::default() })?,
        };
        let mut shots = house.exterior_shots();
        shots.extend(house.room_shots());
        // The file's saved views come last, after the rooms.
        shots.extend(house.cameras.iter().map(|v| (v.name.clone(), v.camera)));
        let lamps = house.clustered_lights();
        let mut view = View { scene, shots, shot: 0, pos: Vec3::ZERO, yaw: 0.0, pitch: 0.0, light, bake: Some(baked), uploaded: false, name: name.to_string(), rooms: house.reflection_boxes(),
            solid: house.solid, tap: None, walk_to: None, lamps };
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

    /// Where a tap at `at` (pixels on a `size` screen) sends the eye: onto
    /// the floor it hits, or in front of the wall it hits, at eye height
    /// over the floor there. None for the sky.
    fn tapped(&self, at: [f32; 2], size: [u32; 2]) -> Option<Vec3> {
        let [w, h] = size.map(|v| v.max(1) as f32);
        let (x, y) = (at[0] / w * 2.0 - 1.0, 1.0 - at[1] / h * 2.0);
        let half = (FOV_DEG.to_radians() * 0.5).tan();
        let fwd = self.forward();
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(fwd);
        let d = (fwd + right * x * half * w / h + up * y * half).normalize();
        let (t, n) = self.solid.hit(self.pos, d)?;
        let n = if n.dot(d) > 0.0 { -n } else { n };
        let hit = self.pos + d * t;
        // A floor: stand on it. Anything else: half a metre in front of it,
        // on the floor below.
        let spot = if n.y > 0.7 { hit } else {
            let front = hit + Vec3::new(n.x, 0.0, n.z).normalize_or(-d) * 0.5;
            let above = Vec3::new(front.x, front.y.max(self.pos.y - 0.5) + 0.1, front.z);
            let (down, _) = self.solid.hit(above, Vec3::NEG_Y).filter(|(t, _)| *t < 4.0)?;
            above - Vec3::Y * down
        };
        Some(spot + Vec3::Y * EYE)
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
        let steer = right * c.mv[0] + fwd * c.mv[1] + Vec3::Y * up;
        if steer != Vec3::ZERO || c.pressed.0 != 0 { self.walk_to = None; }
        self.pos += steer * speed * dt;
        if let Some((at, size)) = self.tap.take() { self.walk_to = self.tapped(at, size).or(self.walk_to); }
        // Glide to a tapped spot: quick at first, settling at the end.
        if let Some(to) = self.walk_to {
            let d = to - self.pos;
            let step = (d.length() * 4.0 * dt).max(1.2 * dt).min(d.length());
            self.pos += d.normalize_or_zero() * step;
            if d.length() < 0.01 { self.walk_to = None; }
        }
    }

    fn pointer(&mut self, pointer: app::Pointer, size: [u32; 2]) {
        if pointer.pressed && let Some(at) = pointer.position { self.tap = Some((at, size)); }
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !std::mem::replace(&mut self.uploaded, true) {
            frame.set_scene(&self.scene, &self.light);
            if !self.lamps.is_empty() { frame.renderer().ok_or("no renderer")?.set_clustered_lights(&self.lamps)?; }
            if let Some(b) = self.bake.take() {
                let renderer = frame.renderer().ok_or("no renderer")?;
                renderer.set_sky_sun_separated(frame.gpu, &b.probes, &b.planes)?;
                renderer.set_probe_visibility(frame.gpu, &b.visibility, &b.offsets, &b.reach)?;
            }
        }
        // Live reflection probes for the rooms nearest the eye.
        let mut near: Vec<usize> = (0..self.rooms.len()).collect();
        let gap = |b: &render::reflection_probes::ProbeBox| (self.pos.clamp(b.min, b.max) - self.pos).length();
        near.sort_by(|&a, &b| gap(&self.rooms[a]).total_cmp(&gap(&self.rooms[b])));
        near.truncate(render::reflection_probes::MAX_PROBES);
        near.sort_unstable();
        let boxes: Vec<_> = near.iter().map(|&i| self.rooms[i]).collect();
        frame.renderer().ok_or("no renderer")?.set_reflection_probes(&boxes)?;
        let cam = Camera { pos: self.pos, target: self.pos + self.forward(), fov_y: FOV_DEG.to_radians(), near: 0.05 };
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
        let b = view.bake.as_ref().ok_or("no bake")?;
        let grid = view.scene.irradiance_volume.map(|(_, g)| g).ok_or("no grid")?;
        std::fs::write(&out, b.to_bytes(&grid, view.light.sun_dir)).map_err(|e| format!("{out}: {e}"))?;
        if arg("--shot").is_none() { return Ok(()); }
    }
    if let Some(n) = arg("--view").and_then(|v| v.parse().ok()) { view.go(n); }
    // --cam px,py,pz,tx,ty,tz: a fixed spot (regression stills).
    if let Some(c) = arg("--cam") {
        let v: Vec<f32> = c.split(',').map(|x| x.parse().map_err(|_| format!("--cam {c}"))).collect::<Result<_, _>>()?;
        if v.len() != 6 { return Err("--cam px,py,pz,tx,ty,tz".into()); }
        view.look(Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]));
    }
    // --tap x,y: a tap at those pixels of the 960x540 still, then walk there.
    if let Some(t) = arg("--tap") {
        let v: Vec<f32> = t.split(',').filter_map(|x| x.parse().ok()).collect();
        if v.len() != 2 { return Err("--tap x,y".into()); }
        let to = view.tapped([v[0], v[1]], [960, 540]).ok_or("the tap hits nothing")?;
        println!("tap walks from {:.2?} to {:.2?}", view.pos, to);
        view.pos = to;
    }
    if let Some(out) = arg("--shot") {
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut view, &gpu, [960, 540], 30, TICK_HZ)?;
        return render::write_png(Path::new(&out), 960, 540, &rgba);
    }
    app::run(app::Config { title: "Warp house".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut view)
}
