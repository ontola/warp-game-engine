//! F12 feedback for Claude: every Warp game saves the frame as shown plus a
//! JSON file of diagnostics on F12 (`app::screenshot`) and notes it in
//! `<token dir>/feedback.jsonl`. The `feedback` tool reads the newest ones
//! back, image and diagnostics together, from any game or none running.
use crate::{Content, Reply};
use serde_json::{Value, json};
use std::path::PathBuf;

/// One line per F12 capture, newest last.
pub fn index() -> PathBuf { crate::token_dir().join("feedback.jsonl") }

/// The tool's MCP listing.
pub fn tool() -> Value {
    json!({ "name": "feedback", "description": "The newest F12 feedback captures from any Warp game: the frame as the player saw it (with HUD) and its diagnostics JSON (game state, camera, GPU, fps). For Sideways, use the JSON beside the PNG with warp inspect F12.json --pixel X Y (original PNG coordinates) or warp compare F12.json --baseline main (replay the drive against a baseline). Growing Guns desktop captures include replay.input_history for inputs and live edits. Set diagnostics=false to locate the files without returning their JSON.",
        "inputSchema": { "type": "object", "properties": {
            "count": { "type": "integer", "description": "how many, newest first (default 1, at most 5)" },
            "max_width": { "type": "integer", "description": "downscale images to this width (default 1280)" },
            "diagnostics": { "type": "boolean", "description": "include the JSON (default true)" } }, "required": [] },
        "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false } })
}

/// The `feedback` tool.
pub fn reply(args: &Value) -> Reply {
    let count = args.get("count").and_then(Value::as_u64).unwrap_or(1).clamp(1, 5) as usize;
    let max = args.get("max_width").and_then(Value::as_u64).unwrap_or(1280).clamp(64, 4096) as u32;
    let diagnostics = args.get("diagnostics").and_then(Value::as_bool).unwrap_or(true);
    let text = std::fs::read_to_string(index()).unwrap_or_default();
    let entries: Vec<Value> = text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(count).collect();
    if entries.is_empty() { return Err(format!("no F12 captures yet: press F12 in a Warp game (index {})", index().display())); }
    let mut out = Vec::new();
    for e in entries {
        let png = e["png"].as_str().unwrap_or_default();
        out.push(Content::Text(format!("{} at {} (unix ms {}): {png}", e["game"].as_str().unwrap_or("game"), e["time"].as_str().unwrap_or("?"), e["unix_ms"])));
        match std::fs::read(png).map_err(|err| err.to_string()).and_then(|bytes| shrink(&bytes, max)) {
            Ok(p) => out.push(Content::Png(p)),
            Err(err) => out.push(Content::Text(format!("image unavailable: {err}"))),
        }
        if diagnostics && let Some(json) = e["json"].as_str() {
            let mut t = std::fs::read_to_string(json).unwrap_or_else(|err| format!("diagnostics unavailable: {err}"));
            if t.len() > 60_000 { t.truncate(t.floor_char_boundary(60_000)); t.push_str("\n… (cut; read the file for the rest)"); }
            out.push(Content::Text(t));
        }
    }
    Ok(out)
}

/// The `feedback` tool's MCP `tools/call` result (for warp-mcp).
pub fn mcp_result(args: &Value) -> Value { crate::server::to_mcp(reply(args)) }

fn shrink(bytes: &[u8], max: u32) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    if img.width() <= max { return Ok(bytes.to_vec()); }
    let h = ((img.height() as u64 * max as u64) / img.width() as u64).max(1) as u32;
    let img = image::imageops::resize(&img, max, h, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(&img, img.width(), img.height(), image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn newest_captures_come_back_with_their_diagnostics() {
        let dir = std::env::temp_dir().join(format!("warp-feedback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("WARP_LIVE_DIR", &dir); }
        assert!(super::reply(&serde_json::json!({})).is_err(), "nothing yet");
        let png = dir.join("shot.png");
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        std::fs::write(&png, bytes).unwrap();
        std::fs::write(png.with_extension("json"), r#"{"fps":119}"#).unwrap();
        let line = serde_json::json!({"game":"sideways","png":png,"json":png.with_extension("json"),"unix_ms":1,"time":"20:00:00.000"});
        std::fs::write(super::index(), format!("{line}\n")).unwrap();
        let reply = super::reply(&serde_json::json!({})).unwrap();
        assert!(matches!(&reply[1], crate::Content::Png(_)));
        assert!(matches!(&reply[2], crate::Content::Text(t) if t.contains("119")));
    }
}
