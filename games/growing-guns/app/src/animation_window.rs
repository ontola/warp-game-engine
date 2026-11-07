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
use render::{Camera, Lighting, Scene};
use std::path::Path;

use app::editor::PoseViewport;
use body::pose_editor::PoseEditor;

struct Preview {
    authored: crate::foregrip_pose::ForegripPose,
    scene: Scene,
    rig: Rig,
    guns: Guns,
    large: Stats,
    actor: ActorHandle,
    model: Character,
    arms: TwoHandHold,
    mount: body::reach::JointMount,
    script: HandAnimation,
    pose: Vec<Mat4>,
    tick: u64,
    inspector: PoseViewport<5>,
}

const ROWS: [(&str, f32, &str); 5] = [
    ("Wrist X", 16.0, "cm"),
    ("Wrist Y", 16.0, "cm"),
    ("Wrist Z", 16.0, "cm"),
    ("Palm roll", 180.0, "deg"),
    ("Aim pitch", 70.0, "deg"),
];
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
    fn function_key(&mut self, n: u8) -> bool {
        self.inspector.function_key(n)
    }
    fn state(&self) -> Option<String> {
        let mut state = self.inspector.snapshot();
        state["tick"] = self.tick.into();
        state["model"] = self.model.name.clone().into();
        Some(state.to_string())
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
        let aim = (body.inverse() * initial).to_scale_rotation_translation().1;
        let sampled_mount = self.mount.place(&self.pose, aim, contacts.primary);
        let target = hand + sampled_mount.transform_vector3(support - contacts.primary);
        let primary = self.arms.apply(&mut self.pose, hand, target);
        self.arms.orient_support(
            &mut self.pose,
            sampled_mount.to_scale_rotation_translation().1
                * aim.inverse()
                * characters::grip_rotation(pitch)
                * Quat::from_rotation_x(roll + palm.to_radians()),
        );
        self.authored
            .set_extension(&mut self.inspector.bones.editor, contacts.extension);
        self.inspector.bones.editor.apply(&mut self.pose);
        let framing =
            Guns::held_transform(body.transform_point3(primary), 0.0, pitch, contacts.primary);
        let model_to_scene = framing.inverse() * body;
        let mount = self.mount.place(&self.pose, aim, contacts.primary);
        self.guns
            .review_mounted(&mut self.scene, &self.rig, model_to_scene * mount);
        self.scene
            .update_instance(self.actor.instance, self.actor.mesh, model_to_scene);
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
        let camera = self.inspector.camera();
        let hud = self.inspector.draw(
            frame.size,
            Mat4::from(model_to_scene),
            &format!("{:.0}%  {}", contacts.extension * 100., self.script.status),
        );
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
    let (index, (_, bind)) = model
        .joints()
        .enumerate()
        .find(|(_, (name, _))| name.rsplit([':', '_']).next() == Some("RightHand"))
        .ok_or("missing right hand mount")?;
    let mount = body::reach::JointMount::new(index, bind, &pose);
    let editor = PoseEditor::bind(model.joints(), crate::foregrip_pose::parent)?;
    let target = Vec3::new(0., 0., -0.12);
    let home = Camera {
        pos: target
            + Quat::from_rotation_y(-2.037) * Quat::from_rotation_x(-0.294) * Vec3::Z * 1.93,
        target,
        near: 0.02,
        fov_y: 38f32.to_radians(),
    };
    let mut inspector = PoseViewport::new(editor, home, ROWS)?;
    inspector.bones.selected = inspector
        .bones
        .editor
        .joints
        .iter()
        .position(|j| j.name.ends_with("LeftArm"))
        .unwrap_or(0);
    let authored = crate::foregrip_pose::ForegripPose::bind(&model, &inspector.bones.editor)?;
    let mut game = Preview {
        mount,
        inspector,
        authored,
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
    };
    app::run(
        Config {
            title: "Warp Animation | Growing Guns".into(),
            fullscreen: false,
            keyboard: false,
            pointer: true,
            size: args.size((1280, 800)).into(),
            target_fps: Some(60.0),
            ..Default::default()
        },
        &mut game,
    )
}
