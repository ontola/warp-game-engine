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

use super::page::{game_meta, queue, with, Host};
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
    /// The early registration (`register_early`) is under way.
    registering: bool,
    /// Seats a profile holds, with its account and profile revision.
    applied: Vec<Option<(String, u64)>>,
    /// Phones that asked to play on the phone alone, by their waiting id:
    /// the player made for them, and whether it was claimed for them yet.
    phone_seats: std::collections::HashMap<String, (String, bool)>,
    /// When each seat's device last did anything: the cloud takes a profile
    /// off a seat nobody touched for half an hour while its phone was away
    /// too (the poll's `idle`).
    heard: Vec<web_time::Instant>,
}

impl Cloud {
    /// The cloud room's code (empty until registered): also the room other
    /// couches follow this page's games in (online.rs).
    pub(crate) fn room_code(&self) -> &str { &self.room_code }

    /// A seat for a player who joined with a pad.
    pub(crate) fn add_seat(&mut self) {
        self.seats.push(json!({"index": self.seats.len(), "player": random_hex(16), "revision": 1}));
        self.applied.push(None);
        self.heard.push(web_time::Instant::now());
        self.refresh = true;
    }

    /// Seat `index`'s player left the table; the seats after it move up.
    pub(crate) fn remove_seat(&mut self, index: usize) {
        if index >= self.seats.len() { return; }
        self.seats.remove(index);
        self.applied.remove(index);
        if index < self.heard.len() { self.heard.remove(index); }
        for (k, seat) in self.seats.iter_mut().enumerate() { seat["index"] = json!(k); }
        self.refresh = true;
    }

    /// Seat `index` has someone else now: a profile on it goes back to the doors.
    pub(crate) fn free_seat(&mut self, index: usize) {
        let Some(seat) = self.seats.get_mut(index) else { return };
        seat["player"] = json!(random_hex(16));
        seat["revision"] = json!(seat["revision"].as_u64().unwrap_or(1) + 1);
        if let Some(at) = self.heard.get_mut(index) { *at = web_time::Instant::now(); }
        self.refresh = true;
    }

    /// Seat `index`'s device pressed, held or moved something.
    pub(crate) fn heard(&mut self, index: usize) {
        if let Some(at) = self.heard.get_mut(index) { *at = web_time::Instant::now(); }
    }

    /// Seconds since each seat's last input, by seat index.
    fn idle(&self) -> Value {
        let idle: serde_json::Map<String, Value> = self.heard.iter().enumerate().map(|(i, at)| (i.to_string(), json!(at.elapsed().as_secs()))).collect();
        Value::Object(idle)
    }
}

/// Where the cloud room lives: this page's origin, `?cloud=URL`, or
/// none with `?cloud=off`.
fn cloud_origin() -> Option<String> {
    match warp::web::param("cloud") {
        Some(o) if o == "off" => None,
        Some(o) => Some(o.trim_end_matches('/').to_string()),
        None => Some(app::web::origin()),
    }
}

/// Register the room at once, while the Game Room is still being built, so
/// the loading screen shows its QR code (`show_join`); the poll loop
/// (`start`) goes on with the same token.
pub(crate) fn register_early() {
    let Some(origin) = cloud_origin() else { return };
    with(|h| h.cloud.registering = true);
    app::web::spawn(async move {
        let registered = match page_room(&origin).await { Some(r) => Ok(r), None => register(&origin).await };
        match registered {
            Ok((token, code)) => {
                with(|h| { h.cloud.token = Some(token); h.cloud.acknowledged = None; });
                show_join(&origin, &code);
            }
            Err(e) => app::web::log(&format!("cloud room: {e}")),
        }
        with(|h| h.cloud.registering = false);
    });
}

/// The Android host has no loading page: it registers itself.
#[cfg(target_os = "android")]
async fn page_room(_: &str) -> Option<(String, String)> { None }

