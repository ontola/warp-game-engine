//! MCP over Streamable HTTP (one endpoint, POST of JSON-RPC, JSON replies;
//! no server-initiated stream), with the bearer token, Host and Origin
//! checks. Runs on its own thread; game work goes through `Shared::jobs`.
use crate::{files, Answer, Content, Job, Logs, Manifest, Reload, Reply, Request};
use base64::Engine;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Response, StatusCode};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

pub(crate) struct Shared {
    pub manifest: Manifest,
    pub token: String,
    pub remote: bool,
    pub jobs: mpsc::UnboundedSender<Request>,
    pub logs: Logs,
    pub sessions: Mutex<HashSet<String>>,
}

/// Newest first; an unknown client version gets the newest.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_BODY: usize = 4 * 1024 * 1024;
const GAME_WAIT: Duration = Duration::from_secs(10);
const FRAME_WAIT: Duration = Duration::from_secs(30);

type Body = Full<Bytes>;

pub(crate) fn run(listener: std::net::TcpListener, shared: Arc<Shared>) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => { eprintln!("live: no runtime: {e}"); return; }
    };
    rt.block_on(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) { Ok(l) => l, Err(e) => { eprintln!("live: {e}"); return; } };
        loop {
            let Ok((stream, _)) = listener.accept().await else { tokio::time::sleep(Duration::from_millis(50)).await; continue };
            let shared = shared.clone();
            tokio::spawn(async move {
                let svc = hyper::service::service_fn(move |req| handle(req, shared.clone()));
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), svc).await;
            });
        }
    });
}

fn reply(status: StatusCode, content_type: &str, body: impl Into<Bytes>) -> Response<Body> {
    let mut r = Response::new(Full::new(body.into()));
    *r.status_mut() = status;
    r.headers_mut().insert(hyper::header::CONTENT_TYPE, content_type.parse().unwrap());
    r
}

fn text(status: StatusCode, msg: &str) -> Response<Body> { reply(status, "text/plain; charset=utf-8", msg.to_string()) }

