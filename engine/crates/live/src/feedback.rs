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
    json!({ "name": "feedback", "description": "The newest F12 feedback captures from any Warp game: the frame as the player saw it (with HUD) and its diagnostics JSON (game state, camera, GPU, fps). For Sideways, warp inspect F12.json reads the point selected by the player in the F12 overlay. Use warp inspect F12.json --pixel X Y for another original PNG pixel or warp compare F12.json --baseline main (replay the drive against a baseline). Growing Guns desktop captures include replay.input_history for inputs and live edits. Set diagnostics=false to locate the files without returning their JSON.",
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
        let metadata=e["json"].as_str().and_then(|p|std::fs::read_to_string(p).ok());
        #[derive(serde::Deserialize)]
        struct Selection { player_point: Option<[u32;2]> }
        let point=metadata.as_deref().and_then(|s|serde_json::from_str::<Selection>(s).ok()).and_then(|m|m.player_point);
        if let Some([x,y])=point {
            out.push(Content::Text(format!("Player marked original PNG pixel ({x}, {y}). The preview circles that point.")));
        }
        match std::fs::read(png).map_err(|err| err.to_string()).and_then(|bytes| shrink(&bytes, max, point)) {
            Ok(p) => out.push(Content::Png(p)),
            Err(err) => out.push(Content::Text(format!("image unavailable: {err}"))),
        }
        if diagnostics && let Some(json) = e["json"].as_str() {
            let mut t = metadata.unwrap_or_else(|| format!("diagnostics unavailable: {json}"));
            if t.len() > 60_000 { t.truncate(t.floor_char_boundary(60_000)); t.push_str("\n… (cut; read the file for the rest)"); }
            out.push(Content::Text(t));
        }
    }
    Ok(out)
}

/// The `feedback` tool's MCP `tools/call` result (for warp-mcp).
pub fn mcp_result(args: &Value) -> Value { crate::server::to_mcp(reply(args)) }

fn shrink(bytes: &[u8], max: u32, point: Option<[u32;2]>) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w,h)=img.dimensions();
    if w <= max && point.is_none() {return Ok(bytes.to_vec());}
    let mut img=if w>max {
        image::imageops::resize(&img,max,((u64::from(h)*u64::from(max))/u64::from(w)).max(1) as u32,image::imageops::FilterType::Triangle)
    } else {img};
    if let Some([x,y])=point.filter(|[x,y]|*x<w && *y<h) {
        let cx=((u64::from(x)*2+1)*u64::from(img.width())/(2*u64::from(w))) as i64;
        let cy=((u64::from(y)*2+1)*u64::from(img.height())/(2*u64::from(h))) as i64;
        for dy in -10..=10 {for dx in -10..=10 {
            let r=dx*dx+dy*dy;
            let (x,y)=(cx+dx,cy+dy);
            if (49..=90).contains(&r) && x>=0 && y>=0 && x<i64::from(img.width()) && y<i64::from(img.height()) {
                let c=if r<=69 {255}else{0};
                img.put_pixel(x as u32,y as u32,image::Rgba([c,c,c,255]));
            }
        }}
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(&img, img.width(), img.height(), image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn player_ring_preserves_the_marked_sample_after_downscaling() {
        use image::ImageEncoder;
        let mut bytes=Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes).write_image(&[90;64*64*4],64,64,image::ExtendedColorType::Rgba8).unwrap();
        let preview=image::load_from_memory(&super::shrink(&bytes,32,Some([32,32])).unwrap()).unwrap().to_rgba8();
        assert_eq!(preview.get_pixel(16,16).0,[90;4]);
        assert_eq!(preview.get_pixel(24,16).0,[255;4]);
    }
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
