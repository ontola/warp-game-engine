//! Four joined rooms, each lit another way, to see how indoor lighting
//! techniques combine. See engine/docs/interior-light.md.
//!
//!   cargo run --release -p app --example interior_rooms
//!   cargo run --release -p app --example interior_rooms -- --shot out.png --at 0,1.65,-7 --yaw 0
//!   (also --pitch, --off 0,2 (lamps off), --fog, --flashlight, --door 0..1)
//!
//! 1. Daylight room (front door and a window): sky and sun only, baked with
//!    several bounces (`room_bake::bake`), sun shadows live.
//! 2. Pendant room: one amber lamp under a shade, swinging on its cord: a
//!    spot (25/40 degree cone) pointing down the cord, so the cone swings
//!    with it. Direct light and shadows are live (shadowed clustered
//!    lights); its bounce cannot be baked, so it is gathered live from its
//!    shadow map (`ClusteredLight::with_bounce`, reflective shadow maps):
//!    what the cone lands on lights the room and the ceiling.
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
//! eye that lags the view a little; what it lights bounces live, as the
//! pendant's does. Tab (or Select) toggles haze in rooms 2 and 4
//! (`Lighting::volumetric` in its bounded mode, `set_volumetric_bounded`):
//! the pendant's cone, the flashlight's beam and the lamp light through room
//! 4's door show in it, shadowed by the walls, furniture and door.
//! Shift sprints, Space jumps.
//!
//! HDR output is requested (`Game::hdr`): on an HDR screen the sky through
//! the door and the lamps go above SDR white. The HUD says whether it is on.
use app::{Buttons, Controls, Draw, Frame, Game};
use glam::{Affine3A, Quat, Vec2, Vec3};
use render::clustered_lights::ClusteredLight;
use render::headlights::LocalLight;
use render::hud::{FontFace, HudDrawList, TextAlign, TextRun};
use render::irradiance_volume::{Grid, Probe};
use render::light_face::LightFace;
use render::light_shapes::LightShape;
use render::room_bake::Occluders;
use render::scene::Material;
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
/// Switch room lamps: candela straight down, and their face (a ceiling panel).
const LAMP_CD: f32 = 450.0;
const PANEL: LightFace = LightFace::Rect { width_m: 0.5, depth_m: 0.5 };
/// Switch room lamp colours: warm, red, teal, blue.
const LAMP_COLOURS: [Vec3; 4] = [WARM, Vec3::new(1.0, 0.22, 0.15), Vec3::new(0.2, 1.0, 0.8), Vec3::new(0.25, 0.4, 1.0)];
/// The pendant: amber, against the teal and blue next door.
const PENDANT: Vec3 = Vec3::new(1.0, 0.5, 0.12);
/// Its bulb: the light's face.
const BULB: LightFace = LightFace::Sphere { radius_m: 0.05 };

/// The pendant's light at `position`, pointing `direction` (down its cord).
fn pendant_light(position: Vec3, direction: Vec3) -> LocalLight {
    BULB.apply(LocalLight { position, direction, intensity_cd: colour(PENDANT, 700.0), range_m: 9.0,
        inner_cos: 25f32.to_radians().cos(), outer_cos: 40f32.to_radians().cos(), source_radius_m: 0.0, falloff: 2.0 })
}
/// Room 3 | 4 doorway in the x = -4.3..-4.0 wall, and the door's hinge.
const DOOR_Z: [f32; 2] = [-12.4, -11.4];
const DOOR_TOP: f32 = 2.1;
const HINGE: Vec3 = Vec3::new(-4.15, 0.0, -12.4);
/// How far the door swings (into room 4) and how long a swing takes.
const DOOR_ANGLE: f32 = 1.75;
const DOOR_SECONDS: f32 = 0.6;

fn bx(min: [f32; 3], max: [f32; 3]) -> Affine3A {
    let (a, b) = (Vec3::from(min), Vec3::from(max));
    Affine3A::from_scale_rotation_translation(b - a, Quat::IDENTITY, (a + b) * 0.5)
}

