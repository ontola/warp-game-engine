//! A catalog entry can host a same-origin HTML game beside the Warp canvas.
use super::page;
use app::{Buttons, Config, Controls, Draw, Frame, Game};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/examples/browser/html-game.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = catalogJSON)]
    pub(super) async fn catalog_json() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch)]
    fn mount(entry: &str, title: &str, description: &str) -> Result<(), JsValue>;
    fn active(session: &str) -> bool;
    fn input(session: &str, frames: &str);
    pub(super) fn pause() -> bool;
    fn resume(session: &str) -> bool;
    fn dispose(session: &str);
}

pub(super) struct HtmlGame {
    entry: String,
    title: String,
    players: usize,
    session: String,
    controllers: Vec<String>,
    mounting: bool,
}

impl HtmlGame {
    pub(super) fn load(entry: String, title: String, players: usize) -> (Config, Box<dyn Game>) {
        let config = Config { title: title.clone(), ..Config::default() };
        (config, Box::new(Self { entry, title, players, session: String::new(), controllers: vec![], mounting: true }))
    }

    fn show(&mut self) {
        let (party, mut prepare) = page::with(|host| {
            let party = &host.party;
            (party.clone(), json!({"type":"prepare", "game":party["active_session"]["game"],
                "session":party["active_session"]["id"], "players":party["players"], "seats":party["seats"]}))
        });
        let Some(session) = prepare["session"].as_str() else { return };
        self.session = session.to_string();
        if let Some(seats) = prepare["seats"].as_array_mut() {
            self.controllers = seats.iter_mut().enumerate().map(|(i, s)| {
                let token = s["controller"].as_str().map(String::from).unwrap_or_else(|| format!("web:seat:{i}"));
                s["controller"] = json!(token);
                token
            }).collect();
        }
        let description = json!({"party":party,"prepare":prepare}).to_string();
        if let Err(e) = mount(&self.entry, &self.title, &description) {
            app::web::log(&format!("Could not open {}: {e:?}", self.entry));
            app::web::spawn(async { page::back_to_room(); });
        }
        self.mounting = false;
    }
}

impl Game for HtmlGame {
    fn local_players(&self) -> usize { self.players }
    fn tick(&mut self, controls: &[Controls]) {
        if self.mounting { self.show(); }
        if !active(&self.session) { return; }
        let clamp = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        let frames: Vec<Value> = self.controllers.iter().zip(controls).map(|(controller, c)| {
            let order = [Buttons::SOUTH, Buttons::EAST, Buttons::WEST, Buttons::NORTH,
                Buttons::LEFT_SHOULDER, Buttons::RIGHT_SHOULDER, Buttons::SELECT, Buttons::START,
                Buttons::LEFT_STICK, Buttons::RIGHT_STICK, Buttons::UP, Buttons::DOWN, Buttons::LEFT, Buttons::RIGHT];
            let buttons = order.iter().enumerate().fold(0u32, |bits, (i, b)| bits | (u32::from(c.held.has(*b)) << i));
            let axes = [c.mv[0], -c.mv[1], c.aim[0], -c.aim[1], c.triggers[0], c.triggers[1]].map(clamp);
            json!({"controller":controller,"buttons":buttons,"axes":axes})
        }).collect();
        input(&self.session, &json!(frames).to_string());
    }
    fn mouse_capture(&self) -> Option<bool> { Some(false) }
    fn draw(&mut self, frame: &mut Frame) -> Result<Draw, String> {
        Ok(Draw { cameras: Vec::new(), light: Default::default(), hud: render::hud::HudDrawList::new(frame.size) })
    }
}

pub(super) fn show_again(session: &str) -> bool { resume(session) }
impl Drop for HtmlGame {
    fn drop(&mut self) { if !self.session.is_empty() { dispose(&self.session); } }
}
