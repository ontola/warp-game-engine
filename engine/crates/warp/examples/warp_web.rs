//! The `warp` runtime in a browser: one engine download, then Luau games
//! as small folders of scripts that load in a moment. `?game=NAME` picks
//! the first game from those `engine/tools/build-web.py --warp` copied next
//! to the page (`games/index.json` lists them, `games/NAME/files.json`
//! their files), `?players=N` how many play here. The page switches games
//! without reloading the engine through [`switch_game`]. See
//! engine/docs/browser.md.
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(target_arch = "wasm32")]
fn main() {
    script::web::init();
    app::web::install_panic_hook();
    wasm_bindgen_futures::spawn_local(async {
        match first().await {
            Ok((cfg, game)) => app::web::run(cfg, Box::new(game)),
            Err(e) => { app::web::progress(Some(0.3), &format!("Could not start: {e}")); app::web::log(&e); }
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn param(k: &str) -> Option<String> {
    let query = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    query.trim_start_matches('?').split('&').find_map(|kv| kv.split_once('=').filter(|(key, _)| *key == k).map(|(_, v)| v.to_owned()))
}

#[cfg(target_arch = "wasm32")]
async fn first() -> Result<(app::Config, warp::LuaGame), String> {
    let index: Vec<String> = serde_json::from_slice(&app::web::fetch("games/index.json").await?).map_err(|e| format!("games/index.json: {e}"))?;
    let name = param("game").or_else(|| index.first().cloned()).ok_or("no games next to the page")?;
    if !index.contains(&name) { return Err(format!("no game {name:?} here; there are {}", index.join(", "))); }
    load(&name).await
}

/// Fetch game `name`'s files (once: they stay in memory for a later
/// switch back), then load its scripts and run `init`.
#[cfg(target_arch = "wasm32")]
async fn load(name: &str) -> Result<(app::Config, warp::LuaGame), String> {
    use std::path::Path;
    let dir = Path::new("games").join(name);
    if !vfs::is_dir(&dir) {
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
    }
    let t1 = web_time::Instant::now();
    // `?players=2`: local players (keyboard, then pads in the order they wake up).
    let players = param("players").and_then(|p| p.parse().ok());
    let over = warp::Overrides { fresh: true, players, ..Default::default() };
    let game = warp::LuaGame::load_with_storage(&dir, &over, None, warp::storage::Location::Memory)?;
    app::web::log(&format!("{name}: scripts loaded and init ran in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
    let cfg = app::Config { title: game.cfg.title.clone(), size: game.cfg.size, tick_hz: game.cfg.tick_rate, keyboard: true, pointer: false, ..Default::default() };
    Ok((cfg, game))
}

/// Switch to game `name` without reloading the page or the engine: the
/// running game keeps playing until the new one has loaded.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn switch_game(name: String) {
    wasm_bindgen_futures::spawn_local(async move {
        let t0 = web_time::Instant::now();
        match load(&name).await {
            Ok((cfg, game)) => {
                app::web::log(&format!("{name}: ready to switch {:.0} ms after the request", t0.elapsed().as_secs_f32() * 1000.0));
                app::web::switch(cfg, Box::new(game));
            }
            Err(e) => app::web::log(&format!("{name}: {e}")),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() { eprintln!("warp_web is the browser build: engine/tools/build-web.py --warp (engine/docs/browser.md)"); }
