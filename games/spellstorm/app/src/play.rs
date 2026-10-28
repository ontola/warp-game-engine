//! The game in a window: local pads and the keyboard drive the human
//! wizards (pad.gd's buttons), bots fill the other seats, and the view
//! follows the sim. The same `Spellstorm` renders headless for shots.
use crate::kits::Kits;
use crate::view::View;
use app::{Buttons, Controls, Draw, Frame};
use glam::Vec2;
use render::hud::HudDrawList;
use spellstorm::wizard::Pad;
use spellstorm::{Seat, Settings, World};

pub struct Spellstorm {
    pub world: World,
    pub view: View,
    pub hud: crate::hud::Hud,
    pub menu: crate::menu::Menu,
    humans: usize,
    quit: bool,
    built: bool,
}

/// The shell's buttons as pad.gd's.
const MAP: [(u32, u32); 16] = [
    (Buttons::SOUTH, Pad::A),
    (Buttons::EAST, Pad::B),
    (Buttons::WEST, Pad::X),
    (Buttons::NORTH, Pad::Y),
    (Buttons::LEFT_SHOULDER, Pad::LB),
    (Buttons::RIGHT_SHOULDER, Pad::RB),
    (Buttons::LEFT_TRIGGER, Pad::LT),
    (Buttons::RIGHT_TRIGGER, Pad::RT),
    (Buttons::SELECT, Pad::BACK),
    (Buttons::START, Pad::START),
    (Buttons::LEFT_STICK, Pad::LS),
    (Buttons::RIGHT_STICK, Pad::RS),
    (Buttons::UP, Pad::UP),
    (Buttons::DOWN, Pad::DOWN),
    (Buttons::LEFT, Pad::LEFT),
    (Buttons::RIGHT, Pad::RIGHT),
];

/// One tick's controls as a wizard's pad (Pad._read_joypad): the shell's
/// stick y is up, the game's is down the screen (towards the camera).
pub fn pad(c: &Controls) -> Pad {
    let mut p = Pad::default();
    for (b, to) in MAP {
        if c.held.has(b) || c.pressed.has(b) {
            p.set(to, true);
        }
    }
    p.mv = Pad::dead(Vec2::new(c.mv[0], -c.mv[1]));
    p.aim = Pad::dead(Vec2::new(c.aim[0], -c.aim[1]));
    p.fire = p.aim.length() > 0.5;
    p
}

impl Spellstorm {
    /// `humans` seats for local players first, then `bots`.
    pub fn new(kits: Kits, humans: usize, bots: usize, seed: u64) -> Spellstorm {
        let mut seats: Vec<Seat> = (0..humans).map(|i| Seat::player(i, spellstorm::world::NAMES[i % 6])).collect();
        seats.extend((humans..humans + bots).map(Seat::bot));
        let world = World::new(&seats, seed, Settings::default());
        let mut hud = crate::hud::Hud::default();
        hud.level(&world);
        let mut menu = crate::menu::Menu::new(true);
        if humans > 0 {
            menu.show(true, &world);
        }
        Spellstorm { world, view: View::new(kits), hud, menu, humans, quit: false, built: false }
    }

    /// One tick with these players' controls.
    pub fn step(&mut self, controls: &[Controls]) {
        let was_open = self.menu.open;
        match self.menu.update(&mut self.world, controls, spellstorm::world::DT as f32) {
            crate::menu::Action::StartAt(b) => self.world.start_at(b),
            crate::menu::Action::Quit => self.quit = true,
            _ => {}
        }
        // The game stands still under the menu; the press that closed it
        // does not reach the wizards.
        self.world.paused = self.menu.open;
        let idle = self.menu.open || was_open;
        for (i, c) in controls.iter().enumerate().take(self.humans) {
            let c = if idle { &Controls::default() } else { c };
            if let Some(&id) = self.world.wizards.get(i) {
                self.world.wiz_mut(id).input = pad(c);
            }
        }
        self.world.step();
        self.view.fx.absorb(&self.world);
        self.hud.absorb(&self.world);
        self.hud.update(spellstorm::world::DT as f32);
    }
}

impl app::Game for Spellstorm {
    fn local_players(&self) -> usize {
        self.humans
    }

    fn tick(&mut self, controls: &[Controls]) {
        self.step(controls);
    }

    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        if !self.built || self.view.stale(&self.world) {
            self.view.build(&self.world)?;
            frame.set_scene(&self.view.scene, &self.view.light);
            let r = frame.renderer().ok_or("no renderer")?;
            self.view.upload(frame.gpu, r)?;
            self.built = true;
        }
        let r = frame.renderer().ok_or("no renderer")?;
        self.view.pose(frame.gpu, r, &self.world, frame.dt)?;
        let mut hud = HudDrawList::new(frame.size);
        self.hud.draw(&mut hud, &self.world, &self.view.camera, frame.size);
        self.menu.draw(&mut hud, &self.world, frame.size);
        Ok(Draw { cameras: vec![self.view.camera], light: self.view.light.clone(), hud })
    }

    fn escape(&mut self) -> bool {
        self.menu.escape(&self.world);
        true
    }

    fn quit(&self) -> bool {
        self.quit
    }

    fn title(&self) -> Option<String> {
        Some("Spellstorm".into())
    }

    fn state(&self) -> Option<String> {
        Some(self.world.snapshot().to_string())
    }

    /// Twin-stick aiming reads the raw sticks; Pad::dead shapes them.
    fn tuning(&self, t: &mut app::Tuning) {
        t.deadzone = 0.0;
        t.curve = 1.0;
    }
}
