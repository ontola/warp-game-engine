//! F12 feedback screenshots: the frame as shown (HUD included) as a PNG, a
//! JSON file next to it with what was on screen (the game's
//! [`crate::Game::state`], camera, GPU, fps), and the image on the clipboard
//! so it can be pasted straight into a chat.
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// `build/screenshots` under the working directory (the repo root for
/// `cargo run`), or `WARP_SCREENSHOTS` when set.
pub fn directory() -> PathBuf {
    std::env::var_os("WARP_SCREENSHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("build/screenshots"))
}

/// Everything the shell knows about the frame; the caller adds `game`.
pub struct Shot<'a> {
    pub title: &'a str,
    pub size: [u32; 2],
    pub rgba: &'a [u8],
    pub metadata: Value,
}

/// Write `<name>.png` and `<name>.json` and copy the image. Returns the
/// line for the toast. A clipboard or JSON failure keeps the PNG.
pub fn save(shot: Shot) -> Result<String, String> {
    save_with(shot, |_| Ok(None))
}

/// A game's optional replay shares the screenshot's basename. A replay
/// failure is visible in both metadata and the toast; it never loses the PNG.
pub fn save_with(shot: Shot, replay: impl FnOnce(&std::path::Path) -> Result<Option<Value>, String>) -> Result<String, String> {
    let [w, h] = shot.size;
    if w == 0 || h == 0 || shot.rgba.len() != (w * h * 4) as usize { return Err("bad screenshot size".into()); }
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis();
    let slug: String = shot.title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let name = format!("{}-{ms}", slug.trim_matches('-'));
    let dir = directory();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let png = dir.join(format!("{name}.png"));
    render::write_png(&png, w, h, shot.rgba)?;
    let png = std::fs::canonicalize(&png).unwrap_or(png);
    let copied = crate::clipboard::set_image(w, h, shot.rgba);
    let mut metadata = shot.metadata;
    metadata["capture"] = json!({
        "png": png, "size": shot.size, "timestamp_unix_ms": ms as u64, "clipboard_image": copied,
        "pixels": "the presented frame including the HUD, sRGB RGBA8",
        "executable": std::env::current_exe().ok(), "args": std::env::args().collect::<Vec<_>>(),
    });
    let replay_note = match replay(&png) {
        Ok(Some(value)) => { metadata["replay"] = value; "  + gameplay replay" }
        Ok(None) => "",
        Err(e) => { eprintln!("replay: {e}"); metadata["replay_error"] = Value::String(e); "  (replay unavailable)" }
    };
    let json_path = png.with_extension("json");
    let wrote = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string()).and_then(|b| std::fs::write(&json_path, b).map_err(|e| e.to_string()));
    if let Err(e) = &wrote { eprintln!("screenshot metadata: {e}"); }
    note(shot.title, &png, wrote.is_ok().then_some(&json_path), ms);
    println!("screenshot: {}", png.display());
    Ok(format!("{name}.png{replay_note}{}{}", if copied { "  copied, paste with Cmd/Ctrl+V" } else { "  saved" }, if wrote.is_err() { "  (no metadata)" } else { "" }))
}

/// Note the capture where the `feedback` live tool (live::feedback) finds
/// it: `feedback.jsonl` in `WARP_LIVE_DIR`, else `<temp>/warp-live`.
fn note(game: &str, png: &std::path::Path, json: Option<&PathBuf>, ms: u128) {
    let dir = std::env::var_os("WARP_LIVE_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("warp-live"));
    let line = json!({ "game": game, "png": png, "json": json, "unix_ms": ms as u64, "time": crate::manners::stamp() });
    let written = std::fs::create_dir_all(&dir).and_then(|_| {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("feedback.jsonl"))?;
        writeln!(f, "{line}")
    });
    if let Err(e) = written { eprintln!("screenshot feedback index: {e}"); }
}

#[cfg(test)]
mod tests {
    #[test]
    fn saves_png_and_metadata() {
        crate::clipboard::memory_only();
        let dir = std::env::temp_dir().join(format!("warp-shot-{}", std::process::id()));
        unsafe { std::env::set_var("WARP_SCREENSHOTS", &dir); std::env::set_var("WARP_LIVE_DIR", &dir); }
        let line = super::save(super::Shot { title: "Terrain fly", size: [2, 1], rgba: &[255, 0, 0, 255, 0, 255, 0, 255], metadata: serde_json::json!({"state": {"seed": 3}}) }).unwrap();
        assert!(line.starts_with("terrain-fly-") && line.contains("saved"));
        let json = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).find(|p| p.extension().is_some_and(|e| e == "json")).unwrap();
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
        assert_eq!(meta["state"]["seed"], 3);
        assert!(std::path::Path::new(meta["capture"]["png"].as_str().unwrap()).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
