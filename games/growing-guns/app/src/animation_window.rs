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
}

impl Game for Preview {
    fn local_players(&self) -> usize {
        1
    }
    fn mouse_capture(&self) -> Option<bool> {
        Some(false)
    }
    fn tick(&mut self, _: &[Controls]) {
        self.tick += 1;
        match self.tick % 420 {
            60 => self.rig.grow_to(self.large),
            270 => self.rig.grow_to(Stats::default()),
            _ => {}
        }
        self.rig.update(1.0 / 60.0);
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
        let initial = Guns::held_transform(body.transform_point3(hand), 0.0, 0.0, contacts.primary);
        let target = body
            .inverse()
            .transform_point3(initial.transform_point3(support));
        let primary = self.arms.apply(&mut self.pose, hand, target);
        self.arms.orient_support(
            &mut self.pose,
            characters::grip_rotation(0.0) * Quat::from_rotation_x(roll),
        );
        let gun = Guns::held_transform(body.transform_point3(primary), 0.0, 0.0, contacts.primary);
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
        let camera = Camera {
            pos: Vec3::new(-1.65, 0.56, -0.95),
            target: Vec3::new(0.0, 0.0, -0.12),
            near: 0.02,
            fov_y: 38f32.to_radians(),
        };
        let mut hud = HudDrawList::new(frame.size);
        hud.text.push(TextRun {
            id: 1,
            text: format!(
                "Foregrip {:.0}%    {}",
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
    let arms = TwoHandHold::bind(model.joints()).ok_or("missing arm joints")?;
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
    };
    app::run(
        Config {
            title: "Warp Animation | Growing Guns".into(),
            fullscreen: false,
            keyboard: false,
            size: args.size((960, 640)).into(),
            target_fps: Some(60.0),
            ..Default::default()
        },
        &mut game,
    )
}
