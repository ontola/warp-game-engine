//! The `warp` runtime in a browser, without a lobby: one engine download,
//! then Luau games as small folders of scripts that load in a moment.
//! `?game=NAME` picks the first game from those `engine/tools/build-web.py
//! --warp --bare` copied next to the page, `?players=N` how many play here.
//! The page switches games without reloading the engine through
//! [`switch_game`]. The Game Room's examples/browser.rs is the same with the
//! lobby in front. See engine/docs/browser.md.
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
fn players() -> Option<usize> { warp::web::param("players").and_then(|p| p.parse().ok()) }

#[cfg(target_arch = "wasm32")]
async fn first() -> Result<(app::Config, warp::LuaGame), String> {
    let index = warp::web::games().await?;
    let name = warp::web::param("game").or_else(|| index.first().cloned()).ok_or("no games next to the page")?;
    if !index.contains(&name) { return Err(format!("no game {name:?} here; there are {}", index.join(", "))); }
    warp::web::load(&name, players()).await
}

/// Switch to game `name` without reloading the page or the engine: the
/// running game keeps playing until the new one has loaded.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn switch_game(name: String) {
    wasm_bindgen_futures::spawn_local(async move {
        let t0 = web_time::Instant::now();
        match warp::web::load(&name, players()).await {
            Ok((cfg, game)) => {
                app::web::log(&format!("{name}: ready to switch {:.0} ms after the request", t0.elapsed().as_secs_f32() * 1000.0));
                app::web::switch(cfg, Box::new(game));
            }
            Err(e) => app::web::log(&format!("{name}: {e}")),
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() { eprintln!("warp_web is the browser build: engine/tools/build-web.py --warp --bare (engine/docs/browser.md)"); }
