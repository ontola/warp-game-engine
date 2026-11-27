//! The warp runtime in a browser page: game folders fetched next to the
//! page (what `engine/tools/build-web.py --warp` copies: `games/index.json`
//! lists them, `games/NAME/files.json` their files) into the [`vfs`], one
//! WebAudio output every game shares, and the page's query string. The
//! pages themselves are examples/warp_web.rs (games only) and the Game
//! Room's examples/browser/main.rs (lobby first). See engine/docs/browser.md.

use crate::party::SeatInfo;
use std::path::{Path, PathBuf};

thread_local! {
    /// The speakers and the synth mixer playing on them, opened once and
    /// shared by every game the page switches to.
    static SOUND: std::cell::RefCell<Option<(app::sound::Speaker, audio::synth::SoundSender)>> = const { std::cell::RefCell::new(None) };
    /// The page host's seats and how often they changed ([`set_seats`]).
    static SEATS: std::cell::RefCell<(u64, Vec<SeatInfo>)> = const { std::cell::RefCell::new((0, Vec::new())) };
    /// Lines games wrote with `game.log`, for the page's room log ([`take_room_log`]).
    static ROOM_LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}


#[derive(Default)]
struct StoryEdit {
    pending: Option<serde_json::Value>,
    reply: Option<Result<String, String>>,
}

thread_local! {
    static STORY_EDIT: std::cell::RefCell<StoryEdit> = Default::default();
}

pub fn reset_story_edit() { STORY_EDIT.with(|s| *s.borrow_mut() = StoryEdit::default()); }

pub fn queue_story(definition: &str) -> Result<(), String> {
    if definition.len() > 128 * 1024 { return Err("Story exceeds 128 KiB".into()); }
    let definition = serde_json::from_str(definition).map_err(|e| format!("Story data: {e}"))?;
    STORY_EDIT.with(|s| {
        let mut edit = s.borrow_mut();
        if edit.pending.is_some() { return Err("A story is already starting".into()); }
        edit.reply = None;
        edit.pending = Some(definition);
        Ok(())
    })
}

pub(crate) fn update_story(game: &mut crate::LuaGame) {
    let edit = STORY_EDIT.with(|s| s.borrow_mut().pending.take());
    if let Some(definition) = edit {
        let result = game.apply_story_data(definition);
        STORY_EDIT.with(|s| s.borrow_mut().reply = Some(result));
    }
}

pub fn story_reply() -> Option<String> {
    STORY_EDIT.with(|s| s.borrow_mut().reply.take().map(|result| match result {
        Ok(title) => serde_json::json!({ "title": title }).to_string(),
        Err(error) => serde_json::json!({ "error": error }).to_string(),
    }))
}

pub(crate) fn room_log(line: &str) { ROOM_LOG.with(|l| l.borrow_mut().push(line.to_string())); }

/// The lines games wrote for the room log (`game.log`) since the last call.
pub fn take_room_log() -> Vec<String> { ROOM_LOG.with(|l| std::mem::take(&mut *l.borrow_mut())) }

/// The page host's people now, on their seats: a running game follows at
/// its next tick (joins, leaves, names, colours), a paused one when it
/// resumes, and [`load`] seats a new game with them.
pub fn set_seats(seats: Vec<SeatInfo>) {
    SEATS.with(|s| {
        let mut s = s.borrow_mut();
        if s.1 != seats { s.0 += 1; s.1 = seats; }
    });
}

/// The seats, when they changed after `version`.
pub(crate) fn seats_after(version: u64) -> Option<(u64, Vec<SeatInfo>)> {
    SEATS.with(|s| { let s = s.borrow(); (s.0 > version).then(|| s.clone()) })
}

/// A sender into the page's one synth mixer, opening WebAudio the first time.
pub fn sound() -> Option<audio::synth::SoundSender> {
    SOUND.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_none() {
            let mut sender = None;
            match app::sound::open(|rate, channels| {
                let (tx, mix) = audio::synth::mixer(audio::synth::Layout::for_channels(channels), 128, rate);
                tx.room(1.2, 0.4, 0.5);
                sender = Some(tx);
                Box::new(mix)
            }) {
                Ok(speaker) => {
                    app::web::log(&format!("sound: {} Hz, {} channels", speaker.rate, speaker.channels));
                    *s = Some((speaker, sender.expect("made in open")));
                }
                Err(e) => app::web::log(&format!("sound off: {e}")),
            }
        }
        s.as_ref().map(|(_, tx)| tx.clone())
    })
}