/// `c` scaled to the warm white's luminance, times `cd` candela.
fn colour(c: Vec3, cd: f32) -> Vec3 {
    let lum = |v: Vec3| v.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    c * (lum(WARM) / lum(c)) * cd
}

/// Light shapes (`render::light_shapes`): the flashlight's cookie, the
/// pendant's lampshade profile, the ceiling lamps' downlight profile.
const FLASH_COOKIE: LightShape = LightShape { cookie: 1, profile: 0, diffuse_only: false };
const PENDANT_PROFILE: LightShape = LightShape { cookie: 0, profile: 1, diffuse_only: false };
const DOWNLIGHT_PROFILE: LightShape = LightShape { cookie: 0, profile: 2, diffuse_only: false };

fn smooth(e0: f32, e1: f32, x: f32) -> f32 { let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// An LED flashlight's beam through its reflector, as a cookie: a hot
/// centre a little off the axis, the reflector's bright ring, a darker band,
/// a soft spill out to the cone's edge and faint concentric streaks.
fn flashlight_cookie() -> Vec<f32> {
    let n = render::light_shapes::SIZE as usize;
    (0..n * n).map(|i| {
        let (x, y) = ((i % n) as f32 / (n - 1) as f32 * 2.0 - 1.0, (i / n) as f32 / (n - 1) as f32 * 2.0 - 1.0);
        let r = (x * x + y * y).sqrt();
        let hot = ((x - 0.04) * (x - 0.04) + (y + 0.03) * (y + 0.03)).sqrt();
        let core = (-(hot / 0.13).powi(2)).exp();
        let ring = 0.45 * (-((r - 0.33) / 0.08).powi(2)).exp();
        let body = 0.45 * (1.0 - smooth(0.2, 0.42, r));
        let spill = 0.22 * (1.0 - smooth(0.5, 1.0, r));
        let streaks = 1.0 + 0.025 * (r * 70.0).sin() * (1.0 - smooth(0.1, 0.6, r));
        ((core + ring + body).min(1.0).max(spill) * streaks).clamp(0.0, 1.0)
    }).collect()
}

/// A profile from a function of the angle from the axis in degrees.
fn profile(f: impl Fn(f32) -> f32) -> Vec<f32> {
    let n = render::light_shapes::SIZE as usize;
    (0..n).map(|i| f(i as f32 / (n - 1) as f32 * 180.0).clamp(0.0, 1.0)).collect()
}

fn ceiling_lamp(i: usize) -> LocalLight {
    // A wide downlight: most light down, some to the walls.
    // On its panel (the light's face), which its shadow excludes.
    PANEL.apply(LocalLight { position: LAMPS[i], direction: Vec3::NEG_Y, intensity_cd: colour(LAMP_COLOURS[i], LAMP_CD), range_m: 10.0,
        inner_cos: 50f32.to_radians().cos(), outer_cos: 80f32.to_radians().cos(), source_radius_m: 0.0, falloff: 2.0 })
}

/// The door panel `open` (0 shut, 1 open) of the way round its hinge.
fn door_xf(open: f32) -> Affine3A {
    Affine3A::from_rotation_translation(Quat::from_rotation_y(-open * DOOR_ANGLE), HINGE)
        * Affine3A::from_scale_rotation_translation(Vec3::new(0.06, DOOR_TOP - 0.01, 0.98), Quat::IDENTITY, Vec3::new(0.0, DOOR_TOP * 0.5, 0.5))
}

/// Reflection probes, one per room: capture point and the room's inside.
const ROOM_PROBES: [render::reflection_probes::ProbeBox; 4] = [
    render::reflection_probes::ProbeBox { capture: Vec3::new(0.0, 1.5, 0.0), min: Vec3::new(-4.0, 0.0, -4.0), max: Vec3::new(4.0, H, 4.0) },
    render::reflection_probes::ProbeBox { capture: Vec3::new(-1.5, 1.5, -6.0), min: Vec3::new(-4.0, 0.0, -10.2), max: Vec3::new(4.0, H, -4.3) },
    render::reflection_probes::ProbeBox { capture: Vec3::new(0.0, 1.5, -13.5), min: Vec3::new(-4.0, 0.0, -16.6), max: Vec3::new(4.0, H, -10.5) },
    render::reflection_probes::ProbeBox { capture: Vec3::new(-6.6, 1.5, -12.6), min: Vec3::new(-9.0, 0.0, -16.6), max: Vec3::new(-4.3, H, -10.5) },
];

/// Smoothstep: the door eases in and out.
fn ease(t: f32) -> f32 { let t = t.clamp(0.0, 1.0); t * t * (3.0 - 2.0 * t) }

/// One baked light state: sky and sun, each lamp's bounce, separation planes.
struct LightState {
    sky: Vec<Probe>,
    lamps: Vec<Vec<Probe>>,
    planes: Vec<[f32; 4]>,
    /// DDGI-style probe visibility (`room_bake::bake_visibility`).
    visibility: Vec<[u32; render::room_bake::VISIBILITY_SIDE * render::room_bake::VISIBILITY_SIDE]>,
    /// Where each probe was moved out of geometry (`room_bake::pack_offsets`).
    offsets: Vec<u32>,
    /// Which voxels around each probe it reaches through open space (`room_bake::bake_reach`).
    reach: Vec<[u32; render::room_bake::REACH_WORDS]>,
}

impl LightState {
    fn bake(grid: &Grid, occluders: &Occluders, sun: Vec3) -> Result<Self, String> {
        let settings = render::room_bake::Settings::default();
        let lamps: Vec<LocalLight> = (0..4).map(ceiling_lamp).collect();
        Ok(LightState {
            sky: render::room_bake::bake(grid, occluders, sun, &settings)?,
            lamps: render::room_bake::bake_lights(grid, occluders, &lamps, &render::room_bake::Settings { bounces: 2, ..settings })?,
            planes: render::room_bake::separation_planes(grid, occluders),
            visibility: render::room_bake::bake_visibility(grid, occluders),
            offsets: render::room_bake::pack_offsets(grid, &render::room_bake::probe_positions(grid, occluders)),
            reach: render::room_bake::bake_reach(grid, occluders),
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
    shade_mesh: usize,
    bulb: SlotId,
    cord: SlotId,
    shade: SlotId,
    door: SlotId,
    /// Everything that blocks light in the fog (static boxes; the door is
    /// added as it stands).
    fog_occluders: Vec<Affine3A>,
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
    /// Which door state's visibility maps are uploaded (true: shut).
    visibility_shut: Option<bool>,
    /// Forget temporal history and exposure on the next frame (a cut: the
    /// regression suite jumps between views).
    cut: bool,
    /// Indoor fog (`Lighting::volumetric`).
    fog: bool,
    flashlight: bool,
    /// The flashlight's direction, lagging the view (per drawn frame).
    flash_dir: Vec3,
    /// A flashlight held elsewhere than the eye (the suite's leak spots):
    /// position and direction.
    flash_pose: Option<(Vec3, Vec3)>,
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
            let m = scene.add_material(BULB.material(&pendant_light(PIVOT, Vec3::NEG_Y)));
            scene.add_mesh(&BULB.primitive(m).mesh())
        };
        // Each lit panel is its light's face (render::light_face).
        let lamp_on_mesh: [usize; 4] = std::array::from_fn(|i| {
            let m = scene.add_material(PANEL.material(&ceiling_lamp(i)));
            scene.add_mesh(&PANEL.primitive(m).mesh())
        });
        let lamp_off_mesh = mat(&mut scene, [0.8, 0.8, 0.78], 0.5, 0, 1.0);
        let cord_mesh = dark;
        // The pendant's shade: an open-looking cone, narrow end up.
        let shade_mesh = {
            let m = scene.add_material(Material { tint: [0.08, 0.07, 0.06], layer: -1, roughness: 0.4, flags: 0, scale: 1.0, metal: 0.3 });
            scene.add_mesh(&render::primitives::cylinder(m, 0.25, 1.0, 20).mesh())
        };
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
            bx([-4.0, 0.0, -10.5], [2.2, H, -10.2]), bx([3.2, 0.0, -10.5], [4.0, H, -10.2]), bx([2.2, 2.1, -10.5], [3.2, H, -10.2]),
            // Back wall.
            bx([-4.3, 0.0, -16.9], [4.3, H, -16.6]),
            // Room 4: north, south and west walls.
            bx([-9.3, 0.0, -10.5], [-4.3, H, -10.2]), bx([-9.3, 0.0, -16.9], [-4.3, H, -16.6]), bx([-9.3, 0.0, -16.9], [-9.0, H, -10.3]),
        ];
        for w in &walls { scene.add_instance(plaster, *w); }
        let roofs = [bx([-4.3, H, -16.9], [4.3, H + 0.3, 4.3]), bx([-9.3, H, -16.9], [-4.3, H + 0.3, -10.2])];
        for r in &roofs { scene.add_instance(plaster, *r); }
        let mut fog_occluders: Vec<Affine3A> = walls.iter().chain(&roofs).copied().collect();
        // Coloured inner faces: thin panels on room 2 (blue) and 3 (green).
        for (m, b) in [
            (blue, bx([-4.0, 0.0, -10.2], [-3.98, H, -4.3])), (blue, bx([3.98, 0.0, -10.2], [4.0, H, -4.3])),
            (green, bx([-4.0, 0.0, -16.6], [-3.98, H, DOOR_Z[0]])), (green, bx([-4.0, 0.0, DOOR_Z[1]], [-3.98, H, -10.5])),
            (green, bx([-4.0, DOOR_TOP, DOOR_Z[0]], [-3.98, H, DOOR_Z[1]])), (green, bx([3.98, 0.0, -16.6], [4.0, H, -10.5])),
        ] { scene.add_instance(m, b); }
        scene.add_instance(wood, bx([-4.0, 0.0, -4.0], [4.0, 0.02, 4.0]));
        scene.add_instance(tiles, bx([-4.0, 0.0, -10.2], [4.0, 0.02, -4.3]));
        scene.add_instance(wood, bx([-4.0, 0.0, -16.6], [4.0, 0.02, -10.5]));
        scene.add_instance(wood, bx([-9.0, 0.0, -16.6], [-4.0, 0.02, -10.5]));
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
        fog_occluders.extend(furniture.iter().map(|f| f.1));
        let footprint = |xf: &Affine3A| {
            let c = Vec3::from(xf.translation);
            let h = Vec3::new(xf.matrix3.x_axis.x, xf.matrix3.y_axis.y, xf.matrix3.z_axis.z) * 0.5;
            // Only what blocks a walker: things that reach below 2 m.
            (c.y - h.y < 2.0).then(|| (Vec2::new(c.x - h.x, c.z - h.z), Vec2::new(c.x + h.x, c.z + h.z)))
        };
        // The window sill is low: keep it solid too.
        walls.retain(|w| w.translation.y < 2.0 || w.matrix3.y_axis.y > 2.0);
        let solid = walls.iter().chain(furniture.iter().map(|f| &f.1)).filter_map(footprint).collect();
        // Probes every 0.5 m over the building; those that land in a wall or on
        // a surface are moved out (room_bake::probe_positions).
        let grid = Grid::covering(Vec3::new(-9.3, 0.0, -16.9), Vec3::new(4.3, H + 0.3, 4.3), 0.5);
        scene.reserve_irradiance_volume(grid).expect("probe grid");
        // The surfaces as voxels, which the live bounce of the pendant
        // and the flashlight (rsm) is tested against: no bounce through walls.
        scene.reserve_bounce_voxels(Vec3::new(-9.3, -0.3, -16.9), Vec3::new(4.3, H + 0.3, 4.3)).expect("bounce voxels");
        if std::env::var_os("ROOMS_NO_DDGI").is_none() { scene.reserve_probe_visibility().expect("probe visibility"); }
        // The static geometry, before the moving pieces join the scene, with
        // and without the shut door.
        let open_occluders = Occluders::from_scene(&scene, |_| true);
        let shut_occluders = open_occluders.merged(&Occluders::from_boxes(&[door_xf(0.0)]));
        let mut objects = Slots::reserve(&mut scene, 16);
        let bulb = objects.add(&mut scene, bulb_mesh, Affine3A::IDENTITY).unwrap();
        let cord = objects.add(&mut scene, cord_mesh, Affine3A::IDENTITY).unwrap();
        let shade = objects.add(&mut scene, shade_mesh, Affine3A::IDENTITY).unwrap();
        let door = objects.add(&mut scene, door_mesh, door_xf(0.0)).unwrap();
        let lamp_slots = (0..4).map(|i| objects.add(&mut scene, lamp_on_mesh[i], PANEL.transform(&ceiling_lamp(i))).unwrap()).collect();
        let mut light = Lighting::default();
        light.fog_start = 1000.0;
        light.fog_end = 2000.0;
        light.fog_density = 0.0;
        // Down to EV -3: a shut dark room is shown as dark as it is.
        light.auto_exposure = Some(render::AutoExposure { min_ev100: -3.0, max_ev100: 16.0, ..Default::default() });
        // ROOMS_LIGHT="gi=0,taa=0": Lighting::set keys, for checks.
        for kv in std::env::var("ROOMS_LIGHT").unwrap_or_default().split(',').filter(|s| !s.is_empty()) { light.set(kv).expect("ROOMS_LIGHT"); }
        let pos = Vec3::new(1.5, EYE, 12.0);
        Rooms { scene, grid, solid, objects, bulb_mesh, cord_mesh, door_mesh, lamp_on_mesh, lamp_off_mesh, shade_mesh, bulb, cord, shade, door, fog_occluders, lamp_slots,
            lamp_on: [true; 4], open_occluders, shut_occluders, shut: None, open: None, door_wanted: false, door_t: 0.0, hold_door: false, uploaded: None, visibility_shut: None, cut: false,
            fog: false, flashlight: false, flash_dir: Vec3::NEG_Z, flash_pose: None, pos, prev: pos, yaw: 0.12, pitch: 0.0, vy: 0.0, t: 0.0, light,
            ready: false, ev: None, hdr: None }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(-self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), -self.yaw.cos() * self.pitch.cos())
    }

    fn door_open(&self) -> f32 { ease(self.door_t) }

    /// The pendant's bulb position: a pendulum swinging in x and a little in z.
    fn bulb_at(&self) -> Vec3 {
        let a = 0.45 * (self.t * 2.0).sin();
        let b = 0.15 * (self.t * 1.37).sin();
        PIVOT + Vec3::new(a.sin(), -a.cos() * b.cos(), b.sin()) * CORD
    }

    /// Down along the cord: the way the shade points.
    fn cord_dir(&self) -> Vec3 { (self.bulb_at() - PIVOT).normalize() }

    /// The pendant: a spot under its shade, pointing down the cord, so its
    /// cone swings with it.
    fn pendant(&self) -> LocalLight { pendant_light(self.bulb_at(), self.cord_dir()) }

    fn flashlight_light(&self) -> LocalLight {
        let f = self.forward();
        let right = f.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(f);
        let (position, direction) = self.flash_pose.unwrap_or((self.pos + right * 0.22 - up * 0.25, self.flash_dir));
        LocalLight { position, direction,
            intensity_cd: Vec3::new(0.92, 0.96, 1.0) * 2600.0, range_m: 25.0,
            inner_cos: 24f32.to_radians().cos(), outer_cos: 28f32.to_radians().cos(), source_radius_m: 0.02, falloff: 2.0 }
    }

    /// What scatters in the fog: the pendant, the lit ceiling lamps (their
    /// light through room 4's door) and the flashlight. The fog fills rooms
    /// 2 and 4 only and is ray-marched with every light's visibility tested
    /// against the walls, furniture and the door (`set_volumetric_bounded`).
    fn beams(&self) -> (Vec<LocalLight>, Vec<LightShape>) {
        let mut beams = vec![(self.pendant(), PENDANT_PROFILE)];
        beams.extend((0..4).filter(|&i| self.lamp_on[i]).map(|i| (ceiling_lamp(i), DOWNLIGHT_PROFILE)));
        if self.flashlight { beams.push((self.flashlight_light(), FLASH_COOKIE)); }
        beams.into_iter().unzip()
    }

    /// The fog's rooms: 2 (the pendant) and 4 (the dark room).
    fn fog_bounds() -> [(Vec3, Vec3); 2] {
        [(Vec3::new(-4.0, 0.0, -10.2), Vec3::new(4.0, H, -4.3)), (Vec3::new(-9.0, 0.0, -16.6), Vec3::new(-4.3, H, -10.5))]
    }

    fn lights(&self) -> Vec<ClusteredLight> {
        let pendant = self.pendant();
        // The light sits inside its bulb and shade: those and the cord (the
        // first three slots) must not cast its shadow.
        let own = Some((self.objects.instance(self.bulb) as u32, 3));
        // It moves, so its bounce (what the cone lands on lighting the room
        // and the ceiling) is gathered live (rsm).
        let mut lit = ClusteredLight::shadowed(pendant, 1).with_shape(PENDANT_PROFILE).with_face(BULB).with_bounce();
        if let Some(s) = &mut lit.shadow { s.exclude = own; }
        let mut lights = vec![lit];
        // The ceiling lamps' bounces are baked (bake_lights).
        for i in 0..4 {
            if self.lamp_on[i] {
                let mut lamp = ClusteredLight::shadowed(ceiling_lamp(i), 10 + i as u64).with_shape(DOWNLIGHT_PROFILE).with_face(PANEL);
                if let Some(s) = &mut lamp.shadow { s.exclude = Some((self.objects.instance(self.lamp_slots[i]) as u32, 1)); }
                lights.push(lamp);
            }
        }
        if self.flashlight {
            let flash = self.flashlight_light();
            lights.push(ClusteredLight::shadowed(flash, 20).with_shape(FLASH_COOKIE).with_bounce());
        }
        lights
    }

    fn toggle(&mut self, i: usize) {
        self.lamp_on[i] = !self.lamp_on[i];
        let mesh = if self.lamp_on[i] { self.lamp_on_mesh[i] } else { self.lamp_off_mesh };
        self.objects.set(&mut self.scene, self.lamp_slots[i], mesh, PANEL.transform(&ceiling_lamp(i)));
    }

    fn set_fog(&mut self, on: bool) {
        self.fog = on;
        // Haze in rooms 2 and 4 (`fog_bounds`): the pendant's cone, the
        // flashlight's beam and the light through room 4's door show. It
        // only adds light (no extinction).
        self.light.volumetric = on.then(|| render::volumetric::Volumetric { density: 0.15, anisotropy: 0.5, intensity: 6.0, ..Default::default() });
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
        // Visibility switches with the planes (the door is a wall until a
        // third open); 3.5 MB, so only when the state changes.
        if std::env::var_os("ROOMS_NO_DDGI").is_none() && self.visibility_shut != Some(k < 0.3) {
            let state = if k < 0.3 { shut } else { open };
            frame.renderer().ok_or("no renderer")?.set_probe_visibility(frame.gpu, &state.visibility, &state.offsets, &state.reach)?;
            self.visibility_shut = Some(k < 0.3);
        }
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
        let face = BULB.transform(&self.pendant());
        self.objects.set(&mut self.scene, self.bulb, self.bulb_mesh, face);
        // The shade over the bulb, its rim just above the bulb's bottom.
        let d = self.cord_dir();
        let shade = Affine3A::from_scale_rotation_translation(Vec3::new(0.22, 0.2, 0.22), Quat::from_rotation_arc(Vec3::Y, -d), bulb - d * 0.14);
        self.objects.set(&mut self.scene, self.shade, self.shade_mesh, shade);
        self.objects.set(&mut self.scene, self.cord, self.cord_mesh, Affine3A::from_scale_rotation_translation(Vec3::new(0.012, CORD - 0.05, 0.012), rot, mid));
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !std::mem::replace(&mut self.ready, true) {
            frame.set_scene(&self.scene, &self.light);
            let renderer = frame.renderer().ok_or("no renderer")?;
            renderer.set_light_cookie(frame.gpu, FLASH_COOKIE.cookie, &flashlight_cookie())?;
            // Lampshade: a softer centre, a bright rim towards the cone's edge.
            renderer.set_light_profile(frame.gpu, PENDANT_PROFILE.profile, &profile(|a| 0.65 + 0.35 * smooth(12.0, 34.0, a)))?;
            // Downlight: brightest around 40 degrees and cut off at about 65:
            // the scallops a recessed fitting throws on a wall.
            renderer.set_light_profile(frame.gpu, DOWNLIGHT_PROFILE.profile,
                &profile(|a| (0.35 + 0.65 * smooth(8.0, 36.0, a) * (1.0 - smooth(44.0, 54.0, a))) * (1.0 - 0.92 * smooth(54.0, 62.0, a))))?;
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
        if std::mem::take(&mut self.cut) {
            let renderer = frame.renderer().ok_or("no renderer")?;
            renderer.reset_history();
            renderer.reset_exposure();
        }
        // The flashlight follows the view with a lag of about 80 ms.
        let k = 1.0 - (-frame.dt.max(0.0) / 0.08).exp();
        self.flash_dir = self.flash_dir.lerp(self.forward(), k).normalize_or(self.forward());
        let renderer = frame.renderer().ok_or("no renderer")?;
        renderer.set_clustered_lights(&self.lights())?;
        // One reflection probe per room: the engine captures them live.
        renderer.set_reflection_probes(&ROOM_PROBES)?;
        let (beams, shapes) = if self.fog { self.beams() } else { (Vec::new(), Vec::new()) };
        renderer.set_volumetric_beams_shaped(&beams, &shapes)?;
        if self.fog {
            let mut occluders = self.fog_occluders.clone();
            occluders.push(door_xf(self.door_open()));
            renderer.set_volumetric_bounded(&Self::fog_bounds(), &occluders, 1.0);
        }
        self.objects.flush(frame.gpu, renderer, &self.scene)?;
        // The engine follows moved instances (the door, the pendant) itself.
        renderer.update_bounce_voxels(frame.gpu, &self.scene)?;
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
            (_, z) if z > -10.35 => "2 pendant",
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
    // --debug N: a shade diagnostic (Lighting::debug; 26 the reflection ray's hit).
    if let Some(d) = arg("--debug").and_then(|d| d.parse::<f32>().ok()) { game.light.debug = d; }
    if args.iter().any(|a| a == "--fog") { game.set_fog(true); }
    if args.iter().any(|a| a == "--flashlight") { game.flashlight = true; game.flash_dir = game.forward(); }
    if let Some(frames) = arg("--pan").and_then(|f| f.parse::<u32>().ok()) {
        // A slow pan (0.3 rad/s) at a fixed exposure (EV 5.5), headless:
        // the mean luminance of every frame, to see light that jumps.
        game.light.auto_exposure = None;
        game.light.ev100 = 5.5;
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [480, 270], TICK_HZ)?;
        for _ in 0..60 { game.tick(&[Controls::default()]); headless.frame(&mut game)?; }
        for i in 0..frames {
            game.yaw -= 0.3 / TICK_HZ;
            game.tick(&[Controls::default()]);
            let rgba = headless.frame(&mut game)?;
            let lum: f64 = rgba.chunks(4).map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64).sum::<f64>() / (rgba.len() / 4) as f64;
            println!("{i},{lum:.3}");
        }
        return Ok(());
    }
    if let Some(frames) = arg("--bench").and_then(|f| f.parse::<u32>().ok()) {
        // GPU time per frame at 4K, headless (`--at`/`--yaw` pick the view).
        let gpu = render::Gpu::new()?;
        let (ms, passes) = app::bench(&mut game, &gpu, [3840, 2160], frames, TICK_HZ)?;
        println!("bench: {ms:.3} ms per frame at 3840x2160 over {frames} frames");
        for (name, ms) in passes { println!("  {name}: {ms:.3} ms"); }
        return Ok(());
    }
    if let Some(file) = arg("--suite") {
        // Regression suite: one bake, then every spot in `file` (lines
        // `name x,y,z yaw pitch lamps door flashlight crop [flash]`, lamps four
        // 0/1 digits, door 0..1, flashlight 0/1, flash x,y,z,yaw,pitch to hold
        // it away from the eye; # comments) rendered still and
        // while turning fast into it (2 rad/s for the last second), as
        // `<out>/<name>_still.png` and `<name>_turn.png`.
        let out = std::path::PathBuf::from(arg("--out").unwrap_or_else(|| "build/suite".into()));
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        let [w, h] = arg("--size").and_then(|s| { let v: Vec<u32> = s.split('x').filter_map(|p| p.parse().ok()).collect(); (v.len() == 2).then(|| [v[0], v[1]]) }).unwrap_or([1920, 1080]);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], TICK_HZ)?;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 7 { return Err(format!("suite line needs at least 7 fields: {line}")); }
            let p: Vec<f32> = f[1].split(',').filter_map(|s| s.parse().ok()).collect();
            let (yaw, pitch): (f32, f32) = (f[2].parse().map_err(|_| line.to_string())?, f[3].parse().map_err(|_| line.to_string())?);
            let lamps: Vec<bool> = f[4].chars().map(|c| c == '1').collect();
            let door: f32 = f[5].parse().map_err(|_| line.to_string())?;
            for (k, turn) in [("still", 0.0f32), ("turn", 2.0)] {
                game.pos = Vec3::new(p[0], p[1], p[2]);
                game.prev = game.pos;
                game.pitch = pitch;
                game.yaw = yaw + turn;
                for i in 0..4 { if game.lamp_on[i] != lamps.get(i).copied().unwrap_or(true) { game.toggle(i); } }
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..30 { let m = 0.5 * (lo + hi); if ease(m) < door.clamp(0.0, 1.0) { lo = m; } else { hi = m; } }
                game.door_t = 0.5 * (lo + hi);
                game.door_wanted = door > 0.0;
                game.hold_door = true;
                game.flashlight = f[6] == "1";
                game.flash_dir = game.forward();
                // Optional ninth field: the flashlight held at x,y,z,yaw,pitch.
                game.flash_pose = f.get(8).map(|s| s.split(',').filter_map(|v| v.parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 5)
                    .map(|v| (Vec3::new(v[0], v[1], v[2]), Vec3::new(-v[3].sin() * v[4].cos(), v[4].sin(), -v[3].cos() * v[4].cos())));
                game.cut = true;
                let frames = 200u32;
                let mut rgba = Vec::new();
                for i in 0..frames {
                    if i + TICK_HZ as u32 >= frames { game.yaw -= turn / TICK_HZ; }
                    game.tick(&[Controls::default()]);
                    rgba = headless.frame(&mut game)?;
                }
                let path = out.join(format!("{}_{k}.png", f[0]));
                render::write_png(&path, w, h, &rgba)?;
                println!("{} {k}: EV {:?}", f[0], game.ev.map(|e| e.0));
            }
        }
        return Ok(());
    }
    if let Some(out) = arg("--shot") {
        let frames = arg("--frames").and_then(|f| f.parse().ok()).unwrap_or(30);
        // Render every tick, so auto exposure adapts and TAA settles as in play.
        // --size WxH (default 960x540); --turn R: yaw R rad/s during the
        // last second, so the shot shows the camera moving as in play.
        let [w, h] = arg("--size").and_then(|s| { let v: Vec<u32> = s.split('x').filter_map(|p| p.parse().ok()).collect(); (v.len() == 2).then(|| [v[0], v[1]]) }).unwrap_or([960, 540]);
        let turn: f32 = arg("--turn").and_then(|t| t.parse().ok()).unwrap_or(0.0);
        let gpu = render::Gpu::new()?;
        let mut headless = app::Headless::new(&gpu, [w, h], TICK_HZ)?;
        let mut rgba = Vec::new();
        let frames = frames.max(1);
        for i in 0..frames {
            if i + TICK_HZ as u32 >= frames { game.yaw -= turn / TICK_HZ; }
            game.tick(&[Controls::default()]);
            rgba = headless.frame(&mut game)?;
        }
        eprintln!("shot: exposure EV {:?}, meter EV {:?}", game.ev.map(|e| e.0), game.ev.map(|e| e.1));
        return render::write_png(std::path::Path::new(&out), w, h, &rgba);
    }
    app::run(app::Config { title: "Warp: four rooms".into(), tick_hz: TICK_HZ, ..Default::default() }, &mut game)
}
