//! Local player input as a gamepad-shaped value: keyboard and mouse map
//! onto the same buttons and sticks, so a game handles one type. Devices
//! drive local players in order (keyboard first, then pads); with a single
//! local player every device drives it.
#[cfg(not(target_os = "android"))]
use gilrs::{Axis, Button as PadButton, EventType, GamepadId, Gilrs};
use std::collections::HashSet;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

/// Buttons as a bit set. Keyboard: Space South, Shift/RMB East, R West,
/// F North, LMB RightTrigger, Q LeftShoulder, E RightShoulder, Tab Select,
/// Enter Start, arrows and the D-pad Up/Down/Left/Right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons(pub u32);

impl Buttons {
    pub const SOUTH: u32 = 1;
    pub const EAST: u32 = 2;
    pub const WEST: u32 = 4;
    pub const NORTH: u32 = 8;
    pub const LEFT_SHOULDER: u32 = 16;
    pub const RIGHT_SHOULDER: u32 = 32;
    pub const LEFT_TRIGGER: u32 = 64;
    pub const RIGHT_TRIGGER: u32 = 128;
    pub const SELECT: u32 = 256;
    pub const START: u32 = 512;
    pub const UP: u32 = 1024;
    pub const DOWN: u32 = 2048;
    pub const LEFT: u32 = 4096;
    pub const RIGHT: u32 = 8192;
    /// Stick clicks (L3, R3).
    pub const LEFT_STICK: u32 = 16384;
    pub const RIGHT_STICK: u32 = 32768;
    pub fn has(self, b: u32) -> bool { self.0 & b != 0 }
}

/// One local player's input for one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// Left stick or WASD: x right, y forward, length at most 1.
    pub mv: [f32; 2],
    /// Look change this tick in radians: x right, y up. Mouse and the right
    /// stick (rate times tick) both land here.
    pub look: [f32; 2],
    /// The right stick's position: x right, y up, length at most 1 (0 on
    /// keyboard and mouse). For twin-stick aiming, where `look` is a rate.
    pub aim: [f32; 2],
    /// Analog left/right trigger pressures, 0..1 (keyboard mouse buttons: 0 or 1).
    pub triggers: [f32; 2],
    /// Held now.
    pub held: Buttons,
    /// Pressed since the previous tick (never lost between ticks).
    pub pressed: Buttons,
}

/// Free cursor input in physical window pixels. Edges are consumed once per
/// tick; a stationary cursor does not take focus from keyboard or gamepad.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pointer {
    pub position: Option<[f32; 2]>,
    pub moved: bool,
    pub pressed: bool,
    /// Secondary mouse button pressed since the previous tick.
    pub right_pressed: bool,
    pub held: bool,
    /// Wheel lines scrolled since the last tick, positive away from the user (up).
    pub wheel: f32,
}

