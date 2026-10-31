//! Explicit opt-in headless harness for the real host; no network debug API.
use super::*;
use std::{
    path::Path,
    time::{Duration, Instant},
};
pub fn run(game: &mut Racer, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let gpu = render::Gpu::new()?;
    let mut headless = app::Headless::new(&gpu, [1600, 900], 120.)?;
    let start = Instant::now();
    let mut n = 0;
    while start.elapsed() < Duration::from_secs(300) && !dir.join("stop").exists() {
        let frame = Instant::now();
        game.tick(&vec![Controls::default(); game.players]);
        if !game.visuals.uploaded {
            headless.frame(game)?;
        }
        let request = dir.join("request.json");
        if request.exists() {
            let v: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&request).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            std::fs::remove_file(&request).map_err(|e| e.to_string())?;
            if let Some(c) = v["command"].as_str() {
                game.command(c)?;
            }
            if let Some(name) = v["capture"].as_str() {
                if !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
                {
                    return Err("capture filename".into());
                }
                let pixels = headless.frame(game)?;
                render::write_png(&dir.join(name), 1600, 900, &pixels)?;
            }
        }
        if n % 12 == 0 {
            std::fs::write(dir.join("state.json"), game.state().unwrap())
                .map_err(|e| e.to_string())?;
        }
        n += 1;
        if let Some(left) = Duration::from_secs_f64(1. / 120.).checked_sub(frame.elapsed()) {
            std::thread::sleep(left);
        }
    }
    Ok(())
}
