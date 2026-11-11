//! Four joined rooms, each lit another way, to see how indoor lighting
//! techniques combine. See engine/docs/interior-light.md.
//!
//!   cargo run --release -p app --example interior_rooms
//!   cargo run --release -p app --example interior_rooms -- --shot out.png --at 0,1.65,-7 --yaw 0
//!   (also --pitch, --off 0,2 (lamps off), --fog, --flashlight, --door 0..1)
//!
//! 1. Daylight room (front door and a window): sky and sun only, baked with
//!    several bounces (`room_bake::bake`), sun shadows live.
//! 2. Pendant room: one amber lamp swinging on its cord. Direct light and
//!    its shadow are live (a shadowed clustered light); its bounce cannot be
//!    baked, so a second, soft light sits over whatever the bulb shines down
//!    on (found by a ray against the baked geometry) and shines back up (one
//!    virtual point light, "instant radiosity").
//! 3. Switch room: four ceiling lamps, warm, red, teal and blue. F toggles
//!    the one you look at, R all. Direct light live and shadowed; each
//!    lamp's bounce is baked on its own (`room_bake::bake_lights`) and the
//!    lit lamps' bounces are added into the probes when a switch flips (Call
//!    of Duty's UberBake idea), so the bounce colour follows the switches.
//! 4. Dark room: no lamp, no window, one hinged door from room 3 (click to
//!    open or close it). Two light states are baked, door shut and door open
//!    (sky, sun and every lamp's bounce); while the door swings the probes
//!    are blended by how far it is open and uploaded again (Modern Warfare's
//!    light-state switching). Direct light through the gap, the door's
//!    shadows and reflections stay live. Shut, it is near black: the eye
//!    adapts down to EV -3 over a few seconds.
//!
//! Q (or LB) toggles a flashlight: a shadowed spot right of and below the
//! eye that lags the view a little, with one soft fake bounce light where
//! its beam lands. Tab (or Select) toggles a light indoor haze
//! (`Lighting::volumetric`) in which the lamps and the flashlight scatter;
//! the medium has no occlusion, so their scattering copies reach only about
//! 3 m, not through the walls. Shift sprints, Space jumps.
//!
//! HDR output is requested (`Game::hdr`): on an HDR screen the sky through
//! the door and the lamps go above SDR white. The HUD says whether it is on.
use app::{Buttons, Controls, Draw, Frame, Game};
use glam::{Affine3A, Quat, Vec2, Vec3};
use render::clustered_lights::ClusteredLight;
use render::headlights::LocalLight;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::{Grid, Probe};
use render::room_bake::Occluders;
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
const WARM: Vec3 = Vec3::new(1.0, 0.78, 0.55);
/// Switch room lamp colours: warm, red, teal, blue.
const LAMP_COLOURS: [Vec3; 4] = [WARM, Vec3::new(1.0, 0.22, 0.15), Vec3::new(0.2, 1.0, 0.8), Vec3::new(0.25, 0.4, 1.0)];
/// The pendant: amber, against the teal and blue next door.
const PENDANT: Vec3 = Vec3::new(1.0, 0.5, 0.12);
/// Room 3 | 4 doorway in the x = -4.3..-4.0 wall, and the door's hinge.
const DOOR_Z: [f32; 2] = [-12.4, -11.4];
const DOOR_TOP: f32 = 2.1;
const HINGE: Vec3 = Vec3::new(-4.15, 0.0, -12.4);
/// How far the door swings (into room 4) and how long a swing takes.
const DOOR_ANGLE: f32 = 1.75;
const DOOR_SECONDS: f32 = 0.6;
/// Diffuse reflectance assumed for the fake bounce lights.
const BOUNCE_REFLECTANCE: f32 = 0.4;

fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
    let (a, b) = (Vec3::from(min), Vec3::from(max));
    Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
}

/// `c` scaled to the warm white's luminance, times `cd` candela.
fn colour(c: Vec3, cd: f32) -> Vec3 {
    let lum = |v: Vec3| v.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    c * (lum(WARM) / lum(c)) * cd
}

fn ceiling_lamp(i: usize) -> LocalLight {
    // A wide downlight: most light down, some to the walls.
    LocalLight { position: LAMPS[i] - Vec3::Y * 0.12, direction: Vec3::NEG_Y, intensity_cd: colour(LAMP_COLOURS[i], 450.0), range_m: 10.0,
        inner_cos: 50f32.to_radians().cos(), outer_cos: 80f32.to_radians().cos(), source_radius_m: 0.12, falloff: 2.0 }
}

