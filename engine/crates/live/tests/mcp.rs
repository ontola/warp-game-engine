//! The server end to end against a fake game: MCP handshake, tool listing
//! and calls over real HTTP on a random port, with the game thread draining
//! requests the way a game loop does.
use live::{Config, Content, Features, LiveGame, Manifest, Reload, Reply, Root, SoundHook, SoundInfo, ToolSpec};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Fake { jumps: u32, written: Vec<String>, volume: f32 }

impl LiveGame for Fake {
    fn state(&mut self) -> Result<Value, String> { Ok(json!({ "jumps": self.jumps, "players": [{ "health": 100 }] })) }
    fn call(&mut self, tool: &str, args: &Value) -> Reply {
        match tool {
            "jump" => { self.jumps += args["times"].as_u64().unwrap_or(1) as u32; Ok(vec![Content::Text(format!("jumped, {} total", self.jumps))]) }
            _ => Err(format!("no tool {tool}")),
        }
    }
    fn luau(&mut self, code: &str) -> Result<String, String> { Ok(format!("ran {code}")) }
    fn file_written(&mut self, root: &str, path: &str) -> Option<String> { self.written.push(format!("{root}/{path}")); Some(format!("reloaded {path}: ok")) }
    fn sound(&mut self) -> Option<&mut dyn SoundHook> { Some(self) }
}

impl SoundHook for Fake {
    fn sounds(&mut self) -> Vec<SoundInfo> { vec![SoundInfo { name: "shot".into(), params: vec![("damage".into(), 1.0)], about: String::new() }] }
    fn play(&mut self, name: &str, params: &[(String, f32)], _at: Option<[f32; 3]>) -> Result<String, String> { Ok(format!("played {name} {params:?}")) }
    fn set_param(&mut self, _: &str, _: &str, v: f32) -> Result<String, String> { self.volume = v; Ok("set".into()) }
}

/// One HTTP/1.1 request; returns status, headers and body.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers { req += &format!("{k}: {v}\r\n"); }
    req += "\r\n";
    req += body;
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, head.to_string(), body.to_string())
}

