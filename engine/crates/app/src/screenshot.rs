//! Engine feedback screenshots: the frame as shown (HUD included) as a PNG, a
//! JSON file next to it with what was on screen (the game's
//! [`crate::Game::state`], camera, GPU, fps), and the image on the clipboard
//! so it can be pasted straight into a chat. Shift+F4 then takes a crop of
//! that frame ([`save_crop`]): a dragged rectangle as `<name>-crop.png`.
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
    save_full(shot, replay).map(|saved| saved.line)
}

/// A saved frame: the toast line, its PNG and the metadata written beside it.
pub struct Saved { pub line: String, pub png: PathBuf, pub metadata: Value }

/// As [`save_with`], also returning where the frame went and its metadata
/// (for a crop of it).
pub fn save_full(shot: Shot, replay: impl FnOnce(&std::path::Path) -> Result<Option<Value>, String>) -> Result<Saved, String> {
    save_annotated(shot, replay, |_, _| Ok(()))
}

/// Add optional diagnostics after the PNG has a final path. Failure keeps the screenshot.
pub fn save_annotated(shot: Shot, replay: impl FnOnce(&std::path::Path) -> Result<Option<Value>, String>,
    annotate: impl FnOnce(&std::path::Path, &mut Value) -> Result<(), String>) -> Result<Saved, String> {
    let destination = Destination::new(shot.title)?;
    save_at(destination, shot, replay, annotate)
}

pub(crate) struct Destination {
    pub png: PathBuf,
    name: String,
    ms: u128,
}

impl Destination {
    pub fn new(title: &str) -> Result<Self, String> {
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis();
        let slug: String = title.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        let name = format!("{}-{ms}", slug.trim_matches('-'));
        let dir = directory();
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let png = std::fs::canonicalize(&dir).unwrap_or(dir).join(format!("{name}.png"));
        Ok(Self { png, name, ms })
    }
}

pub(crate) fn save_at(destination: Destination, shot: Shot, replay: impl FnOnce(&std::path::Path) -> Result<Option<Value>, String>,
    annotate: impl FnOnce(&std::path::Path, &mut Value) -> Result<(), String>) -> Result<Saved, String> {
    let Destination { png, name, ms } = destination;
    let [w, h] = shot.size;
    if w == 0 || h == 0 || shot.rgba.len() != (w * h * 4) as usize { return Err("bad screenshot size".into()); }
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
    let attachment_error = annotate(&png, &mut metadata).err();
    if let Some(e) = &attachment_error { metadata["attachment_error"] = json!(e); }
    let json_path = png.with_extension("json");
    let wrote = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string()).and_then(|b| std::fs::write(&json_path, b).map_err(|e| e.to_string()));
    if let Err(e) = &wrote { eprintln!("screenshot metadata: {e}"); }
    note(shot.title, &png, wrote.is_ok().then_some(&json_path), ms);
    println!("screenshot: {}", png.display());
    let line = format!("{name}.png{replay_note}{}{}", if copied { "  copied, paste with Cmd/Ctrl+V" } else { "  saved" }, if wrote.is_err() { "  (no metadata)" } else { "" });
    let line = if attachment_error.is_some() {format!("{line} (capture attachment failed)")} else {line};
    Ok(Saved { line, png, metadata })
}

/// The pixels of `rect` (x, y, width, height, clamped to the frame).
pub fn crop_rgba(rgba: &[u8], size: [u32; 2], rect: [u32; 4]) -> (Vec<u8>, [u32; 4]) {
    let [w, h] = size;
    let x = rect[0].min(w);
    let y = rect[1].min(h);
    let rect = [x, y, rect[2].min(w - x), rect[3].min(h - y)];
    let mut out = Vec::with_capacity((rect[2] * rect[3] * 4) as usize);
    for row in y..y + rect[3] {
        let start = ((row * w + x) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (rect[2] * 4) as usize]);
    }
    (out, rect)
}

/// Save `rect` of a frame [`save_full`] wrote as `<name>-crop.png` beside it,
/// with the same metadata plus the rectangle, and copy it.
pub fn save_crop(game: &str, full: &Saved, rgba: &[u8], size: [u32; 2], rect: [u32; 4]) -> Result<String, String> {
    let (pixels, rect) = crop_rgba(rgba, size, rect);
    if rect[2] == 0 || rect[3] == 0 { return Err("empty crop".into()); }
    let stem = full.png.file_stem().and_then(|s| s.to_str()).unwrap_or("screenshot").to_owned();
    let png = full.png.with_file_name(format!("{stem}-crop.png"));
    render::write_png(&png, rect[2], rect[3], &pixels)?;
    let copied = crate::clipboard::set_image(rect[2], rect[3], &pixels);
    let mut metadata = full.metadata.clone();
    metadata["crop"] = json!({ "png": png, "rect": rect, "full_png": full.png, "clipboard_image": copied });
    let json_path = png.with_extension("json");
    let wrote = serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string()).and_then(|b| std::fs::write(&json_path, b).map_err(|e| e.to_string()));
    if let Err(e) = &wrote { eprintln!("screenshot crop metadata: {e}"); }
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_millis();
    note(game, &png, wrote.is_ok().then_some(&json_path), ms);
    println!("screenshot crop: {}", png.display());
    Ok(format!("{stem}-crop.png  {}x{}{}", rect[2], rect[3], if copied { "  copied, paste with Cmd/Ctrl+V" } else { "  saved" }))
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
    /// The save tests point WARP_SCREENSHOTS at their own folders.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn saves_png_and_metadata() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn crop_takes_the_rectangle_and_clamps_it() {
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let (pixels, rect) = super::crop_rgba(&rgba, [4, 3], [1, 1, 2, 5]);
        assert_eq!(rect, [1, 1, 2, 2]);
        assert_eq!(pixels, [&rgba[20..28], &rgba[36..44]].concat());
    }

    #[test]
    fn saves_crop_beside_the_frame() {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        crate::clipboard::memory_only();
        let dir = std::env::temp_dir().join(format!("warp-crop-{}", std::process::id()));
        unsafe { std::env::set_var("WARP_SCREENSHOTS", &dir); std::env::set_var("WARP_LIVE_DIR", &dir); }
        let rgba = [255u8; 4 * 4 * 4];
        let full = super::save_full(super::Shot { title: "Terrain fly", size: [4, 4], rgba: &rgba, metadata: serde_json::json!({"state": 1}) }, |_| Ok(None)).unwrap();
        let line = super::save_crop("Terrain fly", &full, &rgba, [4, 4], [1, 1, 2, 2]).unwrap();
        assert!(line.contains("-crop.png  2x2"));
        let stem = full.png.file_stem().unwrap().to_str().unwrap().to_owned();
        let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(full.png.with_file_name(format!("{stem}-crop.json"))).unwrap()).unwrap();
        assert_eq!(meta["crop"]["rect"], serde_json::json!([1, 1, 2, 2]));
        assert_eq!(meta["state"], 1);
        assert!(full.png.exists() && full.png.with_file_name(format!("{stem}-crop.png")).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