/// The page's Luau games share one mixer ([`sound`]): silence it while a
/// paused game waits behind the lobby (`Some(true)`), bring it back with
/// the game (`Some(false)`), or end every voice of a game that quit
/// (`None`). Opens nothing when no game made a sound yet.
pub fn hush(paused: Option<bool>) {
    // Rust games with their own speakers (Growing Guns) listen to this.
    app::sound::set_hushed(paused == Some(true));
    SOUND.with(|s| if let Some((_, tx)) = s.borrow().as_ref() {
        match paused {
            Some(p) => tx.volume(if p { 0.0 } else { 1.0 }),
            None => { tx.stop_all(); tx.volume(1.0); }
        }
    });
}

/// A value from the page's query string (`?game=neon-siege&players=2`).
#[cfg(target_os = "android")]
pub fn param(k: &str) -> Option<String> { app::web::query_param(k) }

/// A value from the page's query string (`?game=neon-siege&players=2`).
#[cfg(target_arch = "wasm32")]
pub fn param(k: &str) -> Option<String> { app::web::query_param(k) }

/// The game folders next to the page (`games/index.json`).
pub async fn games() -> Result<Vec<String>, String> {
    serde_json::from_slice(&app::web::fetch("games/index.json").await?).map_err(|e| format!("games/index.json: {e}"))
}

/// Fetch game `name`'s files into the vfs, once: they stay in memory for
/// a later load. Returns its folder.
pub async fn fetch_game(name: &str) -> Result<PathBuf, String> {
    let dir = Path::new("games").join(name);
    if vfs::is_dir(&dir) { return Ok(dir); }
    let files: Vec<String> = serde_json::from_slice(&app::web::fetch(&format!("games/{name}/files.json")).await?).map_err(|e| format!("games/{name}/files.json: {e}"))?;
    let t0 = web_time::Instant::now();
    let mut bytes = 0;
    for (i, rel) in files.iter().enumerate() {
        app::web::progress(Some(0.05 + 0.7 * i as f32 / files.len().max(1) as f32), &format!("Loading {name} ({}/{})", i + 1, files.len()));
        let data = app::web::fetch(&format!("games/{name}/{rel}")).await?;
        bytes += data.len();
        vfs::insert(dir.join(rel), data);
    }
    app::web::log(&format!("{name}: {} files, {} KB in {:.0} ms", files.len(), bytes / 1024, t0.elapsed().as_secs_f32() * 1000.0));
    Ok(dir)
}

