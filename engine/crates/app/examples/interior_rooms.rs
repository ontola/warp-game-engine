//! Three joined rooms, each lit another way, to see how indoor lighting
//! techniques combine. See engine/docs/interior-light.md.
//!
//!   cargo run --release -p app --example interior_rooms
//!   cargo run --release -p app --example interior_rooms -- --shot out.png --at 0,1.65,-7 --yaw 0
//!
//! 1. Daylight room (front door and a window): sky and sun only, baked with
//!    several bounces (`room_bake::bake`), sun shadows live.
//! 2. Pendant room: one lamp swinging on its cord. Direct light and its
//!    shadow are live (a shadowed clustered light); its bounce cannot be
//!    baked, so a second, soft light follows the lit spot on the floor and
//!    shines back up (one virtual point light, "instant radiosity").
//! 3. Switch room: four ceiling lamps, F toggles the one you look at, R all.
//!    Direct light live and shadowed; each lamp's bounce is baked on its own
//!    (`room_bake::bake_lights`) and the lit lamps' bounces are added into
//!    the probes when a switch flips (Call of Duty's UberBake idea).
//!
//! All rooms share one probe grid: sky, sun and lamp bounce in one upload.
//! Auto exposure adapts between them; the HUD shows its EV.
use app::{Buttons, Controls, Draw, Frame, Game};
use glam::{Affine3A, Quat, Vec2, Vec3};
use render::clustered_lights::ClusteredLight;
use render::headlights::LocalLight;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::{Grid, Probe};
use render::scene::{Material, EMISSIVE};
use render::slots::{SlotId, Slots};
use render::{Camera, Lighting, Scene};

const TICK_HZ: f32 = 60.0;
const EYE: f32 = 1.65;
const H: f32 = 3.0;
/// Pendant pivot in the ceiling of room 2, cord length.
const PIVOT: Vec3 = Vec3::new(0.0, H, -7.45);
const CORD: f32 = 1.3;
/// Switch room lamp positions (just under the ceiling).
const LAMPS: [Vec3; 4] = [Vec3::new(-2.0, 2.9, -12.1), Vec3::new(2.0, 2.9, -12.1), Vec3::new(-2.0, 2.9, -15.1), Vec3::new(2.0, 2.9, -15.1)];

fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
    let (a, b) = (Vec3::from(min), Vec3::from(max));
    Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
}

fn warm(cd: f32) -> Vec3 { Vec3::new(1.0, 0.78, 0.55) * cd }

fn ceiling_lamp(p: Vec3) -> LocalLight {
    // A wide downlight: most light down, some to the walls.
    LocalLight { position: p - Vec3::Y * 0.12, direction: Vec3::NEG_Y, intensity_cd: warm(450.0), range_m: 10.0,
        inner_cos: 50f32.to_radians().cos(), outer_cos: 80f32.to_radians().cos(), source_radius_m: 0.12, falloff: 2.0 }
}

struct Rooms {
    scene: Scene,
    grid: Grid,
    solid: Vec<(Vec2, Vec2)>,
    objects: Slots,
    bulb_mesh: usize,
    cord_mesh: usize,
    lamp_on_mesh: usize,
    lamp_off_mesh: usize,
    bulb: SlotId,
    cord: SlotId,
    lamp_slots: Vec<SlotId>,
    lamp_on: [bool; 4],
    occluders: render::room_bake::Occluders,
    sky: Vec<Probe>,
    planes: Vec<[f32; 4]>,
    lamp_bounce: Vec<Vec<Probe>>,
    dirty: bool,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    vy: f32,
    t: f32,
    light: Lighting,
    ready: bool,
    ev: Option<(f32, f32)>,
}