/// The room the loading page registered while the engine downloaded
/// (index.html `window.splitscreenRoom`, a promise), when it is for this origin.
#[cfg(target_arch = "wasm32")]
async fn page_room(origin: &str) -> Option<(String, String)> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window()?;
    let promise = js_sys::Reflect::get(&window, &"splitscreenRoom".into()).ok()?.dyn_into::<js_sys::Promise>().ok()?;
    let room = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
    let text = |k: &str| js_sys::Reflect::get(&room, &k.into()).ok()?.as_string();
    if text("origin")? != origin { return None; }
    Some((text("token")?, text("room_code")?))
}

/// A new room for this device (its id kept in the browser): the token and room code.
async fn register(origin: &str) -> Result<(String, String), String> {
    let device = app::web::stored("splitscreen-club-device").filter(|d| d.len() == 64).unwrap_or_else(|| {
        let d = random_hex(32);
        app::web::store("splitscreen-club-device", &d);
        d
    });
    let (status, body) = app::web::request("POST", &format!("{origin}/v1/lobbies/register"), &[], Some(&json!({"device": device}).to_string())).await?;
    if !(200..300).contains(&status) { return Err(format!("register: HTTP {status}")); }
    let reply: Value = serde_json::from_slice(&body).map_err(|e| format!("register: {e}"))?;
    let token = reply["token"].as_str().ok_or("register: no token")?.to_string();
    let code = reply["room_code"].as_str().unwrap_or_default().to_string();
    app::web::log(&format!("cloud room {} at {origin}", if code.is_empty() { "?" } else { &code }));
    Ok((token, code))
}

/// The loading screen's "Join with your phone": the room's QR code and code
/// (index.html `#warp-join`).
fn show_join(origin: &str, code: &str) {
    if code.is_empty() { return; }
    // The Android host has no loading page: the room shows its code itself.
    #[cfg(target_arch = "wasm32")]
    {
        let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
        if let Some(e) = doc.get_element_by_id("warp-qr") { e.set_inner_html(&qr_svg(&format!("{origin}/?r={code}"))); }
        if let Some(e) = doc.get_element_by_id("warp-code") { e.set_text_content(Some(code)); }
        let host = origin.split("://").nth(1).unwrap_or(origin);
        if let Some(e) = doc.get_element_by_id("warp-host") { e.set_text_content(Some(host)); }
        if let Some(e) = doc.get_element_by_id("warp-join") { let _ = e.remove_attribute("hidden"); }
        if let Some(root) = doc.document_element() { let _ = root.set_attribute("data-qr", ""); }
        // The address bar shows the room too (index.html splitscreenSetRoom).
        use wasm_bindgen::JsCast;
        if let Some(set) = web_sys::window().and_then(|w| js_sys::Reflect::get(&w, &"splitscreenSetRoom".into()).ok()).and_then(|f| f.dyn_into::<js_sys::Function>().ok()) {
            let _ = set.call1(&wasm_bindgen::JsValue::NULL, &code.into());
        }
    }
    #[cfg(target_os = "android")]
    let _ = origin;
}