/// The door panel `open` (0 shut, 1 open) of the way round its hinge.
fn door_xf(open: f32) -> Affine3A {
    Affine3A::from_rotation_translation(Quat::from_rotation_y(-open * DOOR_ANGLE), HINGE)
        * Affine3A::from_scale_rotation_translation(Vec3::new(0.06, DOOR_TOP - 0.01, 0.98), Quat::IDENTITY, Vec3::new(0.0, DOOR_TOP * 0.5, 0.5))
}

/// Smoothstep: the door eases in and out.
fn ease(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// One baked light state: sky and sun, each lamp's bounce, separation planes.
struct LightState {
    sky: Vec<Probe>,
    lamps: Vec<Vec<Probe>>,
    planes: Vec<[f32; 4]>,
}

impl LightState {
    fn bake(grid: &Grid, occluders: &Occluders, sun: Vec3) -> Result<Self, String> {
        let settings = render::room_bake::Settings::default();
        let lamps: Vec<LocalLight> = (0..4).map(ceiling_lamp).collect();
        Ok(LightState {
            sky: render::room_bake::bake(grid, occluders, sun, &settings)?,
            lamps: render::room_bake::bake_lights(grid, occluders, &lamps, &render::room_bake::Settings { bounces: 2, ..settings })?,
            planes: render::room_bake::separation_planes(grid, occluders),
        })
    }
}

struct Rooms {
    scene: Scene,
    grid: Grid,
    solid: Vec<(Vec2, Vec2)>,
    objects: Slots,
    bulb_mesh: usize,
    cord_mesh: usize,
    door_mesh: usize,
    lamp_on_mesh: [usize; 4],
    lamp_off_mesh: usize,
    bulb: SlotId,
    cord: SlotId,
    door: SlotId,
    lamp_slots: Vec<SlotId>,
    lamp_on: [bool; 4],
    /// The static geometry without the door, and with it shut.
    open_occluders: Occluders,
    shut_occluders: Occluders,
    /// Baked light with the door shut and open.
    shut: Option<LightState>,
    open: Option<LightState>,
    /// The door: target (shut or open), progress 0..1 towards open, and the
    /// fraction the probes were last uploaded at.
    door_wanted: bool,
    door_t: f32,
    /// `--door`: keep the door where it was put (for captures).
    hold_door: bool,
    uploaded: Option<(f32, [bool; 4])>,
    /// Indoor fog (`Lighting::volumetric`).
    fog: bool,
    flashlight: bool,
    /// The flashlight's direction, lagging the view (per drawn frame).
    flash_dir: Vec3,
    pos: Vec3,
    prev: Vec3,
    yaw: f32,
    pitch: f32,
    vy: f32,
    t: f32,
    light: Lighting,
    ready: bool,
    ev: Option<(f32, f32)>,
    hdr: Option<(render::hdr::HdrDisplay, f32)>,
}

impl Rooms {
    fn new() -> Self {
        let mut scene = Scene::default();
        scene.graphics.clustered_lights = true;
        let mat = |scene: &mut Scene, tint: [f32; 3], rough: f32, flags: u32, scale: f32| {
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
        let door_mesh = mat(&mut scene, [0.35, 0.25, 0.18], 0.6, 0, 1.0);
        let bulb_mesh = {
            let m = scene.add_material(Material { tint: PENDANT.to_array(), layer: -1, roughness: 1.0, flags: EMISSIVE, scale: 40_000.0, metal: 0.0 });
            scene.add_mesh(&render::primitives::sphere(m, 8, 12).mesh())
        };
        let lamp_on_mesh = LAMP_COLOURS.map(|c| mat(&mut scene, (c / c.max_element()).to_array(), 1.0, EMISSIVE, 25_000.0));
        let lamp_off_mesh = mat(&mut scene, [0.8, 0.8, 0.78], 0.5, 0, 1.0);
        let cord_mesh = dark;
        scene.add_instance(grass, bx([-150.0, -1.0, -150.0], [150.0, 0.0, 150.0]));
        // Shell: x -4..4 inside, rooms 1-3 one behind the other along -z,
        // room 4 west of room 3 (x -9..-4.3).
        let mut walls = vec![
            // Front wall with the door (x -0.6..0.6).
            bx([-4.3, 0.0, 4.0], [-0.6, H, 4.3]), bx([0.6, 0.0, 4.0], [4.3, H, 4.3]), bx([-0.6, 2.2, 4.0], [0.6, H, 4.3]),
            // West wall of room 1, with a window z -1.2..1.2, y 0.9..2.2.
            bx([-4.3, 0.0, -4.0], [-4.0, H, -1.2]), bx([-4.3, 0.0, 1.2], [-4.0, H, 4.3]),
            bx([-4.3, 0.0, -1.2], [-4.0, 0.9, 1.2]), bx([-4.3, 2.2, -1.2], [-4.0, H, 1.2]),
            // East wall of all rooms; west wall of rooms 2 and 3, with the
            // door to room 4.
            bx([4.0, 0.0, -16.9], [4.3, H, 4.3]), bx([-4.3, 0.0, DOOR_Z[1]], [-4.0, H, -4.0]),
            bx([-4.3, 0.0, -16.9], [-4.0, H, DOOR_Z[0]]), bx([-4.3, DOOR_TOP, DOOR_Z[0]], [-4.0, H, DOOR_Z[1]]),
            // Room 1 | 2, doorway x -0.5..0.5.
            bx([-4.0, 0.0, -4.3], [-0.5, H, -4.0]), bx([0.5, 0.0, -4.3], [4.0, H, -4.0]), bx([-0.5, 2.1, -4.3], [0.5, H, -4.0]),
            // Room 2 | 3, doorway x 2.2..3.2: no straight view from the front door.
            // 0.6 m thick, so that it holds a probe layer (see room 4).
            bx([-4.0, 0.0, -10.8], [2.2, H, -10.2]), bx([3.2, 0.0, -10.8], [4.0, H, -10.2]), bx([2.2, 2.1, -10.8], [3.2, H, -10.2]),
            // Back wall.
            bx([-4.3, 0.0, -16.9], [4.3, H, -16.6]),
            // Room 4: north, south and west walls. Every outside wall holds a
            // probe layer (0.5 m grid): a wall without one lets probes on its
            // far side, in daylight, blend into the room.
            bx([-9.3, 0.0, -10.8], [-4.3, H, -10.2]), bx([-9.3, 0.0, -16.9], [-4.3, H, -16.6]), bx([-9.3, 0.0, -16.9], [-9.0, H, -10.3]),
        ];
        for w in &walls { scene.add_instance(plaster, *w); }
        scene.add_instance(plaster, bx([-4.3, H, -16.9], [4.3, H + 0.3, 4.3]));
        scene.add_instance(plaster, bx([-9.3, H, -16.9], [-4.3, H + 0.3, -10.2]));
        // Coloured inner faces: thin panels on room 2 (blue) and 3 (green).
        for (m, b) in [
            (blue, bx([-4.0, 0.0, -10.2], [-3.98, H, -4.3])), (blue, bx([3.98, 0.0, -10.2], [4.0, H, -4.3])),
            (green, bx([-4.0, 0.0, -16.6], [-3.98, H, DOOR_Z[0]])), (green, bx([-4.0, 0.0, DOOR_Z[1]], [-3.98, H, -10.8])),
            (green, bx([-4.0, DOOR_TOP, DOOR_Z[0]], [-3.98, H, DOOR_Z[1]])), (green, bx([3.98, 0.0, -16.6], [4.0, H, -10.8])),
        ] { scene.add_instance(m, b); }
        scene.add_instance(wood, bx([-4.0, 0.0, -4.0], [4.0, 0.02, 4.0]));
        scene.add_instance(tiles, bx([-4.0, 0.0, -10.2], [4.0, 0.02, -4.3]));
        scene.add_instance(wood, bx([-4.0, 0.0, -16.6], [4.0, 0.02, -10.8]));
        scene.add_instance(wood, bx([-9.0, 0.0, -16.6], [-4.0, 0.02, -10.8]));
        let furniture = [
            // Room 1: table and couch (against the wall: no gap behind it).
            (wood, bx([-3.0, 0.7, -3.2], [-1.2, 0.76, -2.2])), (wood, bx([-2.95, 0.0, -3.15], [-1.25, 0.7, -2.25])),
            (cloth, bx([1.0, 0.0, -4.0], [3.6, 0.45, -3.0])), (cloth, bx([1.0, 0.45, -4.0], [3.6, 1.0, -3.6])),
            // Room 2: a table under the pendant, a cabinet.
            (wood, bx([-0.9, 0.72, -8.0], [0.9, 0.78, -6.9])), (wood, bx([-0.85, 0.0, -7.95], [0.85, 0.72, -6.95])),
            (wood, bx([-3.9, 0.0, -9.5], [-3.3, 1.8, -7.5])),
            // Room 3: two benches and a block.
            (cloth, bx([-3.0, 0.0, -14.0], [-1.0, 0.5, -13.2])), (cloth, bx([1.0, 0.0, -14.0], [3.0, 0.5, -13.2])),
            (plaster, bx([-0.6, 0.0, -16.4], [0.6, 1.2, -15.6])),
            // Room 4: crates and a shelf, for the flashlight's shadows.
            (wood, bx([-8.6, 0.0, -16.2], [-7.6, 1.0, -15.2])), (wood, bx([-8.4, 1.0, -16.0], [-7.8, 1.6, -15.4])),
            (wood, bx([-8.9, 0.0, -13.5], [-8.5, 2.0, -11.0])), (cloth, bx([-6.5, 0.0, -14.8], [-5.5, 0.6, -13.8])),
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
        let grid = Grid { origin: Vec3::new(-9.75, -0.25, -17.75), cell: 0.5, dims: [32, 9, 48] };
        scene.reserve_irradiance_volume(grid).expect("probe grid");
        // The static geometry, before the moving pieces join the scene, with
        // and without the shut door.
        let open_occluders = Occluders::from_scene(&scene, |_| true);
        let shut_occluders = open_occluders.merged(&Occluders::from_boxes(&[door_xf(0.0)]));
        let mut objects = Slots::reserve(&mut scene, 16);
        let bulb = objects.add(&mut scene, bulb_mesh, Affine3A::IDENTITY).unwrap();
        let cord = objects.add(&mut scene, cord_mesh, Affine3A::IDENTITY).unwrap();
        let door = objects.add(&mut scene, door_mesh, door_xf(0.0)).unwrap();
        let lamp_slots = LAMPS.iter().zip(lamp_on_mesh).map(|(p, mesh)| objects.add(&mut scene, mesh,
            Affine3A::from_scale_rotation_translation(Vec3::new(0.5, 0.06, 0.5), Quat::IDENTITY, *p + Vec3::Y * 0.03)).unwrap()).collect();
        let mut light = Lighting::default();
        light.fog_start = 1000.0;
        light.fog_end = 2000.0;
        light.fog_density = 0.0;
        // Down to EV -3: a shut dark room is shown as dark as it is.
        light.auto_exposure = Some(render::AutoExposure { min_ev100: -3.0, max_ev100: 16.0, ..Default::default() });
        // ROOMS_LIGHT="gi=0,taa=0": Lighting::set keys, for checks.
        for kv in std::env::var("ROOMS_LIGHT").unwrap_or_default().split(',').filter(|s| !s.is_empty()) { light.set(kv).expect("ROOMS_LIGHT"); }
        let pos = Vec3::new(1.5, EYE, 12.0);
        Rooms { scene, grid, solid, objects, bulb_mesh, cord_mesh, door_mesh, lamp_on_mesh, lamp_off_mesh, bulb, cord, door, lamp_slots,
            lamp_on: [true; 4], open_occluders, shut_occluders, shut: None, open: None, door_wanted: false, door_t: 0.0, hold_door: false, uploaded: None,
            fog: false, flashlight: false, flash_dir: Vec3::NEG_Z, pos, prev: pos, yaw: 0.12, pitch: 0.0, vy: 0.0, t: 0.0, light,
            ready: false, ev: None, hdr: None }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    fn door_open(&self) -> f32 { ease(self.door_t) }

    /// The geometry the fake bounce lights aim at: the door as it stands
    /// (shut or not).
    fn occluders(&self) -> &Occluders { if self.door_open() < 0.3 { &self.shut_occluders } else { &self.open_occluders } }

    /// The pendant's bulb position: a pendulum swinging in x and a little in z.
    fn bulb_at(&self) -> Vec3 {
        let a = 0.45 * (self.t * 2.0).sin();
        let b = 0.15 * (self.t * 1.37).sin();
        PIVOT + Vec3::new(a.sin(), -a.cos() * b.cos(), b.sin()) * CORD
    }

    fn pendant(&self) -> LocalLight {
        LocalLight { position: self.bulb_at(), direction: Vec3::NEG_Y, intensity_cd: colour(PENDANT, 160.0), range_m: 9.0,
            inner_cos: -0.9, outer_cos: -1.0, source_radius_m: 0.06, falloff: 2.0 }
    }

    /// One virtual point light for `lamp`'s first bounce: where its light
    /// in a cone of `half` (radians) around `dir` lands, averaged over nine
    /// rays (one ray jumps from a table to the floor as the lamp moves; the
    /// average slides), pulled `back` metres off the surface, shining away
    /// from it. `flux_sr`: the solid angle of the lamp's light in the cone.
    /// A Lambertian surface reflecting flux F shines F / pi candela along
    /// its normal, falling off with the angle (a soft cone, not a hemisphere
    /// cut off at the surface: that showed as a blob with a hard edge).
    fn bounce(&self, lamp: &LocalLight, dir: Vec3, half: f32, flux_sr: f32, back: f32) -> Option<LocalLight> {
        let side = dir.any_orthonormal_vector();
        let up = dir.cross(side);
        let rays = std::iter::once(dir).chain((0..8).map(|k| {
            let a = k as f32 * std::f32::consts::TAU / 8.0;
            let s = (half * 0.6).sin();
            (dir * (half * 0.6).cos() + (side * a.cos() + up * a.sin()) * s).normalize()
        }));
        let (mut at, mut normal, mut hits) = (Vec3::ZERO, Vec3::ZERO, 0);
        for d in rays {
            let Some((t, n)) = self.occluders().hit(lamp.position, d) else { continue };
            at += lamp.position + d * t;
            normal += if n.dot(d) > 0.0 { -n } else { n };
            hits += 1;
        }
        if hits == 0 { return None; }
        let (at, n) = (at / hits as f32, normal.normalize_or(-dir));
        Some(LocalLight { position: at + n * back, direction: n,
            intensity_cd: lamp.intensity_cd * BOUNCE_REFLECTANCE * flux_sr * (hits as f32 / 9.0) / std::f32::consts::PI, range_m: 7.0,
            inner_cos: 0.6, outer_cos: -0.4, source_radius_m: 1.0, falloff: 2.0 })
    }

    fn flashlight_light(&self) -> LocalLight {
        let f = self.forward();
        let right = f.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(f);
        LocalLight { position: self.pos + right * 0.22 - up * 0.25, direction: self.flash_dir,
            intensity_cd: Vec3::new(0.92, 0.96, 1.0) * 2000.0, range_m: 25.0,
            inner_cos: 12f32.to_radians().cos(), outer_cos: 28f32.to_radians().cos(), source_radius_m: 0.02, falloff: 2.0 }
    }

    /// What scatters in the fog: the pendant, the lit ceiling lamps and the
    /// flashlight, with a shorter range than their surface light: the
    /// medium has no occlusion, so a long range would glow through walls.
    fn beams(&self) -> Vec<LocalLight> {
        let mut beams = vec![LocalLight { range_m: 3.0, ..self.pendant() }];
        beams.extend((0..4).filter(|&i| self.lamp_on[i]).map(|i| LocalLight { range_m: 3.2, ..ceiling_lamp(i) }));
        if self.flashlight { beams.push(LocalLight { range_m: 6.0, ..self.flashlight_light() }); }
        beams
    }

    fn lights(&self) -> Vec<ClusteredLight> {
        let pendant = self.pendant();
        // The light sits inside its bulb: the bulb and cord (the first two
        // slots) must not cast its shadow, or it shadows itself in blotches.
        let mut lit = ClusteredLight::shadowed(pendant, 1);
        if let Some(s) = &mut lit.shadow { s.exclude = Some((self.objects.instance(self.bulb) as u32, 2)); }
        let mut lights = vec![lit];
        // Instant radiosity: the bulb's brightest reflector is what lies
        // under it (the table, or the floor). A cone of 35 degrees half
        // angle straight down (1.1 sr) holds about what the table catches.
        if let Some(b) = self.bounce(&pendant, Vec3::NEG_Y, 35f32.to_radians(), 1.1, 0.3) { lights.push(ClusteredLight::shadowed(b, 2)); }
        for i in 0..4 {
            if self.lamp_on[i] { lights.push(ClusteredLight::shadowed(ceiling_lamp(i), 10 + i as u64)); }
        }
        if self.flashlight {
            let flash = self.flashlight_light();
            lights.push(ClusteredLight::shadowed(flash, 20));
            // The beam's cone (28 degrees half angle) is 0.74 sr.
            if let Some(b) = self.bounce(&flash, flash.direction, 20f32.to_radians(), 0.74, 0.3) { lights.push(ClusteredLight::unshadowed(b)); }
        }
        lights
    }

    fn toggle(&mut self, i: usize) {
        self.lamp_on[i] = !self.lamp_on[i];
        let mesh = if self.lamp_on[i] { self.lamp_on_mesh[i] } else { self.lamp_off_mesh };
        self.objects.set(&mut self.scene, self.lamp_slots[i], mesh,
            Affine3A::from_scale_rotation_translation(Vec3::new(0.5, 0.06, 0.5), Quat::IDENTITY, LAMPS[i] + Vec3::Y * 0.03));
    }

    fn set_fog(&mut self, on: bool) {
        self.fog = on;
        // A light indoor haze: the lamps' cones show, the room stays clear.
        // It only adds light (no extinction): the distance fog owns that.
        self.light.volumetric = on.then(|| render::volumetric::Volumetric { density: 0.06, anisotropy: 0.45, intensity: 4.0, ..Default::default() });
    }

    /// The probes for the door at `open` and the lamps that are on: the two
    /// baked states blended. Cheap (a few hundred thousand adds): done for
    /// every drawn frame of a swing.
    fn upload_probes(&mut self, frame: &Frame) -> Result<(), String> {
        let (Some(shut), Some(open)) = (&self.shut, &self.open) else { return Ok(()) };
        let k = self.door_open();
        let mix = |a: &Probe, b: &Probe| Probe { axes: std::array::from_fn(|i| a.axes[i].lerp(b.axes[i], k)) };
        let sky: Vec<Probe> = shut.sky.iter().zip(&open.sky).map(|(a, b)| mix(a, b)).collect();
        let mut emission = vec![Probe::default(); sky.len()];
        for l in (0..4).filter(|&l| self.lamp_on[l]) {
            for ((e, a), b) in emission.iter_mut().zip(&shut.lamps[l]).zip(&open.lamps[l]) {
                let p = mix(a, b);
                for i in 0..6 { e.axes[i] += p.axes[i]; }
            }
        }
        // Separation planes cannot blend: the door counts as a wall until
        // it is a third of the way open.
        let planes = if k < 0.3 { &shut.planes } else { &open.planes };
        frame.renderer().ok_or("no renderer")?.set_sky_sun_emission(frame.gpu, &sky, &emission, planes)?;
        self.uploaded = Some((k, self.lamp_on));
        Ok(())
    }

    /// Whether the eye is near the door and looking at it.
    fn at_door(&self) -> bool {
        let centre = Vec3::new(-4.15, 1.2, (DOOR_Z[0] + DOOR_Z[1]) * 0.5);
        let to = centre - self.pos;
        to.length() < 2.5 && self.forward().dot(to.normalize()) > 0.6
    }
}

impl Game for Rooms {
    fn local_players(&self) -> usize { 1 }

    fn hdr(&self) -> Option<bool> { Some(true) }

    fn tick(&mut self, c: &[Controls]) {
        let c = c[0];
        let dt = 1.0 / TICK_HZ;
        self.t += dt;
        self.prev = self.pos;
        self.yaw -= c.look[0];
        self.pitch = (self.pitch + c.look[1]).clamp(-1.4, 1.4);
        let fwd = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        // Shift sprints, space jumps.
        let speed = if c.held.has(Buttons::RIGHT_SHOULDER) { 7.0 } else { 3.0 };
        let step = (right * c.mv[0] + fwd * c.mv[1]) * speed * dt;
        let ground = self.pos.y <= EYE + 1e-3;
        if ground && c.pressed.has(Buttons::SOUTH) { self.vy = 4.5; }
        self.vy -= 12.0 * dt;
        self.pos.y += self.vy * dt;
        if self.pos.y < EYE { self.pos.y = EYE; self.vy = 0.0; }
        // The door blocks while it is less than half open.
        let door_box = (self.door_open() < 0.5).then(|| (Vec2::new(-4.3, DOOR_Z[0]), Vec2::new(-4.0, DOOR_Z[1])));
        for axis in [0, 2] {
            let mut next = self.pos;
            next[axis] += step[axis];
            let p = Vec2::new(next.x, next.z);
            if !self.solid.iter().chain(door_box.iter()).any(|(lo, hi)| p.cmpgt(*lo - 0.25).all() && p.cmplt(*hi + 0.25).all()) { self.pos = next; }
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
        if c.pressed.has(Buttons::SELECT) { self.set_fog(!self.fog); }
        if c.pressed.has(Buttons::LEFT_SHOULDER) {
            self.flashlight = !self.flashlight;
            self.flash_dir = self.forward();
        }
        if (c.pressed.has(Buttons::RIGHT_TRIGGER) || c.pressed.has(Buttons::EAST)) && self.at_door() { self.door_wanted = !self.door_wanted; }
        let target = if self.door_wanted { 1.0 } else { 0.0 };
        if !self.hold_door { self.door_t += (target - self.door_t).clamp(-dt / DOOR_SECONDS, dt / DOOR_SECONDS); }
        let door = door_xf(self.door_open());
        self.objects.set(&mut self.scene, self.door, self.door_mesh, door);
        // The pendant: cord from the pivot to the bulb.
        let bulb = self.bulb_at();
        let mid = (PIVOT + bulb) * 0.5;
        let rot = Quat::from_rotation_arc(Vec3::Y, (PIVOT - bulb).normalize());
        self.objects.set(&mut self.scene, self.bulb, self.bulb_mesh, Affine3A::from_scale_rotation_translation(Vec3::splat(0.05), Quat::IDENTITY, bulb));
        self.objects.set(&mut self.scene, self.cord, self.cord_mesh, Affine3A::from_scale_rotation_translation(Vec3::new(0.012, CORD - 0.05, 0.012), rot, mid));
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !std::mem::replace(&mut self.ready, true) {
            frame.set_scene(&self.scene, &self.light);
            let t = std::time::Instant::now();
            let sun = self.light.sun_dir;
            let (grid, shut_occ, open_occ) = (&self.grid, &self.shut_occluders, &self.open_occluders);
            let (shut, open) = std::thread::scope(|s| {
                let shut = s.spawn(|| LightState::bake(grid, shut_occ, sun));
                let open = LightState::bake(grid, open_occ, sun);
                (shut.join().map_err(|_| "bake thread panicked".to_string()).and_then(|r| r), open)
            });
            self.shut = Some(shut?);
            self.open = Some(open?);
            eprintln!("room bake: {} triangles, {} probes, two door states, {:.1} s", self.open_occluders.triangles(), self.grid.count(), t.elapsed().as_secs_f32());
        }
        if self.uploaded != Some((self.door_open(), self.lamp_on)) { self.upload_probes(frame)?; }
        // The flashlight follows the view with a lag of about 80 ms.
        let k = 1.0 - (-frame.dt.max(0.0) / 0.08).exp();
        self.flash_dir = self.flash_dir.lerp(self.forward(), k).normalize_or(self.forward());
        let renderer = frame.renderer().ok_or("no renderer")?;
        renderer.set_clustered_lights(&self.lights())?;
        renderer.set_volumetric_beams(if self.fog { self.beams() } else { Vec::new() }.as_slice())?;
        self.objects.flush(frame.gpu, renderer, &self.scene)?;
        self.ev = renderer.exposure_ev100(0).zip(renderer.metered_ev100(0));
        self.hdr = frame.hdr.map(|d| (d, renderer.hdr_headroom()));
        let eye = self.prev.lerp(self.pos, frame.alpha);
        let cam = Camera { pos: eye, target: eye + self.forward(), fov_y: 70f32.to_radians(), near: 0.05 };
        let mut hud = HudDrawList::new(frame.size);
        let [w, h] = frame.size.map(|v| v as f32);
        let room = match (self.pos.x, self.pos.z) {
            (x, _) if x < -4.15 => "4 dark (click: door)",
            (_, z) if z > 4.0 => "outside",
            (_, z) if z > -4.15 => "1 daylight",
            (_, z) if z > -10.45 => "2 pendant",
            _ if self.at_door() => "3 switches (F: lamp, R: all, click: door)",
            _ => "3 switches (F: lamp, R: all)",
        };
        let ev = self.ev.map_or("metering".into(), |(ev, m)| format!("EV {ev:.1} (meter {m:.1})"));
        let onoff = |b: bool| if b { "on" } else { "off" };
        let hdr = match self.hdr {
            Some((d, headroom)) => {
                let white = d.sdr_white_nits.unwrap_or(80.0 * d.white_scale);
                format!("HDR on: headroom {headroom:.1}x, peak {:.0} nits (display {})", white * headroom,
                    d.peak_nits.map_or("?".into(), |p| format!("{p:.0}")))
            }
            None => "HDR off (SDR)".into(),
        };
        let lines = [
            format!("{room}   {ev}"),
            format!("Q: flashlight {}   Tab: fog {}   {hdr}", onoff(self.flashlight), onoff(self.fog)),
        ];
        for (i, text) in lines.into_iter().enumerate() {
            hud.text.push(TextRun { id: 1 + i as u64, text, position: [20.0, 16.0 + 34.0 * i as f32], bounds: [0.0, 0.0, w, h],
                font_size: 26.0, color: [255, 255, 255, 220], font: FontFace::Body, align: TextAlign::Left, width: 1400.0 });
        }
        Ok(Draw { cameras: vec![cam], light: self.light.clone(), hud })
    }

    fn state(&self) -> Option<String> {
        Some(format!(r#"{{"pos":[{},{},{}],"yaw":{},"pitch":{},"lamps":{:?},"door":{},"flashlight":{},"fog":{},"ev":{:?}}}"#,
            self.pos.x, self.pos.y, self.pos.z, self.yaw, self.pitch, self.lamp_on, self.door_open(), self.flashlight, self.fog, self.ev.map(|e| e.0)))
    }
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |k: &str| args.windows(2).find(|w| w[0] == k).map(|w| w[1].clone());
    let mut game = Rooms::new();
    if let Some(at) = arg("--at") {
        let v: Vec<f32> = at.split(',').filter_map(|s| s.parse().ok()).collect();
        if v.len() == 3 { game.pos = Vec3::new(v[0], v[1], v[2]); game.prev = game.pos; }
    }
    if let Some(yaw) = arg("--yaw").and_then(|y| y.parse().ok()) { game.yaw = yaw; }
    if let Some(pitch) = arg("--pitch").and_then(|y| y.parse().ok()) { game.pitch = pitch; }
    if let Some(off) = arg("--off") { for i in off.split(',').filter_map(|s| s.parse::<usize>().ok()).filter(|&i| i < 4) { game.toggle(i); } }
    if let Some(door) = arg("--door").and_then(|d| d.parse::<f32>().ok()) {
        // The swing fraction, eased: invert the smoothstep by bisection.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..30 { let m = 0.5 * (lo + hi); if ease(m) < door.clamp(0.0, 1.0) { lo = m; } else { hi = m; } }
        game.door_t = 0.5 * (lo + hi);
        game.door_wanted = door > 0.0;
        game.hold_door = true;
    }
    if args.iter().any(|a| a == "--fog") { game.set_fog(true); }
    if args.iter().any(|a| a == "--flashlight") { game.flashlight = true; game.flash_dir = game.forward(); }
    if let Some(frames) = arg("--bench").and_then(|f| f.parse::<u32>().ok()) {
        // GPU time per frame at 4K, headless (`--at`/`--yaw` pick the view).
        let gpu = render::Gpu::new()?;
        let (ms, passes) = app::bench(&mut game, &gpu, [3840, 2160], frames, TICK_HZ)?;
        println!("bench: {ms:.3} ms per frame at 3840x2160 over {frames} frames");
        for (name, ms) in passes { println!("  {name}: {ms:.3} ms"); }
        return Ok(());
    }
    if let Some(out) = arg("--shot") {
        let frames = arg("--frames").and_then(|f| f.parse().ok()).unwrap_or(30);
        // Render every tick, so auto exposure adapts and TAA settles as in play.
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [960, 540], TICK_HZ)?;
        let mut rgba = Vec::new();
        for _ in 0..frames.max(1) {
            game.tick(&[Controls::default()]);
            rgba = headless.frame(&mut game)?;
        }
        eprintln!("shot: exposure EV {:?}, meter EV {:?}", game.ev.map(|e| e.0), game.ev.map(|e| e.1));
        return render::write_png(std::path::Path::new(&out), 960, 540, &rgba);
    }
    app::run(app::Config { title: "Warp: four rooms".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut game)
}
