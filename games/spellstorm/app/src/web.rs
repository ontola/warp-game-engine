//! Spellstorm in a browser: the page fetches the game's assets into the
//! vfs and starts it like any other game (`app::web::switch_keeping`).
//!
//! The page serves `games/spellstorm/` with `files.json` (the asset paths
//! below `app/assets`, e.g. `kits/kits.json`) beside those files;
//! `engine/tools/build-web.py` copies them. The mods are built in.

/// Where the kits live in the vfs (kits.rs `default_dir` in a browser);
/// the characters are beside them in `spellstorm/assets/characters`.
pub const KITS: &str = "spellstorm/assets/kits";

#[cfg(target_arch = "wasm32")]
pub use page::*;

#[cfg(target_arch = "wasm32")]
mod page {
    use std::path::Path;

    /// The served folder, relative to the page.
    const SERVED: &str = "games/spellstorm";

    /// Fetch the assets into the vfs, once: they stay for a later load.
    pub async fn fetch_assets() -> Result<(), String> {
        let root = Path::new("spellstorm/assets");
        if vfs::is_file(root.join("kits/kits.json")) {
            return Ok(());
        }
        let files: Vec<String> = serde_json::from_slice(&app::web::fetch(&format!("{SERVED}/files.json")).await?).map_err(|e| format!("{SERVED}/files.json: {e}"))?;
        let t0 = web_time::Instant::now();
        let mut bytes = 0;
        for (i, rel) in files.iter().enumerate() {
            app::web::progress(Some(0.3 + 0.4 * i as f32 / files.len().max(1) as f32), &format!("Loading Spellstorm ({}/{})", i + 1, files.len()));
            let data = app::web::fetch(&format!("{SERVED}/{rel}")).await?;
            bytes += data.len();
            vfs::insert(root.join(rel), data);
        }
        app::web::log(&format!("spellstorm: {} files, {} KB in {:.0} ms", files.len(), bytes / 1024, t0.elapsed().as_secs_f32() * 1000.0));
        Ok(())
    }

    /// Fetch the assets ([`fetch_assets`]) and make a run for `players`
    /// local players (keyboard, then pads in the order they wake up).
    pub async fn load(players: usize, seed: u64) -> Result<(app::Config, Box<dyn app::Game>), String> {
        fetch_assets().await?;
        app::web::yield_to_browser().await;
        let t1 = web_time::Instant::now();
        let kits = crate::kits::Kits::load(None)?;
        let game = crate::play::Spellstorm::with_party(kits, players.max(1), 0, seed, None);
        app::web::log(&format!("spellstorm: kits loaded in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
        let cfg = app::Config { title: "Spellstorm".into(), size: [1920, 1080], tick_hz: 60.0, keyboard: true, pointer: false, ..Default::default() };
        Ok((cfg, Box::new(game)))
    }
}
