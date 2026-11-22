//! The page host's line to the Splitscreen Club cloud room, as the native
//! host's cloud bridge (gamenight `crates/gamenight-local-web/src/cloud.rs`)
//! does it: register a room once (`/v1/lobbies/register`, by a device id the
//! page keeps), then poll every few seconds (`/v1/lobbies/poll`) with this
//! table's seats and games. The reply carries the room code (shown with its
//! QR code in the room), the profiles waiting at the doors, the profiles on
//! seats, and a game a phone picked. A door pickup in the room claims a
//! waiting profile for a seat (`/v1/lobbies/pickup`). The page must come
//! from the cloud's own origin (it sends no CORS headers), or name it with
//! `?cloud=`; `?cloud=off` plays without.

use crate::page::{game_meta, queue, with, Host};
use serde_json::{json, Value};

/// Polls this often, and at once after a pickup or a change of game.
const POLL_MS: u32 = 3000;

#[derive(Default)]
pub(crate) struct Cloud {
    origin: String,
    token: Option<String>,
    room_code: String,
    /// Each seat: its index, player id (stable for the page) and revision.
    seats: Vec<Value>,
    /// The last phone selection this host took.
    acknowledged: Option<String>,
    /// Poll now instead of waiting.
    pub(crate) refresh: bool,
    /// Seats a profile holds, with its account and profile revision.
    applied: Vec<Option<(String, u64)>>,
}

impl Cloud {
    /// A seat for a player who joined with a pad.
    pub(crate) fn add_seat(&mut self) {
        self.seats.push(json!({"index": self.seats.len(), "player": random_hex(16), "revision": 1}));
        self.applied.push(None);
        self.refresh = true;
    }

    /// The last seat's player left the table.
    pub(crate) fn drop_seat(&mut self) {
        self.seats.pop();
        self.applied.pop();
        self.refresh = true;
    }

    /// Seat `index` has someone else now: a profile on it goes back to the doors.
    pub(crate) fn free_seat(&mut self, index: usize) {
        let Some(seat) = self.seats.get_mut(index) else { return };
        seat["player"] = json!(random_hex(16));
        seat["revision"] = json!(seat["revision"].as_u64().unwrap_or(1) + 1);
        self.refresh = true;
    }
}

/// Start the room's poll loop for `players` seats.
pub(crate) fn start(players: usize) {
    let origin = match warp::web::param("cloud") {
        Some(o) if o == "off" => return,
        Some(o) => o.trim_end_matches('/').to_string(),
        None => app::web::origin(),
    };
    with(|h| {
        h.cloud.origin = origin;
        h.cloud.seats = (0..players).map(|i| json!({"index": i, "player": random_hex(16), "revision": 1})).collect();
        h.cloud.applied = vec![None; players];
    });
    wasm_bindgen_futures::spawn_local(async {
        let mut failures = 0u32;
        loop {
            match poll().await {
                Ok(()) => failures = 0,
                Err(e) => {
                    failures += 1;
                    // Once, then now and then: a page without a cloud plays on.
                    if failures == 1 || failures % 20 == 0 { app::web::log(&format!("cloud room: {e}")); }
                }
            }
            let wait = if failures > 3 { POLL_MS * 10 } else { POLL_MS };
            let mut waited = 0;
            while waited < wait && !with(|h| std::mem::take(&mut h.cloud.refresh)) {
                sleep(250).await;
                waited += 250;
            }
        }
    });
}

/// Claim waiting profile `pending` for seat `index` (a door pickup in the room).
pub(crate) fn pickup(pending: String, index: usize) {
    wasm_bindgen_futures::spawn_local(async move {
        let Some((origin, token, seat)) = with(|h| Some((h.cloud.origin.clone(), h.cloud.token.clone()?, h.cloud.seats.get(index)?.clone()))) else { return };
        let body = json!({"pending": pending, "seat": seat}).to_string();
        match app::web::request("POST", &format!("{origin}/v1/lobbies/pickup"), &[("authorization", &format!("Bearer {token}"))], Some(&body)).await {
            Ok((status, _)) if (200..300).contains(&status) => {}
            Ok((status, _)) => app::web::log(&format!("cloud room: pickup refused (HTTP {status})")),
            Err(e) => app::web::log(&format!("cloud room: pickup: {e}")),
        }
        with(|h| h.cloud.refresh = true);
    });
}

async fn poll() -> Result<(), String> {
    let origin = with(|h| h.cloud.origin.clone());
    let token = match with(|h| h.cloud.token.clone()) {
        Some(t) => t,
        None => {
            let device = app::web::stored("splitscreen-club-device").filter(|d| d.len() == 64).unwrap_or_else(|| {
                let d = random_hex(32);
                app::web::store("splitscreen-club-device", &d);
                d
            });
            let (status, body) = app::web::request("POST", &format!("{origin}/v1/lobbies/register"), &[], Some(&json!({"device": device}).to_string())).await?;
            if !(200..300).contains(&status) { return Err(format!("register: HTTP {status}")); }
            let reply: Value = serde_json::from_slice(&body).map_err(|e| format!("register: {e}"))?;
            let token = reply["token"].as_str().ok_or("register: no token")?.to_string();
            app::web::log(&format!("cloud room {} at {origin}", reply["room_code"].as_str().unwrap_or("?")));
            with(|h| { h.cloud.token = Some(token.clone()); h.cloud.acknowledged = None; });
            token
        }
    };
    let body = with(|h| snapshot(h)).to_string();
    let (status, reply) = app::web::request("POST", &format!("{origin}/v1/lobbies/poll"), &[("authorization", &format!("Bearer {token}"))], Some(&body)).await?;
    if status == 401 {
        // The cloud forgot the room (a restart, or it expired): register again.
        with(|h| { h.cloud.token = None; h.cloud.refresh = true; });
        return Ok(());
    }
    if !(200..300).contains(&status) { return Err(format!("poll: HTTP {status}")); }
    let reply: Value = serde_json::from_slice(&reply).map_err(|e| format!("poll: {e}"))?;
    if let Some((game, now)) = with(|h| apply(h, &reply)) {
        if now { crate::page::start(game); }
    }
    Ok(())
}