impl Rooms {
    fn new() -> Self {
        let mut scene = Scene::default();
        scene.graphics.clustered_lights = true;
        let mut mat = |scene: &mut Scene, tint: [f32; 3], rough: f32, flags: u32, scale: f32| {
            let m = scene.add_material(Material { tint, layer: -1, roughness: rough, flags, scale, metal: 0.0 });
            scene.add_mesh(&render::primitives::cube(m).mesh())
        };
        let plaster = mat(&mut scene, [0.55, 0.52, 0.48], 0.85, 0, 1.0);
        let blue = mat(&mut scene, [0.25, 0.32, 0.45], 0.85, 0, 1.0);
        let green = mat(&mut scene, [0.30, 0.40, 0.28], 0.85, 0, 1.0);
        let wood = mat(&mut scene, [0.30, 0.18, 0.10], 0.35, 0, 1.0);
        let tiles = mat(&mut scene, [0.45, 0.45, 0.42], 0.2, 0, 1.0);
        let grass = mat(&mut scene, [0.12, 0.25, 0.06], 0.9, 0, 1.0);
        let cloth = mat(&mut scene, [0.45, 0.10, 0.08], 0.9, 0, 1.0);
        let dark = mat(&mut scene, [0.05, 0.05, 0.05], 0.5, 0, 1.0);
        let bulb_mesh = mat(&mut scene, [1.0, 0.78, 0.55], 1.0, EMISSIVE, 40_000.0);
        let lamp_on_mesh = mat(&mut scene, [1.0, 0.85, 0.65], 1.0, EMISSIVE, 25_000.0);
        let lamp_off_mesh = mat(&mut scene, [0.8, 0.8, 0.78], 0.5, 0, 1.0);
        let cord_mesh = dark;
        scene.add_instance(grass, bx([-150.0, -0.2, -150.0], [150.0, 0.0, 150.0]));
        // Shell: x -4..4 inside, three rooms one behind the other along -z.
        let mut walls = vec![
            // Front wall with the door (x -0.6..0.6).
            bx([-4.3, 0.0, 4.0], [-0.6, H, 4.3]), bx([0.6, 0.0, 4.0], [4.3, H, 4.3]), bx([-0.6, 2.2, 4.0], [0.6, H, 4.3]),
            // West wall of room 1, with a window z -1.2..1.2, y 0.9..2.2.
            bx([-4.3, 0.0, -4.0], [-4.0, H, -1.2]), bx([-4.3, 0.0, 1.2], [-4.0, H, 4.3]),
            bx([-4.3, 0.0, -1.2], [-4.0, 0.9, 1.2]), bx([-4.3, 2.2, -1.2], [-4.0, H, 1.2]),
            // East wall of all rooms, west wall of rooms 2 and 3.
            bx([4.0, 0.0, -16.9], [4.3, H, 4.3]), bx([-4.3, 0.0, -16.9], [-4.0, H, -4.0]),
            // Room 1 | 2, doorway x -0.5..0.5.
            bx([-4.0, 0.0, -4.3], [-0.5, H, -4.0]), bx([0.5, 0.0, -4.3], [4.0, H, -4.0]), bx([-0.5, 2.1, -4.3], [0.5, H, -4.0]),
            // Room 2 | 3, doorway x 2.2..3.2: no straight view from the front door.
            bx([-4.0, 0.0, -10.6], [2.2, H, -10.3]), bx([3.2, 0.0, -10.6], [4.0, H, -10.3]), bx([2.2, 2.1, -10.6], [3.2, H, -10.3]),
            // Back wall.
            bx([-4.3, 0.0, -16.9], [4.3, H, -16.6]),
        ];
        let roof = bx([-4.3, H, -16.9], [4.3, H + 0.3, 4.3]);
        for w in &walls { scene.add_instance(plaster, *w); }
        scene.add_instance(plaster, roof);
        // Coloured inner faces: thin panels on room 2 (blue) and 3 (green).
        for (m, b) in [
            (blue, bx([-4.0, 0.0, -10.3], [-3.98, H, -4.3])), (blue, bx([3.98, 0.0, -10.3], [4.0, H, -4.3])),
            (green, bx([-4.0, 0.0, -16.6], [-3.98, H, -10.6])), (green, bx([3.98, 0.0, -16.6], [4.0, H, -10.6])),
        ] { scene.add_instance(m, b); }
        scene.add_instance(wood, bx([-4.0, 0.0, -4.0], [4.0, 0.02, 4.0]));
        scene.add_instance(tiles, bx([-4.0, 0.0, -10.3], [4.0, 0.02, -4.3]));
        scene.add_instance(wood, bx([-4.0, 0.0, -16.6], [4.0, 0.02, -10.6]));
        let furniture = [
            // Room 1: table and couch.
            (wood, bx([-3.0, 0.7, -3.2], [-1.2, 0.76, -2.2])), (wood, bx([-2.95, 0.0, -3.15], [-1.25, 0.7, -2.25])),
            (cloth, bx([1.0, 0.0, -3.9], [3.6, 0.45, -3.0])), (cloth, bx([1.0, 0.45, -3.9], [3.6, 1.0, -3.6])),
            // Room 2: a table under the pendant, a cabinet.
            (wood, bx([-0.9, 0.72, -8.0], [0.9, 0.78, -6.9])), (wood, bx([-0.85, 0.0, -7.95], [0.85, 0.72, -6.95])),
            (wood, bx([-3.9, 0.0, -9.5], [-3.3, 1.8, -7.5])),
            // Room 3: two benches and a block.
            (cloth, bx([-3.0, 0.0, -14.0], [-1.0, 0.5, -13.2])), (cloth, bx([1.0, 0.0, -14.0], [3.0, 0.5, -13.2])),
            (plaster, bx([-0.6, 0.0, -16.4], [0.6, 1.2, -15.6])),
        ];
        for (m, f) in &furniture { scene.add_instance(*m, *f); }
        let footprint = |xf: &Affine3A| {
            let c = Vec3::from(xf.translation);
            let h = Vec3::new(xf.matrix3.x_axis.x, xf.matrix3.y_axis.y, xf.matrix3.z_axis.z) * 0.5;
            // Only what blocks a walker: things that reach below 2 m.
            (c.y - h.y < 2.0).then(|| (Vec2::new(c.x - h.x, c.z - h.z), Vec2::new(c.x + h.x, c.z + h.z)))
        };
        // The window sill is low: keep it solid too.
        walls.retain(|w| w.translation.y < 2.0 || w.matrix3.y_axis.y > 2.0);
        let solid = walls.iter().chain(furniture.iter().map(|f| &f.1)).filter_map(footprint).collect();
        let grid = Grid { origin: Vec3::new(-5.75, -0.25, -17.75), cell: 0.5, dims: [24, 9, 48] };
        scene.reserve_irradiance_volume(grid).expect("probe grid");
        // The static geometry, before the moving pieces join the scene.
        let occluders = render::room_bake::Occluders::from_scene(&scene, |_| true);
        let mut objects = Slots::reserve(&mut scene, 16);
        let bulb = objects.add(&mut scene, bulb_mesh, Affine3A::IDENTITY).unwrap();
        let cord = objects.add(&mut scene, cord_mesh, Affine3A::IDENTITY).unwrap();
        let lamp_slots = LAMPS.iter().map(|p| objects.add(&mut scene, lamp_on_mesh,
            Affine3A::from_scale_rotation_translation(Vec3::new(0.5, 0.06, 0.5), Quat::IDENTITY, *p + Vec3::Y * 0.03)).unwrap()).collect();
        let mut light = Lighting::default();
        light.fog_start = 1000.0;
        light.fog_end = 2000.0;
        light.fog_density = 0.0;
        light.auto_exposure = Some(render::AutoExposure { min_ev100: -1.0, max_ev100: 16.0, ..Default::default() });
        Rooms { scene, grid, solid, objects, bulb_mesh, cord_mesh, lamp_on_mesh, lamp_off_mesh, bulb, cord, lamp_slots,
            lamp_on: [true; 4], occluders, sky: Vec::new(), planes: Vec::new(), lamp_bounce: Vec::new(), dirty: true,
            pos: Vec3::new(1.5, EYE, 12.0), yaw: 0.12, pitch: 0.0, vy: 0.0, t: 0.0, light, ready: false, ev: None }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    /// The pendant's bulb position: a pendulum swinging in x and a little in z.
    fn bulb_at(&self) -> Vec3 {
        let a = 0.45 * (self.t * 2.0).sin();
        let b = 0.15 * (self.t * 1.37).sin();
        PIVOT + Vec3::new(a.sin(), -a.cos() * b.cos(), b.sin()) * CORD
    }

    fn lights(&self) -> Vec<ClusteredLight> {
        let bulb = self.bulb_at();
        let pendant = LocalLight { position: bulb, direction: Vec3::NEG_Y, intensity_cd: warm(160.0), range_m: 9.0,
            inner_cos: -0.9, outer_cos: -1.0, source_radius_m: 0.06, falloff: 2.0 };
        // Instant radiosity: the floor under the bulb is its brightest
        // reflector. A Lambertian floor lit by a point lamp reflects
        // reflectance * I * 2 pi lumen; as an upward Lambertian emitter that
        // is reflectance * I * 2 candela on its axis. 0.45: tiles.
        let spot = Vec3::new(bulb.x, 0.35, bulb.z);
        let bounce = LocalLight { position: spot, direction: Vec3::Y, intensity_cd: warm(160.0) * 0.45 * 2.0, range_m: 7.0,
            inner_cos: 0.0, outer_cos: -0.2, source_radius_m: 0.9, falloff: 2.0 };
        let mut lights = vec![ClusteredLight::shadowed(pendant, 1), ClusteredLight::shadowed(bounce, 2)];
        for (i, p) in LAMPS.iter().enumerate() {
            if self.lamp_on[i] { lights.push(ClusteredLight::shadowed(ceiling_lamp(*p), 10 + i as u64)); }
        }
        lights
    }

    fn toggle(&mut self, i: usize) {
        self.lamp_on[i] = !self.lamp_on[i];
        let mesh = if self.lamp_on[i] { self.lamp_on_mesh } else { self.lamp_off_mesh };
        self.objects.set(&mut self.scene, self.lamp_slots[i], mesh,
            Affine3A::from_scale_rotation_translation(Vec3::new(0.5, 0.06, 0.5), Quat::IDENTITY, LAMPS[i] + Vec3::Y * 0.03));
        self.dirty = true;
    }

    fn upload_probes(&mut self, frame: &Frame) -> Result<(), String> {
        let mut emission = vec![Probe::default(); self.sky.len()];
        for (l, set) in self.lamp_bounce.iter().enumerate() {
            if !self.lamp_on[l] { continue; }
            for (e, p) in emission.iter_mut().zip(set) { for a in 0..6 { e.axes[a] += p.axes[a]; } }
        }
        frame.renderer().ok_or("no renderer")?.set_sky_sun_emission(frame.gpu, &self.sky, &emission, &self.planes)
    }
}

impl Game for Rooms {
    fn local_players(&self) -> usize { 1 }

