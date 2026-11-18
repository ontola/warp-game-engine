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

use app::editor::PoseViewport;
use body::pose_editor::PoseEditor;

struct Preview {
    scene: Scene,
    rig: Rig,
    guns: Guns,
    large: Stats,
    weapon: PreviewWeapon,
    actor: ActorHandle,
    model: Character,
    standing: characters::Standing,
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
const ANIMATION_MENU: u32 = app::debug::GAME_IDS;
const PAUSED: u32 = ANIMATION_MENU + 1;
const BONES: u32 = ANIMATION_MENU + 2;
const CLEAR_OFFSETS: u32 = ANIMATION_MENU + 3;

const WEAPON: u32 = ANIMATION_MENU + 4;

#[derive(Clone, Copy, PartialEq)]
enum PreviewWeapon { Cycle, Stock, Sniper }

impl PreviewWeapon {
    const CHOICES: [Self; 3] = [Self::Cycle, Self::Stock, Self::Sniper];

    fn label(self) -> &'static str {
        match self {
            Self::Cycle => "Growth cycle",
            Self::Stock => "Stock",
            Self::Sniper => "Sniper",
        }
    }
}

impl Preview {
    fn select_weapon(&mut self, weapon: PreviewWeapon) {
        self.weapon = weapon;
        if weapon == PreviewWeapon::Cycle {
            self.tick = 0;
        }
        self.rig.set(if weapon == PreviewWeapon::Sniper { self.large } else { Stats::default() });
    }
}

impl Game for Preview {
    fn debug(
        &self,
    ) -> (
        Vec<render::debug_ui::DebugMenu>,
        Vec<render::debug_ui::DebugControl>,
    ) {
        use render::debug_ui::DebugValue;
        let mut menus = app::editor::EditorMenus::default();
        menus.page(
            ANIMATION_MENU,
            None,
            "Animation preview",
            "Inspect and adjust the current pose.",
        );
        menus.control(
            ANIMATION_MENU,
            PAUSED,
            "Pause playback",
            DebugValue::Toggle(self.inspector.paused),
            "Freeze the animation while editing.",
        );
        menus.control(
            ANIMATION_MENU,
            BONES,
            "Show skeleton",
            DebugValue::Toggle(self.inspector.bones.shown),
            "Display the editable joints.",
        );
        menus.control(
            ANIMATION_MENU,
            WEAPON,
            "Weapon",
            DebugValue::Choice {
                index: self.weapon as usize,
                labels: PreviewWeapon::CHOICES.map(|choice| choice.label().to_owned()).into(),
            },
            "Compare weapons at the same pose or replay the growth cycle.",
        );
        menus
            .control(
                ANIMATION_MENU,
                CLEAR_OFFSETS,
                "Reset grip sliders",
                DebugValue::Button,
                "Clear wrist, palm and aim offsets.",
            )
            .enabled = self.inspector.values.iter().any(|&v| v != 0.);
        menus.finish()
    }

