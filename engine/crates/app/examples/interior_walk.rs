//! Walk from a sunny field into a room with one door: the room bake
//! (`render::room_bake`) and auto exposure, playable. See
//! engine/docs/interior-light.md.
//!
//!   cargo run --release -p app --example interior_walk
//!   cargo run --release -p app --example interior_walk -- --shot walk.png --at 0,1.6,-2 --yaw 0
//!
//! WASD or the left stick walks, the mouse or right stick looks. The HUD
//! shows the exposure the eye is at and what the meter reads.
use app::{Controls, Draw, Frame, Game};
use glam::{Affine3A, Quat, Vec2, Vec3};
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::Grid;
use render::scene::Material;
use render::{Camera, Lighting, Scene};

const TICK_HZ: f32 = 60.0;
const EYE: f32 = 1.65;

fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
    let (a, b) = (Vec3::from(min), Vec3::from(max));
    Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
}

struct Walk {
    scene: Scene,
    grid: Grid,
    /// Wall footprints (min xz, max xz) the walker cannot enter.
    solid: Vec<(Vec2, Vec2)>,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    light: Lighting,
    ready: bool,
    ev: Option<(f32, f32)>,
}

impl Walk {
    fn new() -> Self {
        let mut scene = Scene::default();
        let mut mat = |scene: &mut Scene, tint: [f32; 3], rough: f32| {
            let m = scene.add_material(Material { tint, layer: -1, roughness: rough, flags: 0, scale: 1.0, metal: 0.0 });
            scene.add_mesh(&render::primitives::cube(m).mesh())
        };
        let plaster = mat(&mut scene, [0.55, 0.52, 0.48], 0.85);
        let wood = mat(&mut scene, [0.30, 0.18, 0.10], 0.35);
        let grass = mat(&mut scene, [0.12, 0.25, 0.06], 0.9);
        let cloth = mat(&mut scene, [0.45, 0.10, 0.08], 0.9);
        scene.add_instance(grass, bx([-150.0, -0.2, -150.0], [150.0, 0.0, 150.0]));
        // 8 x 3 x 8 m inside, 0.3 m walls, a 1.2 x 2.2 m door in the +z wall.
        let walls = [
            bx([-4.3, 0.0, -4.3], [4.3, 3.0, -4.0]),
            bx([-4.3, 0.0, -4.0], [-4.0, 3.0, 4.0]),
            bx([4.0, 0.0, -4.0], [4.3, 3.0, 4.0]),
            bx([-4.3, 0.0, 4.0], [-0.6, 3.0, 4.3]),
            bx([0.6, 0.0, 4.0], [4.3, 3.0, 4.3]),
        ];
        for w in &walls { scene.add_instance(plaster, *w); }
        scene.add_instance(plaster, bx([-0.6, 2.2, 4.0], [0.6, 3.0, 4.3]));
        scene.add_instance(plaster, bx([-4.3, 3.0, -4.3], [4.3, 3.3, 4.3]));
        scene.add_instance(wood, bx([-4.0, 0.0, -4.0], [4.0, 0.02, 4.0]));
        let furniture = [
            (wood, bx([-3.0, 0.7, -3.2], [-1.2, 0.76, -2.2])),
            (wood, bx([-2.95, 0.0, -3.15], [-1.25, 0.7, -2.25])),
            (cloth, bx([1.0, 0.0, -3.9], [3.6, 0.45, -3.0])),
            (cloth, bx([1.0, 0.45, -3.9], [3.6, 1.0, -3.6])),
        ];
        for (m, f) in &furniture { scene.add_instance(*m, *f); }
        let footprint = |xf: &Affine3A| {
            let c = Vec3::from(xf.translation);
            let h = Vec3::new(xf.matrix3.x_axis.x, 0.0, xf.matrix3.z_axis.z) * 0.5;
            (Vec2::new(c.x - h.x, c.z - h.z), Vec2::new(c.x + h.x, c.z + h.z))
        };
        let solid = walls.iter().chain(furniture.iter().map(|f| &f.1)).map(footprint).collect();
        let grid = Grid { origin: Vec3::new(-5.75, -0.25, -5.75), cell: 0.5, dims: [24, 9, 24] };
        scene.reserve_irradiance_volume(grid).expect("probe grid");
        let mut light = Lighting::default();
        light.fog_start = 1000.0;
        light.fog_end = 2000.0;
        light.fog_density = 0.0;
        light.auto_exposure = Some(render::AutoExposure { min_ev100: 0.0, max_ev100: 16.0, ..Default::default() });
        Walk { scene, grid, solid, pos: Vec3::new(1.5, EYE, 12.0), yaw: 0.12, pitch: 0.0, light, ready: false, ev: None }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }
}