    fn tick(&mut self, c: &[Controls]) {
        let c = c[0];
        let dt = 1.0 / TICK_HZ;
        self.t += dt;
        self.yaw -= c.look[0];
        self.pitch = (self.pitch + c.look[1]).clamp(-1.4, 1.4);
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        // Shift sprints, space jumps.
        let speed = if c.held.has(app::Buttons::RIGHT_SHOULDER) { 7.0 } else { 3.0 };
        let step = (right * c.mv[0] + fwd * c.mv[1]) * speed * dt;
        let ground = self.pos.y <= EYE + 1e-3;
        if ground && c.pressed.has(app::Buttons::SOUTH) { self.vy = 4.5; }
        self.vy -= 12.0 * dt;
        self.pos.y += self.vy * dt;
        if self.pos.y < EYE { self.pos.y = EYE; self.vy = 0.0; }
        for axis in [0, 2] {
            let mut next = self.pos;
            next[axis] += step[axis];
            let p = Vec2::new(next.x, next.z);
            if !self.solid.iter().any(|(lo, hi)| p.cmpgt(*lo - 0.25).all() && p.cmplt(*hi + 0.25).all()) { self.pos = next; }
        }
        if c.pressed.has(Buttons::NORTH) {
            // The lamp nearest the view ray.
            let f = self.forward();
            let best = (0..4).map(|i| (i, f.dot((LAMPS[i] - self.pos).normalize()))).max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, d)) = best { if d > 0.8 { self.toggle(i); } }
        }
        if c.pressed.has(Buttons::WEST) {
            let on = !self.lamp_on.iter().any(|&o| o);
            for i in 0..4 { if self.lamp_on[i] != on { self.toggle(i); } }
        }
        // The pendant: cord from the pivot to the bulb.
        let bulb = self.bulb_at();
        let mid = (PIVOT + bulb) * 0.5;
        let rot = Quat::from_rotation_arc(Vec3::Y, (PIVOT - bulb).normalize());
        self.objects.set(&mut self.scene, self.bulb, self.bulb_mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(0.12), Quat::IDENTITY, bulb));
        self.objects.set(&mut self.scene, self.cord, self.cord_mesh, Affine3A::from_scale_rotation_translation(Vec3::new(0.015, CORD, 0.015), rot, mid));
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !std::mem::replace(&mut self.ready, true) {
            frame.set_scene(&self.scene, &self.light);
            let occluders = &self.occluders;
            let settings = render::room_bake::Settings::default();
            let t = std::time::Instant::now();
            self.sky = render::room_bake::bake(&self.grid, occluders, self.light.sun_dir, &settings)?;
            self.planes = render::room_bake::separation_planes(&self.grid, occluders);
            let lamps: Vec<LocalLight> = LAMPS.iter().map(|p| ceiling_lamp(*p)).collect();
            self.lamp_bounce = render::room_bake::bake_lights(&self.grid, occluders, &lamps, &render::room_bake::Settings { bounces: 2, ..settings })?;
            eprintln!("room bake: {} triangles, {} probes, {:.1} s", occluders.triangles(), self.grid.count(), t.elapsed().as_secs_f32());
        }
        if std::mem::take(&mut self.dirty) { self.upload_probes(frame)?; }
        let renderer = frame.renderer().ok_or("no renderer")?;
        renderer.set_clustered_lights(&self.lights())?;
        self.objects.flush(frame.gpu, renderer, &self.scene)?;
        self.ev = renderer.exposure_ev100(0).zip(renderer.metered_ev100(0));
        let cam = Camera { pos: self.pos, target: self.pos + self.forward(), fov_y: 70f32.to_radians(), near: 0.05 };
        let mut hud = HudDrawList::new(frame.size);
        let [w, h] = frame.size.map(|v| v as f32);
        let room = match self.pos.z { z if z > 4.0 => "outside", z if z > -4.15 => "1 daylight", z if z > -10.45 => "2 pendant", _ => "3 switches (F: lamp, R: all)" };
        let ev = self.ev.map_or("metering".into(), |(ev, m)| format!("EV {ev:.1} (meter {m:.1})"));
        hud.text.push(TextRun { id: 1, text: format!("{room}   {ev}"), position: [20.0, 16.0], bounds: [0.0, 0.0, w, h],
            font_size: 26.0, color: [255, 255, 255, 220], font: FontFace::Body, align: TextAlign::Left, width: 900.0 });
        Ok(Draw { cameras: vec![cam], light: self.light.clone(), hud })
    }

    fn state(&self) -> Option<String> {
        Some(format!(r#"{{"pos":[{},{},{}],"yaw":{},"lamps":{:?},"ev":{:?}}}"#, self.pos.x, self.pos.y, self.pos.z, self.yaw, self.lamp_on, self.ev.map(|e| e.0)))
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let mut game = Rooms::new();
    if let Some(at) = arg("--at") {
        let v: Vec<f32> = at.split(',').filter_map(|s| s.parse().ok()).collect();
        if v.len() == 3 { game.pos = Vec3::new(v[0], v[1], v[2]); }
    }
    if let Some(yaw) = arg("--yaw").and_then(|y| y.parse().ok()) { game.yaw = yaw; }
    if let Some(pitch) = arg("--pitch").and_then(|y| y.parse().ok()) { game.pitch = pitch; }
    if let Some(off) = arg("--off") { for i in off.split(',').filter_map(|s| s.parse::<usize>().ok()).filter(|&i| i < 4) { game.toggle(i); } }
    if let Some(out) = arg("--shot") {
        let gpu = render::Gpu::new()?;
        let rgba = app::capture(&mut game, &gpu, [960, 540], 30, TICK_HZ)?;
        return render::write_png(std::path::Path::new(&out), 960, 540, &rgba);
    }
    app::run(app::Config { title: "Warp: three rooms".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut game)
}
