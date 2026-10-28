//! Live control of a running Warp game over MCP, the Model Context
//! Protocol (modelcontextprotocol.io), so a person can iterate on audio,
//! shaders and gameplay together with Claude, ChatGPT or an in-game
//! assistant while playing. See engine/docs/live-control.md.
//!
//! The server runs on its own thread (a small tokio runtime with hyper) and
//! speaks MCP over Streamable HTTP on `127.0.0.1` by default. Every request
//! needs the bearer token printed at startup (also written to a file the
//! `warp-mcp` stdio bridge reads). The game never waits on the network: it
//! calls [`Live::serve`] once per tick, which applies the queued requests on
//! the game thread between ticks and replies; screenshots and shader
//! reloads finish on the next drawn frame ([`Live::wants_capture`],
//! [`Live::wants_shader_reload`]).
//!
//! Games describe themselves with a [`Manifest`] (their own tools, the
//! folders that may be read and written, what the generic tools can do) and
//! implement [`LiveGame`]. Sound goes through [`SoundHook`], so a game can
//! route `play_sound` and `set_param` to whatever synth it uses.
//!
//! Generic tools: `describe`, `state`, `query`, `screenshot`, `run_luau`,
//! `list_files`, `read_file`, `write_file`, `reload_shaders`, `logs`,
//! `play_sound`, `set_param`; each is listed only when the game supports it.
mod console;
mod files;
mod server;

use serde_json::Value;
use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

pub use files::Root;

/// The default port of `--live` without a number.
pub const DEFAULT_PORT: u16 = 7878;

/// How to serve. Built from command-line flags by [`Config::from_args`].
#[derive(Clone, Debug)]
pub struct Config {
    /// The game's name: the token file is `<dir>/<name>.json`.
    pub name: String,
    /// 0 picks a free port.
    pub port: u16,
    pub bind: IpAddr,
    /// Accept any Host and Origin header (tunnels, LAN). The token is still
    /// required. Binding to a non-loopback address also needs this.
    pub remote: bool,
    /// A fixed token; otherwise a random one per run.
    pub token: Option<String>,
    /// Where to write `{url, token, ...}`; `None` uses [`token_dir`].
    pub token_file: Option<PathBuf>,
    /// Tee stdout and stderr into the `logs` tool (Unix).
    pub capture_console: bool,
    /// Print the URL and token at startup.
    pub announce: bool,
}

impl Config {
    pub fn new(name: &str) -> Self {
        Config { name: name.into(), port: DEFAULT_PORT, bind: IpAddr::V4(Ipv4Addr::LOCALHOST), remote: false, token: None, token_file: None, capture_console: true, announce: true }
    }

    /// `--live [PORT]` turns the server on (no port: 7878, 0: any free one).
    /// `--live-bind ADDR` (default 127.0.0.1), `--live-remote` (accept any
    /// Host/Origin; needed for tunnels and non-loopback binds),
    /// `--live-token TOKEN` or `WARP_LIVE_TOKEN` (default: random),
    /// `--live-token-file PATH`. `None` without `--live`.
    pub fn from_args(name: &str, args: &[String]) -> Result<Option<Config>, String> {
        let Some(k) = args.iter().position(|a| a == "--live") else { return Ok(None) };
        let mut cfg = Config::new(name);
        if let Some(p) = args.get(k + 1).filter(|a| !a.starts_with("--")) {
            cfg.port = p.parse().map_err(|_| format!("--live {p}: not a port number"))?;
        }
        let value = |key: &str| args.windows(2).find(|w| w[0] == key).map(|w| w[1].clone());
        if let Some(b) = value("--live-bind") { cfg.bind = b.parse().map_err(|_| format!("--live-bind {b}: not an IP address"))?; }
        cfg.remote = args.iter().any(|a| a == "--live-remote");
        cfg.token = value("--live-token").or_else(|| std::env::var("WARP_LIVE_TOKEN").ok()).filter(|t| !t.is_empty());
        cfg.token_file = value("--live-token-file").map(PathBuf::from);
        if !cfg.bind.is_loopback() && !cfg.remote {
            return Err(format!("--live-bind {}: a non-loopback address also needs --live-remote (read engine/docs/live-control.md first)", cfg.bind));
        }
        Ok(Some(cfg))
    }
}

/// Where token files go: `WARP_LIVE_DIR`, else `<temp>/warp-live`.
pub fn token_dir() -> PathBuf {
    std::env::var_os("WARP_LIVE_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("warp-live"))
}

/// A game-specific tool. `input_schema` is a JSON Schema object.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// Only reads (MCP `readOnlyHint`); clients may skip confirmation.
    pub read_only: bool,
}