impl Pointer {
    pub fn take(&mut self) -> Self {
        let sample = *self;
        self.moved = false;
        self.pressed = false;
        self.right_pressed = false;
        self.wheel = 0.0;
        sample
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    /// Radians per mouse count.
    pub mouse: f32,
    /// Right stick rate in radians per second at full deflection.
    pub stick: f32,
    pub deadzone: f32,
    /// Response exponent after the dead zone.
    pub curve: f32,
}

impl Default for Tuning {
    fn default() -> Self { Self { mouse: 0.0022, stick: 3.0, deadzone: 0.18, curve: 1.5 } }
}

fn key_button(k: KeyCode) -> u32 {
    match k {
        KeyCode::Space => Buttons::SOUTH,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Buttons::RIGHT_SHOULDER,
        KeyCode::KeyR => Buttons::WEST,
        KeyCode::KeyF => Buttons::NORTH,
        KeyCode::KeyQ => Buttons::LEFT_SHOULDER,
        KeyCode::KeyE => Buttons::RIGHT_SHOULDER,
        KeyCode::Tab => Buttons::SELECT,
        KeyCode::Enter => Buttons::START,
        KeyCode::ArrowUp => Buttons::UP,
        KeyCode::ArrowDown => Buttons::DOWN,
        KeyCode::ArrowLeft => Buttons::LEFT,
        KeyCode::ArrowRight => Buttons::RIGHT,
        _ => 0,
    }
}

#[cfg(not(target_os = "android"))]
fn pad_button(b: PadButton) -> u32 {
    match b {
        PadButton::South => Buttons::SOUTH,
        PadButton::East => Buttons::EAST,
        PadButton::West => Buttons::WEST,
        PadButton::North => Buttons::NORTH,
        PadButton::LeftTrigger => Buttons::LEFT_SHOULDER,
        PadButton::RightTrigger => Buttons::RIGHT_SHOULDER,
        PadButton::LeftTrigger2 => Buttons::LEFT_TRIGGER,
        PadButton::RightTrigger2 => Buttons::RIGHT_TRIGGER,
        PadButton::Select => Buttons::SELECT,
        PadButton::Start => Buttons::START,
        PadButton::DPadUp => Buttons::UP,
        PadButton::DPadDown => Buttons::DOWN,
        PadButton::DPadLeft => Buttons::LEFT,
        PadButton::DPadRight => Buttons::RIGHT,
        PadButton::LeftThumb => Buttons::LEFT_STICK,
        PadButton::RightThumb => Buttons::RIGHT_STICK,
        _ => 0,
    }
}

#[cfg(any(not(target_os = "android"), test))]
fn shape(v: f32, t: &Tuning) -> f32 {
    let a = v.abs();
    if a < t.deadzone { 0.0 } else { v.signum() * ((a - t.deadzone) / (1.0 - t.deadzone)).powf(t.curve) }
}

#[derive(Default)]
struct Keyboard { keys: HashSet<KeyCode>, mouse: u32, look: [f64; 2], pressed: u32 }

pub struct Devices {
    pub tuning: Tuning,
    pub keyboard_enabled: bool,
    /// The free cursor reaches the game even without a keyboard player
    /// (a party lobby: the mouse points and clicks but is nobody).
    pub pointer_enabled: bool,
    /// The mouse only steers while captured.
    pub mouse_captured: bool,
    pointer: Pointer,
    kb: Keyboard,
    #[cfg(not(target_os = "android"))]
    gilrs: Option<Gilrs>,
    #[cfg(not(target_os = "android"))]
    pads: Vec<(GamepadId, u32)>,
    /// Live rumble per pad; dropping an effect stops it.
    #[cfg(not(target_os = "android"))]
    rumble: std::collections::HashMap<GamepadId, gilrs::ff::Effect>,
}

impl Devices {
    pub fn new(keyboard_enabled: bool) -> Self {
        #[cfg(not(target_os = "android"))]
        let gilrs = Gilrs::new().map_err(|e| eprintln!("gamepads unavailable: {e}")).ok();
        #[cfg(not(target_os = "android"))]
        let pads = gilrs.as_ref().map_or(Vec::new(), |g| g.gamepads().map(|(id, _)| (id, 0)).collect());
        Self { tuning: Tuning::default(), keyboard_enabled, pointer_enabled: false, mouse_captured: false, pointer: Pointer::default(), kb: Keyboard::default(),
            #[cfg(not(target_os = "android"))] gilrs,
            #[cfg(not(target_os = "android"))] pads,
            #[cfg(not(target_os = "android"))] rumble: Default::default() }
    }

    /// Vibrate local pad `pad` (in `controls` order, after the keyboard):
    /// `weak` is the light buzzy motor, `strong` the heavy one, both 0..1, for
    /// `seconds` (at most 5). Each call replaces the last; zeros stop it.
    /// Under GameNight a game rumbles through `gamenight::Client::rumble`.
    /// Returns false when the pad cannot vibrate.
    #[cfg(not(target_os = "android"))]
    pub fn rumble(&mut self, pad: usize, weak: f32, strong: f32, seconds: f32) -> bool {
        use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Replay, Ticks};
        let Some(g) = &mut self.gilrs else { return false };
        let Some(&(id, _)) = self.pads.get(pad) else { return false };
        self.rumble.remove(&id);
        let (weak, strong) = (weak.clamp(0.0, 1.0), strong.clamp(0.0, 1.0));
        if weak == 0.0 && strong == 0.0 { return true; }
        if !g.gamepad(id).is_ff_supported() { return false; }
        let ms = (seconds.clamp(0.0, 5.0) * 1000.0) as u32;
        let base = |kind| BaseEffect { kind, scheduling: Replay { play_for: Ticks::from_ms(ms), ..Default::default() }, envelope: Default::default() };
        let effect = EffectBuilder::new()
            .add_effect(base(BaseEffectType::Weak { magnitude: (weak * 65535.0) as u16 }))
            .add_effect(base(BaseEffectType::Strong { magnitude: (strong * 65535.0) as u16 }))
            .gamepads(&[id]).finish(g);
        match effect.and_then(|e| e.play().map(|_| e)) {
            Ok(e) => { self.rumble.insert(id, e); true }
            Err(_) => false,
        }
    }

