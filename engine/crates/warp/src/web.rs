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
                    app::web::log(&format!("sound: WebAudio, {} Hz, {} channels", speaker.rate, speaker.channels));
                    *s = Some((speaker, sender.expect("made in open")));
                }
                Err(e) => app::web::log(&format!("sound off: {e}")),
            }
        }
        s.as_ref().map(|(_, tx)| tx.clone())
    })
}

/// A value from the page's query string (`?game=neon-siege&players=2`).
pub fn param(k: &str) -> Option<String> {
    let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    query.trim_start_matches('?').split('&').find_map(|kv| kv.split_once('=').filter(|(key, _)| *key == k).map(|(_, v)| v.to_owned()))
}

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
        app::web::progress(Some(0.3 + 0.4 * i as f32 / files.len().max(1) as f32), &format!("Loading {name} ({}/{})", i + 1, files.len()));
        let data = app::web::fetch(&format!("games/{name}/{rel}")).await?;
        bytes += data.len();
        vfs::insert(dir.join(rel), data);
    }
    app::web::log(&format!("{name}: {} files, {} KB in {:.0} ms", files.len(), bytes / 1024, t0.elapsed().as_secs_f32() * 1000.0));
    Ok(dir)
}

/// Fetch game `name` ([`fetch_game`]), load its scripts and run `init`,
/// for `players` local players (keyboard, then pads in the order they wake
/// up; None: the game's own count). A page that seats its people
/// ([`set_seats`]) gets them in the game, with their names and colours.
pub async fn load(name: &str, players: Option<usize>) -> Result<(app::Config, crate::LuaGame), String> {
    let dir = fetch_game(name).await?;
    let t1 = web_time::Instant::now();
    let seats = SEATS.with(|s| { let s = s.borrow(); (s.0 > 0).then(|| s.1.clone()) });
    let over = crate::Overrides { fresh: true, players, seats, ..Default::default() };
    let game = crate::LuaGame::load_with_storage(&dir, &over, sound(), crate::storage::Location::Memory)?;
    app::web::log(&format!("{name}: scripts loaded and init ran in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
    let cfg = app::Config { title: game.cfg.title.clone(), size: game.cfg.size, tick_hz: game.cfg.tick_rate, keyboard: true, pointer: false, ..Default::default() };
    Ok((cfg, game))
}
