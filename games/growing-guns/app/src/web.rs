//! Growing Guns in a browser: the page fetches the mods, sound patches and
//! the character rig into the vfs and starts a local match (bots fill the
//! arena to four) straight away, as under Splitscreen Club.
//!
//! The page serves `games/growing-guns/` with `files.json` (paths below the
//! game folder, e.g. `mods/base/cards.luau`, `sounds/shot.luau`) beside those
//! files; `engine/tools/build-web.py` copies them. Everything runs on the
//! page's one thread (lighting bakes too); there is no online play, music
//! (the Godot stems are not served), replays or Warp Humans crowd.

/// The game folder in the vfs (mods/, sounds/, assets/characters/).
pub const ROOT: &str = "growing-guns";

/// `rel` inside the fetched game folder, if the page fetched it.
#[cfg(target_arch = "wasm32")]
pub(crate) fn served(rel: &str) -> Option<std::path::PathBuf> {
    let dir = std::path::Path::new(ROOT).join(rel);
    vfs::is_dir(&dir).then_some(dir)
}

#[cfg(target_arch = "wasm32")]
pub use page::*;

#[cfg(target_arch = "wasm32")]
mod page {
    use super::ROOT;
    use std::path::Path;

    /// The served folder, relative to the page.
    const SERVED: &str = "games/growing-guns";

    /// Fetch the game's files into the vfs, once: they stay for a later load.
    pub async fn fetch_assets() -> Result<(), String> {
        let root = Path::new(ROOT);
        if vfs::is_file(root.join("mods/base/cards.luau")) {
            return Ok(());
        }
        let files: Vec<String> = serde_json::from_slice(&app::web::fetch(&format!("{SERVED}/files.json")).await?).map_err(|e| format!("{SERVED}/files.json: {e}"))?;
        let t0 = web_time::Instant::now();
        let mut bytes = 0;
        for (i, rel) in files.iter().enumerate() {
            app::web::progress(Some(0.05 + 0.45 * i as f32 / files.len().max(1) as f32), &format!("Loading Growing Guns ({}/{})", i + 1, files.len()));
            let data = app::web::fetch(&format!("{SERVED}/{rel}")).await?;
            bytes += data.len();
            vfs::insert(root.join(rel), data);
        }
        app::web::log(&format!("growing-guns: {} files, {} KB in {:.0} ms", files.len(), bytes / 1024, t0.elapsed().as_secs_f32() * 1000.0));
        Ok(())
    }

    /// Fetch the files ([`fetch_assets`]) and make a match for `players`
    /// local players and bots up to four, straight into the first round.
    pub async fn load(players: usize, seed: u64) -> Result<(app::Config, Box<dyn app::Game>), String> {
        fetch_assets().await?;
        app::web::progress(Some(0.55), "Building the arena");
        app::web::yield_to_browser().await;
        let t1 = web_time::Instant::now();
        let humans = players.clamp(1, 4);
        let total = humans.max(4);
        let bots: Vec<bool> = (0..total).map(|i| i >= humans).collect();
        let world = growing_guns::World::new(seed, &bots);
        let mut game = crate::play::Gg::new(world, crate::session::Session::Local, (0..humans).collect());
        game.vis.stadium.bake_in_background = false;
        game.ui.player_name = format!("Player_{}", seed % 1000);
        game.set_sky(crate::sky::SkyChoice::Godot)?;
        app::web::progress(Some(0.85), "Tuning the speakers");
        app::web::yield_to_browser().await;
        match crate::sounds::speakers(None, None) {
            Ok((speaker, tx, crowd)) => {
                game.set_crowd(None, None, &crowd);
                game.set_sounds(crate::sounds::Sounds::new(tx, crate::sounds::find_patches(None), None));
                game.speaker = Some(speaker);
            }
            Err(e) => app::web::log(&format!("growing-guns: silent: {e}")),
        }
        app::web::log(&format!("growing-guns: match built in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
        let cfg = app::Config { title: "Growing Guns".into(), tick_hz: growing_guns::TICK_HZ as f32, keyboard: true, ..Default::default() };
        Ok((cfg, Box::new(game)))
    }
}
