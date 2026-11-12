//! The `warp` executable headless on a GPU adapter, never a window:
//! games/hello-warp's smoke script (frames checked), and `warp serve
//! --live` driven over MCP. KERN_ADAPTER defaults to `software` (a CPU
//! rasteriser) for the child processes.
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..") }

fn folder(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("warp-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn warp() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_warp"));
    c.env("KERN_ADAPTER", std::env::var("KERN_ADAPTER").unwrap_or_else(|_| "software".into()));
    c
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() { copy_dir(&p, &to.join(e.file_name())); } else { std::fs::copy(&p, to.join(e.file_name())).unwrap(); }
    }
}

#[test]
fn hello_warp_smoke_script_runs_headless() {
    let out = folder("smoke");
    let game = repo().join("games/hello-warp");
    let r = warp().arg("script").arg(&game).arg(game.join("scripts/smoke.json")).arg("--out").arg(&out).output().unwrap();
    let stdout = String::from_utf8_lossy(&r.stdout);
    assert!(r.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&r.stderr));
    assert!(stdout.contains(r#""failed":0"#), "{stdout}");
    let state: Value = serde_json::from_str(&std::fs::read_to_string(out.join("hello.json")).unwrap()).unwrap();
    assert_eq!(state["state"]["runners"][0]["score"], 1);
    // The frame: a sky above, the green floor below, the red capsule in the middle, the HUD's dark box.
    let img = image::open(out.join("hello.png")).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (640, 360));
    let px = |x, y| { let p = img.get_pixel(x, y); [p[0] as i32, p[1] as i32, p[2] as i32] };
    let sky = px(320, 10);
    assert!(sky[2] > sky[0] && sky[2] > 120, "sky at the top: {sky:?}");
    let floor = px(320, 330);
    assert!(floor[1] > floor[0] && floor[1] > floor[2], "green floor at the bottom: {floor:?}");
    let reddish = (130..230).flat_map(|y| (250..390).map(move |x| (x, y))).filter(|&(x, y)| { let p = px(x, y); p[0] > 150 && p[0] > p[1] + 60 }).count();
    assert!(reddish > 300, "the capsule is in the middle ({reddish} red pixels)");
    let wav = std::fs::metadata(out.join("pickup-and-jump.wav")).unwrap().len();
    assert!(wav > 100_000, "sound was recorded ({wav} bytes)");
}

fn http(port: u16, token: &str, body: &str) -> Value {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(120))).unwrap();
    write!(s, "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let body = out.split_once("\r\n\r\n").map_or("", |b| b.1);
    if body.is_empty() { Value::Null } else { serde_json::from_str(body).unwrap_or_else(|e| panic!("{e}: {out}")) }
}

struct Kill(std::process::Child);
impl Drop for Kill { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }

