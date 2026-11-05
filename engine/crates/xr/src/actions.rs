use anyhow::Result;
use app::{Buttons, Controls};
use openxr as xr;
use crate::Pose;

pub struct Actions {
    set: xr::ActionSet,
    movement: xr::Action<xr::Vector2f>, turn: xr::Action<xr::Vector2f>,
    fire: xr::Action<f32>, grenade: xr::Action<f32>, dash: xr::Action<f32>,
    jump: xr::Action<bool>, grip: xr::Action<f32>, reload: xr::Action<bool>, accept: xr::Action<bool>, back: xr::Action<bool>, menu: xr::Action<bool>,
    left_stick: xr::Action<bool>, right_stick: xr::Action<bool>,
    aim: xr::Action<xr::Posef>, space: xr::Space, previous: u32,
}
impl Actions {
    pub fn new(instance: &xr::Instance, session: &xr::Session<xr::Vulkan>) -> Result<Self> {
        let set=instance.create_action_set("game", "Growing Guns", 0)?;
        let movement=set.create_action("move", "Move / menu", &[])?;
        let turn=set.create_action("turn", "Snap turn", &[])?;
        let fire=set.create_action("fire", "Fire", &[])?;
        let grenade=set.create_action("grenade", "Special", &[])?;
        let dash=set.create_action("dash", "Dash", &[])?;
        let jump=set.create_action("jump", "Jump / select", &[])?;
        let grip=set.create_action("grip", "Jump", &[])?;
        let reload=set.create_action("reload", "Reload", &[])?;
        let accept=set.create_action("accept", "Select", &[])?;
        let back=set.create_action("back", "Back", &[])?;
        let menu=set.create_action("menu", "Pause", &[])?;
        let left_stick=set.create_action("left_stick", "Left stick press", &[])?;
        let right_stick=set.create_action("right_stick", "Right stick press", &[])?;
        let aim=set.create_action::<xr::Posef>("aim", "Gun aim", &[])?;
        instance.suggest_interaction_profile_bindings(instance.string_to_path("/interaction_profiles/oculus/touch_controller")?, &[
            xr::Binding::new(&movement,instance.string_to_path("/user/hand/left/input/thumbstick")?),
            xr::Binding::new(&turn,instance.string_to_path("/user/hand/right/input/thumbstick")?),
            xr::Binding::new(&fire,instance.string_to_path("/user/hand/right/input/trigger/value")?),
            xr::Binding::new(&grenade,instance.string_to_path("/user/hand/left/input/trigger/value")?),
            xr::Binding::new(&dash,instance.string_to_path("/user/hand/right/input/squeeze/value")?),
            xr::Binding::new(&jump,instance.string_to_path("/user/hand/right/input/a/click")?),
            xr::Binding::new(&grip,instance.string_to_path("/user/hand/left/input/squeeze/value")?),
            xr::Binding::new(&reload,instance.string_to_path("/user/hand/left/input/x/click")?),
            xr::Binding::new(&accept,instance.string_to_path("/user/hand/left/input/y/click")?),
            xr::Binding::new(&back,instance.string_to_path("/user/hand/right/input/b/click")?),
            xr::Binding::new(&menu,instance.string_to_path("/user/hand/left/input/menu/click")?),
            xr::Binding::new(&left_stick,instance.string_to_path("/user/hand/left/input/thumbstick/click")?),
            xr::Binding::new(&right_stick,instance.string_to_path("/user/hand/right/input/thumbstick/click")?),
            xr::Binding::new(&aim,instance.string_to_path("/user/hand/right/input/aim/pose")?),
        ])?;
        let space=aim.create_space(session,xr::Path::NULL,xr::Posef::IDENTITY)?;
        session.attach_action_sets(&[&set])?;
        Ok(Self {set,movement,turn,fire,grenade,dash,jump,grip,reload,accept,back,menu,left_stick,right_stick,aim,space,previous:0})
    }
    pub fn sample(&mut self, session: &xr::Session<xr::Vulkan>, base: &xr::Space, time: xr::Time, focused: bool) -> Result<(Controls,f32,Option<Pose>)> {
        session.sync_actions(&[(&self.set).into()])?;
        if !focused { self.previous=0; return Ok((Controls::default(),0.0,None)); }
        let path=xr::Path::NULL;
        let vec=|a:&xr::Action<xr::Vector2f>| -> Result<[f32;2]> { let s=a.state(session,path)?; Ok(if s.is_active {[s.current_state.x,s.current_state.y]}else{[0.0;2]}) };
        let boolean=|a:&xr::Action<bool>| -> Result<bool> {let s=a.state(session,path)?;Ok(s.is_active&&s.current_state)};
        let analog=|a:&xr::Action<f32>| -> Result<bool> {let s=a.state(session,path)?;Ok(s.is_active&&s.current_state>0.55)};
        let mv=vec(&self.movement)?.map(|x| if x.abs()<0.18 {0.0}else{x});
        let mut held=0;
        for (down,b) in [(analog(&self.fire)?,Buttons::RIGHT_TRIGGER),(analog(&self.grenade)?,Buttons::LEFT_TRIGGER),
            (analog(&self.dash)?,Buttons::RIGHT_SHOULDER),(analog(&self.grip)?,Buttons::LEFT_SHOULDER),(boolean(&self.jump)?,Buttons::SOUTH),(boolean(&self.reload)?,Buttons::WEST),(boolean(&self.accept)?,Buttons::SOUTH),
            (boolean(&self.back)?,Buttons::EAST),(boolean(&self.menu)?,Buttons::START),
            (boolean(&self.left_stick)?,Buttons::LEFT_STICK),(boolean(&self.right_stick)?,Buttons::RIGHT_STICK)] { if down {held|=b;} }
        // Menu navigation uses the same controls as the desktop game's menus.
        // Analog pressures for games that read them (throttle, brake); the
        // thresholded buttons above stay for everything else.
        let level=|a:&xr::Action<f32>| -> Result<f32> {let s=a.state(session,path)?;Ok(if s.is_active {s.current_state.clamp(0.0,1.0)} else {0.0})};
        let triggers=[level(&self.grenade)?,level(&self.fire)?];
        let controls=Controls {mv, triggers, held:Buttons(held),pressed:Buttons(held & !self.previous),..Default::default()};
        self.previous=held;
        let aim=if self.aim.is_active(session,path)? { let loc=self.space.locate(base,time)?;
            loc.location_flags.contains(xr::SpaceLocationFlags::POSITION_VALID|xr::SpaceLocationFlags::ORIENTATION_VALID).then(||crate::android::pose(loc.pose))
        } else {None};
        Ok((controls,vec(&self.turn)?[0],aim))
    }
}
