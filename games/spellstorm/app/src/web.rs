//! Spellstorm in a browser: the page fetches the game's assets into the
//! vfs and starts it like any other game (`app::web::switch_keeping`).
//!
//! The page serves `games/spellstorm/` with `files.json` (the asset paths
//! below `app/assets`, e.g. `kits/kits.json`) beside those files;
//! `engine/tools/build-web.py` copies them. The mods are built in.

/// Where the kits live in the vfs (kits.rs `default_dir` in a browser);
/// the characters are beside them in `spellstorm/assets/characters`.
pub const KITS: &str = "spellstorm/assets/kits";

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
pub use page::*;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
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
            // The files are the first 60% of the bar: reading the kits and
            // building the arena come after (Joep: it sat at 200/200).
            app::web::progress(Some(0.05 + 0.55 * i as f32 / files.len().max(1) as f32), &format!("Loading Spellstorm ({}/{})", i + 1, files.len()));
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
        app::web::progress(Some(0.62), "Spellstorm: reading the wizards and monsters");
        app::web::yield_to_browser().await;
        let t1 = web_time::Instant::now();
        let kits = crate::kits::Kits::load(None)?;
        let mut game = crate::play::Spellstorm::with_party(kits, players.max(1), 0, seed, None);
        app::web::log(&format!("spellstorm: kits loaded in {:.0} ms", t1.elapsed().as_secs_f32() * 1000.0));
        app::web::progress(Some(0.78), "Spellstorm: building the arena");
        app::web::yield_to_browser().await;
        let t2 = web_time::Instant::now();
        // The scene now, not in the first frame, where it froze the screen.
        game.prepare()?;
        app::web::log(&format!("spellstorm: arena built in {:.0} ms", t2.elapsed().as_secs_f32() * 1000.0));
        // On a phone the renderer (buffers, texture arrays, mips) is made on a
        // helper thread, so the room's loading card keeps moving; in a
        // browser there is one thread and the first frame makes it.
        #[cfg(target_os = "android")]
        if let Some(gpu) = app::android::gpu() {
            app::web::progress(Some(0.85), "Spellstorm: preparing the graphics");
            let t3 = web_time::Instant::now();
            let (scene, light) = (std::mem::take(&mut game.view.scene), game.view.light.clone());
            // Low priority: in round 32 the room froze for the 11 s this took.
            let (renderer, scene) = app::android::off_thread_low(move || {
                render::startup_profile::force(true);
                let r = render::Renderer::with_generated_streaming(&gpu, &scene, &light, true);
                render::startup_profile::force(false);
                (r, scene)
            }).await;
            app::web::log(&format!("spellstorm: renderer made ahead in {:.0} ms", t3.elapsed().as_secs_f32() * 1000.0));
            // Where those seconds went: the slowest sections and texture arrays.
            if let Some(p) = renderer.startup_profile() {
                let mut s: Vec<_> = p.sections.iter().map(|s| (s.name, s.cpu_ms)).collect();
                s.sort_by(|a, b| b.1.total_cmp(&a.1));
                let top: Vec<String> = s.iter().take(8).map(|(n, ms)| format!("{n} {ms:.0}")).collect();
                let tex: Vec<String> = p.textures.iter().map(|t| format!("{} {}x{} build {:.0} upload {:.0}", t.name, t.size, t.layers.len(), t.build_wall_ms, t.upload_cpu_ms)).collect();
                app::web::log(&format!("spellstorm: renderer sections: {}; textures: {}", top.join(", "), tex.join(", ")));
            }
            game.view.scene = scene;
            game.set_renderer_ahead(renderer);
        }
        let cfg = app::Config { title: "Spellstorm".into(), size: [1920, 1080], tick_hz: 60.0, keyboard: true, pointer: false, ..Default::default() };
        Ok((cfg, Box::new(game)))
    }
}