/// A player's own game from the cloud, by its play link (gamenight-cloud
/// `/v1/game-files/TOKEN`: `{id, title, files: {path: text}}`), into the
/// vfs as `games/<id>`, where [`fetch_game`] leaves a bundled game, so
/// [`load`] runs it as any other. Fetched again each time: the player may
/// have changed it. Returns its id (the host's shelf id) and title.
pub async fn fetch_project(url: &str) -> Result<(String, String), String> {
    let v: serde_json::Value = serde_json::from_slice(&app::web::fetch(url).await?).map_err(|e| format!("{url}: {e}"))?;
    let id = v["id"].as_str()
        .filter(|id| !id.is_empty() && id.len() <= 120 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .ok_or_else(|| format!("{url}: not a game"))?;
    let files = v["files"].as_object().ok_or_else(|| format!("{url}: no files"))?;
    let dir = Path::new("games").join(id);
    let mut bytes = 0;
    for (rel, text) in files {
        // Relative paths in the game's folder only (the cloud checks too).
        if rel.is_empty() || rel.starts_with('/') || rel.split('/').any(|p| p.is_empty() || p == "." || p == "..") { continue; }
        let text = text.as_str().unwrap_or_default().as_bytes().to_vec();
        bytes += text.len();
        vfs::insert(dir.join(rel), text);
    }
    app::web::log(&format!("{id}: own game, {} files, {} KB", files.len(), bytes / 1024));
    Ok((id.to_string(), v["title"].as_str().unwrap_or(id).to_string()))
}

/// Fetch game `name` ([`fetch_game`]), load its scripts and run `init`,
/// for `players` local players (keyboard, then pads in the order they wake
/// up; None: the game's own count). A page that seats its people
/// ([`set_seats`]) gets them in the game, with their names and colours.
pub async fn load(name: &str, players: Option<usize>) -> Result<(app::Config, crate::LuaGame), String> {
    let dir = fetch_game(name).await?;
    app::web::progress(Some(0.8), &format!("Starting {name}"));
    app::web::yield_to_browser().await;
    let t1 = web_time::Instant::now();
    let seats = SEATS.with(|s| { let s = s.borrow(); (s.0 > 0).then(|| s.1.clone()) });
    let over = crate::Overrides { fresh: true, players, seats, ..Default::default() };
    // On a phone the scripts load and init runs on a helper thread too
    // (Warp Theft Auto builds its city there): the room keeps moving.
    #[cfg(target_os = "android")]
    let mut game = {
        let (dir, sound) = (dir.clone(), sound());
        /// A game fresh from load: its online link (the only part that is
        /// not Send, a page's WebRTC handle) is Off until the page thread
        /// sets it, after this.
        struct Fresh(crate::LuaGame);
        // SAFETY: see above; LuaGame::load_with_storage leaves online Off.
        unsafe impl Send for Fresh {}
        app::android::off_thread_low(move || crate::LuaGame::load_with_storage(&dir, &over, sound, crate::storage::Location::Memory).map(Fresh)).await?.0
    };
    #[cfg(not(target_os = "android"))]
    #[allow(unused_mut)]
    let mut game = crate::LuaGame::load_with_storage(&dir, &over, sound(), crate::storage::Location::Memory)?;
    app::web::log(&format!("{name}: scripts loaded and init ran in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
    // On a phone the renderer (pipelines, buffers, texture arrays: seconds
    // on a tablet) is made on a helper thread, so the room keeps moving;
    // a browser has one thread and makes it in the game's first frame.
    #[cfg(target_os = "android")]
    if let Some(gpu) = app::android::gpu() {
        app::web::progress(Some(0.88), &format!("{name}: preparing the graphics"));
        app::web::yield_to_browser().await;
        let t2 = web_time::Instant::now();
        let (scene, light) = game.scene_for_renderer(u64::from(gpu.limits.max_storage_buffer_binding_size))?;
        let (renderer, scene) = app::android::off_thread_low(move || {
            render::startup_profile::force(true);
            let r = render::Renderer::with_generated_streaming(&gpu, &scene, &light, true);
            render::startup_profile::force(false);
            (r, scene)
        }).await;
        if let Some(p) = renderer.startup_profile() { app::web::log(&format!("{name}: renderer made ahead in {:.0} ms: {}", t2.elapsed().as_secs_f32() * 1000.0, p.slowest(6))); }
        game.set_renderer_ahead(renderer, scene);
    }
    // In a browser the pipelines compile in parallel while the room keeps
    // moving (app::web::prewarm_with), and the renderer is made here, before
    // the switch: the game's first frame only draws.
    #[cfg(all(target_arch = "wasm32", not(target_os = "android")))]
    if app::web::prewarm() && let Some(gpu) = app::web::gpu() {
        app::web::progress(Some(0.88), &format!("{name}: preparing the graphics"));
        app::web::yield_to_browser().await;
        game.surface_ahead();
        let (scene, light) = game.scene_for_renderer(u64::from(gpu.limits.max_storage_buffer_binding_size))?;
        let kept = app::web::prewarm_with(&gpu, name, || render::Renderer::with_generated_streaming(&gpu, &scene, &light, true)).await;
        app::web::yield_to_browser().await;
        let t2 = web_time::Instant::now();
        render::startup_profile::force(true);
        let renderer = render::Renderer::with_generated_streaming(&gpu, &scene, &light, true);
        render::startup_profile::force(false);
        drop(kept);
        if let Some(p) = renderer.startup_profile() { app::web::log(&format!("{name}: renderer made ahead in {:.0} ms: {}", t2.elapsed().as_secs_f32() * 1000.0, p.slowest(6))); }
        game.set_renderer_ahead(renderer, scene);
        app::web::prepared();
    }
    let cfg = app::Config { title: game.cfg.title.clone(), size: game.cfg.size, tick_hz: game.cfg.tick_rate, keyboard: true, pointer: false, ..Default::default() };
    Ok((cfg, game))
}