fn loopback_name(host: &str) -> bool {
    let h = host.trim();
    let name = if let Some(rest) = h.strip_prefix('[') { rest.split(']').next().unwrap_or("") } else { h.rsplit_once(':').map_or(h, |(a, _)| a) };
    matches!(name.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "::1")
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn handle(req: hyper::Request<Incoming>, shared: Arc<Shared>) -> Result<Response<Body>, std::convert::Infallible> {
    // DNS rebinding: a web page must not reach the local server by name.
    if !shared.remote {
        let host = req.headers().get(hyper::header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
        if !loopback_name(host) { return Ok(text(StatusCode::FORBIDDEN, "live: Host must be localhost (start with --live-remote for tunnels)")); }
        if let Some(origin) = req.headers().get(hyper::header::ORIGIN).and_then(|h| h.to_str().ok()) {
            let host = origin.split_once("://").map_or(origin, |(_, h)| h);
            if !loopback_name(host) { return Ok(text(StatusCode::FORBIDDEN, "live: cross-origin requests are refused")); }
        }
    }
    let path = req.uri().path().to_string();
    let path_token = match path.as_str() {
        "/mcp" | "/mcp/" => None,
        p if p.starts_with("/mcp/") => Some(p["/mcp/".len()..].trim_end_matches('/').to_string()),
        "/" => return Ok(text(StatusCode::OK, "Warp live control: MCP at /mcp (see engine/docs/live-control.md)")),
        _ => return Ok(text(StatusCode::NOT_FOUND, "not found")),
    };
    let bearer = req.headers().get(hyper::header::AUTHORIZATION).and_then(|h| h.to_str().ok()).and_then(|h| h.strip_prefix("Bearer ")).map(str::trim);
    if !(bearer.is_some_and(|t| same(t, &shared.token)) || path_token.as_deref().is_some_and(|t| same(t, &shared.token))) {
        let mut r = text(StatusCode::UNAUTHORIZED, "live: missing or wrong bearer token (printed at startup and in the token file)");
        r.headers_mut().insert(hyper::header::WWW_AUTHENTICATE, "Bearer".parse().unwrap());
        return Ok(r);
    }
    match *req.method() {
        Method::POST => {}
        Method::DELETE => {
            if let Some(id) = req.headers().get("mcp-session-id").and_then(|h| h.to_str().ok()) { shared.sessions.lock().unwrap().remove(id); }
            return Ok(text(StatusCode::OK, ""));
        }
        Method::GET => {
            // No server-initiated messages: no event stream to open.
            let mut r = text(StatusCode::METHOD_NOT_ALLOWED, "live: this server sends no server-initiated messages; POST JSON-RPC to /mcp");
            r.headers_mut().insert(hyper::header::ALLOW, "POST, DELETE".parse().unwrap());
            return Ok(r);
        }
        _ => return Ok(text(StatusCode::METHOD_NOT_ALLOWED, "POST JSON-RPC to /mcp")),
    }
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => return Ok(text(StatusCode::PAYLOAD_TOO_LARGE, &format!("live: request body: {e}"))),
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return Ok(json_reply(StatusCode::BAD_REQUEST, &error(Value::Null, -32700, &format!("parse error: {e}")), None)),
    };
    let mut session = None;
    let batch = msg.is_array();
    let mut out = Vec::new();
    for m in if batch { msg.as_array().cloned().unwrap_or_default() } else { vec![msg] } {
        if m.get("method").and_then(Value::as_str) == Some("initialize") {
            let id = crate::random_token().unwrap_or_else(|_| format!("{}", std::process::id()));
            shared.sessions.lock().unwrap().insert(id.clone());
            session = Some(id);
        }
        if let Some(r) = message(&shared, m).await { out.push(r); }
    }
    if out.is_empty() { return Ok(text(StatusCode::ACCEPTED, "")); }
    let v = if batch { Value::Array(out) } else { out.pop().unwrap() };
    Ok(json_reply(StatusCode::OK, &v, session))
}

fn json_reply(status: StatusCode, v: &Value, session: Option<String>) -> Response<Body> {
    let mut r = reply(status, "application/json", v.to_string());
    if let Some(s) = session { r.headers_mut().insert("mcp-session-id", s.parse().unwrap()); }
    r
}

fn error(id: Value, code: i64, msg: &str) -> Value { json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } }) }

/// One JSON-RPC message; `None` for notifications and responses.
async fn message(shared: &Shared, m: Value) -> Option<Value> {
    let id = m.get("id").cloned();
    let Some(method) = m.get("method").and_then(Value::as_str) else {
        return id.map(|id| error(id, -32600, "invalid request: no method"));
    };
    let id = id?; // a notification: nothing to answer
    let params = m.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(VERSIONS[0]);
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            let client = params.pointer("/clientInfo/name").and_then(Value::as_str).unwrap_or("?");
            shared.logs.push(format!("live: {client} connected (MCP {version})"));
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false }, "logging": {} },
                "serverInfo": { "name": format!("warp-live-{}", shared.manifest.name), "title": format!("{} (Warp live control)", shared.manifest.name), "version": shared.manifest.version },
                "instructions": instructions(&shared.manifest),
            }))
        }
        "ping" | "logging/setLevel" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools(&shared.manifest) })),
        "resources/list" => Ok(json!({ "resources": [] })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !tools(&shared.manifest).iter().any(|t| t["name"] == name.as_str()) {
                Err((-32602, format!("unknown tool {name:?}; tools/list shows what this game has")))
            } else {
                let r = call(shared, &name, &args).await;
                let shown = { let a = args.to_string(); if a.len() > 160 { format!("{}...", &a[..a.floor_char_boundary(160)]) } else { a } };
                shared.logs.push(format!("live: {name} {shown} -> {}", match &r { Ok(_) => "ok".to_string(), Err(e) => format!("error: {}", e.lines().next().unwrap_or("")) }));
                Ok(result(r).await)
            }
        }
        other => Err((-32601, format!("method {other} is not supported"))),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, msg)) => error(id, code, &msg),
    })
}

