//! Local player input as a gamepad-shaped value: keyboard and mouse map
//! onto the same buttons and sticks, so a game handles one type. Devices
//! drive local players in order (keyboard first, then pads); with a single
//! local player every device drives it.
#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
use gilrs::{Axis, Button as PadButton, EventType, GamepadId, Gilrs};
use std::collections::HashSet;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

/// Buttons as a bit set. Keyboard: Space South, Backspace/RMB East, R West,
/// F North, LMB RightTrigger, Q/Ctrl LeftShoulder, E/Shift RightShoulder,
/// C RightStick, Tab Select, Enter Start, arrows and the D-pad
/// Up/Down/Left/Right.
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
    pub right_held: bool,
    pub middle_pressed: bool,
    pub middle_held: bool,
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
        self.middle_pressed = false;
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
        KeyCode::Backspace => Buttons::EAST,
        KeyCode::KeyR => Buttons::WEST,
        KeyCode::KeyF => Buttons::NORTH,
        KeyCode::KeyC => Buttons::RIGHT_STICK,
        KeyCode::KeyQ | KeyCode::ControlLeft | KeyCode::ControlRight => Buttons::LEFT_SHOULDER,
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

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
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

/// Touch screens, for first-person walking: a touch that starts in the
/// left 40 % of the screen is a thumb stick (it moves, from where the
/// thumb landed), any other touch drags the view, and a short touch that
/// barely moves is a tap: a pointer press at that spot (tap to walk,
/// buttons). Several fingers at once: one stick, any number of drags.
#[derive(Default)]
struct Touches {
    /// (finger, where it landed, where it is now).
    stick: Option<(u64, [f32; 2], [f32; 2])>,
    /// Look drags: finger, last position, landed at, landed when, moved too far for a tap.
    drags: Vec<(u64, [f32; 2], [f32; 2], web_time::Instant, bool)>,
    /// Drag since the last tick, in pixels.
    look: [f32; 2],
    /// Screen height and short side in pixels, for the look rate and the stick's reach.
    height: f32,
    short: f32,
}

/// The thumb stick's reach in pixels on a screen whose short side is `short`.
pub fn touch_stick_radius(short: f32) -> f32 { (short * 0.09).max(40.0) }

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
    touches: Touches,
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    gilrs: Option<Gilrs>,
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    pads: Vec<(GamepadId, u32)>,
    /// Live rumble per pad; dropping an effect stops it.
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    rumble: std::collections::HashMap<GamepadId, gilrs::ff::Effect>,
    /// Browser pads (crate::web_pads): (getGamepads slot, buttons held at
    /// the last poll, presses since the last tick), and the latest readings.
    #[cfg(target_arch = "wasm32")]
    web_pads: Vec<(u32, u32, u32)>,
    #[cfg(target_arch = "wasm32")]
    web_readings: Vec<crate::web_pads::Reading>,
}