    fn debug_action(&mut self, action: render::debug_ui::DebugAction) {
        use render::debug_ui::DebugAction::*;
        match action {
            SetToggle { id: PAUSED, value } => self.inspector.paused = value,
            SetToggle { id: BONES, value } => self.inspector.bones.shown = value,
            Activate { id: CLEAR_OFFSETS } => self.inspector.values.fill(0.),
            SetChoice { id: WEAPON, index } => {
                if let Some(&weapon) = PreviewWeapon::CHOICES.get(index) {
                    self.select_weapon(weapon);
                }
            }
            ResetDefaults => {
                self.select_weapon(PreviewWeapon::Cycle);
                self.inspector.paused = false;
                self.inspector.bones.shown = true;
                self.inspector.values.fill(0.);
            }
            _ => {}
        }
    }

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
        match (self.weapon, self.tick % 420) {
            (PreviewWeapon::Cycle, 60) => self.rig.grow_to(self.large),
            (PreviewWeapon::Cycle, 270) => self.rig.grow_to(Stats::default()),
            _ => {}
        }
        self.rig.update(1.0 / 60.0);
    }

    fn pointer(&mut self, p: app::Pointer, size: [u32; 2]) {
        self.inspector.pointer(p, size);
    }
    fn editor_shortcut(&self,key:u8)->Option<(&'static str,bool)> { (key==12).then(||("Pose",self.inspector.windows_open())) }
    fn editor_windows_open(&self) -> bool { self.inspector.windows_open() }
    fn set_editor_windows_open(&mut self, open: bool) { self.inspector.set_windows_open(open); }
    fn function_key(&mut self, n: u8) -> bool {
        self.inspector.function_key(n)
    }
    fn state(&self) -> Option<String> {
        let mut state = self.inspector.snapshot();
        state["weapon"] = self.weapon.label().into();
        state["tick"] = self.tick.into();
        state["model"] = self.model.name.clone().into();
        Some(state.to_string())
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        let time = self.tick as f32 / 60.0;
        let hand = self.standing.pose(time, &mut self.pose);
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

fn parent(name: &str) -> Option<String> {
    let core = name.rsplit([':', '_']).next()?;
    let prefix = &name[..name.len() - core.len()];
    let center = match core {
        "Spine" => "Hips",
        "Spine1" => "Spine",
        "Spine2" => "Spine1",
        "Neck" => "Spine2",
        "Head" => "Neck",
        _ => "",
    };
    if !center.is_empty() {
        return Some(format!("{prefix}{center}"));
    }
    let side = ["Left", "Right"]
        .into_iter()
        .find(|s| core.starts_with(s))?;
    let limb = &core[side.len()..];
    let base = match limb {
        "Shoulder" => return Some(format!("{prefix}Spine2")),
        "Arm" => "Shoulder",
        "ForeArm" => "Arm",
        "Hand" => "ForeArm",
        "UpLeg" => return Some(format!("{prefix}Hips")),
        "Leg" => "UpLeg",
        "Foot" => "Leg",
        "ToeBase" => "Foot",
        "Eye" => return Some(format!("{prefix}Head")),
        n if n.starts_with("Hand") && n.ends_with('1') => "Hand",
        n if n.starts_with("Hand") => {
            let digit = n.chars().last()?.to_digit(10)?.checked_sub(1)?;
            return Some(format!("{prefix}{side}{}{digit}", &n[..n.len() - 1]));
        }
        _ => return None,
    };
    Some(format!("{prefix}{side}{base}"))
}

fn load_preview(seed: u64) -> Result<Preview, String> {
    let model = crate::view::characters::preview_model(seed)?;
    let mut scene = Scene::default();
    let guns = Guns::reserve(&mut scene, 1);
    let actor = model.add_actor(&mut scene, Affine3A::IDENTITY)?;
    let rig = Rig::with_detail(guns.detail);
    let mut world = growing_guns::World::new(3, &[false]);
    let _scripts = crate::scripts::attach(&mut world)?;
    let id = world.cards.find("sniper").ok_or("missing sniper card")?;
    world.give(0, id);
    let arms = body::reach::TwoHandHold::bind(model.joints()).ok_or("missing arm joints")?;
    let (pose, _) = characters::standing_pose(&model, 0.0)?;
    let (index, (_, bind)) = model
        .joints()
        .enumerate()
        .find(|(_, (name, _))| name.rsplit([':', '_']).next() == Some("RightHand"))
        .ok_or("missing right hand mount")?;
    let mount = body::reach::JointMount::new(index, bind, &pose);
    let editor = PoseEditor::bind(model.joints(), parent)?;
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
    Ok(Preview {
        mount,
        inspector,
        scene,
        rig,
        guns,
        large: Stats::of(&world.players[0]),
        actor,
        standing: characters::Standing::bind(&model)?,
        model,
        arms,
        script: HandAnimation::load()?,
        pose,
        tick: 0,
        weapon: PreviewWeapon::Cycle,
    })
}

pub fn run(args: &crate::Args) -> Result<(), String> {
    let mut game = load_preview(args.num("--seed", 3))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use render::debug_ui::DebugAction::{SetChoice, SetToggle};

    #[test]
    fn paused_preview_changes_the_weapon_at_the_same_pose() {
        let mut preview = load_preview(3).unwrap();
        for _ in 0..90 { preview.tick(&[]); }
        assert!(preview.rig.is_growing());
        preview.debug_action(SetToggle { id: PAUSED, value: true });
        for (index, expected) in [(2, preview.large), (1, Stats::default())] {
            preview.debug_action(SetChoice { id: WEAPON, index });
            preview.tick(&[]);
            assert_eq!(preview.rig.shown, expected);
            assert!(!preview.rig.is_growing());
            assert_eq!(preview.tick, 90);
        }
        preview.debug_action(SetChoice { id: WEAPON, index: 0 });
        preview.debug_action(SetToggle { id: PAUSED, value: false });
        for _ in 0..60 { preview.tick(&[]); }
        assert!(preview.rig.is_growing());
    }
}
