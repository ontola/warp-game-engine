//! The smallest Warp game: roll a ball around a field and collect cubes.
//!
//!   cargo run -p app --example hello                      # window
//!   cargo run -p app --example hello -- --shot hello.png  # headless frame
//!   cargo run -p app --example hello -- --script s.json --out dir
//!
//! Shows the pieces every game uses: a scene from primitives, Slots for
//! objects that come and go, app::Game for tick/draw, a HUD and state().
use app::{Buttons, Controls, Draw, Frame, Game};
use glam::{Affine3A, Quat, Vec3};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::scene::{Material, EMISSIVE};
use render::slots::{SlotId, Slots};
use render::{primitives, Camera, Lighting, Scene};

const TICK_HZ: f32 = 60.0;

struct Hello {
    scene: Scene,
    ball_mesh: usize,
    gem_mesh: usize,
    objects: Slots,
    ball: SlotId,
    gems: Vec<(SlotId, Vec3)>,
    pos: Vec3,
    vel: Vec3,
    yaw: f32,
    score: u32,
    tick: u64,
    uploaded: bool,
}

impl Hello {
    fn new() -> Self {
        let mut scene = Scene::default();
        scene.sky = Some(primitives::gradient_sky([0.25, 0.45, 0.8], [0.75, 0.82, 0.9], [0.35, 0.33, 0.3]));
        let mat = |scene: &mut Scene, tint: [f32; 3], flags, scale| scene.add_material(Material { tint, layer: -1, roughness: 0.7, flags, scale, metal: 0.0 });
        let ground_m = mat(&mut scene, [0.30, 0.42, 0.25], 0, 1.0);
        let ball_m = mat(&mut scene, [0.8, 0.15, 0.1], 0, 1.0);
        let gem_m = mat(&mut scene, [0.2, 0.7, 1.0], EMISSIVE, 4000.0);
        let ground = scene.add_mesh(&primitives::cube(ground_m).mesh());
        let ball_mesh = scene.add_mesh(&primitives::sphere(ball_m, 12, 18).mesh());
        let gem_mesh = scene.add_mesh(&primitives::cube(gem_m).mesh());
        // Static geometry: ordinary instances.
        scene.add_instance(ground, Affine3A::from_scale_rotation_translation(Vec3::new(60.0, 1.0, 60.0), Quat::IDENTITY, Vec3::new(0.0, -0.5, 0.0)));
        // Things that appear and disappear: a slot block.
        let mut objects = Slots::reserve(&mut scene, 64);
        let ball = objects.add(&mut scene, ball_mesh, Affine3A::IDENTITY).unwrap();
        let mut game = Hello { scene, ball_mesh, gem_mesh, objects, ball, gems: Vec::new(), pos: Vec3::Y * 0.5, vel: Vec3::ZERO, yaw: 0.0, score: 0, tick: 0, uploaded: false };
        for k in 0..8 { game.spawn_gem(k); }
        game
    }

    fn spawn_gem(&mut self, k: u32) {
        let a = k as f32 * 2.399 + self.score as f32;
        let p = Vec3::new(a.cos() * (6.0 + k as f32 * 2.5), 0.6, a.sin() * (6.0 + k as f32 * 2.5));
        let id = self.objects.add(&mut self.scene, self.gem_mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(0.6), Quat::IDENTITY, p)).unwrap();
        self.gems.push((id, p));
    }
}

impl Game for Hello {
    fn local_players(&self) -> usize { 1 }

    fn tick(&mut self, c: &[Controls]) {
        let dt = 1.0 / TICK_HZ;
        let c = c[0];
        self.tick += 1;
        self.yaw -= c.look[0];
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        self.vel += (right * c.mv[0] + fwd * c.mv[1]) * 30.0 * dt;
        if c.pressed.has(Buttons::SOUTH) && self.pos.y <= 0.51 { self.vel.y = 8.0; }
        self.vel.y -= 25.0 * dt;
        self.vel.x *= 0.96;
        self.vel.z *= 0.96;
        self.pos += self.vel * dt;
        if self.pos.y < 0.5 { self.pos.y = 0.5; self.vel.y = 0.0; }
        let roll = Quat::from_rotation_x(-self.pos.z * 2.0) * Quat::from_rotation_z(self.pos.x * 2.0);
        self.objects.set(&mut self.scene, self.ball, self.ball_mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(0.5), roll, self.pos));
        // Collect gems near the ball and respawn them elsewhere.
        let near: Vec<usize> = self.gems.iter().enumerate().filter(|(_, g)| g.1.distance(self.pos) < 1.1).map(|(i, _)| i).collect();
        for i in near.into_iter().rev() {
            let (id, _) = self.gems.remove(i);
            self.objects.remove(&mut self.scene, id);
            self.score += 1;
            self.spawn_gem(self.score % 8);
        }
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let light = Lighting::default();
        if !std::mem::replace(&mut self.uploaded, true) { frame.set_scene(&self.scene, &light); }
        self.objects.flush(frame.gpu, frame.renderer().unwrap(), &self.scene)?;
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let cam = Camera { pos: self.pos - fwd * 7.0 + Vec3::Y * 4.0, target: self.pos, fov_y: 1.0, near: 0.1 };
        let mut hud = HudDrawList::new(frame.size);
        let [w, h] = frame.size.map(|v| v as f32);
        hud.text.push(TextRun { id: 1, text: format!("gems {}", self.score), position: [20.0, 16.0], bounds: [0.0, 0.0, w, h],
            font_size: 32.0, color: [255, 255, 255, 255], font: FontFace::Body, align: TextAlign::Left, width: 300.0 });
        Ok(Draw { cameras: vec![cam], light, hud })
    }

    fn state(&self) -> Option<String> {
        Some(format!(r#"{{"tick":{},"score":{},"ball":[{},{},{}],"gems":{}}}"#, self.tick, self.score, self.pos.x, self.pos.y, self.pos.z, self.gems.len()))
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let mut game = Hello::new();
    if let Some(out) = arg("--shot") {
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut game, &gpu, [960, 540], 30, TICK_HZ)?;
        return render::write_png(std::path::Path::new(&out), 960, 540, &rgba);
    }
    if let Some(script) = arg("--script") {
        let gpu = render::Gpu::new()?;
        let out = arg("--out").unwrap_or_else(|| "hello-out".into());
        let failed = app::script::run_file(&mut game, &gpu, TICK_HZ, script.as_ref(), out.as_ref())?;
        std::process::exit(i32::from(failed > 0));
    }
    app::run(app::Config { title: "Hello Warp".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut game)
}
