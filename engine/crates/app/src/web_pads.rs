//! Gamepads in a browser, through the Gamepad API (`navigator.getGamepads`).
//! Pads in the "standard" layout (every Xbox, PlayStation and Switch Pro
//! controller in Chrome, Edge and Firefox) map onto [`Buttons`]; a browser
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
    list.iter().filter_map(|p| p.dyn_into::<web_sys::Gamepad>().ok()).filter(|p| p.connected() && p.mapping() == web_sys::GamepadMappingType::Standard).collect()
}

/// Every connected standard pad, now.
pub fn read() -> Vec<Reading> {
    pads().into_iter().map(|p| {
        let buttons = p.buttons();
        let button = |i: u32| buttons.get(i).dyn_into::<web_sys::GamepadButton>().ok();
        let mut held = 0;
        for (i, b) in BUTTONS.iter().enumerate() {
            if let Some(btn) = button(i as u32) && (btn.pressed() || btn.value() > 0.4) { held |= b; }
        }
        let axes = p.axes();
        let axis = |i: u32| axes.get(i).as_f64().unwrap_or(0.0) as f32;
        let trigger = |i: u32| button(i).map_or(0.0, |b| b.value() as f32);
        Reading { index: p.index(), held, sticks: [axis(0), -axis(1), axis(2), -axis(3)], triggers: [trigger(6), trigger(7)] }
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