impl Game for Walk {
    fn local_players(&self) -> usize { 1 }

    fn tick(&mut self, c: &[Controls]) {
        let c = c[0];
        let dt = 1.0 / TICK_HZ;
        self.yaw -= c.look[0];
        self.pitch = (self.pitch - c.look[1]).clamp(-1.4, 1.4);
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let step = (right * c.mv[0] + fwd * c.mv[1]) * 3.0 * dt;
        // Slide along walls and furniture: move per axis, refuse a step
        // that ends within 0.25 m of a footprint.
        for axis in [0, 2] {
            let mut next = self.pos;
            next[axis] += step[axis];
            let p = Vec2::new(next.x, next.z);
            if !self.solid.iter().any(|(lo, hi)| p.cmpgt(*lo - 0.25).all() && p.cmplt(*hi + 0.25).all()) { self.pos = next; }
        }
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !std::mem::replace(&mut self.ready, true) {
            frame.set_scene(&self.scene, &self.light);
            let occluders = render::room_bake::Occluders::from_scene(&self.scene, |_| true);
            let probes = render::room_bake::bake(&self.grid, &occluders, self.light.sun_dir, &Default::default())?;
            let planes = render::room_bake::separation_planes(&self.grid, &occluders);
            frame.renderer().ok_or("no renderer")?.set_sky_sun_separated(frame.gpu, &probes, &planes)?;
        }
        if let Some(r) = frame.renderer() { self.ev = r.exposure_ev100(0).zip(r.metered_ev100(0)); }
        let cam = Camera { pos: self.pos, target: self.pos + self.forward(), fov_y: 70f32.to_radians(), near: 0.05 };
        let mut hud = HudDrawList::new(frame.size);
        let [w, h] = frame.size.map(|v| v as f32);
        let text = match self.ev {
            Some((ev, metered)) => format!("exposure EV {ev:.1}  meter EV {metered:.1}"),
            None => "metering...".into(),
        };
        hud.text.push(TextRun { id: 1, text, position: [20.0, 16.0], bounds: [0.0, 0.0, w, h],
            font_size: 26.0, color: [255, 255, 255, 220], font: FontFace::Body, align: TextAlign::Left, width: 600.0 });
        Ok(Draw { cameras: vec![cam], light: self.light.clone(), hud })
    }

    fn state(&self) -> Option<String> {
        Some(format!(r#"{{"pos":[{},{},{}],"yaw":{},"ev":{:?}}}"#, self.pos.x, self.pos.y, self.pos.z, self.yaw, self.ev.map(|e| e.0)))
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let mut game = Walk::new();
    if let Some(at) = arg("--at") {
        let v: Vec<f32> = at.split(',').filter_map(|s| s.parse().ok()).collect();
        if v.len() == 3 { game.pos = Vec3::new(v[0], v[1], v[2]); }
    }
    if let Some(yaw) = arg("--yaw").and_then(|y| y.parse().ok()) { game.yaw = yaw; }
    if let Some(out) = arg("--shot") {
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut game, &gpu, [960, 540], 30, TICK_HZ)?;
        return render::write_png(std::path::Path::new(&out), 960, 540, &rgba);
    }
    app::run(app::Config { title: "Warp: into the room".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut game)
}