impl ToolSpec {
    pub fn new(name: &str, description: &str, input_schema: Value) -> Self {
        ToolSpec { name: name.into(), description: description.into(), input_schema, read_only: false }
    }
    pub fn read_only(mut self) -> Self { self.read_only = true; self }
}

/// What the generic tools can do for this game.
#[derive(Clone, Debug, Default)]
pub struct Features {
    /// `state` and `query` (the game's JSON).
    pub state: bool,
    /// `screenshot` (the game forwards [`Live::wants_capture`] to its shell).
    pub screenshot: bool,
    /// `run_luau` against the game's script host.
    pub luau: bool,
    /// `reload_shaders` (the game forwards [`Live::wants_shader_reload`]).
    pub shaders: bool,
    /// `play_sound` and `set_param` through [`LiveGame::sound`].
    pub sound: bool,
}

/// Everything the server can answer without the game thread.
#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// What the game is, for `describe` and the MCP `instructions`.
    pub about: String,
    /// How to tweak it: which tool or file changes what.
    pub guide: String,
    pub tools: Vec<ToolSpec>,
    pub roots: Vec<Root>,
    pub features: Features,
}

/// A piece of a tool result.
#[derive(Clone, Debug)]
pub enum Content {
    Text(String),
    /// Sent as text and as MCP `structuredContent` when it is the only JSON.
    Json(Value),
    Png(Vec<u8>),
}

pub type Reply = Result<Vec<Content>, String>;

/// A sound the game can play, with its parameters and defaults.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SoundInfo {
    pub name: String,
    pub params: Vec<(String, f32)>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub about: String,
}

/// The game side of `play_sound` and `set_param`.
pub trait SoundHook {
    fn sounds(&mut self) -> Vec<SoundInfo>;
    /// Play `name` once with these parameters, at a world position or (None)
    /// at the listener. Returns a short report (length, peak).
    fn play(&mut self, name: &str, params: &[(String, f32)], at: Option<[f32; 3]>) -> Result<String, String>;
    /// Change a parameter of `sound` (its default for later plays, and any
    /// playing voice the game can reach).
    fn set_param(&mut self, sound: &str, param: &str, value: f32) -> Result<String, String>;
}

/// The game side, called on the game thread from [`Live::serve`].
pub trait LiveGame {
    /// The game's state as JSON (`state`, and `query` reads pointers in it).
    fn state(&mut self) -> Result<Value, String> { Err("this game exposes no state".into()) }
    /// One of the game's own tools from [`Manifest::tools`].
    fn call(&mut self, tool: &str, args: &Value) -> Reply;
    /// A line of Luau against the live game.
    fn luau(&mut self, code: &str) -> Result<String, String> { Err(format!("this game has no Luau console ({code})")) }
    /// `write_file` changed `path` in the root `root` (a root with
    /// [`Reload::Game`]): reload now and say what happened.
    fn file_written(&mut self, root: &str, path: &str) -> Option<String> { let _ = (root, path); None }
    fn sound(&mut self) -> Option<&mut dyn SoundHook> { None }
}

/// What a write to a root reloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reload {
    /// Nothing (or the game's own watcher picks it up).
    None,
    /// Tell the game ([`LiveGame::file_written`]) and report its answer.
    Game,
    /// Reload shaders on the next frame and report the result.
    Shaders,
}

/// Recent console output and server events, newest last.
#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<LogBuf>>);

#[derive(Default)]
struct LogBuf { lines: VecDeque<(u64, String)>, next: u64 }

const LOG_LINES: usize = 1000;

impl Logs {
    pub fn push(&self, line: impl Into<String>) {
        let mut b = self.0.lock().unwrap();
        let n = b.next;
        b.next += 1;
        for l in line.into().lines() { b.lines.push_back((n, l.to_string())); }
        while b.lines.len() > LOG_LINES { b.lines.pop_front(); }
    }
    /// The last `n` lines, optionally only those containing `filter`.
    pub fn tail(&self, n: usize, filter: Option<&str>) -> Vec<String> {
        let b = self.0.lock().unwrap();
        let mut out: Vec<String> = b.lines.iter().rev().filter(|(_, l)| filter.is_none_or(|f| l.contains(f))).take(n).map(|(_, l)| l.clone()).collect();
        out.reverse();
        out
    }
}