    #[cfg(target_os = "android")]
    pub fn rumble(&mut self, _: usize, _: f32, _: f32, _: f32) -> bool { false }
    #[cfg(not(target_os = "android"))]
    pub fn pad_count(&self) -> usize { self.pads.len() }
    #[cfg(target_os = "android")]
    pub fn pad_count(&self) -> usize { 0 }

    pub fn key(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        let b = key_button(code);
        if state == ElementState::Pressed {
            if !repeat { self.kb.pressed |= b; }
            self.kb.keys.insert(code);
        } else {
            self.kb.keys.remove(&code);
        }
    }

    pub fn mouse_button(&mut self, button: MouseButton, state: ElementState) {
        if button == MouseButton::Left && !self.mouse_captured && (self.keyboard_enabled || self.pointer_enabled) {
            self.pointer.held = state == ElementState::Pressed;
            self.pointer.pressed |= self.pointer.held;
        }
        if button == MouseButton::Right && !self.mouse_captured && (self.keyboard_enabled || self.pointer_enabled) {
            self.pointer.right_pressed |= state == ElementState::Pressed;
        }
        let b = match button { MouseButton::Left => Buttons::RIGHT_TRIGGER, MouseButton::Right => Buttons::LEFT_TRIGGER, _ => 0 };
        if state == ElementState::Pressed && self.mouse_captured { self.kb.mouse |= b; self.kb.pressed |= b; } else { self.kb.mouse &= !b; }
    }

    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        if self.mouse_captured { self.kb.look[0] += dx; self.kb.look[1] += dy; }
    }

    /// The mouse wheel over the free cursor, in lines (positive up).
    pub fn wheel(&mut self, lines: f32) {
        if !self.mouse_captured { self.pointer.wheel += lines; }
    }

    pub fn cursor(&mut self, position: Option<[f32; 2]>) {
        self.pointer.moved |= self.pointer.position != position;
        self.pointer.position = position;
    }

    pub fn pointer(&mut self) -> Pointer {
        let p = self.pointer.take();
        if (self.keyboard_enabled || self.pointer_enabled) && !self.mouse_captured { p } else { Pointer::default() }
    }