#[test]
fn serve_under_live_control() {
    let tmp = folder("live");
    let game = tmp.join("hello");
    copy_dir(&repo().join("games/hello-warp"), &game);
    let token_file = tmp.join("token.json");
    let child = warp().arg("serve").arg(&game).args(["--live", "0", "--live-token-file", token_file.to_str().unwrap(), "--seconds", "300", "--size", "480x270"])
        .env("WARP_LIVE_CONSOLE", "0").spawn().unwrap();
    let _child = Kill(child);
    let start = std::time::Instant::now();
    let info: Value = loop {
        if let Ok(t) = std::fs::read_to_string(&token_file) && let Ok(v) = serde_json::from_str(&t) { break v; }
        assert!(start.elapsed().as_secs() < 120, "warp serve did not start");
        std::thread::sleep(std::time::Duration::from_millis(100));
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
    let init = rpc("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "warp-test", "version": "0" } }));
    assert_eq!(init["serverInfo"]["name"], "warp-live-hello");
    http(port, &token, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let tools = rpc("tools/list", json!({}));
    let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["describe", "state", "query", "screenshot", "run_luau", "write_file", "edit_file", "read_file", "find_text", "logs", "play_sound", "set_param", "render_sound", "reload", "restart", "command"] {
        assert!(names.contains(&n), "{n} missing: {names:?}");
    }
    assert!(!names.contains(&"reload_shaders"));
    let mut call = |name: &str, args: Value| -> Value { rpc("tools/call", json!({ "name": name, "arguments": args })) };
    let text = |r: &Value| -> String {
        assert_eq!(r["isError"], false, "{r}");
        r["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("\n")
    };

    let s = call("state", json!({}));
    assert_eq!(s["structuredContent"]["errors"], json!([]), "{s}");
    assert!(text(&call("describe", json!({}))).contains("main.luau"));
    // Score a point through Luau, then edit main.luau: the score survives the reload.
    assert_eq!(text(&call("run_luau", json!({ "code": "state.runners[1].score = 41; return state.runners[1].score" }))), "41");
    let src = text(&call("read_file", json!({ "path": "game/main.luau" })));
    let edited = src.replace("local SPEED = 6 ", "state.edited = true\nlocal SPEED = 9 ");
    assert_ne!(src, edited);
    let w = text(&call("write_file", json!({ "path": "game/main.luau", "content": edited })));
    assert!(w.contains("main.luau: reloaded"), "{w}");
    let q = call("query", json!({ "pointer": "/state" }));
    let q = text(&q);
    assert!(q.contains("\"edited\":true") && q.contains("\"score\":41"), "{q}");
    // A broken edit is reported and the old code keeps running.
    let bad = text(&call("write_file", json!({ "path": "game/main.luau", "content": edited.replace("local function init()", "local function init(") })));
    assert!(bad.contains("main.luau:"), "{bad}");
    assert!(text(&call("reload", json!({}))).contains("nothing changed"));
    call("write_file", json!({ "path": "game/main.luau", "content": edited }));
    // A long file in parts: find a line, read around it, change only that.
    let found = text(&call("find_text", json!({ "text": "state.edited = true" })));
    assert!(found.starts_with("game/main.luau:"), "{found}");
    let line: u64 = found.split(':').nth(1).unwrap().parse().unwrap();
    assert!(text(&call("read_file", json!({ "path": "game/main.luau", "offset": line, "limit": 1 }))).contains("| state.edited = true"));
    let e = text(&call("edit_file", json!({ "path": "game/main.luau", "old": "state.edited = true", "new": "state.edited = 2" })));
    assert!(e.contains("main.luau: reloaded"), "{e}");
    assert!(text(&call("query", json!({ "pointer": "/state" }))).contains("\"edited\":2"));
    // Sounds: list, play, change a default, render offline.
    let sounds = text(&call("play_sound", json!({})));
    assert!(sounds.contains("pickup") && sounds.contains("bright"), "{sounds}");
    assert!(text(&call("play_sound", json!({ "name": "pickup", "params": { "bright": 2 } }))).contains("playing pickup"));
    assert!(text(&call("set_param", json!({ "sound": "pickup", "param": "bright", "value": 0.5 }))).contains("0.5"));
    let r = text(&call("render_sound", json!({ "name": "pickup" })));
    assert!(r.contains("peak") && r.contains("centroid"), "{r}");
    // A screenshot (rendered on demand), the restart, a command.
    let shot = call("screenshot", json!({ "max_width": 480 }));
    let png = shot["content"].as_array().unwrap().iter().find(|c| c["type"] == "image").expect("an image");
    assert_eq!(png["mimeType"], "image/png");
    assert!(png["data"].as_str().unwrap().len() > 2000);
    assert!(text(&call("restart", json!({}))).contains("restarted"));
    assert_eq!(text(&call("run_luau", json!({ "code": "state.runners[1].score" }))), "0");
    assert!(text(&call("command", json!({ "line": "sky night" }))).contains("night:clear"));
    let logs = text(&call("logs", json!({ "lines": 200 })));
    assert!(logs.contains("loaded main.luau"), "{logs}");
}

#[test]
fn draw_may_not_change_the_game() {
    let dir = folder("draw");
    std::fs::write(dir.join("main.luau"), r#"
return {
    init = function() world.spawn("cube", { pos = vector.create(0, -0.5, 0), size = vector.create(10, 1, 10) }) end,
    draw = function()
        hud.text("drawn", 10, 10)
        state.spawn = tostring(select(2, pcall(world.spawn, "cube")))
        state.random = tostring(select(2, pcall(random.float)))
        state.camera_ok = pcall(camera.set, 1, { fov = 50 })
        state.draws = (state.draws or 0) + 1
    end,
}"#).unwrap();
    let script = dir.join("s.json");
    std::fs::write(&script, r#"{ "size": [160, 90], "steps": [
        { "ticks": 3 }, { "expect": "/state/draws", "equals": null, "note": "a script draws only for captures" },
        { "capture": "f.png" }, { "expect": "/state/draws", "equals": 1 }, { "expect": "/state/camera_ok", "equals": true },
        { "expect": "/entities", "equals": 1 }, { "state": "s.json" } ] }"#).unwrap();
    let r = warp().arg("script").arg(&dir).arg(&script).arg("--out").arg(dir.join("out")).output().unwrap();
    assert!(r.status.success(), "{}\n{}", String::from_utf8_lossy(&r.stdout), String::from_utf8_lossy(&r.stderr));
    let s: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("out/s.json")).unwrap()).unwrap();
    assert!(s["state"]["spawn"].as_str().unwrap().contains("not in draw"), "{s}");
    assert!(s["state"]["random"].as_str().unwrap().contains("not in draw"), "{s}");
}

#[test]
fn finish_reads_the_visible_part_and_clamps_samples_to_each_view() {
    let dir = folder("game-finish");
    std::fs::write(dir.join("game.toml"), "players = 2\nsky = \"gradient\"\n").unwrap();
    std::fs::write(dir.join("main.luau"), r#"
        local v = vector.create
        return {init=function()
            world.spawn("cube", {pos=v(0,0,0),size=v(40,20,.3),surface=1})
            world.spawn("cube", {pos=v(0,0,1),size=v(1,10,.1)})
            world.spawn("cube", {pos=v(100,0,0),size=v(40,20,.3),surface=2})
            for i,p in players.list() do
                local x=(i-1)*100
                camera.set(p.id,{pos=v(x,0,4),target=v(x,0,0),fov=45})
            end
            post.set({bloom=0})
        end}
    "#).unwrap();
    std::fs::write(dir.join("lighting.wgsl"), r#"
        fn game_lighting(s: LightingSample) -> StyledLight {
            let tag = select(select(0u, 1u, s.kind == 1), 2047u, s.kind == 2);
            return StyledLight(s.radiance, tag);
        }
    "#).unwrap();
    std::fs::write(dir.join("finish.wgsl"), r#"
        fn game_finish(px: vec2<i32>, s: FinishSample) -> vec3<f32> {
            let edge = scene_sample(vec2<i32>(-99999, px.y));
            if s.depth == 0 || s.tag == 0u { return vec3<f32>(0,10000,0); }
            if edge.tag != s.tag { return vec3<f32>(0); }
            if s.tag == 2047u { return vec3<f32>(0,0,10000); }
            return vec3<f32>(10000,0,0);
        }
    "#).unwrap();
    let script = dir.join("capture.json");
    std::fs::write(&script, r#"{"size":[401,219],"steps":[{"ticks":2},{"capture":"parts.png"}]}"#).unwrap();
    let run = warp().arg("script").arg(&dir).arg(script).arg("--out").arg(&dir).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let image = image::open(dir.join("parts.png")).unwrap().to_rgb8();
    let channel = |x, y, index: usize| {
        let rgb = image.get_pixel(x, y).0.map(i32::from);
        assert!(rgb[index] > rgb[(index+1)%3]+45 && rgb[index] > rgb[(index+2)%3]+45,
            "pixel ({x},{y}) expected channel {index}, got {rgb:?}");
    };
    channel(90, 50, 0);   // First view's tagged wall.
    channel(200, 50, 1);  // Untagged foreground covers that wall.
    channel(200, 164, 2); // Second view reads its own left edge.
}
