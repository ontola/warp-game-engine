//! Gamepads in a browser, through the Gamepad API (`navigator.getGamepads`).
//! Pads in the "standard" layout (every Xbox, PlayStation and Switch Pro
//! controller in Chrome, Edge and Firefox) map onto [`Buttons`]; others
//! (8BitDo pads over Bluetooth on macOS) through [`layout`]. A browser
//! shows a pad to the page only after one of its buttons is pressed.
use crate::input::Buttons;
use wasm_bindgen::JsCast;

/// One pad as read this frame: its slot in `getGamepads()`, the held
/// buttons, the sticks (left x, y, right x, y; up is +y) and the triggers.
pub struct Reading { pub index: u32, pub held: u32, pub sticks: [f32; 4], pub triggers: [f32; 2] }

/// Standard-layout button `i` as [`Buttons`].
const BUTTONS: [u32; 16] = [
    Buttons::SOUTH, Buttons::EAST, Buttons::WEST, Buttons::NORTH,
    Buttons::LEFT_SHOULDER, Buttons::RIGHT_SHOULDER, Buttons::LEFT_TRIGGER, Buttons::RIGHT_TRIGGER,
    Buttons::SELECT, Buttons::START, Buttons::LEFT_STICK, Buttons::RIGHT_STICK,
    Buttons::UP, Buttons::DOWN, Buttons::LEFT, Buttons::RIGHT,
];

fn pads() -> Vec<web_sys::Gamepad> {
    let Some(nav) = web_sys::window().map(|w| w.navigator()) else { return Vec::new() };
    let Ok(list) = nav.get_gamepads() else { return Vec::new() };
    list.iter().filter_map(|p| p.dyn_into::<web_sys::Gamepad>().ok()).filter(|p| p.connected()).collect()
}

/// Where a pad without the browser's standard mapping keeps its controls:
/// the browser button for each of [`BUTTONS`]' first twelve (None: absent),
/// the stick axes (left x, y, right x, y), and the axis that holds the
/// d-pad as a hat, if any.
struct Layout { buttons: [Option<u32>; 12], sticks: [u32; 4], hat: Option<u32> }

/// Chrome on macOS shows some pads as plain HID devices, `mapping` "":
/// 8BitDo pads over Bluetooth (D-input) have gaps in their button numbers
/// and their d-pad on a hat axis. Others get the standard order.
fn layout(pad: &web_sys::Gamepad) -> Layout {
    let id = pad.id().to_ascii_lowercase();
    let axes = pad.axes().length();
    if id.contains("2dc8") || id.contains("8bitdo") {
        // A B _ X Y _ LB RB LT RT Select Start Home L3 R3.
        return Layout { buttons: [Some(0), Some(1), Some(3), Some(4), Some(6), Some(7), Some(8), Some(9), Some(10), Some(11), Some(13), Some(14)],
            sticks: [0, 1, 2, 5], hat: (axes > 9).then_some(9) };
    }
    Layout { buttons: std::array::from_fn(|i| Some(i as u32)), sticks: [0, 1, 2, 3], hat: (axes > 9).then_some(9) }
}

/// A hat axis as d-pad buttons: Chrome reports -1 (up) to 1 (up-left) in
/// eight steps of 2/7 going clockwise, and about 1.29 when centred.
fn hat(value: f64) -> u32 {
    if !(-1.05..=1.05).contains(&value) { return 0; }
    let step = (((value + 1.0) * 3.5).round() as i32).rem_euclid(8);
    let mut held = 0;
    if matches!(step, 7 | 0 | 1) { held |= Buttons::UP; }
    if matches!(step, 1..=3) { held |= Buttons::RIGHT; }
    if matches!(step, 3..=5) { held |= Buttons::DOWN; }
    if matches!(step, 5..=7) { held |= Buttons::LEFT; }
    held
}

/// Every connected pad, now: standard ones as the browser maps them,
/// others through [`layout`].
pub fn read() -> Vec<Reading> {
    pads().into_iter().map(|p| {
        let buttons = p.buttons();
        let button = |i: u32| buttons.get(i).dyn_into::<web_sys::GamepadButton>().ok();
        let down = |i: u32| button(i).is_some_and(|b| b.pressed() || b.value() > 0.4);
        let value = |i: u32| button(i).map_or(0.0, |b| b.value() as f32);
        let axes = p.axes();
        let axis = |i: u32| axes.get(i).as_f64().unwrap_or(0.0) as f32;
        if p.mapping() == web_sys::GamepadMappingType::Standard {
            let mut held = 0;
            for (i, b) in BUTTONS.iter().enumerate() { if down(i as u32) { held |= b; } }
            return Reading { index: p.index(), held, sticks: [axis(0), -axis(1), axis(2), -axis(3)], triggers: [value(6), value(7)] };
        }
        let l = layout(&p);
        let mut held = 0;
        for (k, b) in l.buttons.iter().enumerate() { if b.is_some_and(down) { held |= BUTTONS[k]; } }
        if let Some(h) = l.hat { held |= hat(axes.get(h).as_f64().unwrap_or(9.0)); }
        let trigger = |k: usize| l.buttons[k].map_or(0.0, |b| if down(b) { value(b).max(1.0) } else { 0.0 });
        let [lx, ly, rx, ry] = l.sticks.map(axis);
        Reading { index: p.index(), held, sticks: [lx, -ly, rx, -ry], triggers: [trigger(6), trigger(7)] }
    }).collect()
}

/// Vibrate pad `index` (Chrome and Edge: `vibrationActuator`, "dual-rumble").
/// Returns false when the browser or the pad cannot.
pub fn rumble(index: u32, weak: f32, strong: f32, seconds: f32) -> bool {
    let Some(pad) = pads().into_iter().find(|p| p.index() == index) else { return false };
    let Ok(actuator) = js_sys::Reflect::get(&pad, &"vibrationActuator".into()) else { return false };
    let Ok(play) = js_sys::Reflect::get(&actuator, &"playEffect".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) else { return false };
    let params = js_sys::Object::new();
    let set = |k: &str, v: f64| { let _ = js_sys::Reflect::set(&params, &k.into(), &v.into()); };
    set("duration", (seconds.clamp(0.0, 5.0) * 1000.0) as f64);
    set("weakMagnitude", weak.clamp(0.0, 1.0) as f64);
    set("strongMagnitude", strong.clamp(0.0, 1.0) as f64);
    play.call2(&actuator, &"dual-rumble".into(), &params).is_ok()
}