/// What the cloud needs of this table: seats and the games it can play.
fn snapshot(h: &Host) -> Value {
    let party = &h.party;
    let people = h.cloud.seats.len() as u64;
    let session = party["active_session"].as_object().map(|s| s["game"].clone()).unwrap_or(Value::Null);
    let games: Vec<Value> = party["library"].as_array().into_iter().flatten().map(|g| {
        let fits = g["min_players"].as_u64().unwrap_or(1) <= people && people <= g["max_players"].as_u64().unwrap_or(8);
        let state = if session == g["id"] { "playing" } else if h.loading.as_deref() == g["id"].as_str() { "loading" } else if fits { "available" } else { "unavailable" };
        json!({"id": g["id"], "selectable": fits, "state": state})
    }).collect();
    let current = if session.is_null() { h.loading.clone().map(Value::from).unwrap_or(Value::Null) } else { session.clone() };
    let next = party["playlist"]["entries"].get(0).map(|e| e["game"].clone()).unwrap_or(Value::Null);
    json!({"seats": h.cloud.seats, "discovery": {"games": games, "playlist": party["playlist"], "session": party["active_session"],
        "current": current, "next": next, "acknowledged": h.cloud.acknowledged}})
}

/// Take a poll reply: the room's links, profiles on seats, a phone's pick.
/// Returns a game to start now.
fn apply(h: &mut Host, reply: &Value) -> Option<(String, bool)> {
    // The room code, its QR code and the profiles waiting at the doors.
    let code = reply["room_code"].as_str().unwrap_or_default().to_string();
    let mut room = h.links["room"].clone();
    if code != h.cloud.room_code && !code.is_empty() {
        h.cloud.room_code = code.clone();
        let url = format!("{}/?r={code}", h.cloud.origin);
        room["room_code"] = json!(code);
        room["qr_svg"] = json!(qr_svg(&url));
        room["join_url"] = json!(url);
    }
    room["pending"] = reply["pending"].clone();
    if room != h.links["room"] {
        h.links["room"] = room;
        h.links_dirty = true;
    }
    // Profiles on seats: the player there takes its name, skin and face.
    let mut held = vec![None; h.cloud.seats.len()];
    for u in reply["updates"].as_array().into_iter().flatten() {
        let Some(index) = u["seat"]["index"].as_u64().map(|i| i as usize) else { continue };
        if h.cloud.seats.get(index) != Some(&u["seat"]) { continue; }
        held[index] = Some((u["account"].as_str().unwrap_or_default().to_string(), u["profile_revision"].as_u64().unwrap_or(0), u["profile"].clone()));
    }
    for (index, now) in held.iter().enumerate() {
        let key = now.as_ref().map(|(a, r, _)| (a.clone(), *r));
        if h.cloud.applied[index] == key { continue; }
        h.cloud.applied[index] = key;
        let Some(player) = h.party["players"].get_mut(index) else { continue };
        match now {
            Some((_, _, p)) => {
                player["name"] = p["display_name"].clone();
                player["skin_color"] = p["skin_color"].clone();
                player["avatar"] = p["avatar"].clone();
                if p["preferred_color"].is_string() { player["color"] = p["preferred_color"].clone(); }
            }
            // The profile left the seat: the player looks as before.
            None => if let Some(own) = h.own_looks.get(index) { *player = own.clone(); },
        }
        h.dirty = true;
    }
    // A game a phone on a seat picked (first in the queue, or started now),
    // or its change to the queue.
    let s = &reply["selection"];
    let id = s["id"].as_str()?.to_string();
    if h.cloud.acknowledged.as_deref() == Some(id.as_str()) || !s["command"].is_null() || !s["game_command"].is_null() { return None; }
    let seat = s["seat"]["index"].as_u64().map(|i| i as usize)?;
    if h.cloud.seats.get(seat) != Some(&s["seat"]) || held[seat].is_none() { return None; }
    if let Some(edit) = s["edit"].as_object() {
        h.cloud.acknowledged = Some(id);
        h.cloud.refresh = true;
        h.dirty = true;
        let entries = h.party["playlist"]["entries"].as_array_mut()?;
        let from = edit.get("from")?.as_u64()? as usize;
        if from >= entries.len() { return None; }
        let entry = entries.remove(from);
        if let Some(to) = edit.get("to").and_then(Value::as_u64) { entries.insert((to as usize).min(entries.len()), entry); }
        return None;
    }
    let game = s["game"].as_str()?.to_string();
    game_meta(&h.party, &game)?;
    h.cloud.acknowledged = Some(id);
    h.cloud.refresh = true;
    queue(h, &game, true);
    Some((game, s["start"] == true))
}

/// The room's QR code as the native host draws it (game_room::qr reads it back).
fn qr_svg(url: &str) -> String {
    use qrcode::{render::svg, QrCode};
    QrCode::new(url.as_bytes()).map(|qr| qr.render::<svg::Color>().quiet_zone(true).min_dimensions(256, 256).build()).unwrap_or_default()
}

pub(crate) fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    let _ = getrandom::fill(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// A random (version 4) UUID.
pub(crate) fn uuid() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

async fn sleep(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() { let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms); }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