/// Work for the game thread.
pub(crate) enum Job {
    State,
    Call { tool: String, args: Value },
    Luau(String),
    FileWritten { root: String, path: String, reload: Reload },
    Screenshot,
    ReloadShaders,
    Sounds,
    Play { name: String, params: Vec<(String, f32)>, at: Option<[f32; 3]> },
    SetParam { sound: String, param: String, value: f32 },
}

pub(crate) enum Answer {
    Done(Reply),
    Value(Result<Value, String>),
    Frame { size: [u32; 2], rgba: Vec<u8> },
}

pub(crate) struct Request { pub job: Job, pub reply: oneshot::Sender<Answer> }

/// The game's handle on the running server.
pub struct Live {
    jobs: mpsc::UnboundedReceiver<Request>,
    captures: Vec<oneshot::Sender<Answer>>,
    reloads: Vec<oneshot::Sender<Answer>>,
    logs: Logs,
    url: String,
    token: String,
    token_file: Option<PathBuf>,
    _server: std::thread::JoinHandle<()>,
    /// Dropped last: restores stdout and stderr.
    _console: Option<console::Capture>,
}

/// At most this many requests per [`Live::serve`] (the rest wait a tick).
const JOBS_PER_TICK: usize = 16;

/// Start serving. Prints the URL and token (unless `cfg.announce` is off)
/// and writes them to the token file.
pub fn start(cfg: Config, manifest: Manifest) -> Result<Live, String> {
    let logs = Logs::default();
    let console = if cfg.capture_console { console::capture(logs.clone()) } else { None };
    let token = match &cfg.token { Some(t) => t.clone(), None => random_token()? };
    let (tx, rx) = mpsc::unbounded_channel();
    let addr = SocketAddr::new(cfg.bind, cfg.port);
    let listener = std::net::TcpListener::bind(addr).map_err(|e| format!("live: cannot listen on {addr}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    let shown = if local.ip().is_unspecified() { SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), local.port()) } else { local };
    let url = format!("http://{shown}/mcp");
    let shared = Arc::new(server::Shared { manifest, token: token.clone(), remote: cfg.remote, jobs: tx, logs: logs.clone(), sessions: Default::default() });
    let handle = std::thread::Builder::new().name("live-mcp".into()).spawn(move || server::run(listener, shared)).map_err(|e| e.to_string())?;
    let token_file = cfg.token_file.clone().or_else(|| Some(token_dir().join(format!("{}.json", cfg.name))));
    if let Some(path) = &token_file {
        let info = serde_json::json!({ "game": cfg.name, "url": url, "token": token, "pid": std::process::id() });
        if let Err(e) = write_private(path, &serde_json::to_string_pretty(&info).unwrap()) { eprintln!("live: cannot write {}: {e}", path.display()); }
    }
    if cfg.announce {
        println!("live: MCP on {url} (bearer token {token}){}", if cfg.remote { "; REMOTE access allowed" } else { "" });
        if let Some(p) = &token_file { println!("live: token file {} (the warp-mcp bridge reads it)", p.display()); }
    }
    logs.push(format!("live: serving {url}"));
    Ok(Live { jobs: rx, captures: Vec::new(), reloads: Vec::new(), logs, url, token, token_file, _server: handle, _console: console })
}

fn random_token() -> Result<String, String> {
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).map_err(|e| format!("live: no randomness for the token: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Write a file only the current user can read (Unix: mode 0600).
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    { use std::os::unix::fs::OpenOptionsExt; o.mode(0o600); }
    use std::io::Write;
    let mut f = o.open(path)?;
    #[cfg(unix)]
    { use std::os::unix::fs::PermissionsExt; f.set_permissions(std::fs::Permissions::from_mode(0o600))?; }
    f.write_all(text.as_bytes())
}

impl Live {
    pub fn url(&self) -> &str { &self.url }
    pub fn token(&self) -> &str { &self.token }
    pub fn logs(&self) -> &Logs { &self.logs }
    /// Add a line to the `logs` tool (console capture adds stdout/stderr).
    pub fn log(&self, line: impl Into<String>) { self.logs.push(line); }

    /// Apply queued requests on the game thread. Call once per tick (or
    /// frame); it never waits.
    pub fn serve(&mut self, game: &mut dyn LiveGame) {
        for _ in 0..JOBS_PER_TICK {
            let Ok(Request { job, reply }) = self.jobs.try_recv() else { return };
            // Timed out on the server side: the caller was told nothing changed.
            if reply.is_closed() { continue; }
            let answer = match job {
                Job::State => Answer::Value(game.state()),
                Job::Call { tool, args } => Answer::Done(game.call(&tool, &args)),
                Job::Luau(code) => Answer::Done(game.luau(&code).map(|t| vec![Content::Text(t)])),
                Job::FileWritten { reload: Reload::Shaders, .. } | Job::ReloadShaders => {
                    self.reloads.push(reply);
                    continue;
                }
                Job::FileWritten { root, path, .. } => Answer::Done(Ok(game.file_written(&root, &path).map(Content::Text).into_iter().collect())),
                Job::Screenshot => { self.captures.push(reply); continue; }
                Job::Sounds => Answer::Done(match game.sound() {
                    Some(s) => Ok(vec![Content::Json(serde_json::json!({ "sounds": s.sounds() }))]),
                    None => Err("this game has no sound hook".into()),
                }),
                Job::Play { name, params, at } => Answer::Done(match game.sound() {
                    Some(s) => s.play(&name, &params, at).map(|t| vec![Content::Text(t)]),
                    None => Err("this game has no sound hook".into()),
                }),
                Job::SetParam { sound, param, value } => Answer::Done(match game.sound() {
                    Some(s) => s.set_param(&sound, &param, value).map(|t| vec![Content::Text(t)]),
                    None => Err("this game has no sound hook".into()),
                }),
            };
            let _ = reply.send(answer);
        }
    }

    /// A screenshot is waiting: render the next frame into memory and pass
    /// it to [`Live::captured`].
    pub fn wants_capture(&self) -> bool { !self.captures.is_empty() }

    /// The frame for pending screenshots, RGBA8 rows top to bottom. PNG
    /// encoding happens on the server thread.
    pub fn captured(&mut self, size: [u32; 2], rgba: Vec<u8>) {
        let mut waiting = std::mem::take(&mut self.captures).into_iter();
        let Some(last) = waiting.next_back() else { return };
        for w in waiting { let _ = w.send(Answer::Frame { size, rgba: rgba.clone() }); }
        let _ = last.send(Answer::Frame { size, rgba });
    }

    /// A shader reload is waiting: call the renderer's reload and pass the
    /// result to [`Live::shaders_reloaded`].
    pub fn wants_shader_reload(&self) -> bool { !self.reloads.is_empty() }

    /// `Ok(true)`: a new generation is live; `Ok(false)`: nothing changed.
    pub fn shaders_reloaded(&mut self, result: Result<bool, String>) {
        let text = match &result {
            Ok(true) => Ok("shaders reloaded; the new generation is live".to_string()),
            Ok(false) => Ok("shaders unchanged (nothing to rebuild)".to_string()),
            Err(e) => Err(format!("shader reload failed, the previous shaders stay live: {e}")),
        };
        match &text { Ok(t) => self.logs.push(format!("live: {t}")), Err(e) => self.logs.push(format!("live: {e}")) }
        for reply in self.reloads.drain(..) { let _ = reply.send(Answer::Done(text.clone().map(|t| vec![Content::Text(t)]))); }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // The token dies with the process; leave no stale file behind.
        if let Some(p) = &self.token_file && let Ok(text) = std::fs::read_to_string(p) && text.contains(&self.token) { let _ = std::fs::remove_file(p); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> { s.split_whitespace().map(String::from).collect() }

    #[test]
    fn flags_parse() {
        assert!(Config::from_args("g", &args("play --players 2")).unwrap().is_none());
        let c = Config::from_args("g", &args("play --live")).unwrap().unwrap();
        assert_eq!((c.port, c.bind.is_loopback(), c.remote), (DEFAULT_PORT, true, false));
        let c = Config::from_args("g", &args("play --live 0 --seed 3")).unwrap().unwrap();
        assert_eq!(c.port, 0);
        let c = Config::from_args("g", &args("play --live --seed 3")).unwrap().unwrap();
        assert_eq!(c.port, DEFAULT_PORT);
        assert!(Config::from_args("g", &args("play --live 9 --live-bind 0.0.0.0")).is_err(), "remote binds are opt-in");
        let c = Config::from_args("g", &args("play --live 9 --live-bind 0.0.0.0 --live-remote --live-token abc")).unwrap().unwrap();
        assert_eq!((c.remote, c.token.as_deref()), (true, Some("abc")));
        assert!(Config::from_args("g", &args("play --live x")).is_err());
    }

    #[test]
    fn logs_keep_the_tail() {
        let l = Logs::default();
        for i in 0..1200 { l.push(format!("line {i}")); }
        let t = l.tail(3, None);
        assert_eq!(t, ["line 1197", "line 1198", "line 1199"]);
        assert_eq!(l.tail(2, Some("line 11")), ["line 1198", "line 1199"]);
        assert_eq!(random_token().unwrap().len(), 48);
    }
}