impl Devices {
    pub fn new(keyboard_enabled: bool) -> Self {
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        let gilrs = Gilrs::new().map_err(|e| eprintln!("gamepads unavailable: {e}")).ok();
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        let pads = gilrs.as_ref().map_or(Vec::new(), |g| g.gamepads().map(|(id, _)| (id, 0)).collect());
        Self { tuning: Tuning::default(), keyboard_enabled, pointer_enabled: false, mouse_captured: false, pointer: Pointer::default(), kb: Keyboard::default(), touches: Touches::default(),
            #[cfg(not(any(target_os = "android", target_arch = "wasm32")))] gilrs,
            #[cfg(not(any(target_os = "android", target_arch = "wasm32")))] pads,
            #[cfg(not(any(target_os = "android", target_arch = "wasm32")))] rumble: Default::default(),
            #[cfg(target_arch = "wasm32")] web_pads: Vec::new(),
            #[cfg(target_arch = "wasm32")] web_readings: Vec::new() }
    }

    /// Vibrate local pad `pad` (in `controls` order, after the keyboard):
    /// `weak` is the light buzzy motor, `strong` the heavy one, both 0..1, for
    /// `seconds` (at most 5). Each call replaces the last; zeros stop it.
    /// Under Splitscreen Club a game rumbles through `splitscreen_club::Client::rumble`.
    /// Returns false when the pad cannot vibrate.
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
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
    #[cfg(target_arch = "wasm32")]
    pub fn rumble(&mut self, pad: usize, weak: f32, strong: f32, seconds: f32) -> bool {
        self.web_pads.get(pad).is_some_and(|p| crate::web_pads::rumble(p.0, weak, strong, seconds))
    }
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
    pub fn pad_count(&self) -> usize { self.pads.len() }
    #[cfg(target_os = "android")]
    pub fn pad_count(&self) -> usize { 0 }
    #[cfg(target_arch = "wasm32")]
    pub fn pad_count(&self) -> usize { self.web_pads.len() }

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
            self.pointer.right_held = state == ElementState::Pressed;
            self.pointer.right_pressed |= self.pointer.right_held;
        }
        if button == MouseButton::Middle && !self.mouse_captured && (self.keyboard_enabled || self.pointer_enabled) {
            self.pointer.middle_held = state == ElementState::Pressed;
            self.pointer.middle_pressed |= self.pointer.middle_held;
        }
        let b = match button { MouseButton::Left => Buttons::RIGHT_TRIGGER, MouseButton::Right => Buttons::LEFT_TRIGGER, _ => 0 };
        if state == ElementState::Pressed && self.mouse_captured { self.kb.mouse |= b; self.kb.pressed |= b; } else { self.kb.mouse &= !b; }
    }

    pub fn mouse_motion(&mut self, dx: f64, dy: f64) {
        if self.mouse_captured { self.kb.look[0] += dx; self.kb.look[1] += dy; }
    }

    /// A finger on a touch screen (`id` per finger) at `pos` in physical
    /// pixels on a `size` screen. See [`Touches`]: the stick and drags reach
    /// the keyboard player's `mv` and `look`, a tap the pointer.
    pub fn touch(&mut self, id: u64, phase: winit::event::TouchPhase, pos: [f32; 2], size: [u32; 2]) {
        use winit::event::TouchPhase::*;
        let t = &mut self.touches;
        t.height = size[1].max(1) as f32;
        t.short = size[0].min(size[1]).max(1) as f32;
        let slop = (size[0].min(size[1]) as f32 * 0.02).max(8.0);
        match phase {
            Started => {
                if t.stick.is_none() && pos[0] < size[0] as f32 * 0.4 { t.stick = Some((id, pos, pos)); }
                else { t.drags.push((id, pos, pos, web_time::Instant::now(), false)); }
            }
            Moved => {
                if let Some(s) = t.stick.as_mut().filter(|s| s.0 == id) { s.2 = pos; }
                if let Some(d) = t.drags.iter_mut().find(|d| d.0 == id) {
                    t.look[0] += pos[0] - d.1[0];
                    t.look[1] += pos[1] - d.1[1];
                    d.1 = pos;
                    d.4 |= (pos[0] - d.2[0]).hypot(pos[1] - d.2[1]) > slop;
                }
            }
            Ended | Cancelled => {
                if t.stick.is_some_and(|s| s.0 == id) { t.stick = None; }
                if let Some(i) = t.drags.iter().position(|d| d.0 == id) {
                    let d = t.drags.remove(i);
                    if phase == Ended && !d.4 && d.3.elapsed().as_secs_f32() < 0.35 {
                        self.pointer.moved |= self.pointer.position != Some(pos);
                        self.pointer.position = Some(pos);
                        self.pointer.pressed = true;
                    }
                }
            }
        }
    }

    /// The touch stick to draw, if a thumb is on it: where it landed, where
    /// the knob is (within reach), and the reach, in physical pixels.
    pub fn touch_stick(&self, size: [u32; 2]) -> Option<([f32; 2], [f32; 2], f32)> {
        let (_, from, to) = self.touches.stick?;
        let r = touch_stick_radius(size[0].min(size[1]) as f32);
        let d = [to[0] - from[0], to[1] - from[1]];
        let k = (d[0].hypot(d[1]) / r).max(1.0);
        Some((from, [from[0] + d[0] / k, from[1] + d[1] / k], r))
    }

    /// The mouse wheel in lines (positive up). Also while the mouse is
    /// captured for looking: flying games set their speed with it.
    pub fn wheel(&mut self, lines: f32) {
        self.pointer.wheel += lines;
    }

    pub fn cursor(&mut self, position: Option<[f32; 2]>) {
        self.pointer.moved |= self.pointer.position != position;
        self.pointer.position = position;
    }

    pub fn pointer(&mut self) -> Pointer {
        let p = self.pointer.take();
        if !(self.keyboard_enabled || self.pointer_enabled) { return Pointer::default(); }
        // A captured mouse has no cursor, but its wheel still counts.
        if self.mouse_captured { Pointer { wheel: p.wheel, ..Pointer::default() } } else { p }
    }

    pub(crate) fn reset_keyboard(&mut self) {
        self.kb.keys.clear();
        self.kb.pressed=0;
    }

    /// Capture transitions must not carry a menu click or old look delta into play.
    pub fn reset_mouse(&mut self) {
        self.kb.mouse = 0;
        self.kb.pressed &= !(Buttons::LEFT_TRIGGER | Buttons::RIGHT_TRIGGER);
        self.kb.look = [0.0; 2];
        self.pointer.pressed = false;
        self.pointer.held = false;
        self.pointer.right_pressed = false;
        self.pointer.right_held = false;
        self.pointer.middle_pressed = false;
        self.pointer.middle_held = false;
        self.pointer.moved = false;
        self.pointer.wheel = 0.0;
    }

    /// Drain pad events (edges) once per frame.
    #[cfg(target_os = "android")]
    pub fn poll(&mut self) {}
    /// Read the browser's pads once per frame: new pads join in the order
    /// they appear, presses collect until the next tick.
    #[cfg(target_arch = "wasm32")]
    pub fn poll(&mut self) {
        let readings = crate::web_pads::read();
        self.web_pads.retain(|p| readings.iter().any(|r| r.index == p.0));
        for r in &readings {
            match self.web_pads.iter_mut().find(|p| p.0 == r.index) {
                Some(p) => { p.2 |= r.held & !p.1; p.1 = r.held; }
                None => self.web_pads.push((r.index, r.held, r.held)),
            }
        }
        self.web_readings = readings;
    }
    #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
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
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
        if let Some(g) = &self.gilrs { v.extend(self.pads.iter().filter(|p| g.gamepad(p.0).is_connected()).map(|p| Device::Pad(usize::from(p.0) as u32))); }
        #[cfg(target_arch = "wasm32")]
        v.extend(self.web_pads.iter().map(|p| Device::Pad(p.0)));
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
            // Touch: the stick adds to WASD, drags to the mouse. A drag
            // across the screen's height turns the view 1.6 radians.
            let touch = &mut self.touches;
            if let Some((_, from, to)) = touch.stick {
                let r = touch_stick_radius(touch.short);
                let d = [(to[0] - from[0]) / r, (from[1] - to[1]) / r];
                mv = [(mv[0] + d[0]).clamp(-1.0, 1.0), (mv[1] + d[1]).clamp(-1.0, 1.0)];
                let len = (mv[0] * mv[0] + mv[1] * mv[1]).sqrt();
                if len > 1.0 { mv = [mv[0] / len, mv[1] / len]; }
            }
            let rate = 1.6 / touch.height.max(1.0);
            let drag = std::mem::take(&mut touch.look);
            devices.push((Device::Keyboard, Controls {
                mv,
                look: [k.look[0] as f32 * t.mouse + drag[0] * rate, -k.look[1] as f32 * t.mouse - drag[1] * rate],
                aim: [0.0; 2],
                triggers: [f32::from(held & Buttons::LEFT_TRIGGER != 0), f32::from(held & Buttons::RIGHT_TRIGGER != 0)],
                held: Buttons(held),
                pressed: Buttons(std::mem::take(&mut k.pressed)),
            }));
            k.look = [0.0; 2];
        }
        #[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
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
        #[cfg(target_arch = "wasm32")]
        for (index, _, pressed) in &mut self.web_pads {
            let Some(r) = self.web_readings.iter().find(|r| r.index == *index) else { continue };
            let aim = [shape(r.sticks[2], &t), shape(r.sticks[3], &t)];
            devices.push((Device::Pad(*index), Controls {
                mv: [shape(r.sticks[0], &t), shape(r.sticks[1], &t)],
                look: [aim[0] * t.stick * dt, aim[1] * t.stick * dt],
                aim,
                triggers: r.triggers.map(|v| v.clamp(0.0, 1.0)),
                held: Buttons(r.held),
                pressed: Buttons(std::mem::take(pressed)),
            }));
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
            pointer: Pointer::default(), kb: Keyboard::default(), touches: Touches::default(), gilrs: None, pads: vec![], rumble: Default::default() };
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