/// Start the room's poll loop for `players` seats.
pub(crate) fn start(players: usize) {
    let Some(origin) = cloud_origin() else { return };
    with(|h| {
        h.cloud.origin = origin;
        h.cloud.seats = (0..players).map(|i| json!({"index": i, "player": random_hex(16), "revision": 1})).collect();
        h.cloud.applied = vec![None; players];
        h.cloud.heard = vec![web_time::Instant::now(); players];
    });
    app::web::spawn(async {
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

/// Report a bug to the cloud room (`/v1/lobbies/bug-report`), as the
/// desktop host's Back + Y does: the game on screen (None: the room), its
/// title and a JPEG of `shot`. The cloud answers how many phones at the
/// table it asked and a describe link: the room then shows the bug panel
/// with that link's QR code (hud.rs `bug_panel`), so a table without a
/// linked phone is never told to use one.
pub(crate) fn report_bug(game: Option<String>, title: Option<String>, shot: Option<&image::RgbaImage>) {
    let screenshot = shot.and_then(jpeg_data_uri);
    let id = uuid();
    let report = json!({
        "id": id,
        "app_version": env!("CARGO_PKG_VERSION"),
        "platform": if cfg!(target_os = "android") { "android-host" } else { "web" },
        "game": game,
        "title": title,
        "screenshot": screenshot,
    });
    app::web::spawn(async move {
        let Some((origin, token)) = with(|h| Some((h.cloud.origin.clone(), h.cloud.token.clone()?))) else {
            app::web::log("bug report: no cloud room, nothing sent");
            super::page::bug_failed("This room is offline, so the bug report was not sent");
            return;
        };
        let body = report.to_string();
        for attempt in 0..3 {
            match app::web::request("POST", &format!("{origin}/v1/lobbies/bug-report"), &[("authorization", &format!("Bearer {token}"))], Some(&body)).await {
                Ok((status, reply)) if (200..300).contains(&status) => {
                    app::web::log("bug report sent");
                    let reply: Value = serde_json::from_slice(&reply).unwrap_or_default();
                    with(|h| bug_panel(h, &origin, &id, title.as_deref(), &reply));
                    return;
                }
                Ok((status, _)) if status == 400 || status == 413 => { app::web::log(&format!("bug report refused (HTTP {status})")); return; }
                Ok((status, _)) => app::web::log(&format!("bug report: HTTP {status}")),
                Err(e) => app::web::log(&format!("bug report: {e}")),
            }
            sleep(2000 * (attempt + 1)).await;
        }
        super::page::bug_failed("Could not reach Splitscreen Club, so the bug report was not sent");
    });
}

/// The room's bug panel for the cloud's answer to a bug report: who to ask
/// (`asked` phones in the app) and the describe link's QR code.
fn bug_panel(h: &mut Host, origin: &str, id: &str, title: Option<&str>, reply: &Value) {
    let Some(path) = reply["link"].as_str().filter(|p| p.starts_with('/')) else { return };
    let url = format!("{origin}{path}");
    let short = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest).to_string();
    let phones = reply["asked"].as_u64().unwrap_or(0) > 0;
    let tap = cfg!(target_os = "android");
    let line = match (phones, tap) {
        (true, true) => "Say it in the Splitscreen Club app, scan the code with any phone, or tap the screen to type it here",
        (true, false) => "Say it in the Splitscreen Club app, or scan the code with any phone",
        (false, true) => "Scan the code with any phone, or tap the screen to type it here · no account needed",
        (false, false) => "Scan the code with any phone to say what happened · no account needed",
    };
    app::web::tap_opens(Some(&url), game_room::paint::BUG_SECONDS);
    h.links["bug"] = json!({
        "id": id,
        "title": match title { Some(t) => format!("What went wrong in {t}?"), None => "What went wrong?".to_string() },
        "line": line,
        "link": short,
        "qr_svg": qr_svg(&url),
    });
    h.links_dirty = true;
}

/// A JPEG data URI of a bug screenshot: up to 1920 wide, under the cloud's 3 MB.
fn jpeg_data_uri(img: &image::RgbaImage) -> Option<String> {
    use base64::Engine;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 { return None; }
    let width = w.min(1920);
    let height = ((u64::from(h) * u64::from(width)) / u64::from(w)).max(1) as u32;
    let rgb = image::DynamicImage::ImageRgba8(if width == w { img.clone() } else { image::imageops::resize(img, width, height, image::imageops::FilterType::Triangle) }).to_rgb8();
    for quality in [85, 70, 50] {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).encode_image(&rgb).ok()?;
        if jpeg.len() <= 2_500_000 {
            return Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpeg)));
        }
    }
    None
}

