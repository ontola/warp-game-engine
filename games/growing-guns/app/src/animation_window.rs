//! Live third-person contact preview. Uses the game rig, skin and Luau pose.
use crate::{
    gun::{Rig, Stats},
    hand_animation::HandAnimation,
    view::{characters, guns::Guns},
};
use app::{Config, Controls, Draw, Frame, Game};
use assets::skinned::{ActorHandle, Character};
use body::reach::TwoHandHold;
use glam::{Affine3A, Mat4, Quat, Vec3};
use render::{
    Camera, Lighting, Scene,
    hud::{FontFace, HudDrawList, TextAlign, TextRun},
};
use std::path::Path;

struct Preview {
    scene: Scene,
    rig: Rig,
    guns: Guns,
    large: Stats,
    actor: ActorHandle,
    model: Character,
    arms: TwoHandHold,
    script: HandAnimation,
    pose: Vec<Mat4>,
    tick: u64,
    inspector: Inspector,
}

const ROWS: [(&str, f32, &str); 5] = [
    ("Wrist X", 16.0, "cm"),
    ("Wrist Y", 16.0, "cm"),
    ("Wrist Z", 16.0, "cm"),
    ("Palm roll", 180.0, "deg"),
    ("Aim pitch", 70.0, "deg"),
];
const WIDTH: f32 = 242.0;
const ROW: f32 = 26.0;
const TRACK: f32 = 134.0;

#[derive(Default)]
struct Inspector {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    values: [f32; 5],
    paused: bool,
    slider: Option<usize>,
    orbit: Option<[f32; 2]>,
}

impl Inspector {
    fn top(height: u32) -> f32 {
        (height as f32 - ROW * 6.0 - 10.0).max(0.0)
    }
    fn pointer(&mut self, p: app::Pointer, size: [u32; 2]) {
        if !p.held && !p.pressed {
            self.orbit = None;
            self.slider = None;
        }
        let Some([x, y]) = p.position else {
            self.orbit = None;
            return;
        };
        let row = (y - Self::top(size[1])) / ROW;
        let panel = (8.0..WIDTH).contains(&x) && (0.0..6.0).contains(&row);
        if !panel {
            self.zoom = (self.zoom - p.wheel * 0.12).clamp(-1.6, 1.3);
        }
        if p.pressed {
            self.slider = None;
            self.orbit = None;
            if panel {
                let index = row as usize;
                if index < ROWS.len() {
                    self.slider = Some(index);
                } else if x < WIDTH * 0.5 {
                    self.paused = !self.paused;
                } else {
                    *self = Self::default();
                }
            } else {
                self.orbit = Some([x, y]);
            }
        }
        if p.held || p.pressed {
            if let Some(index) = self.slider {
                self.values[index] = (((x - TRACK) / (WIDTH - 12.0 - TRACK)).clamp(0.0, 1.0) * 2.0
                    - 1.0)
                    * ROWS[index].1;
            } else if let Some(prev) = self.orbit {
                self.yaw -= (x - prev[0]) * 0.008;
                self.pitch = (self.pitch + (y - prev[1]) * 0.008).clamp(-1.55, 1.0);
                self.orbit = Some([x, y]);
            }
        }
    }
    fn hud(&self, hud: &mut HudDrawList) {
        let clip = [0.0, 0.0, hud.size[0] as f32, hud.size[1] as f32];
        let top = Self::top(hud.size[1]);
        hud.rect(
            [8.0, top, WIDTH - 8.0, ROW * 6.0],
            [0.04, 0.05, 0.07, 0.94],
            clip,
        );
        for (i, (name, limit, unit)) in ROWS.iter().enumerate() {
            let y = top + i as f32 * ROW;
            let value = self.values[i];
            let w = WIDTH - 12.0 - TRACK;
            hud.rect([TRACK, y + 12.0, w, 3.0], [0.45, 0.48, 0.5, 1.0], clip);
            hud.rect(
                [
                    TRACK + w * (value / limit + 1.0) * 0.5 - 3.0,
                    y + 6.0,
                    6.0,
                    15.0,
                ],
                [0.37, 0.85, 0.72, 1.0],
                clip,
            );
            label(
                hud,
                10 + i as u64,
                [16.0, y + 3.0],
                format!("{name}  {value:.0} {unit}"),
            );
        }
        label(
            hud,
            20,
            [16.0, top + ROW * 5.0 + 3.0],
            if self.paused { "Resume" } else { "Pause" }.into(),
        );
        label(hud, 21, [TRACK, top + ROW * 5.0 + 3.0], "Reset".into());
    }
}
fn label(hud: &mut HudDrawList, id: u64, position: [f32; 2], text: String) {
    let bounds = [0.0, 0.0, hud.size[0] as f32, hud.size[1] as f32];
    hud.text.push(TextRun {
        id,
        text,
        position,
        bounds,
        font_size: 15.0,
        color: [255; 4],
        font: FontFace::Body,
        align: TextAlign::Left,
        width: WIDTH,
    });
}

impl Game for Preview {
    fn local_players(&self) -> usize {
        1
    }
    fn mouse_capture(&self) -> Option<bool> {
        Some(false)
    }
    fn tick(&mut self, _: &[Controls]) {
        if self.inspector.paused {
            return;
        }
        self.tick += 1;
        match self.tick % 420 {
            60 => self.rig.grow_to(self.large),
            270 => self.rig.grow_to(Stats::default()),
            _ => {}
        }
        self.rig.update(1.0 / 60.0);
    }

