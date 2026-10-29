//! warp-mcp: a stdio MCP server that forwards to a running Warp game's live
//! control endpoint (`--live`), for clients that start local servers as
//! commands (Claude Desktop, Claude Code).
//!
//!   warp-mcp [--game NAME] [--url http://127.0.0.1:7878/mcp --token TOKEN]
//!
//! Without `--url` it reads the token file the game writes at startup
//! (`<temp>/warp-live/<game>.json`, or the newest one there; `WARP_LIVE_DIR`
//! moves it), and reads it again when the game restarts. Until a game is
//! running it answers on its own with one `warp_status` tool, and tells the
//! client (tools/list_changed) when a game appears or goes away. The
//! `feedback` tool (the newest F12 captures) always works, game or not.
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

#[derive(Clone, Debug, PartialEq)]
struct Target { url: String, token: String, game: String }

struct Bridge {
    fixed: Option<Target>,
    game: Option<String>,
    current: Mutex<Option<Target>>,
    session: Mutex<Option<String>>,
}

fn arg(args: &[String], key: &str) -> Option<String> { args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone()) }

impl Bridge {
    /// The game to talk to: fixed, or from the newest token file.
    fn locate(&self) -> Option<Target> {
        if let Some(t) = &self.fixed { return Some(t.clone()); }
        let dir = live::token_dir();
        let file: PathBuf = match &self.game {
            Some(g) => dir.join(format!("{g}.json")),
            None => std::fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json"))
                .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())?,
        };
        let v: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        Some(Target { url: v["url"].as_str()?.into(), token: v["token"].as_str()?.into(), game: v["game"].as_str().unwrap_or("game").into() })
    }

    async fn post(&self, t: &Target, body: &str) -> Result<(u16, Bytes), String> {
        let rest = t.url.strip_prefix("http://").ok_or("warp-mcp speaks plain http to a local game; remote clients connect to the HTTPS URL directly")?;
        let (authority, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(a, p)| (a, format!("/{p}")));
        let stream = tokio::net::TcpStream::connect(authority).await.map_err(|e| format!("{authority}: {e}"))?;
        let (mut send, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.map_err(|e| e.to_string())?;
        tokio::spawn(conn);
        let mut req = hyper::Request::post(path).header("host", authority).header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream").header("authorization", format!("Bearer {}", t.token));
        if let Some(s) = self.session.lock().unwrap().clone() { req = req.header("mcp-session-id", s); }
        let res = send.send_request(req.body(Full::new(Bytes::from(body.to_string()))).map_err(|e| e.to_string())?).await.map_err(|e| e.to_string())?;
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|h| h.to_str().ok()) { *self.session.lock().unwrap() = Some(s.to_string()); }
        let status = res.status().as_u16();
        let body = res.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
        Ok((status, body))
    }

    /// Forward one message; on failure find the game again and retry once.
    async fn forward(&self, body: &str) -> Result<Option<Value>, String> {
        let mut last = String::from("no Warp game with --live is running");
        for attempt in 0..2 {
            let target = if attempt == 0 { self.current.lock().unwrap().clone().or_else(|| self.locate()) } else { self.locate() };
            let Some(t) = target else { break };
            match self.post(&t, body).await {
                Ok((401, _)) => { last = "the game refused the token (restarted?)".into(); *self.current.lock().unwrap() = None; }
                Ok((202, _)) => { *self.current.lock().unwrap() = Some(t); return Ok(None); }
                Ok((200, b)) => {
                    *self.current.lock().unwrap() = Some(t);
                    let text = String::from_utf8_lossy(&b);
                    // An event stream carries the reply in data lines.
                    let json = if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') { text.to_string() }
                        else { text.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).last().unwrap_or("").to_string() };
                    return serde_json::from_str(&json).map(Some).map_err(|e| format!("bad reply from the game: {e}"));
                }
                Ok((s, b)) => { last = format!("HTTP {s}: {}", String::from_utf8_lossy(&b)); *self.current.lock().unwrap() = None; }
                Err(e) => { last = format!("cannot reach {}: {e}", t.url); *self.current.lock().unwrap() = None; }
            }
        }
        Err(last)
    }
}

