//! The real game headless under live control: `growing-guns serve --live 0`
//! on the software rasteriser (no window), driven over MCP by a small
//! client. Needs a GPU adapter, so it is opt-in:
//!
//!   KERN_ADAPTER=software cargo test -p growing-guns-app --test live -- --ignored
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn http(port: u16, token: &str, body: &str) -> Value {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(60))).unwrap();
    write!(s, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let body = out.split_once("\r\n\r\n").map_or("", |b| b.1);
    if body.is_empty() { Value::Null } else { serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {out}")) }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() { copy_dir(&p, &to.join(e.file_name())); } else { std::fs::copy(&p, to.join(e.file_name())).unwrap(); }
    }
}

struct Kill(std::process::Child);
impl Drop for Kill { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

#[test]
#[ignore = "runs the game headless on a GPU adapter (KERN_ADAPTER=software works)"]
fn headless_game_under_live_control() {
    let game_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let tmp = std::env::temp_dir().join(format!("gg-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    // Mods and patches are copies: the test writes into them.
    copy_dir(&game_dir.join("mods"), &tmp.join("mods"));
    copy_dir(&game_dir.join("sounds"), &tmp.join("sounds"));
    copy_dir(&game_dir.join("../../engine/crates/render/src/shaders"), &tmp.join("shaders"));
    let token_file = tmp.join("token.json");
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_growing-guns"))
        .args(["serve", "--live", "0", "--live-token-file", token_file.to_str().unwrap(), "--seconds", "600", "--size", "480x270", "--seed", "5"])
        .env("GG_MODS", tmp.join("mods")).env("GG_SFX", tmp.join("sounds")).env("KERN_SHADER_DIR", tmp.join("shaders"))
        .env("KERN_ADAPTER", std::env::var("KERN_ADAPTER").unwrap_or_else(|_| "software".into()))
        .stdout(std::process::Stdio::inherit()).spawn().unwrap();
    let _child = Kill(child);
    let start = std::time::Instant::now();
    let info: Value = loop {
        if let Ok(t) = std::fs::read_to_string(&token_file) && let Ok(v) = serde_json::from_str(&t) { break v; }
        assert!(start.elapsed().as_secs() < 300, "the game did not start serving");
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let token = info["token"].as_str().unwrap().to_string();
    let port: u16 = info["url"].as_str().unwrap().trim_end_matches("/mcp").rsplit(':').next().unwrap().parse().unwrap();
    let mut id = 0;
    let mut rpc = |method: &str, params: Value| -> Value {
        id += 1;
        let r = http(port, &token, &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string());
        assert!(r.get("error").is_none(), "{method}: {r}");
        r["result"].clone()
    };
    let init = rpc("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "gg-test", "version": "0" } }));
    assert_eq!(init["serverInfo"]["name"], "warp-live-growing-guns");
    http(port, &token, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let tools = rpc("tools/list", json!({}));
    let names: Vec<String> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    for n in ["describe", "match_state", "give_card", "set_weapon", "spawn_bots", "next_round", "restart_match", "load_mod", "screenshot", "run_luau", "play_sound", "write_file"] {
        assert!(names.iter().any(|x| x == n), "{n} missing: {names:?}");
    }
    let mut call = |name: &str, args: Value| -> Value {
        let r = rpc("tools/call", json!({ "name": name, "arguments": args }));
        println!("{name}: {}", r["content"][0]["text"].as_str().unwrap_or("").chars().take(300).collect::<String>());
        r
    };
    let ok = |r: &Value| {
        assert_eq!(r["isError"], false, "{r}");
        r["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
    };
    let state = |r: &Value| r["structuredContent"].clone();

    let s = call("match_state", json!({}));
    assert_eq!(state(&s)["players"].as_array().unwrap().len(), 4);
    assert_eq!(state(&s)["session"], "local");
    ok(&call("give_card", json!({ "player": 0, "card": "explosive" })));
    ok(&call("set_weapon", json!({ "player": 1, "fields": { "damage_mult": 3, "special": "ion_cannon", "bullet_color": [1, 0, 0] } })));
    assert!(call("set_weapon", json!({ "player": 1, "fields": { "nope": 1 } }))["isError"] == true);
    let s = state(&call("match_state", json!({ "player": 1 })));
    assert!(s["players"][0]["cards"].as_array().unwrap().contains(&json!("explosive")));
    assert_eq!((s["players"][1]["weapon"]["damage_mult"].clone(), s["players"][1]["special"].clone()), (json!(3.0), json!("ion_cannon")));
    ok(&call("spawn_bots", json!({ "count": 2 })));
    let round = state(&call("match_state", json!({})))["round"].as_u64().unwrap();
    ok(&call("next_round", json!({})));
    let s = state(&call("match_state", json!({})));
    assert_eq!((s["players"].as_array().unwrap().len(), s["round"].as_u64().unwrap()), (6, round + 1));
    assert!(s["players"][0]["cards"].as_array().unwrap().contains(&json!("explosive")), "cards survive a round");
    assert_eq!(ok(&call("run_luau", json!({ "code": "g:players()" }))), "6");

    // Mods: a hook that prints, then a broken module, which reports its error.
    let r = ok(&call("load_mod", json!({ "name": "live/test", "source": "return { round_start = function(g) print('live mod round ' .. g:round()) end }" })));
    assert!(r.contains("live/test.luau: ok"), "{r}");
    let r = ok(&call("load_mod", json!({ "name": "live/broken", "source": "return { tick = function(g) end" })));
    assert!(r.contains("live/broken.luau: not loaded") && r.contains("Expected '}'"), "{r}");
    ok(&call("restart_match", json!({})));
    let s = state(&call("match_state", json!({})));
    assert_eq!(s["round"], 1);
    assert!(s["players"][0]["cards"].as_array().unwrap().is_empty());
    let mut logs = String::new();
    for _ in 0..20 {
        logs = ok(&call("logs", json!({ "filter": "live mod round" })));
        if logs.contains("live mod round 1") { break; }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(logs.contains("live mod round 1"), "console output reaches logs: {logs}");

    let shot = call("screenshot", json!({ "max_width": 480 }));
    ok(&shot);
    assert_eq!(shot["content"][1]["type"], "image");
    let png = shot["content"][1]["data"].as_str().unwrap();
    assert!(png.len() > 1000, "a real frame");
    let sounds = ok(&call("play_sound", json!({})));
    assert!(sounds.contains("\"shot\""), "{sounds}");
    let r = ok(&call("play_sound", json!({ "name": "shot", "params": { "damage": 3 } })));
    assert!(r.contains("playing shot") && r.contains("damage=3"), "{r}");
    assert!(call("play_sound", json!({ "name": "shot", "params": { "nope": 1 } }))["isError"] == true);
    let r = ok(&call("render_sound", json!({ "name": "shot" })));
    assert!(r.contains("peak") && r.contains("centroid") && r.contains("20.0k |"), "a level report and a spectrogram: {r}");
    let r = ok(&call("set_param", json!({ "sound": "shot", "param": "damage", "value": 2 })));
    assert!(r.contains("1 -> 2"), "{r}");
    assert!(ok(&call("play_sound", json!({ "name": "shot" }))).contains("damage=2"), "the new default");
    // A patch edit is live at once; a broken one keeps the last good version.
    let shot = std::fs::read_to_string(tmp.join("sounds/shot.luau")).unwrap();
    let r = ok(&call("write_file", json!({ "path": "sounds/shot.luau", "content": format!("{shot}\n-- live edit\n") })));
    assert!(r.contains("shot: loaded"), "{r}");
    let r = ok(&call("write_file", json!({ "path": "sounds/shot.luau", "content": "return {" })));
    assert!(r.contains("load error"), "{r}");
    std::fs::write(tmp.join("sounds/shot.luau"), &shot).unwrap();
    let r = ok(&call("write_file", json!({ "path": "mods/live/test2.luau", "content": "return {}" })));
    assert!(r.contains("test2.luau: ok"), "{r}");

    // Shaders: unchanged, broken (the old ones stay live), then edited.
    let r = ok(&call("reload_shaders", json!({})));
    assert!(r.contains("unchanged"), "{r}");
    let shade = std::fs::read_to_string(tmp.join("shaders/shade.wgsl")).unwrap();
    let r = call("write_file", json!({ "path": "shaders/shade.wgsl", "content": format!("{shade}\nfn broken( {{") }));
    let text = r["content"].to_string();
    assert!(text.contains("shader reload failed"), "{text}");
    let r = ok(&call("write_file", json!({ "path": "shaders/shade.wgsl", "content": format!("{shade}\n// live edit\n") })));
    assert!(r.contains("shaders reloaded"), "{r}");
    ok(&call("screenshot", json!({})));
    let _ = std::fs::remove_dir_all(&tmp);
}