    /// Capture transitions must not carry a menu click or old look delta into play.
    pub fn reset_mouse(&mut self) {
        self.kb.mouse = 0;
        self.kb.pressed &= !(Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
        self.kb.look = [0.0; 2];
        self.pointer.pressed = false;
        self.pointer.held = false;
        self.pointer.moved = false;
        self.pointer.wheel = 0.0;
    }

    /// Drain pad events (edges) once per frame.
    #[cfg(target_os = "android")]
    pub fn poll(&mut self) {}
    #[cfg(not(target_os = "android"))]
    pub fn poll(&mut self) {
        let Some(g) = &mut self.gilrs else { return };
        while let Some(ev) = g.next_event() {
            match ev.event {
                EventType::Connected => if !self.pads.iter().any(|p| p.0 == ev.id) { self.pads.push((ev.id, 0)) },
                EventType::ButtonPressed(b, _) => if let Some(p) = self.pads.iter_mut().find(|p| p.0 == ev.id) { p.1 |= pad_button(b) },
                _ => {}
            }
        }
    }

    /// Connected devices in [`Devices::controls`] order: the keyboard (when
    /// enabled), then the pads in connection order.
    pub fn connected(&self) -> Vec<Device> {
        let mut v = Vec::new();
        if self.keyboard_enabled { v.push(Device::Keyboard); }
        #[cfg(not(target_os = "android"))]
        if let Some(g) = &self.gilrs { v.extend(self.pads.iter().filter(|p| g.gamepad(p.0).is_connected()).map(|p| Device::Pad(usize::from(p.0) as u32))); }
        v
    }

    /// Controls for `players` local players for one tick of `dt` seconds;
    /// consumes edges and mouse motion. Devices drive seats in order.
    pub fn controls(&mut self, players: usize, dt: f32) -> Vec<Controls> {
        let devices = self.device_controls(dt);
        route(&devices, None, players)
    }

    /// One tick of every connected device's input, keyboard first, then
    /// pads; consumes edges and mouse motion.
    pub fn device_controls(&mut self, dt: f32) -> Vec<(Device, Controls)> {
        #[cfg(target_os = "android")]
        let _ = dt;
        let t = self.tuning;
        let mut devices = Vec::new();
        if self.keyboard_enabled {
            let k = &mut self.kb;
            let key = |c| k.keys.contains(&c) as i32 as f32;
            let held = k.keys.iter().fold(k.mouse, |a, &c| a | key_button(c));
            let mut mv = [key(KeyCode::KeyD) - key(KeyCode::KeyA), key(KeyCode::KeyW) - key(KeyCode::KeyS)];
            let len = (mv[0] * mv[0] + mv[1] * mv[1]).sqrt();
            if len > 1.0 { mv = [mv[0] / len, mv[1] / len]; }
            devices.push((Device::Keyboard, Controls {
                mv,
                look: [k.look[0] as f32 * t.mouse, -k.look[1] as f32 * t.mouse],
                aim: [0.0; 2],
                triggers: [f32::from(held & Buttons::LEFT_TRIGGER != 0), f32::from(held & Buttons::RIGHT_TRIGGER != 0)],
                held: Buttons(held),
                pressed: Buttons(std::mem::take(&mut k.pressed)),
            }));
            k.look = [0.0; 2];
        }
        #[cfg(not(target_os = "android"))]
        if let Some(g) = &self.gilrs {
            for (id, pressed) in &mut self.pads {
                let gp = g.gamepad(*id);
                if !gp.is_connected() { continue; }
                let mut held = 0;
                for b in [PadButton::South, PadButton::East, PadButton::West, PadButton::North, PadButton::LeftTrigger, PadButton::RightTrigger,
                    PadButton::LeftTrigger2, PadButton::RightTrigger2, PadButton::Select, PadButton::Start,
                    PadButton::DPadUp, PadButton::DPadDown, PadButton::DPadLeft, PadButton::DPadRight,
                    PadButton::LeftThumb, PadButton::RightThumb] {
                    let analog = gp.button_data(b).is_some_and(|d| d.value() > 0.4);
                    if gp.is_pressed(b) || analog { held |= pad_button(b); }
                }
                let aim = [shape(gp.value(Axis::RightStickX), &t), shape(gp.value(Axis::RightStickY), &t)];
                devices.push((Device::Pad(usize::from(*id) as u32), Controls {
                    mv: [shape(gp.value(Axis::LeftStickX), &t), shape(gp.value(Axis::LeftStickY), &t)],
                    look: [aim[0] * t.stick * dt, aim[1] * t.stick * dt],
                    aim,
                    triggers: [PadButton::LeftTrigger2, PadButton::RightTrigger2].map(|b| gp.button_data(b).map_or(0.0, |d| d.value().clamp(0.0, 1.0))),
                    held: Buttons(held),
                    pressed: Buttons(std::mem::take(pressed)),
                }));
            }
        }
        devices
    }
}

/// An input device on this machine. Pad numbers are stable while the
/// program runs (a reconnected pad may get a new one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Device { Keyboard, Pad(u32) }

impl Device {
    /// `keyboard`, `pad0`, `pad1`.. (scripts, logs).
    pub fn parse(s: &str) -> Result<Device, String> {
        let s = s.trim().to_ascii_lowercase();
        if s == "keyboard" || s == "kbm" { return Ok(Device::Keyboard); }
        s.strip_prefix("pad").and_then(|n| n.parse().ok()).map(Device::Pad).ok_or(format!("unknown device {s} (keyboard, pad0, pad1..)"))
    }
    pub fn is_pad(self) -> bool { matches!(self, Device::Pad(_)) }
}

impl std::fmt::Display for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Device::Keyboard => write!(f, "keyboard"), Device::Pad(n) => write!(f, "pad{n}") }
    }
}

/// Fold one device's input into a seat's.
pub fn merge_into(o: &mut Controls, d: &Controls) {
    o.mv = [(o.mv[0] + d.mv[0]).clamp(-1.0, 1.0), (o.mv[1] + d.mv[1]).clamp(-1.0, 1.0)];
    o.look = [o.look[0] + d.look[0], o.look[1] + d.look[1]];
    o.aim = [(o.aim[0] + d.aim[0]).clamp(-1.0, 1.0), (o.aim[1] + d.aim[1]).clamp(-1.0, 1.0)];
    o.triggers = [o.triggers[0].max(d.triggers[0]), o.triggers[1].max(d.triggers[1])];
    o.held.0 |= d.held.0;
    o.pressed.0 |= d.pressed.0;
}

