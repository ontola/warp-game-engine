//! Gamepads on Android, from the activity's input events: key events for
//! the buttons, joystick motion events for the sticks, triggers and hat.
//! The input thread ([`crate::android`]) feeds every event to [`key`] and
//! [`motion`]; the render thread reads the table once per frame ([`read`]),
//! as the browser shell reads `navigator.getGamepads()`. Android numbers
//! the axes the same for every pad it knows (Xbox, PlayStation, 8BitDo,
//! Switch Pro): left stick X/Y, right stick Z/RZ, triggers LTRIGGER and
//! RTRIGGER (or BRAKE and GAS), d-pad as HAT_X/HAT_Y or as d-pad keys.
use crate::input::Buttons;
use std::sync::Mutex;

/// One pad as read this frame: its Android device id, the held buttons,
/// the sticks (left x, y, right x, y; up is +y) and the triggers.
pub struct Reading { pub index: u32, pub held: u32, pub sticks: [f32; 4], pub triggers: [f32; 2] }

#[derive(Clone, Default)]
struct Pad {
    id: i32,
    /// Buttons held through key events.
    keys: u32,
    /// The d-pad from the hat axes.
    hat: u32,
    sticks: [f32; 4],
    triggers: [f32; 2],
}

/// Pads in the order they first sent something.
static PADS: Mutex<Vec<Pad>> = Mutex::new(Vec::new());

fn with_pad(id: i32, f: impl FnOnce(&mut Pad)) {
    let mut pads = PADS.lock().unwrap_or_else(|e| e.into_inner());
    let at = match pads.iter().position(|p| p.id == id) {
        Some(i) => i,
        None => { pads.push(Pad { id, ..Default::default() }); pads.len() - 1 }
    };
    f(&mut pads[at]);
}

/// Android key code `code` as [`Buttons`] (0: not a pad button). Some pads
/// send BACK for their Select/View button, and the d-pad as key events.
pub(crate) fn button(code: u32) -> u32 {
    match code {
        96 => Buttons::SOUTH,          // BUTTON_A
        97 => Buttons::EAST,           // BUTTON_B
        99 => Buttons::WEST,           // BUTTON_X
        100 => Buttons::NORTH,         // BUTTON_Y
        102 => Buttons::LEFT_SHOULDER, // BUTTON_L1
        103 => Buttons::RIGHT_SHOULDER,
        104 => Buttons::LEFT_TRIGGER,  // BUTTON_L2
        105 => Buttons::RIGHT_TRIGGER,
        106 => Buttons::LEFT_STICK,    // BUTTON_THUMBL
        107 => Buttons::RIGHT_STICK,
        108 => Buttons::START,         // BUTTON_START
        109 | 4 => Buttons::SELECT,    // BUTTON_SELECT, BACK
        19 => Buttons::UP,             // DPAD_UP
        20 => Buttons::DOWN,
        21 => Buttons::LEFT,
        22 => Buttons::RIGHT,
        _ => 0,
    }
}

/// A pad's key went down or up. Returns whether it was a pad button.
pub(crate) fn key(id: i32, code: u32, down: bool) -> bool {
    let b = button(code);
    if b == 0 { return false; }
    with_pad(id, |p| if down { p.keys |= b } else { p.keys &= !b });
    true
}

/// A pad's joystick motion: the axes as Android reports them (y down is +).
pub(crate) fn motion(id: i32, axes: Axes) {
    with_pad(id, |p| {
        p.sticks = [axes.x, -axes.y, axes.z, -axes.rz];
        p.triggers = [axes.ltrigger.max(axes.brake), axes.rtrigger.max(axes.gas)];
        let mut hat = 0;
        if axes.hat_y < -0.5 { hat |= Buttons::UP; }
        if axes.hat_y > 0.5 { hat |= Buttons::DOWN; }
        if axes.hat_x < -0.5 { hat |= Buttons::LEFT; }
        if axes.hat_x > 0.5 { hat |= Buttons::RIGHT; }
        p.hat = hat;
    });
}

/// The axes of one joystick motion event.
#[derive(Clone, Copy, Default)]
pub(crate) struct Axes { pub x: f32, pub y: f32, pub z: f32, pub rz: f32, pub hat_x: f32, pub hat_y: f32, pub ltrigger: f32, pub rtrigger: f32, pub brake: f32, pub gas: f32 }

/// Forget pads Android no longer lists (`connected`: the device ids it has).
pub(crate) fn keep_only(connected: &[i32]) {
    let mut pads = PADS.lock().unwrap_or_else(|e| e.into_inner());
    pads.retain(|p| connected.contains(&p.id));
}

/// Every pad that has sent something and is still connected, now.
pub fn read() -> Vec<Reading> {
    let pads = PADS.lock().unwrap_or_else(|e| e.into_inner());
    pads.iter().map(|p| {
        let mut held = p.keys | p.hat;
        // Analog triggers on pads that send no L2/R2 keys.
        if p.triggers[0] > 0.4 { held |= Buttons::LEFT_TRIGGER; }
        if p.triggers[1] > 0.4 { held |= Buttons::RIGHT_TRIGGER; }
        let digital = |b: u32, v: f32| if p.keys & b != 0 { v.max(1.0) } else { v };
        Reading { index: p.id as u32, held, sticks: p.sticks, triggers: [digital(Buttons::LEFT_TRIGGER, p.triggers[0]), digital(Buttons::RIGHT_TRIGGER, p.triggers[1])] }
    }).collect()
}
