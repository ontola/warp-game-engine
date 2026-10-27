//! Local player input as a gamepad-shaped value: keyboard and mouse map
//! onto the same buttons and sticks, so a game handles one type. Devices
//! drive local players in order (keyboard first, then pads); with a single
//! local player every device drives it.
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
    /// Held now.
    pub held: Buttons,
    /// Pressed since the previous tick (never lost between ticks).
    pub pressed: Buttons,
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
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Buttons::EAST,
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
        _ => 0,
    }
}

fn shape(v: f32, t: &Tuning) -> f32 {
    let a = v.abs();
    if a < t.deadzone { 0.0 } else { v.signum() * ((a - t.deadzone) / (1.0 - t.deadzone)).powf(t.curve) }
}

#[derive(Default)]
struct Keyboard { keys: HashSet<KeyCode>, mouse: u32, look: [f64; 2], pressed: u32 }

pub struct Devices {
    pub tuning: Tuning,
    pub keyboard_enabled: bool,
    /// The mouse only steers while captured.
    pub mouse_captured: bool,
    kb: Keyboard,
    gilrs: Option<Gilrs>,
    pads: Vec<(GamepadId, u32)>,
}

impl Devices {
    pub fn new(keyboard_enabled: bool) -> Self {
        let gilrs = Gilrs::new().map_err(|e| eprintln!("gamepads unavailable: {e}")).ok();
        let pads = gilrs.as_ref().map_or(Vec::new(), |g| g.gamepads().map(|(id, _)| (id, 0)).collect());
        Self { tuning: Tuning::default(), keyboard_enabled, mouse_captured: false, kb: Keyboard::default(), gilrs, pads }
    }

    pub fn pad_count(&self) -> usize { self.pads.len() }

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
        let b = match button { MouseButton::Left => Buttons::RIGHT_TRIGGER, MouseButton::Right => Buttons::EAST, _ => 0 };
        if state == ElementState::Pressed && self.mouse_captured { self.kb.mouse |= b; self.kb.pressed |= b; } else { self.kb.mouse &= !b; }
    }

    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        if self.mouse_captured { self.kb.look[0] += dx; self.kb.look[1] += dy; }
    }

    /// Drain pad events (edges) once per frame.
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

    /// Controls for `players` local players for one tick of `dt` seconds;
    /// consumes edges and mouse motion.
    pub fn controls(&mut self, players: usize, dt: f32) -> Vec<Controls> {
        let t = self.tuning;
        let mut devices = Vec::new();
        if self.keyboard_enabled {
            let k = &mut self.kb;
            let key = |c| k.keys.contains(&c) as i32 as f32;
            let held = k.keys.iter().fold(k.mouse, |a, &c| a | key_button(c));
            let mut mv = [key(KeyCode::KeyD) - key(KeyCode::KeyA), key(KeyCode::KeyW) - key(KeyCode::KeyS)];
            let len = (mv[0] * mv[0] + mv[1] * mv[1]).sqrt();
            if len > 1.0 { mv = [mv[0] / len, mv[1] / len]; }
            devices.push(Controls {
                mv,
                look: [k.look[0] as f32 * t.mouse, -k.look[1] as f32 * t.mouse],
                held: Buttons(held),
                pressed: Buttons(std::mem::take(&mut k.pressed)),
            });
            k.look = [0.0; 2];
        }
        if let Some(g) = &self.gilrs {
            for (id, pressed) in &mut self.pads {
                let gp = g.gamepad(*id);
                if !gp.is_connected() { continue; }
                let mut held = 0;
                for b in [PadButton::South, PadButton::East, PadButton::West, PadButton::North, PadButton::LeftTrigger, PadButton::RightTrigger,
                    PadButton::LeftTrigger2, PadButton::RightTrigger2, PadButton::Select, PadButton::Start,
                    PadButton::DPadUp, PadButton::DPadDown, PadButton::DPadLeft, PadButton::DPadRight] {
                    let analog = gp.button_data(b).is_some_and(|d| d.value() > 0.4);
                    if gp.is_pressed(b) || analog { held |= pad_button(b); }
                }
                devices.push(Controls {
                    mv: [shape(gp.value(Axis::LeftStickX), &t), shape(gp.value(Axis::LeftStickY), &t)],
                    look: [shape(gp.value(Axis::RightStickX), &t) * t.stick * dt, shape(gp.value(Axis::RightStickY), &t) * t.stick * dt],
                    held: Buttons(held),
                    pressed: Buttons(std::mem::take(pressed)),
                });
            }
        }
        let mut out = vec![Controls::default(); players];
        for (k, d) in devices.into_iter().enumerate() {
            let slot = if players == 1 { 0 } else { k };
            let Some(o) = out.get_mut(slot) else { break };
            o.mv = [(o.mv[0] + d.mv[0]).clamp(-1.0, 1.0), (o.mv[1] + d.mv[1]).clamp(-1.0, 1.0)];
            o.look = [o.look[0] + d.look[0], o.look[1] + d.look[1]];
            o.held.0 |= d.held.0;
            o.pressed.0 |= d.pressed.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_shaping_has_a_dead_zone_and_reaches_one() {
        let t = Tuning::default();
        assert_eq!(shape(0.1, &t), 0.0);
        assert!((shape(1.0, &t) - 1.0).abs() < 1e-6);
        assert!((shape(-1.0, &t) + 1.0).abs() < 1e-6);
        assert!(shape(0.5, &t) < 0.5);
    }
}