/// Answers while no game is running.
fn offline(msg: &Value, why: &str) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let help = format!("{why}. Start a Warp game with --live (for example `growing-guns play --live`), then call warp_status. Token files are read from {}.", live::token_dir().display());
    let result = match msg.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": msg.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": true } },
            "serverInfo": { "name": "warp-mcp", "version": env!("CARGO_PKG_VERSION") },
            "instructions": help,
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [{ "name": "warp_status", "description": "Whether a Warp game with live control is running; its tools appear once it is.",
            "inputSchema": { "type": "object", "properties": {} }, "annotations": { "readOnlyHint": true } }, live::feedback::tool()] }),
        "tools/call" => json!({ "content": [{ "type": "text", "text": help }], "isError": true }),
        "resources/list" => json!({ "resources": [] }),
        "prompts/list" => json!({ "prompts": [] }),
        m => return Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": format!("{m}: {help}") } })),
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("warp-mcp [--game NAME] [--url http://127.0.0.1:7878/mcp --token TOKEN]: stdio MCP bridge to a Warp game started with --live");
        return;
    }
    let fixed = arg(&args, "--url").map(|url| Target { url, token: arg(&args, "--token").or_else(|| std::env::var("WARP_LIVE_TOKEN").ok()).unwrap_or_default(), game: "game".into() });
    let bridge = Arc::new(Bridge { fixed, game: arg(&args, "--game"), current: Mutex::new(None), session: Mutex::new(None) });
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(v) = out_rx.recv().await {
            let mut line = v.to_string();
            line.push('\n');
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() { break; }
        }
    });
    // Tell the client when a game comes or goes, so it lists tools again.
    {
        let (bridge, out) = (bridge.clone(), out_tx.clone());
        tokio::spawn(async move {
            let mut up = None;
            loop {
                let now = match bridge.locate() { Some(t) => bridge.post(&t, r#"{"jsonrpc":"2.0","id":"warp-mcp-probe","method":"ping"}"#).await.ok().filter(|r| r.0 == 200).map(|_| t.url), None => None };
                if up.is_some() && up != Some(now.clone()) && out.send(json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" })).is_err() { break; }
                up = Some(now);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
    }
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() { continue; }
        let (bridge, out) = (bridge.clone(), out_tx.clone());
        tokio::spawn(async move {
            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => { let _ = out.send(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": e.to_string() } })); return; }
            };
            // F12 captures are files: answered here, with or without a game.
            if msg.get("method").and_then(Value::as_str) == Some("tools/call") && msg.pointer("/params/name").and_then(Value::as_str) == Some("feedback") && let Some(id) = msg.get("id") {
                let args = msg.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let _ = out.send(json!({ "jsonrpc": "2.0", "id": id, "result": live::feedback::mcp_result(&args) }));
                return;
            }
            if msg.pointer("/params/name").and_then(Value::as_str) == Some("warp_status") && let Some(id) = msg.get("id") {
                let text = match bridge.forward(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#).await {
                    Ok(_) => { let t = bridge.current.lock().unwrap().clone(); format!("connected to {} at {}; list the tools again to see its tools", t.as_ref().map_or("?", |t| t.game.as_str()), t.as_ref().map_or("?", |t| t.url.as_str())) }
                    Err(why) => offline(&json!({ "id": 0, "method": "tools/call" }), &why).and_then(|v| v.pointer("/result/content/0/text").and_then(Value::as_str).map(String::from)).unwrap_or(why),
                };
                let _ = out.send(json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": text }], "isError": false } }));
                return;
            }
            match bridge.forward(&line).await {
                Ok(Some(mut reply)) => {
                    // Games come and go behind the bridge: tools may change.
                    if msg.get("method").and_then(Value::as_str) == Some("initialize") && let Some(t) = reply.pointer_mut("/result/capabilities/tools") { t["listChanged"] = json!(true); }
                    let _ = out.send(reply);
                }
                Ok(None) => {}
                Err(why) => {
                    if msg.get("method").and_then(Value::as_str) == Some("tools/call") && msg.pointer("/params/name").and_then(Value::as_str) != Some("warp_status") {
                        if let Some(id) = msg.get("id") { let _ = out.send(json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": format!("{why}. Is the game still running with --live?") }], "isError": true } })); }
                    } else if let Some(r) = offline(&msg, &why) { let _ = out.send(r); }
                }
            }
        });
    }
    // The client closed stdin: let replies already written go out, then stop
    // (the probe task would otherwise keep the writer alive).
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    drop(writer);
    std::process::exit(0);
}