/// Per-device input to `players` seats. `seats`: the devices bound to each
/// seat ([`crate::Game::seats`]); devices in no seat drive nothing. None:
/// devices drive seats in order, and a single seat takes every device.
pub fn route(devices: &[(Device, Controls)], seats: Option<&[Vec<Device>]>, players: usize) -> Vec<Controls> {
    let mut out = vec![Controls::default(); players];
    match seats {
        Some(seats) => {
            for (o, bound) in out.iter_mut().zip(seats) {
                for (dev, c) in devices { if bound.contains(dev) { merge_into(o, c); } }
            }
        }
        None => {
            for (k, (_, d)) in devices.iter().enumerate() {
                let slot = if players == 1 { 0 } else { k };
                let Some(o) = out.get_mut(slot) else { break };
                merge_into(o, d);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seat_routing_preserves_analog_triggers_and_right_stick() {
        let pad = Controls { aim: [0.6, -0.4], triggers: [0.25, 0.8], ..Default::default() };
        let devices = [(Device::Keyboard, Controls::default()), (Device::Pad(7), pad)];
        let routed = route(&devices, Some(&[vec![Device::Pad(7)]]), 1);
        assert_eq!(routed[0].aim, pad.aim);
        assert_eq!(routed[0].triggers, pad.triggers);
    }

    #[test]
    fn pointer_edges_are_consumed_once_and_menu_clicks_do_not_fire() {
        let mut d = Devices { tuning: Tuning::default(), keyboard_enabled: true, pointer_enabled: false, mouse_captured: false,
            pointer: Pointer::default(), kb: Keyboard::default(), gilrs: None, pads: vec![], rumble: Default::default() };
        d.cursor(Some([30.0, 40.0]));
        d.mouse_button(MouseButton::Left, ElementState::Pressed);
        d.mouse_button(MouseButton::Left, ElementState::Released);
        d.mouse_button(MouseButton::Right, ElementState::Pressed);
        d.mouse_button(MouseButton::Right, ElementState::Released);
        let p = d.pointer();
        assert!(p.right_pressed);
        assert!(p.pressed && p.moved && !p.held);
        assert_eq!(d.pointer(), Pointer { position: p.position, ..Default::default() });
        assert!(!d.controls(1, 1.0 / 60.0)[0].pressed.has(Buttons::RIGHT_TRIGGER));
        d.mouse_captured = true;
        d.mouse_button(MouseButton::Left, ElementState::Pressed);
        d.mouse_motion(300.0, 100.0);
        d.reset_mouse();
        let c = d.controls(1, 1.0 / 60.0)[0];
        assert_eq!(c.look, [0.0; 2]);
        assert!(!c.pressed.has(Buttons::RIGHT_TRIGGER) && !c.held.has(Buttons::RIGHT_TRIGGER));
    }

    #[test]
    fn stick_shaping_has_a_dead_zone_and_reaches_one() {
        let t = Tuning::default();
        assert_eq!(shape(0.1, &t), 0.0);
        assert!((shape(1.0, &t) - 1.0).abs() < 1e-6);
        assert!((shape(-1.0, &t) + 1.0).abs() < 1e-6);
        assert!(shape(0.5, &t) < 0.5);
    }

    fn press(b: u32) -> Controls { Controls { held: Buttons(b), pressed: Buttons(b), ..Default::default() } }

    #[test]
    fn routing_follows_order_or_bound_seats() {
        let devs = [(Device::Keyboard, press(Buttons::SOUTH)), (Device::Pad(3), press(Buttons::WEST)), (Device::Pad(5), press(Buttons::NORTH))];
        // Default: one seat takes everything; more seats go in order.
        assert_eq!(route(&devs, None, 1)[0].pressed.0, Buttons::SOUTH | Buttons::WEST | Buttons::NORTH);
        let two = route(&devs, None, 2);
        assert_eq!((two[0].pressed.0, two[1].pressed.0), (Buttons::SOUTH, Buttons::WEST));
        // Bound: a spare pad drives nothing; a seat can have two devices.
        let seats = vec![vec![Device::Keyboard, Device::Pad(5)], vec![]];
        let b = route(&devs, Some(&seats), 2);
        assert_eq!(b[0].pressed.0, Buttons::SOUTH | Buttons::NORTH);
        assert_eq!(b[1], Controls::default());
        assert_eq!(Device::parse("pad3"), Ok(Device::Pad(3)));
        assert_eq!(Device::parse("Keyboard"), Ok(Device::Keyboard));
        assert!(Device::parse("joystick").is_err());
        assert_eq!(Device::Pad(2).to_string(), "pad2");
    }
}