#[cfg(test)]
mod touch_tests {
    use super::*;
    use winit::event::TouchPhase::*;

    #[test]
    fn stick_drag_and_tap() {
        let size = [1000, 500];
        let mut d = Devices::new(true);
        // Left thumb pushed up by its full reach: walk forward.
        let r = touch_stick_radius(500.0);
        d.touch(1, Started, [100.0, 400.0], size);
        d.touch(1, Moved, [100.0, 400.0 - 2.0 * r], size);
        // Right finger dragged a quarter screen height to the right: turn.
        d.touch(2, Started, [700.0, 250.0], size);
        d.touch(2, Moved, [825.0, 250.0], size);
        let c = d.controls(1, 1.0 / 60.0)[0];
        assert!((c.mv[1] - 1.0).abs() < 1e-5 && c.mv[0].abs() < 1e-5, "{:?}", c.mv);
        assert!((c.look[0] - 0.4).abs() < 1e-4 && c.look[1].abs() < 1e-6, "{:?}", c.look);
        assert!(d.touch_stick(size).is_some());
        d.touch(1, Ended, [100.0, 300.0], size);
        d.touch(2, Ended, [825.0, 250.0], size);
        assert!(d.touch_stick(size).is_none());
        // A drag is not a tap; a still touch is.
        assert!(!d.pointer().pressed);
        d.touch(3, Started, [600.0, 200.0], size);
        d.touch(3, Ended, [602.0, 201.0], size);
        let p = d.pointer();
        assert!(p.pressed && p.position == Some([602.0, 201.0]));
        assert_eq!(d.controls(1, 1.0 / 60.0)[0].mv, [0.0, 0.0]);
    }
}