#[test]
fn mcp_handshake_tools_and_calls() {
    let dir = std::env::temp_dir().join(format!("live-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    let mut cfg = Config::new("fake");
    cfg.port = 0;
    cfg.capture_console = false;
    cfg.announce = false;
    cfg.token_file = Some(dir.join("fake.json"));
    let manifest = Manifest {
        name: "fake".into(), version: "1".into(), about: "A fake game.".into(), guide: "Jump.".into(),
        tools: vec![ToolSpec::new("jump", "Jump some times.", json!({ "type": "object", "properties": { "times": { "type": "integer" } } }))],
        roots: vec![Root::new("mods", dir.join("mods"), Reload::Game, "Luau mods").extensions(&["luau"])],
        features: Features { state: true, screenshot: true, luau: true, shaders: true, sound: true },
    };
    let mut server = live::start(cfg, manifest).unwrap();
    let port: u16 = server.url().rsplit(':').next().unwrap().trim_end_matches("/mcp").parse().unwrap();
    let token = server.token().to_string();
    let info: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("fake.json")).unwrap()).unwrap();
    assert_eq!(info["token"], token.as_str());

    // The game loop: drain once per "tick", finish frame work on "frames".
    let stop = Arc::new(AtomicBool::new(false));
    let game = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut fake = Fake { jumps: 0, written: Vec::new(), volume: 0.0 };
            while !stop.load(Ordering::Relaxed) {
                server.serve(&mut fake);
                if server.wants_capture() { server.captured([128, 64], vec![200; 128 * 64 * 4]); }
                if server.wants_shader_reload() { server.shaders_reloaded(Err("shade.wgsl:3: expected ';'".into())); }
                std::thread::sleep(std::time::Duration::from_millis(4));
            }
            (fake.jumps, fake.written, fake.volume)
        })
    };

    let auth = format!("Bearer {token}");
    let rpc = |id: u32, method: &str, params: Value| -> Value {
        let (status, _, body) = http(port, "POST", "/mcp", &[("Authorization", &auth)], &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string());
        assert_eq!(status, 200, "{body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["id"], id);
        v
    };
    let tool = |id: u32, name: &str, args: Value| -> Value { rpc(id, "tools/call", json!({ "name": name, "arguments": args }))["result"].clone() };
    let text = |r: &Value| r["content"][0]["text"].as_str().unwrap_or("").to_string();

    // Security: no token, wrong token, foreign Host and Origin are refused.
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } } }).to_string();
    assert_eq!(http(port, "POST", "/mcp", &[], &init).0, 401);
    assert_eq!(http(port, "POST", "/mcp", &[("Authorization", "Bearer nope")], &init).0, 401);
    assert_eq!(http(port, "POST", "/mcp", &[("Authorization", &auth), ("Origin", "https://evil.example")], &init).0, 403);
    {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "POST /mcp HTTP/1.1\r\nHost: rebind.example:{port}\r\nAuthorization: {auth}\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    }
    assert_eq!(http(port, "GET", "/mcp", &[("Authorization", &auth)], "").0, 405);

    // Handshake.
    let (status, head, body) = http(port, "POST", "/mcp", &[("Authorization", &auth)], &init);
    assert_eq!(status, 200);
    assert!(head.to_ascii_lowercase().contains("mcp-session-id:"), "{head}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    assert!(v["result"]["instructions"].as_str().unwrap().contains("fake"));
    let (status, _, _) = http(port, "POST", "/mcp", &[("Authorization", &auth)], r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert_eq!(status, 202);
    // The token also works in the path, for clients that cannot set headers.
    assert_eq!(http(port, "POST", &format!("/mcp/{token}"), &[], r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#).0, 200);

    let list = rpc(2, "tools/list", json!({}));
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["describe", "state", "query", "screenshot", "run_luau", "list_files", "read_file", "write_file", "reload_shaders", "logs", "play_sound", "set_param", "jump"] {
        assert!(names.contains(&n), "{n} missing from {names:?}");
    }
    assert!(list["result"]["tools"].as_array().unwrap().iter().all(|t| t["inputSchema"]["type"] == "object"));

    assert!(text(&tool(3, "describe", json!({}))).contains("## Folders"));
    let r = tool(4, "jump", json!({ "times": 2 }));
    assert_eq!((text(&r).as_str(), &r["isError"]), ("jumped, 2 total", &json!(false)));
    let r = tool(5, "state", json!({}));
    assert_eq!(r["structuredContent"]["jumps"], 2);
    let r = tool(6, "query", json!({ "pointers": ["/players/0/health", "jumps", "/nope"] }));
    assert_eq!(r["structuredContent"], json!({ "/players/0/health": 100, "/jumps": 2, "/nope": null }));
    let r = tool(7, "screenshot", json!({ "max_width": 64 }));
    assert_eq!(r["content"][1]["type"], "image");
    assert!(text(&r).contains("128x64, sent at 64x32"), "{}", text(&r));
    let png = base64_decode(r["content"][1]["data"].as_str().unwrap());
    assert_eq!(&png[1..4], b"PNG");
    assert_eq!(text(&tool(8, "run_luau", json!({ "code": "1+1" }))), "ran 1+1");
    let r = tool(9, "write_file", json!({ "path": "mods/live/test.luau", "content": "return {}" }));
    assert_eq!(r["content"][1]["text"], "reloaded live/test.luau: ok");
    assert_eq!(text(&tool(10, "read_file", json!({ "path": "mods/live/test.luau" }))), "return {}");
    assert_eq!(tool(11, "write_file", json!({ "path": "mods/../x.luau", "content": "" }))["isError"], true);
    let r = tool(12, "reload_shaders", json!({}));
    assert_eq!(r["isError"], true);
    assert!(text(&r).contains("expected ';'"), "compiler errors come back: {}", text(&r));
    assert!(text(&tool(13, "play_sound", json!({}))).contains("\"damage\""));
    assert!(text(&tool(14, "play_sound", json!({ "name": "shot", "params": { "damage": 3 } }))).contains("played shot"));
    tool(15, "set_param", json!({ "sound": "shot", "param": "damage", "value": 0.5 }));
    let logs = text(&tool(16, "logs", json!({ "filter": "jump" })));
    assert!(logs.contains("jump {\"times\":2} -> ok"), "{logs}");
    // Unknown tools are protocol errors; unknown methods too.
    assert_eq!(rpc(17, "tools/call", json!({ "name": "nope" }))["error"]["code"], -32602);
    assert_eq!(rpc(18, "sampling/createMessage", json!({}))["error"]["code"], -32601);

    stop.store(true, Ordering::Relaxed);
    let (jumps, written, volume) = game.join().unwrap();
    assert_eq!((jumps, written, volume), (2, vec!["mods/live/test.luau".to_string()], 0.5));
    // The game has stopped: calls fail cleanly instead of hanging.
    let (status, _, body) = http(port, "POST", "/mcp", &[("Authorization", &auth)], &json!({ "jsonrpc": "2.0", "id": 20, "method": "tools/call", "params": { "name": "state" } }).to_string());
    assert_eq!(status, 200);
    assert!(body.contains("the game has stopped"), "{body}");
    assert!(!dir.join("fake.json").exists(), "the token file goes with the game");
    let _ = std::fs::remove_dir_all(&dir);
}

fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' => 62, _ => 63 };
    let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=').map(val).collect();
    bytes.chunks(4).flat_map(|c| {
        let n = c.iter().enumerate().fold(0u32, |n, (i, &v)| n | (v as u32) << (18 - 6 * i));
        (0..c.len().saturating_sub(1)).map(move |i| (n >> (16 - 8 * i)) as u8)
    }).collect()
}

/// The stdio bridge forwards to the HTTP endpoint, and answers alone (with
/// warp_status) when no game is running.
#[test]
fn stdio_bridge_forwards() {
    use std::io::BufRead;
    let mut cfg = Config::new("bridge-test");
    cfg.port = 0;
    cfg.capture_console = false;
    cfg.announce = false;
    cfg.token_file = Some(std::env::temp_dir().join(format!("live-bridge-{}.json", std::process::id())));
    let manifest = Manifest { name: "bridge-test".into(), features: Features { state: true, ..Default::default() }, ..Default::default() };
    let mut server = live::start(cfg, manifest).unwrap();
    let (url, token) = (server.url().to_string(), server.token().to_string());
    let stop = Arc::new(AtomicBool::new(false));
    let game = { let stop = stop.clone(); std::thread::spawn(move || {
        let mut fake = Fake { jumps: 7, written: Vec::new(), volume: 0.0 };
        while !stop.load(Ordering::Relaxed) { server.serve(&mut fake); std::thread::sleep(std::time::Duration::from_millis(4)); }
    }) };
    let talk = |args: &[&str], lines: &[Value]| -> Vec<Value> {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_warp-mcp")).args(args)
            .env("WARP_LIVE_DIR", std::env::temp_dir().join(format!("live-bridge-empty-{}", std::process::id())))
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut replies = Vec::new();
        for l in lines {
            writeln!(stdin, "{l}").unwrap();
            if l.get("id").is_none() { continue; }
            let mut line = String::new();
            out.read_line(&mut line).unwrap();
            replies.push(serde_json::from_str(&line).unwrap());
        }
        drop(stdin);
        child.wait().unwrap();
        replies
    };
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } });
    let r = talk(&["--url", &url, "--token", &token], &[init.clone(), json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "query", "arguments": { "pointer": "/jumps" } } })]);
    assert_eq!(r[0]["result"]["serverInfo"]["name"], "warp-live-bridge-test");
    assert_eq!(r[0]["result"]["capabilities"]["tools"]["listChanged"], true);
    assert_eq!(r[1]["result"]["structuredContent"]["/jumps"], 7);
    stop.store(true, Ordering::Relaxed);
    game.join().unwrap();
    // No game: the bridge still initializes and explains how to start one.
    let r = talk(&[], &[init, json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })]);
    assert_eq!(r[0]["result"]["serverInfo"]["name"], "warp-mcp");
    assert_eq!(r[1]["result"]["tools"][0]["name"], "warp_status");
}
