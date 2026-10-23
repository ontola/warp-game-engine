//! Local input for the explicit `play` command. Headless tools never open it.

use gilrs::{Axis, Button, GamepadId, Gilrs};
use sideways::race::Control;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use winit::keyboard::KeyCode;

pub struct Input {
    pads: Option<Gilrs>,
    slots: [Option<GamepadId>; 4],
    keys: HashSet<KeyCode>,
    previous: [Buttons; 4],
    key_edges: HashSet<KeyCode>,
    pub native_active_samples: u64,
}

#[derive(Clone, Copy, Default)]
struct Buttons { recover: bool, camera: bool, pause: bool, up: bool, down: bool }

#[derive(Clone, Copy, Default)]
pub struct SeatInput {
    pub control: Control,
    pub look: [f32; 2],
    pub camera_pressed: bool,
    pub pause_pressed: bool,
}

pub fn deadzone(v: f32) -> f32 {
    if !v.is_finite() || v.abs() <= 0.14 { 0.0 } else { v.signum() * ((v.abs() - 0.14) / 0.86).min(1.0) }
}

impl Input {
    pub fn backend() -> &'static str {
        if cfg!(windows) { "gilrs-xinput" } else { "gilrs-platform-default" }
    }

    pub fn new() -> Self {
        let pads = match Gilrs::new() {
            Ok(pads) => Some(pads),
            Err(e) => { eprintln!("controllers unavailable, keyboard remains active: {e}"); None }
        };
        let mut input = Self { pads, slots: [None; 4], keys: HashSet::new(), previous: [Buttons::default(); 4], key_edges: HashSet::new(), native_active_samples: 0 };
        input.poll();
        println!("native controllers ({}): {}", Self::backend(), input.controllers());
        input
    }

    pub fn list() -> Result<usize, String> {
        let mut pads = Gilrs::new().map_err(|e| e.to_string())?;
        while pads.next_event().is_some() {}
        let mut count = 0;
        for (id, pad) in pads.gamepads() {
            println!("controller {}: {} connected={} mapping={:?}", usize::from(id), pad.name(), pad.is_connected(), pad.mapping_source());
            count += 1;
        }
        println!("controllers: {count}; backend={}; keyboard available for player 1", Self::backend());
        Ok(count)
    }

    /// Real native controls through the exact same `seat` path as play. This
    /// creates no window, GPU, keyboard events or synthetic controller input.
    pub fn probe(seconds: u64, state_out: Option<&str>) -> Result<(), String> {
        if !(1..=60).contains(&seconds) { return Err("probe duration must be 1..=60 seconds".into()); }
        let mut input=Self::new();
        if input.pads.is_none() { return Err("native controller backend unavailable".into()); }
        let start=Instant::now();
        let mut last_report=Instant::now();
        let mut previous=serde_json::Value::Null;
        let mut samples=Vec::new();
        println!("controller probe: {seconds}s; press sticks, triggers, A/X/Y/Start and shoulder buttons now");
        while start.elapsed()<Duration::from_secs(seconds) {
            input.poll();
            let seats:Vec<_>=(0..4).map(|seat| {
                let value=input.seat(seat,true);
                let control=value.control;
                serde_json::json!({"seat":seat,"controller_id":input.slots[seat].map(usize::from),
                    "throttle":control.throttle,"brake":control.brake,"steer":control.steer,
                    "handbrake":control.handbrake,"look":value.look,
                    "shift_pressed":control.shift,"recover_pressed":control.recover,
                    "camera_pressed":value.camera_pressed,"pause_pressed":value.pause_pressed})
            }).collect();
            let reading=serde_json::json!({"controllers":input.controllers(),"seats":seats});
            if reading!=previous || last_report.elapsed()>=Duration::from_millis(250) {
                let sample=serde_json::json!({"elapsed_ms":start.elapsed().as_millis(),"reading":reading});
                println!("{sample}");
                samples.push(sample);
                previous=reading;
                last_report=Instant::now();
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        let summary=serde_json::json!({"schema":1,"mode":"native_controller_probe","backend":Self::backend(),
            "elapsed_ms":start.elapsed().as_millis(),"native_active_samples":input.native_active_samples,
            "controllers":input.controllers(),"samples":samples});
        if let Some(path)=state_out {
            std::fs::write(path,serde_json::to_vec_pretty(&summary).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        }
        println!("controller probe finished: {} active native samples; backend={}",input.native_active_samples,Self::backend());
        Ok(())
    }

    pub fn key(&mut self, key: KeyCode, pressed: bool) {
        if pressed {
            if self.keys.insert(key) { self.key_edges.insert(key); }
        } else { self.keys.remove(&key); }
    }

    pub fn clear_keys(&mut self) { self.keys.clear(); self.key_edges.clear(); }

    pub fn take_key(&mut self, key: KeyCode) -> bool { self.key_edges.remove(&key) }

    pub fn controllers(&self) -> serde_json::Value {
        serde_json::json!(self.pads.as_ref().map(|pads|pads.gamepads().map(|(id,pad)|serde_json::json!({"id":usize::from(id),"name":pad.name(),"connected":pad.is_connected(),"mapping":format!("{:?}",pad.mapping_source())})).collect::<Vec<_>>()).unwrap_or_default())
    }

    pub fn poll(&mut self) {
        let Some(pads) = &mut self.pads else { return; };
        while pads.next_event().is_some() {}
        for slot in &mut self.slots {
            if slot.is_some_and(|id| !pads.gamepad(id).is_connected()) { *slot = None; }
        }
        for (id, _) in pads.gamepads() {
            if self.slots.contains(&Some(id)) { continue; }
            if let Some(slot) = self.slots.iter_mut().find(|s| s.is_none()) { *slot = Some(id); }
        }
    }

    pub fn seat(&mut self, seat: usize, focused: bool) -> SeatInput {
        if seat >= 4 { return SeatInput::default(); }
        let mut out = SeatInput::default();
        let mut now = Buttons::default();
        if let (Some(pads), Some(id)) = (&self.pads, self.slots[seat]) {
            let pad = pads.gamepad(id);
            if pad.is_connected() {
                out.control.steer = deadzone(pad.value(Axis::LeftStickX)) as f64;
                let trigger = |b| pad.button_data(b).map(|v| v.value()).unwrap_or(0.0).clamp(0.0, 1.0) as f64;
                out.control.throttle = trigger(Button::RightTrigger2);
                out.control.brake = trigger(Button::LeftTrigger2);
                out.control.handbrake = if pad.is_pressed(Button::South) { 1.0 } else { 0.0 };
                out.look = [deadzone(pad.value(Axis::RightStickX)), deadzone(pad.value(Axis::RightStickY))];
                now = Buttons { recover: pad.is_pressed(Button::West), camera: pad.is_pressed(Button::North), pause: pad.is_pressed(Button::Start), up: pad.is_pressed(Button::RightTrigger), down: pad.is_pressed(Button::LeftTrigger) };
                if focused && (out.control.throttle>0.05 || out.control.brake>0.05 || out.control.steer.abs()>0.05 || out.control.handbrake>0.0 || out.look.iter().any(|x|x.abs()>0.05) || now.recover || now.camera || now.pause || now.up || now.down) { self.native_active_samples += 1; }
            }
        }
        if seat == 0 {
            let held = |a, b| self.keys.contains(&a) || self.keys.contains(&b);
            out.control.throttle = out.control.throttle.max(if held(KeyCode::KeyW, KeyCode::ArrowUp) { 1.0 } else { 0.0 });
            out.control.brake = out.control.brake.max(if held(KeyCode::KeyS, KeyCode::ArrowDown) { 1.0 } else { 0.0 });
            let steer = i32::from(held(KeyCode::KeyD, KeyCode::ArrowRight)) - i32::from(held(KeyCode::KeyA, KeyCode::ArrowLeft));
            if steer != 0 { out.control.steer = steer as f64; }
            if self.keys.contains(&KeyCode::Space) { out.control.handbrake = 1.0; }
            now.recover |= self.keys.contains(&KeyCode::KeyR);
            now.camera |= self.keys.contains(&KeyCode::KeyC);
            now.pause |= self.keys.contains(&KeyCode::Escape);
            now.up |= self.keys.contains(&KeyCode::KeyE);
            now.down |= self.keys.contains(&KeyCode::KeyQ);
        }
        let old = self.previous[seat];
        out.control.recover = now.recover && !old.recover;
        out.control.shift = i8::from(now.up && !old.up) - i8::from(now.down && !old.down);
        out.camera_pressed = now.camera && !old.camera;
        out.pause_pressed = now.pause && !old.pause;
        self.previous[seat] = now;
        if focused { out } else { SeatInput::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stick_deadzone_never_returns_nonfinite_or_out_of_range() {
        for input in [f32::NAN, f32::INFINITY, -2.0, -1.0, -0.14, 0.0, 0.14, 1.0, 2.0] {
            let output = deadzone(input);
            assert!(output.is_finite() && (-1.0..=1.0).contains(&output));
        }
        assert_eq!(deadzone(0.14), 0.0);
        assert_eq!(deadzone(-1.0), -1.0);
    }
}