/// Claim waiting profile `pending` for seat `index` (a door pickup in the room).
pub(crate) fn pickup(pending: String, index: usize) {
    app::web::spawn(async move {
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
        // The early registration has not answered yet: next round.
        None if with(|h| h.cloud.registering) => return Ok(()),
        None => {
            let (token, code) = register(&origin).await?;
            with(|h| { h.cloud.token = Some(token.clone()); h.cloud.acknowledged = None; });
            show_join(&origin, &code);
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
        if now { super::page::start(game); }
    }
    Ok(())
}

/// What the cloud needs of this table: seats and the games it can play.
fn snapshot(h: &Host) -> Value {
    let party = &h.party;
    let people = h.cloud.seats.len() as u64;
    let session = party["active_session"].as_object().map(|s| s["game"].clone()).unwrap_or(Value::Null);
    let games: Vec<Value> = party["library"].as_array().into_iter().flatten().map(|g| {
        let fits = super::page::fits(party, g["id"].as_str().unwrap_or_default(), people as usize) && people <= g["max_players"].as_u64().unwrap_or(8);
        let state = if session == g["id"] { "playing" } else if h.loading.as_deref() == g["id"].as_str() { "loading" } else if fits { "available" } else { "unavailable" };
        json!({"id": g["id"], "selectable": fits, "state": state})
    }).collect();
    let current = if session.is_null() { h.loading.clone().map(Value::from).unwrap_or(Value::Null) } else { session.clone() };
    let next = party["playlist"]["entries"].get(0).map(|e| e["game"].clone()).unwrap_or(Value::Null);
    json!({"seats": h.cloud.seats, "discovery": {"games": games, "playlist": party["playlist"], "session": party["active_session"],
        "current": current, "next": next, "acknowledged": h.cloud.acknowledged}, "activity": h.activity, "idle": h.cloud.idle()})
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
        show_join(&h.cloud.origin, &code);
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
    phones(h, reply, &held);
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

/// Phones that asked to play on the phone alone (the cloud's phone seat,
/// as the native host's bridge does it): a player without a controller is
/// added for each, and claimed for it once a poll has reported that seat. A
/// phone player whose profile left, or whose phone gave up waiting before
/// the claim, leaves the table again.
fn phones(h: &mut Host, reply: &Value, held: &[Option<(String, u64, Value)>]) {
    let waiting: Vec<&Value> = reply["pending"].as_array().into_iter().flatten().filter(|p| p["phone_seat"] == true).collect();
    let mut leave = vec![];
    for p in &waiting {
        let Some(id) = p["id"].as_str() else { continue };
        match h.cloud.phone_seats.get(id).cloned() {
            None if h.cloud.seats.len() < 8 => {
                let seat = super::page::add_player(h, true);
                let player = h.party["players"][seat]["id"].as_str().unwrap_or_default().to_string();
                if let Some(name) = p["profile"]["display_name"].as_str() { h.party["players"][seat]["name"] = json!(name); }
                h.cloud.phone_seats.insert(id.to_string(), (player, false));
            }
            // The poll that brought this reply already reported the new seat.
            Some((player, false)) => if let Some(seat) = super::page::seat_of(h, &player) {
                pickup(id.to_string(), seat);
                h.cloud.phone_seats.insert(id.to_string(), (player, true));
            },
            _ => {}
        }
    }
    let gone: Vec<String> = h.cloud.phone_seats.keys().filter(|id| !waiting.iter().any(|p| p["id"] == id.as_str())).cloned().collect();
    for id in gone {
        let Some((player, _)) = h.cloud.phone_seats.remove(&id) else { continue };
        let linked = super::page::seat_of(h, &player).is_some_and(|seat| held.get(seat).is_some_and(Option::is_some));
        if !linked { leave.push(player); }
    }
    // A phone player whose profile left its seat.
    for (seat, now) in held.iter().enumerate() {
        if now.is_none() && h.phone.get(seat) == Some(&true) && !h.cloud.phone_seats.values().any(|(p, _)| h.party["players"][seat]["id"] == p.as_str()) {
            if let Some(id) = h.party["players"][seat]["id"].as_str() { leave.push(id.to_string()); }
        }
    }
    for player in leave { super::page::leave_party(h, &player); }
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

async fn sleep(ms: u32) { app::web::sleep(ms).await }