async fn result(r: Reply) -> Value {
    match r {
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
        Ok(parts) => {
            let structured = match parts.as_slice() { [Content::Json(v)] if v.is_object() => Some(v.clone()), _ => None };
            let content: Vec<Value> = parts.into_iter().map(|c| match c {
                Content::Text(t) => json!({ "type": "text", "text": t }),
                Content::Json(v) => json!({ "type": "text", "text": v.to_string() }),
                Content::Png(png) => json!({ "type": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(png) }),
            }).collect();
            let mut out = json!({ "content": content, "isError": false });
            if let Some(s) = structured { out["structuredContent"] = s; }
            out
        }
    }
}

async fn ask(shared: &Shared, job: Job, wait: Duration) -> Result<Answer, String> {
    let (tx, rx) = oneshot::channel();
    shared.jobs.send(Request { job, reply: tx }).map_err(|_| "the game has stopped".to_string())?;
    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(a)) => Ok(a),
        Ok(Err(_)) => Err("the game dropped the request".into()),
        // The request is withdrawn: the game skips it when it gets there.
        Err(_) => Err(format!("the game did not answer within {} s (paused, loading, or not drawing frames?); nothing was changed", wait.as_secs())),
    }
}

async fn done(shared: &Shared, job: Job, wait: Duration) -> Reply {
    match ask(shared, job, wait).await? {
        Answer::Done(r) => r,
        Answer::Value(v) => v.map(|v| vec![Content::Json(v)]),
        Answer::Frame { .. } => Err("unexpected frame".into()),
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> { args.get(key).and_then(Value::as_str) }

async fn call(shared: &Shared, name: &str, args: &Value) -> Reply {
    let m = &shared.manifest;
    match name {
        "describe" => Ok(vec![Content::Text(describe(m))]),
        "logs" => {
            let n = args.get("lines").and_then(Value::as_u64).unwrap_or(100).clamp(1, 1000) as usize;
            let lines = shared.logs.tail(n, arg_str(args, "filter"));
            Ok(vec![Content::Text(if lines.is_empty() { "(no log lines yet)".into() } else { lines.join("\n") })])
        }
        "list_files" => files::list(&m.roots, arg_str(args, "folder"), arg_str(args, "path").unwrap_or("")).map(|v| vec![Content::Json(v)]),
        "read_file" => {
            let path = arg_str(args, "path").ok_or("read_file needs a path, like mods/base/cards.luau")?;
            files::read(&m.roots, arg_str(args, "folder"), path).map(|t| vec![Content::Text(t)])
        }
        "write_file" => {
            let path = arg_str(args, "path").ok_or("write_file needs a path, like mods/my_rules.luau")?;
            let content = arg_str(args, "content").ok_or("write_file needs the file's full content")?;
            let (root, rel) = files::write(&m.roots, arg_str(args, "folder"), path, content)?;
            let mut out = vec![Content::Text(format!("wrote {}/{rel} ({} bytes)", root.name, content.len()))];
            if root.reload != Reload::None {
                let wait = if root.reload == Reload::Shaders { FRAME_WAIT } else { GAME_WAIT };
                match done(shared, Job::FileWritten { root: root.name.clone(), path: rel, reload: root.reload }, wait).await {
                    Ok(more) => out.extend(more),
                    Err(e) => out.push(Content::Text(format!("reload: {e}"))),
                }
            }
            Ok(out)
        }
        "state" => done(shared, Job::State, GAME_WAIT).await,
        "query" => {
            let pointers: Vec<String> = match (args.get("pointer"), args.get("pointers")) {
                (Some(Value::String(p)), _) => vec![p.clone()],
                (_, Some(Value::Array(list))) => list.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                _ => return Err("query needs a JSON pointer, like /players/0/health (or pointers: [...])".into()),
            };
            let state = match ask(shared, Job::State, GAME_WAIT).await? { Answer::Value(v) => v?, _ => return Err("unexpected answer".into()) };
            let mut out = serde_json::Map::new();
            for p in pointers {
                let p = if p.is_empty() || p.starts_with('/') { p } else { format!("/{}", p.replace('.', "/")) };
                out.insert(p.clone(), state.pointer(&p).cloned().unwrap_or(Value::Null));
            }
            Ok(vec![Content::Json(Value::Object(out))])
        }
        "run_luau" => {
            let code = arg_str(args, "code").ok_or("run_luau needs code")?;
            done(shared, Job::Luau(code.to_string()), GAME_WAIT).await
        }
        "reload_shaders" => done(shared, Job::ReloadShaders, FRAME_WAIT).await,
        "screenshot" => {
            let max = args.get("max_width").and_then(Value::as_u64).unwrap_or(1024).clamp(64, 4096) as u32;
            match ask(shared, Job::Screenshot, FRAME_WAIT).await? {
                Answer::Frame { size, rgba } => tokio::task::spawn_blocking(move || png(size, rgba, max)).await.map_err(|e| e.to_string())?,
                Answer::Done(r) => r,
                Answer::Value(_) => Err("unexpected answer".into()),
            }
        }
        "play_sound" => match arg_str(args, "name") {
            None => done(shared, Job::Sounds, GAME_WAIT).await,
            Some(n) => {
                let params = match args.get("params") {
                    Some(Value::Object(o)) => o.iter().map(|(k, v)| v.as_f64().map(|x| (k.clone(), x as f32)).ok_or(format!("param {k} must be a number"))).collect::<Result<Vec<_>, _>>()?,
                    None | Some(Value::Null) => Vec::new(),
                    _ => return Err("params must be an object of name: number".into()),
                };
                let at = match args.get("at") {
                    Some(Value::Array(a)) if a.len() == 3 => Some([0, 1, 2].map(|k| a[k].as_f64().unwrap_or(0.0) as f32)),
                    None | Some(Value::Null) => None,
                    _ => return Err("at must be [x, y, z]".into()),
                };
                done(shared, Job::Play { name: n.to_string(), params, at }, GAME_WAIT).await
            }
        },
        "set_param" => {
            let sound = arg_str(args, "sound").ok_or("set_param needs a sound")?.to_string();
            let param = arg_str(args, "param").ok_or("set_param needs a param")?.to_string();
            let value = args.get("value").and_then(Value::as_f64).ok_or("set_param needs a numeric value")? as f32;
            done(shared, Job::SetParam { sound, param, value }, GAME_WAIT).await
        }
        _ => done(shared, Job::Call { tool: name.to_string(), args: args.clone() }, GAME_WAIT).await,
    }
}

/// Downscale to `max` wide and encode.
fn png(size: [u32; 2], rgba: Vec<u8>, max: u32) -> Reply {
    use image::ImageEncoder;
    let img = image::RgbaImage::from_raw(size[0], size[1], rgba).ok_or("frame has the wrong size")?;
    let img = if size[0] > max {
        let h = ((size[1] as u64 * max as u64) / size[0] as u64).max(1) as u32;
        image::imageops::resize(&img, max, h, image::imageops::FilterType::Triangle)
    } else { img };
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(&img, img.width(), img.height(), image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())?;
    Ok(vec![Content::Text(format!("frame {}x{}, sent at {}x{}", size[0], size[1], img.width(), img.height())), Content::Png(out)])
}

fn schema(props: Value, required: &[&str]) -> Value { json!({ "type": "object", "properties": props, "required": required }) }

/// The generic tools this game supports, then its own.
pub(crate) fn tools(m: &Manifest) -> Vec<Value> {
    let f = &m.features;
    let folders: Vec<&str> = m.roots.iter().map(|r| r.name.as_str()).collect();
    let writable: Vec<&str> = m.roots.iter().filter(|r| r.writable).map(|r| r.name.as_str()).collect();
    let mut t = vec![(
        "describe", format!("What {} is, its tools and how to tweak it live. Call this first.", m.name), schema(json!({}), &[]), true,
    )];
    if f.state {
        t.push(("state", "The game's full live state as JSON (game-defined).".into(), schema(json!({}), &[]), true));
        t.push(("query", "Read parts of the state by JSON pointer (/players/0/health), cheaper than state.".into(),
            schema(json!({ "pointer": { "type": "string" }, "pointers": { "type": "array", "items": { "type": "string" } } }), &[]), true));
    }
    if f.screenshot {
        t.push(("screenshot", "A PNG of the frame being shown now (with HUD).".into(),
            schema(json!({ "max_width": { "type": "integer", "description": "downscale to this width (default 1024)" } }), &[]), true));
    }
    if f.luau {
        t.push(("run_luau", "Run Luau in the game's script console against the live game and return what it prints and returns.".into(),
            schema(json!({ "code": { "type": "string" } }), &["code"]), false));
    }
    if !m.roots.is_empty() {
        t.push(("list_files", format!("List the moddable folders ({}) or the files in one.", folders.join(", ")),
            schema(json!({ "folder": { "type": "string", "enum": folders }, "path": { "type": "string", "description": "a subfolder, or folder/subfolder" } }), &[]), true));
        t.push(("read_file", "Read a text file from a moddable folder, path like mods/base/cards.luau.".into(),
            schema(json!({ "path": { "type": "string" }, "folder": { "type": "string", "enum": folders } }), &["path"]), true));
        if !writable.is_empty() {
            t.push(("write_file", format!("Create or replace a whole text file in {}; the game hot-reloads it and the reply says whether it loaded.", writable.join(", ")),
                schema(json!({ "path": { "type": "string" }, "content": { "type": "string" }, "folder": { "type": "string", "enum": writable } }), &["path", "content"]), false));
        }
    }
    if f.shaders {
        t.push(("reload_shaders", "Recompile the shaders from the shader folder on the next frame; a failure keeps the old ones and returns the compiler error.".into(), schema(json!({}), &[]), false));
    }
    t.push(("logs", "Recent console output, errors and tool calls.".into(),
        schema(json!({ "lines": { "type": "integer", "description": "default 100" }, "filter": { "type": "string", "description": "only lines containing this" } }), &[]), true));
    if f.sound {
        t.push(("play_sound", "Play a sound once with parameter values; without a name, list the sounds and their parameters.".into(),
            schema(json!({ "name": { "type": "string" }, "params": { "type": "object", "additionalProperties": { "type": "number" } },
                "at": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "world position; default: at the listener" } }), &[]), false));
        t.push(("set_param", "Set a sound parameter (its default for later plays, and playing voices).".into(),
            schema(json!({ "sound": { "type": "string" }, "param": { "type": "string" }, "value": { "type": "number" } }), &["sound", "param", "value"]), false));
    }
    let mut out: Vec<Value> = t.into_iter().map(|(name, desc, input, ro)| tool_json(name, &desc, input, ro)).collect();
    out.extend(m.tools.iter().map(|g| tool_json(&g.name, &g.description, g.input_schema.clone(), g.read_only)));
    out
}

fn tool_json(name: &str, desc: &str, input: Value, read_only: bool) -> Value {
    json!({ "name": name, "description": desc, "inputSchema": input,
        "annotations": { "readOnlyHint": read_only, "destructiveHint": false, "openWorldHint": false } })
}

fn instructions(m: &Manifest) -> String {
    format!("{} is running live. {}\nCall `describe` for the tools, folders and how to tweak the game; `screenshot` shows the current frame.", m.name, m.about)
}

fn describe(m: &Manifest) -> String {
    let mut s = format!("# {} (live)\n\n{}\n\n## How to tweak it\n\n{}\n\n## Tools\n\n", m.name, m.about.trim(), m.guide.trim());
    for t in tools(m) { s += &format!("- `{}`: {}\n", t["name"].as_str().unwrap_or(""), t["description"].as_str().unwrap_or("")); }
    if !m.roots.is_empty() {
        s += "\n## Folders\n\n";
        for r in &m.roots {
            let ext = if r.extensions.is_empty() { String::new() } else { format!(" (.{})", r.extensions.join(", .")) };
            s += &format!("- `{}`{}{}: {}\n", r.name, ext, if r.writable { "" } else { ", read-only" }, r.about);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_tokens() {
        for h in ["localhost:7878", "127.0.0.1:1", "[::1]:9", "LOCALHOST"] { assert!(loopback_name(h), "{h}"); }
        for h in ["evil.com", "localhost.evil.com:80", "10.0.0.1:7878", ""] { assert!(!loopback_name(h), "{h}"); }
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
