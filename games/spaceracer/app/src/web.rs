//! Space Racer in a browser: the page fetches the game's rules and assets
//! into the vfs and starts it like any other game (`app::web::switch_keeping`).
//!
//! The page serves `games/spaceracer/` with `files.json` (paths below the
//! game folder, e.g. `rules/game.luau`, `assets/source/assets/paint-wear.png`)
//! beside those files; `engine/tools/build-web.py` copies them. Races build
//! on the page's one thread (natively a worker builds them), and there are
//! no command-line tools, VR or hot-reloaded shaders.

/// The game folder in the vfs (rules/, assets/).
pub const ROOT: &str = "spaceracer";

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub use page::*;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
mod page {
    use super::ROOT;
    use std::path::Path;

    /// The served folder, relative to the page.
    const SERVED: &str = "games/spaceracer";

    /// Fetch the rules and assets into the vfs, once: they stay for a later load.
    pub async fn fetch_assets() -> Result<(), String> {
        let root = Path::new(ROOT);
        if vfs::is_file(root.join("rules/game.luau")) {
            return Ok(());
        }
        let files: Vec<String> = serde_json::from_slice(&app::web::fetch(&format!("{SERVED}/files.json")).await?).map_err(|e| format!("{SERVED}/files.json: {e}"))?;
        let t0 = web_time::Instant::now();
        let mut bytes = 0;
        for (i, rel) in files.iter().enumerate() {
            app::web::progress(Some(0.3 + 0.4 * i as f32 / files.len().max(1) as f32), &format!("Loading Space Racer ({}/{})", i + 1, files.len()));
            let data = app::web::fetch(&format!("{SERVED}/{rel}")).await?;
            bytes += data.len();
            vfs::insert(root.join(rel), data);
        }
        app::web::log(&format!("spaceracer: {} files, {} KB in {:.0} ms", files.len(), bytes / 1024, t0.elapsed().as_secs_f32() * 1000.0));
        Ok(())
    }

    /// Fetch the files ([`fetch_assets`]) and make a race for `players`
    /// local players, straight into the race (as under Splitscreen Club),
    /// world, difficulty and track on Random.
    pub async fn load(players: usize, seed: u64) -> Result<(app::Config, Box<dyn app::Game>), String> {
        fetch_assets().await?;
        app::web::progress(Some(0.75), "Building the track");
        app::web::yield_to_browser().await;
        let t1 = web_time::Instant::now();
        crate::install_shaders(false);
        let game = crate::Racer::for_page(Path::new(ROOT).to_path_buf(), players.clamp(1, 4), seed)?;
        app::web::log(&format!("spaceracer: race built in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
        let cfg = app::Config { title: "Space Racer".into(), tick_hz: 120.0, keyboard: true, pointer: false, ..Default::default() };
        Ok((cfg, Box::new(game)))
    }
}
