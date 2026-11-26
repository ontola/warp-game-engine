//! Online couches in the browser host (engine/docs/online.md): every Luau
//! game this page starts is open to other couches at once. The way in is
//! the room's own code and QR code: on a computer that can host (WebGPU,
//! a mouse), the join link (`/?r=CODE`, `/join?room=CODE`) opens
//! `/play/?follow=CODE`, and that page follows whatever game this one
//! plays, and the next one too. A phone still joins as a controller.
//!
//! The page holds one WebRTC link for its whole life, signalled in room
//! `club-CODE` (gamenight-cloud signal.rs), and hands each game a side of
//! it. Page messages (`Down::Lobby`, JSON): `{"game": FOLDER}` when the
//! host's game changes, sent again to a couch that connects later.

use serde_json::{json, Value};
use std::cell::RefCell;
use warp::online::{Down, FollowLink, HostLink, Shared};
use wasm_bindgen::{closure::Closure, JsCast};

enum Page {
    Off,
    /// This page hosts: its link, the game on offer and the couches told about it.
    Host { link: Shared<dyn HostLink>, game: Option<String>, told: Vec<u64> },
    /// This page follows a host's room: the game it was told to play.
    Follow { link: Shared<dyn FollowLink>, code: String, game: Option<String>, gone: bool },
}

thread_local! {
    static PAGE: RefCell<Page> = const { RefCell::new(Page::Off) };
    /// The Game Room's online switch: Public (the default) or Friends only.
    static PUBLIC: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

/// The online switch was thrown. Friends only and Public both take anyone
/// with the code or QR; Public will also list the room for strangers (the
/// "play with strangers" button, not built yet).
pub(crate) fn set_public(public: bool) {
    PUBLIC.with(|p| p.set(public));
    app::web::log(&format!("online: the room is {}", if public { "public" } else { "friends only" }));
}

#[allow(dead_code)]
pub(crate) fn public() -> bool { PUBLIC.with(|p| p.get()) }

fn signal_room(code: &str) -> String { format!("club-{}", code.to_ascii_uppercase()) }

fn off() -> bool { warp::web::param("online").as_deref() == Some("off") }

/// At the page's start: `?follow=CODE` follows that room (its games); this
/// page hosts once its first Luau game loads. `?online=off` keeps it local.
pub(crate) fn start() {
    if off() { return; }
    if let Some(code) = warp::web::param("follow") {
        match warp::online_web::Client::join(&signal_room(&code)) {
            Ok(c) => {
                let link: Shared<dyn FollowLink> = Shared::new(Box::new(c));
                PAGE.with(|p| *p.borrow_mut() = Page::Follow { link, code: code.clone(), game: None, gone: false });
                app::web::log(&format!("online: following room {code}"));
                // The switch back (Joep: by device, with a switch both ways).
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    if let Some(a) = doc.get_element_by_id("as-controller") { let _ = a.set_attribute("href", &format!("/app/?room={}", code.to_ascii_uppercase())); }
                    if let Some(root) = doc.document_element() { let _ = root.set_attribute("data-following", ""); }
                }
            }
            Err(e) => app::web::log(&format!("online: could not follow {code}: {e}")),
        }
    }
    // Four times a second, also while a game has the screen (the room's
    // tick is parked then).
    let tick = Closure::<dyn FnMut()>::new(tick);
    if let Some(w) = web_sys::window() {
        let _ = w.set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), 250);
    }
    tick.forget();
}

/// This page follows another couch's games: its own Play goes nowhere.
pub(crate) fn following() -> bool { PAGE.with(|p| matches!(*p.borrow(), Page::Follow { .. })) }

/// A Luau game loaded: open it to other couches, or follow the host's.
pub(crate) fn loaded(folder: &str, code: &str, cfg: &mut app::Config, game: &mut warp::LuaGame) {
    PAGE.with(|p| {
        let mut p = p.borrow_mut();
        if matches!(*p, Page::Off) && !off() && !code.is_empty() {
            match warp::online_web::Host::start(&signal_room(code)) {
                Ok(h) => *p = Page::Host { link: Shared::new(Box::new(h)), game: None, told: Vec::new() },
                Err(e) => { app::web::log(&format!("online: could not open the game to others: {e}")); return; }
            }
        }
        match &mut *p {
            Page::Off => {}
            Page::Host { link, game: on, told } => {
                game.host_with(link.host_side());
                *on = Some(folder.to_string());
                told.clear();
                app::web::log(&format!("online: {folder} is open to other couches (room {code})"));
            }
            Page::Follow { link, game: on, .. } => {
                if on.as_deref() != Some(folder) { return; }
                game.follow_with(link.follow_side(), cfg.size);
                cfg.tick_hz = game.cfg.tick_rate / warp::online::send_every(game.cfg.tick_rate) as f32;
            }
        }
    });
}

/// A game this page started that cannot go online (an HTML or Rust game).
pub(crate) fn local_game() {
    PAGE.with(|p| if let Page::Host { link, game, told } = &mut *p.borrow_mut() { link.set_in_game(false); *game = None; told.clear(); });
}

/// The game went off screen (Back: paused) or came back (Resume).
pub(crate) fn on_screen(on: bool) {
    PAGE.with(|p| match &*p.borrow() {
        Page::Host { link, game: Some(_), .. } => link.set_in_game(on),
        Page::Follow { link, game: Some(_), .. } => link.set_in_game(on),
        _ => {}
    });
}

fn tick() {
    let mut start = None;
    PAGE.with(|p| match &mut *p.borrow_mut() {
        Page::Off => {}
        Page::Host { link, game, told } => {
            let _ = link.poll_lobby();
            let clients = link.state().clients.clone();
            told.retain(|c| clients.contains(c));
            let Some(g) = game.clone() else { return };
            let new: Vec<u64> = clients.into_iter().filter(|c| !told.contains(c)).collect();
            for c in new {
                link.send(c, Down::Lobby(json!({"game": g}).to_string()));
                told.push(c);
            }
        }
        Page::Follow { link, code, game, gone } => {
            for (_, text) in link.poll_lobby() {
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(g) = v["game"].as_str() {
                    if game.as_deref() != Some(g) { *game = Some(g.to_string()); start = Some(g.to_string()); }
                }
            }
            if link.closed() && !*gone {
                *gone = true;
                app::web::log(&format!("online: room {code} is not playing online"));
                // Nobody plays online under that code (a desktop host, or
                // it ended before a game): join it as a controller instead.
                if game.is_none() {
                    if let Some(w) = web_sys::window() { let _ = w.location().replace(&format!("/app/?room={}", code.to_ascii_uppercase())); }
                }
            }
        }
    });
    if let Some(g) = start {
        app::web::log(&format!("online: the host plays {g}"));
        super::page::start(g);
    }
}