    fn pointer(&mut self, p: app::Pointer, size: [u32; 2]) {
        self.inspector.pointer(p, size);
    }
    fn state(&self) -> Option<String> {
        Some(serde_json::json!({"tick":self.tick,"paused":self.inspector.paused,"yaw":self.inspector.yaw,"pitch":self.inspector.pitch,"zoom":self.inspector.zoom,"pose_offsets":self.inspector.values}).to_string())
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let time = self.tick as f32 / 60.0;
        let clip = "pistol_idle";
        self.model.sample_into(clip, time, true, &mut self.pose)?;
        let hand = self
            .model
            .sample_socket(clip, "RightHand", time, true)?
            .w_axis
            .truncate();
        let body = characters::model_transform(Vec3::ZERO, 0.0);
        let contacts = self.rig.contacts_at(1.0);
        let (support, roll) = self.script.pose(contacts, time);
        let [x, y, z, palm, aim] = self.inspector.values;
        let support = support + Vec3::new(x, y, z) * 0.01;
        let pitch = aim.to_radians();
        let initial =
            Guns::held_transform(body.transform_point3(hand), 0.0, pitch, contacts.primary);
        let target = body
            .inverse()
            .transform_point3(initial.transform_point3(support));
        let primary = self.arms.apply(&mut self.pose, hand, target);
        self.arms.orient_support(
            &mut self.pose,
            characters::grip_rotation(pitch) * Quat::from_rotation_x(roll + palm.to_radians()),
        );
        let gun =
            Guns::held_transform(body.transform_point3(primary), 0.0, pitch, contacts.primary);
        self.guns.review_pose(&mut self.scene, &self.rig);
        self.scene
            .update_instance(self.actor.instance, self.actor.mesh, gun.inverse() * body);
        let light = Lighting {
            sun_dir: Vec3::new(-0.6, 0.9, -0.6).normalize(),
            sky_lux: 4500.0,
            sun_lux: 9000.0,
            ev100: 12.0,
            taa: 0.0,
            fog_density: 0.0,
            ..Default::default()
        };
        if frame.renderer().is_none() {
            frame.set_scene(&self.scene, &light);
        }
        let renderer = frame.renderer().unwrap();
        renderer.update_instances(frame.gpu, 0, &self.scene.instances)?;
        renderer.set_skinning_palettes(frame.gpu, std::iter::once(self.pose.as_slice()))?;
        let target = Vec3::new(0.0, 0.0, -0.12);
        let orbit = Quat::from_rotation_y(self.inspector.yaw - 2.037)
            * Quat::from_rotation_x(-self.inspector.pitch - 0.294);
        let camera = Camera {
            pos: target + orbit * Vec3::Z * (1.93 * self.inspector.zoom.exp()),
            target,
            near: 0.02,
            fov_y: 38f32.to_radians(),
        };
        let mut hud = HudDrawList::new(frame.size);
        self.inspector.hud(&mut hud);
        hud.text.push(TextRun {
            id: 1,
            text: format!(
                "Drag to orbit   |   Scroll to zoom   |   Foregrip {:.0}%    {}",
                contacts.extension * 100.0,
                self.script.status
            ),
            position: [14.0, 12.0],
            bounds: [0.0, 0.0, frame.size[0] as f32, frame.size[1] as f32],
            font_size: 17.0,
            color: [255; 4],
            font: FontFace::Body,
            align: TextAlign::Left,
            width: frame.size[0] as f32 - 28.0,
        });
        Ok(Draw {
            cameras: vec![camera],
            light,
            hud,
        })
    }
}

pub fn run(args: &crate::Args) -> Result<(), String> {
    let name = args.value("--model").unwrap_or("knight");
    if !["knight", "paladin", "ch10"].contains(&name) {
        return Err("unknown roster model".into());
    }
    let model = Character::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/characters")
            .join(format!("{name}.json")),
    )?;
    let mut scene = Scene::default();
    let guns = Guns::reserve(&mut scene, 1);
    let actor = model.add_actor(&mut scene, Affine3A::IDENTITY)?;
    let rig = Rig::with_detail(guns.detail);
    let mut world = growing_guns::World::new(3, &[false]);
    let _scripts = crate::scripts::attach(&mut world)?;
    let id = world.cards.find("sniper").ok_or("missing sniper card")?;
    world.give(0, id);
    let arms = crate::view::characters::grip_arms(&model).ok_or("missing arm joints")?;
    let pose = model.sample("pistol_idle", 0.0, true)?;
    let mut game = Preview {
        scene,
        rig,
        guns,
        large: Stats::of(&world.players[0]),
        actor,
        model,
        arms,
        script: HandAnimation::load()?,
        pose,
        tick: 0,
        inspector: Inspector::default(),
    };
    app::run(
        Config {
            title: "Warp Animation | Growing Guns".into(),
            fullscreen: false,
            keyboard: false,
            pointer: true,
            size: args.size((960, 640)).into(),
            target_fps: Some(60.0),
            ..Default::default()
        },
        &mut game,
    )
}
