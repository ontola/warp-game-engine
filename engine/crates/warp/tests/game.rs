//! The Luau game API, hot reload and the starter, without a GPU: games are
//! loaded and ticked directly (no window, no rendering).
use app::{Buttons, Controls, Game};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use warp::{LuaGame, Overrides};

fn repo() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..") }

/// A fresh folder for one test (under the target dir; never reused).
fn folder(name: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("warp-{name}-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn game(dir: &Path) -> LuaGame {
    let mut g = LuaGame::load(dir, &Overrides::default(), None).unwrap();
    g.echo = false;
    g
}

fn ticks(g: &mut LuaGame, n: usize, c: Controls) { for _ in 0..n { g.tick(&[c]); } }

fn state(g: &LuaGame) -> Value { g.state_json() }

/// Edits must change the file's length or a later mtime to be seen.
fn write(path: &Path, text: &str) {
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(path, text).unwrap();
}

#[test]
fn the_repo_copy_of_hello_warp_is_the_starter() {
    let dir = repo().join("games/hello-warp");
    for (rel, text) in warp::starter::FILES {
        let on_disk = std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|e| panic!("games/hello-warp/{rel}: {e}"));
        assert_eq!(&on_disk, text, "games/hello-warp/{rel} differs from engine/crates/warp/starter/{rel}; edit both");
    }
}

#[test]
fn warp_new_writes_a_game_that_plays() {
    let dir = folder("new").join("my-game");
    let files = warp::starter::write(&dir, "My Game").unwrap();
    assert!(files.contains(&"main.luau".to_string()) && dir.join("assets").is_dir());
    assert!(warp::starter::write(&dir, "again").unwrap_err().contains("already a game"));
    let mut g = game(&dir);
    assert_eq!(g.cfg.title, "My Game");
    ticks(&mut g, 30, Controls::default());
    let s = state(&g);
    assert_eq!(s["errors"], json!([]), "{s}");
    assert_eq!((s["state"]["runners"][0]["score"].clone(), s["state"]["runners"][0]["grounded"].clone()), (json!(0), json!(true)));
    assert_eq!(s["tags"], json!({ "crate": 3, "floor": 1, "orb": 6, "player": 1 }));
    // Hold forward and jump: the player moves and leaves the ground.
    ticks(&mut g, 20, Controls { mv: [0.0, 1.0], ..Default::default() });
    g.tick(&[Controls { pressed: Buttons(Buttons::SOUTH), held: Buttons(Buttons::SOUTH), ..Default::default() }]);
    ticks(&mut g, 5, Controls::default());
    let s = state(&g);
    assert!(s["state"]["runners"][0]["vel"][1].as_f64().unwrap() > 3.0, "jumping: {s}");
    assert_eq!(s["sounds"]["recent"].as_array().unwrap().last().unwrap()["sound"], "jump");
}

#[test]
fn a_one_file_game_plays_alone_with_its_sounds_inside() {
    let dir = folder("one-file");
    let file = dir.join("orb-hunt.luau");
    warp::starter::write_file(&file, "Orb Hunt").unwrap();
    assert!(warp::starter::write_file(&file, "again").unwrap_err().contains("exists"));
    // Other scripts beside it are not part of the game.
    write(&dir.join("other.luau"), "error('not a module of orb-hunt.luau')");
    let mut g = game(&file);
    assert_eq!((g.cfg.title.as_str(), g.cfg.seed, g.cfg.sky.as_str()), ("Orb Hunt", 7, "afternoon"));
    assert_eq!(g.live_name(), "orb-hunt");
    ticks(&mut g, 30, Controls::default());
    let s = state(&g);
    assert_eq!(s["errors"], json!([]), "{s}");
    assert_eq!(s["modules"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["tags"], json!({ "crate": 3, "floor": 1, "orb": 6, "player": 1 }));
    g.tick(&[Controls { pressed: Buttons(Buttons::SOUTH), held: Buttons(Buttons::SOUTH), ..Default::default() }]);
    ticks(&mut g, 3, Controls::default());
    let s = state(&g);
    assert_eq!(s["sounds"]["recent"].as_array().unwrap().last().unwrap()["sound"], "jump", "the inline patch plays: {s}");
    // Editing an inline patch reloads it; a broken one keeps the last good one.
    let text = std::fs::read_to_string(&file).unwrap();
    write(&file, &text.replace("length(0.18)", "length(0.2"));
    g.reload();
    assert!(state(&g)["errors"].to_string().contains("sound jump"), "{}", state(&g));
    write(&file, &text.replace("length(0.18)", "length(0.25)"));
    g.reload();
    assert_eq!(state(&g)["errors"], json!([]));
    // A folder whose only script is the game runs it (a packaged one-file game).
    let packed = folder("one-file-packed");
    std::fs::copy(&file, packed.join("orb-hunt.luau")).unwrap();
    write(&packed.join("types.d.luau"), "-- types");
    assert!(warp::config::is_game(&packed));
    let mut g = game(&packed);
    ticks(&mut g, 5, Controls::default());
    assert_eq!((state(&g)["errors"].clone(), g.cfg.title.as_str()), (json!([]), "Orb Hunt"));
    write(&packed.join("second.luau"), "return {}");
    assert!(!warp::config::is_game(&packed), "two scripts and no main.luau: which one?");
}

#[test]
fn the_same_seed_plays_the_same() {
    let dir = repo().join("games/hello-warp");
    let run = |seed| {
        let mut g = LuaGame::load(&dir, &Overrides { seed: Some(seed), ..Default::default() }, None).unwrap();
        g.echo = false;
        ticks(&mut g, 200, Controls { mv: [0.3, 1.0], look: [0.01, 0.0], ..Default::default() });
        g.command_line("entities orb").unwrap()
    };
    assert_eq!(run(3), run(3));
    assert_ne!(run(3), run(4), "orbs are placed from the seed");
}

const API_GAME: &str = r##"
local function init()
    state.floor = world.spawn("cube", { pos = vector.create(0, -0.5, 0), size = vector.create(20, 1, 20), tag = "floor" })
    state.wall = world.spawn("box", { pos = vector.create(0, 1, -5), size = vector.create(4, 2, 1), color = "#336699", tag = "wall" })
    state.orb = world.spawn("sphere", { pos = vector.create(2, 1, 0), size = 0.5, solid = false, emissive = 2, color = {1, 0.5, 0}, tag = "orb" })
    state.body = world.spawn("capsule", { pos = vector.create(0, 3, 0), size = vector.create(0.8, 0.9, 0.8), tag = "player" })
    local hit = world.raycast(vector.create(0, 1, 0), vector.create(0, 0, -1))
    state.hit_wall = hit and hit.id == state.wall and math.abs(hit.distance - 4.5) < 1e-3
    state.ray_skips_orb = world.raycast(vector.create(0, 1, 0), vector.create(1, 0, 0), 10, { ignore = state.body }) == nil
    state.ray_all = world.raycast(vector.create(0, 1, 0), vector.create(1, 0, 0), 10, { all = true, ignore = { state.body } }).id == state.orb
    state.near_orb = #world.overlap(vector.create(1.6, 1, 0), 0.3, { tag = "orb" })
    state.r = { random.float(), random.int(1, 6), random.range(5, 6) }
    state.typo = tostring(select(2, pcall(world.spawn, "cube", { colour = "red" })))
    state.bad_shape = tostring(select(2, pcall(world.spawn, "cone")))
    state.ticks = 0
    camera.set(1, { pos = vector.create(0, 2, 8), target = vector.create(0, 1, 0), fov = 70 })
    sky.set("sunset:haze")
    light.set({ exposure = 12 })
end

local function tick(dt, input)
    state.ticks += 1
    state.dt = dt
    local vel, grounded = world.move(state.body, vector.create(0, -20 * dt * state.ticks, 0), dt)
    state.grounded = grounded
    state.y = world.get(state.body).pos.y
end

local function draw()
    hud.text("hi", 10, 10)
    state.draw_spawn = tostring(select(2, pcall(world.spawn, "cube")))
    state.draw_random = tostring(select(2, pcall(random.float)))
end

return { init = init, tick = tick, draw = draw }
"##;

#[test]
fn the_api_spawns_queries_and_moves() {
    let dir = folder("api");
    std::fs::write(dir.join("main.luau"), API_GAME).unwrap();
    std::fs::write(dir.join("game.toml"), "title = \"API\"\nseed = 11\ntick_rate = 50\n").unwrap();
    let mut g = game(&dir);
    ticks(&mut g, 60, Controls::default());
    let s = state(&g);
    let st = &s["state"];
    assert_eq!(s["errors"], json!([]), "{s}");
    for k in ["hit_wall", "ray_skips_orb", "ray_all", "grounded"] { assert_eq!(st[k], true, "{k}: {s}"); }
    assert_eq!(st["near_orb"], 1);
    assert!((st["y"].as_f64().unwrap() - 0.9).abs() < 0.02, "{s}");
    assert_eq!(st["dt"].as_f64().unwrap() as f32, 1.0 / 50.0);
    assert!(st["typo"].as_str().unwrap().contains("unknown option \"colour\""), "{s}");
    assert!(st["bad_shape"].as_str().unwrap().contains("unknown shape"), "{s}");
    assert_eq!((s["sky"].clone(), s["cameras"][0]["fov"].clone(), s["title"].clone()), (json!("sunset:haze"), json!(70.0), json!("API")));
    // Seeded: a second load draws the same numbers.
    let again = state(&game(&dir));
    assert_eq!(again["state"]["r"], st["r"]);
    // Console lines see the live game and main.luau's state.
    assert_eq!(g.command_line("lua state.ticks").unwrap(), "60");
    assert_eq!(g.command_line("lua world.count()").unwrap(), "4");
    assert!(g.command_line("lua nope()").is_err());
    assert!(g.command_line("frobnicate").unwrap_err().contains("unknown command"));
}

#[test]
fn hot_reload_keeps_state_and_the_world() {
    let dir = folder("reload");
    let main = dir.join("main.luau");
    std::fs::write(&main, "state.loads = (state.loads or 0) + 1\nreturn { init = function() state.inits = (state.inits or 0) + 1; state.box = world.spawn('cube') end, tick = function() state.n = (state.n or 0) + 1 end }").unwrap();
    let mut g = game(&dir);
    ticks(&mut g, 10, Controls::default());
    assert_eq!(state(&g)["state"], json!({ "loads": 1, "inits": 1, "box": 1, "n": 10 }));

    // An edit: the module reloads, keeps `state` and the entity, runs reload() and not init().
    write(&main, "state.loads = (state.loads or 0) + 1\nreturn { init = function() state.inits = (state.inits or 0) + 1 end, reload = function() state.reloaded = true end, tick = function() state.n += 10 end }");
    let changed = g.command_line("reload").unwrap();
    assert!(changed.contains("main.luau: reloaded"), "{changed}");
    ticks(&mut g, 1, Controls::default());
    let s = state(&g);
    assert_eq!(s["state"], json!({ "loads": 2, "inits": 1, "box": 1, "n": 20, "reloaded": true }), "{s}");
    assert_eq!(s["entities"], 1);

    // A broken edit keeps the previous version running and reports the line.
    write(&main, "return { tick = function() state.n += 1000 end,, }");
    g.reload();
    ticks(&mut g, 1, Controls::default());
    let s = state(&g);
    assert_eq!(s["state"]["n"], 30, "the last good version still runs");
    assert!(s["errors"][0].as_str().unwrap().contains("main.luau:1"), "{s}");

    // A runtime error turns the hooks off until the file changes; ticks are polled for edits.
    write(&main, "return { tick = function() error('boom') end }");
    g.reload();
    ticks(&mut g, 2, Controls::default());
    let s = state(&g);
    assert!(s["errors"][0].as_str().unwrap().contains("hooks are off"), "{s}");
    assert!(g.logs().iter().any(|l| l.contains("boom")));
    write(&main, "return { tick = function() state.n += 1 end }");
    ticks(&mut g, 16, Controls::default());
    assert!(state(&g)["errors"] == json!([]) && state(&g)["state"]["n"].as_u64().unwrap() > 30, "{}", state(&g));

    // restart: a clean world and state, and init again.
    write(&main, "return { init = function() state.inits = (state.inits or 0) + 1; world.spawn('sphere') end }");
    g.reload();
    g.restart();
    let s = state(&g);
    assert_eq!((s["state"].clone(), s["entities"].clone(), s["tick"].clone()), (json!({ "inits": 1 }), json!(1), json!(0)), "{s}");
}

#[test]
fn sound_patches_reload_and_play() {
    let dir = folder("sounds");
    std::fs::create_dir_all(dir.join("sounds")).unwrap();
    std::fs::write(dir.join("sounds/beep.luau"), "length(0.1)\nreturn sine(param('freq', 440)) * decay(0.1, 2)").unwrap();
    std::fs::write(dir.join("main.luau"), "return { tick = function() if game.tick() == 2 then state.h = sound.play('beep', { freq = 880, at = vector.create(1, 0, 0) }); state.bad = tostring(select(2, pcall(sound.play, 'beep', { frq = 1 }))) end end }").unwrap();
    let (tx, mut mix) = warp::sounds::offline();
    let mut g = LuaGame::load(&dir, &Overrides::default(), Some(tx)).unwrap();
    g.echo = false;
    ticks(&mut g, 4, Controls::default());
    let s = state(&g);
    assert_eq!((s["state"]["h"].clone(), s["sounds"]["played"].clone()), (json!(1), json!(1)), "{s}");
    assert!(s["state"]["bad"].as_str().unwrap().contains("no parameter \"frq\""), "{s}");
    let mut out = vec![0.0f32; 4800 * 2];
    audio::Source::render(&mut mix, &mut out, 2);
    assert!(out.iter().any(|x| x.abs() > 0.01), "the beep reached the mixer");
    // A broken patch keeps the last good one; a fixed one reloads.
    write(&dir.join("sounds/beep.luau"), "return sine(");
    g.reload();
    assert!(state(&g)["errors"][0].as_str().unwrap().contains("sounds/beep.luau"));
    assert_eq!(state(&g)["sounds"]["loaded"], json!(["beep"]));
    write(&dir.join("sounds/beep.luau"), "length(0.2)\nreturn saw(param('freq', 220)) * decay(0.2, 2)");
    assert!(g.reload().iter().any(|l| l.contains("beep.luau: reloaded")));
    assert_eq!(state(&g)["errors"], json!([]));
}

#[test]
fn bots_play_split_screen_and_replay_exactly() {
    let run = || {
        let mut g = LuaGame::load(&repo().join("games/hello-warp"), &Overrides { players: Some(2), bots: Some(2), ..Default::default() }, None).unwrap();
        g.echo = false;
        assert_eq!(g.local_players(), 2, "two people, two views; bots need no controls");
        for _ in 0..900 { g.tick(&[Controls::default(), Controls::default()]); }
        state(&g)
    };
    let s = run();
    assert_eq!(s["errors"], json!([]), "{s}");
    let players = s["players"].as_array().unwrap();
    assert_eq!(players.iter().map(|p| (p["id"].as_u64().unwrap(), p["bot"].as_bool().unwrap())).collect::<Vec<_>>(), vec![(1, false), (2, false), (3, true), (4, true)]);
    assert_eq!(players.iter().map(|p| p["view"].clone()).collect::<Vec<_>>(), vec![json!(0), json!(1), Value::Null, Value::Null]);
    assert_eq!(s["cameras"].as_array().unwrap().len(), 2);
    let runners = s["state"]["runners"].as_array().unwrap();
    assert_eq!(runners.len(), 4);
    assert!(runners[2..].iter().any(|r| r["score"].as_u64().unwrap() > 0), "bots collect orbs: {runners:?}");
    assert_eq!(run()["state"], s["state"], "the same seed replays exactly, bots included");
}

#[test]
fn min_players_fills_with_bots() {
    let dir = folder("min");
    std::fs::write(dir.join("game.toml"), "min_players = 3\nid = \"duel\"").unwrap();
    std::fs::write(dir.join("main.luau"), "return { init = function() state.n = players.count(); state.names = {}; for _, p in players.list() do table.insert(state.names, p.name) end; state.bad = tostring(select(2, pcall(camera.set, 9, {}))) end }").unwrap();
    let g = game(&dir);
    let s = state(&g);
    assert_eq!(s["state"]["names"], json!(["Player 1", "Bot 1", "Bot 2"]), "{s}");
    assert!(s["state"]["bad"].as_str().unwrap().contains("player 9: no such player"), "{s}");
    assert_eq!(g.game_id(), "duel");
}

#[test]
fn the_start_menu_freezes_the_game_and_starts_a_new_round() {
    let mut g = game(&repo().join("games/hello-warp"));
    ticks(&mut g, 10, Controls::default());
    let start = Controls { pressed: Buttons(Buttons::START), held: Buttons(Buttons::START), ..Default::default() };
    g.tick(&[start]);
    let s = state(&g);
    assert_eq!(s["menu"], json!({ "items": ["Resume", "New round", "Quit"], "selected": 0 }), "{s}");
    ticks(&mut g, 30, Controls::default());
    assert_eq!(state(&g)["tick"], json!(10), "the game is frozen while the menu is open");
    // Down to New round, then confirm: the game starts over.
    g.tick(&[Controls { pressed: Buttons(Buttons::DOWN), ..Default::default() }]);
    assert_eq!(state(&g)["menu"]["selected"], json!(1));
    g.tick(&[Controls { pressed: Buttons(Buttons::SOUTH), ..Default::default() }]);
    let s = state(&g);
    assert_eq!((s["menu"].clone(), s["tick"].clone()), (Value::Null, json!(0)), "{s}");
    // Scripts and agents use the console.
    assert_eq!(g.command_line("menu").unwrap(), "menu open");
    assert_eq!(g.command_line("menu resume").unwrap(), "Resume");
    assert!(!g.menu_open());
}

/// A fake party host on loopback: one couch seat and one AI seat; the game
/// starts over with them, reports ready, runs on `start`, follows the
/// seat's controller, renames live, takes a new seat mid-game and reports
/// the room log and the end of the match.
#[test]
fn a_party_host_seats_the_players() {
    use std::net::TcpListener;
    use tungstenite::Message;
    let dir = folder("party");
    std::fs::write(dir.join("main.luau"), r#"
local function init()
    state.runs = (state.runs or 0) + 1
    state.ticks = 0
    state.names = {}
    state.joined = {}
    for _, p in players.list() do table.insert(state.names, p.name) end
end
local function tick(dt, input)
    state.ticks += 1
    state.forward = input[1].move_y
    if state.ticks == 5 then game.log("hello room"); game.finish() end
end
local function players_changed()
    state.names = {}
    for _, p in players.list() do table.insert(state.names, p.name) end
end
local function player_joined(p) table.insert(state.joined, p.name) end
return { init = init, tick = tick, players_changed = players_changed, player_joined = player_joined }
"#).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, got) = std::sync::mpsc::channel::<String>();
    let (go, step) = std::sync::mpsc::channel::<&str>();
    let host = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_millis(20))).unwrap();
        let mut ws = tungstenite::accept(stream).unwrap();
        let send = |ws: &mut tungstenite::WebSocket<std::net::TcpStream>, v: Value| ws.send(Message::Text(v.to_string().into())).unwrap();
        let seats = json!([{"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}},{"index":1,"occupant":{"kind":"ai"}}]);
        let mut sent_welcome = false;
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap();
                    let kind = v["type"].as_str().unwrap_or_default().to_string();
                    if kind == "hello" && !sent_welcome {
                        sent_welcome = true;
                        send(&mut ws, json!({"type":"welcome","protocol_version":1,"godot_sdk":6,"party":{}}));
                        send(&mut ws, json!({"type":"prepare","game":v["game"],"session":"s1","seats":seats,"players":[{"id":"p1","name":"Joep","color":"#ff0000","couch":0}]}));
                    }
                    if kind == "room_log" { tx.send(format!("room_log {}", v["text"].as_str().unwrap())).unwrap(); } else { tx.send(kind).unwrap(); }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return,
            }
            match step.try_recv() {
                Ok("start") => send(&mut ws, json!({"type":"start","session":"s1"})),
                Ok("pause") => send(&mut ws, json!({"type":"pause","session":"s1"})),
                Ok("resume") => send(&mut ws, json!({"type":"resume","session":"s1"})),
                Ok("tap") => send(&mut ws, json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":128,"axes":[0,0,0,0,0,0]}]})),
                Ok("release") => send(&mut ws, json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":0,"axes":[0,0,0,0,0,0]}]})),
                Ok("drive") => send(&mut ws, json!({"type":"controller_frame","controllers":[{"controller":"pad-a","buttons":0,"axes":[0,-32767,0,0,0,0]}]})),
                Ok("rename") => send(&mut ws, json!({"type":"players_updated","session":"s1","players":[{"id":"p1","name":"Joep B","color":"#ff0000","couch":0}]})),
                Ok("join") => send(&mut ws, json!({"type":"party_updated","session":"s1",
                    "seats":[{"index":0,"controller":"pad-a","occupant":{"kind":"local","player_id":"p1"}},{"index":1,"occupant":{"kind":"ai"}},{"index":2,"controller":"pad-b","occupant":{"kind":"local","player_id":"p2"}}],
                    "players":[{"id":"p1","name":"Joep B","couch":0},{"id":"p2","name":"Polle","couch":1}]})),
                Ok(_) => return,
                Err(_) => {}
            }
        }
    });
    let mut g = game(&dir);
    let client = gamenight::Client::connect(gamenight::Config { url: format!("ws://{address}"), game_id: "party-test".into(), token: String::new(), launched_by_daemon: true }).unwrap();
    g.party = Some(warp::party::Party::new(client));
    let mut seen = Vec::new();
    let mut wait = |g: &mut LuaGame, what: &str, seen: &mut Vec<String>| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            g.tick(&[Controls::default()]);
            while let Ok(m) = got.try_recv() { seen.push(m); }
            if seen.iter().any(|m| m == what) { return; }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        panic!("no {what}: {seen:?}\n{}\n{:?}", state(g), g.logs());
    };
    wait(&mut g, "ready", &mut seen);
    let s = state(&g);
    assert_eq!((s["phase"].clone(), s["state"]["names"].clone(), s["state"]["ticks"].clone()), (json!("ready"), json!(["Joep", "Player 2"]), json!(0)), "the AI seat is a bot; nothing ticks before start: {s}");
    assert_eq!(s["players"][0]["color"], json!([1.0, 0.0, 0.0]));
    assert_eq!(g.screen(), app::Screen::Hidden);
    go.send("start").unwrap();
    wait(&mut g, "room_log hello room", &mut seen);
    wait(&mut g, "finished", &mut seen);
    assert_eq!(g.screen(), app::Screen::Fullscreen);
    // A Start tapped as the game starts does not open it on its menu.
    let tap = |g: &mut LuaGame, go: &std::sync::mpsc::Sender<&str>| for step in ["release", "tap", "release"] {
        go.send(step).unwrap();
        for _ in 0..8 { g.tick(&[Controls::default()]); std::thread::sleep(std::time::Duration::from_millis(8)); }
    };
    tap(&mut g, &go);
    assert_eq!(state(&g)["menu"], Value::Null, "Start in the first second of play: {:?}", g.logs());
    for _ in 0..70 { g.tick(&[Controls::default()]); }
    tap(&mut g, &go);
    assert_eq!(state(&g)["menu"]["selected"], json!(0), "Start once the game runs opens the menu");
    tap(&mut g, &go);
    assert_eq!(state(&g)["menu"], Value::Null);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state(&g)["state"]["forward"].as_f64().unwrap_or(0.0) < 0.9 {
        go.send("drive").unwrap();
        g.tick(&[Controls::default()]);
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(std::time::Instant::now() < deadline, "the seat's controller drives player 1: {}", state(&g));
    }
    go.send("rename").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state(&g)["state"]["names"][0] != json!("Joep B") { g.tick(&[Controls::default()]); std::thread::sleep(std::time::Duration::from_millis(2)); assert!(std::time::Instant::now() < deadline); }
    go.send("join").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state(&g)["state"]["joined"] != json!(["Polle"]) { g.tick(&[Controls::default()]); std::thread::sleep(std::time::Duration::from_millis(2)); assert!(std::time::Instant::now() < deadline, "{}", state(&g)); }
    let s = state(&g);
    assert_eq!(s["state"]["runs"], json!(1), "a seat change never restarts the game");
    assert_eq!(s["players"].as_array().unwrap().iter().map(|p| (p["name"].as_str().unwrap().to_string(), p["view"].clone())).collect::<Vec<_>>(),
        vec![("Joep B".to_string(), json!(0)), ("Polle".to_string(), json!(1)), ("Player 2".to_string(), Value::Null)], "people first, each with a view");
    assert_eq!(g.local_players(), 2);
    // A host pause (Back, Alt+Tab) hands the screen to the lobby.
    for (step, screen) in [("pause", app::Screen::Hidden), ("resume", app::Screen::Fullscreen)] {
        go.send(step).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while g.screen() != screen { g.tick(&[Controls::default()]); std::thread::sleep(std::time::Duration::from_millis(2)); assert!(std::time::Instant::now() < deadline, "{step}: {:?}", g.screen()); }
    }
    go.send("stop").unwrap();
    host.join().unwrap();
}

/// Neon Trails, ported from the LÖVE party pack: four riders (one person,
/// three bots) on one shared view play rounds, and the same seed replays.
#[test]
fn neon_trails_plays_rounds_with_bots() {
    let run = || {
        let mut g = LuaGame::load(&repo().join("games/neon-trails"), &Overrides { bots: Some(3), ..Default::default() }, None).unwrap();
        g.echo = false;
        for _ in 0..3000 { g.tick(&[Controls::default()]); }
        state(&g)
    };
    let s = run();
    assert_eq!(s["errors"], json!([]), "{s}");
    assert_eq!(s["cameras"].as_array().unwrap().len(), 1, "one shared view");
    assert!(s["state"]["round"].as_u64().unwrap() >= 2, "rounds end and start again: {}", s["state"]);
    let points: u64 = s["state"]["scores"].as_object().unwrap().values().map(|v| v.as_u64().unwrap()).sum();
    assert!(points >= 3 && points % 3 == 0, "the last rider scores 3: {}", s["state"]["scores"]);
    assert_eq!(run()["state"], s["state"]);
}

const FX_GAME: &str = r##"
local function init()
    world.warm("cube", { color = "#ff00ff", emissive = 0.06 })
    state.box = world.spawn("cube", { solid = false })
    fx.burst(vector.create(0, 1, 0), { color = "#ff00ff", count = 20, speed = 5, life = 0.5, stretch = 0.02, flat = true })
    fx.ring(vector.create(0, 0.1, 0), { color = "#ff00ff", radius = 3, segments = 12 })
    local parts, rings = fx.count()
    state.parts, state.rings, state.fx = parts, rings, #world.find("fx")
    state.typo = tostring(select(2, pcall(fx.burst, vector.create(0, 0, 0), { colour = "red" })))
    post.set({ bloom = 0.5, bloom_radius = 3, bloom_wide = false, vignette = 0.5, grain = 0.2, aberration = 1 })
    state.ring = world.get(world.spawn("outline6:0.05", { solid = false })).shape
    state.bad_ring = tostring(select(2, pcall(world.spawn, "outline2")))
    local many = {}
    for i = 1, 33 do many[i] = { pos = vector.create(i, 1, 0), color = "#ff8000", intensity = 100 } end
    state.too_many = tostring(select(2, pcall(lights.set, many)))
    table.remove(many)
    lights.set(many)
end
local function tick(dt, input)
    world.place(state.box, vector.create(2, 0, 3), 1.5)
    state.aim = input[1].aim_x
    local parts, rings = fx.count()
    state.left = parts + rings
end
return { init = init, tick = tick }
"##;

#[test]
fn effects_fly_out_and_are_gone_after_their_life() {
    let dir = folder("fx");
    std::fs::write(dir.join("main.luau"), FX_GAME).unwrap();
    let mut g = game(&dir);
    let s = state(&g);
    assert_eq!((s["state"]["parts"].clone(), s["state"]["rings"].clone(), s["state"]["fx"].clone()), (json!(20), json!(1), json!(32)), "{s}");
    assert!(s["state"]["typo"].as_str().unwrap().contains("unknown option \"colour\""), "{s}");
    assert!(s["state"]["too_many"].as_str().unwrap().contains("at most 32 lights"), "{s}");
    assert_eq!(g.core().lamps.len(), 32);
    let post = g.core().post.clone();
    assert_eq!((post.bloom, post.vignette, post.bloom_wide), (Some(0.5), 0.5, false));
    assert_eq!(s["state"]["ring"], json!("outline6:0.05"));
    assert!(s["state"]["bad_ring"].as_str().unwrap().contains("3 to 64 sides"), "{s}");
    ticks(&mut g, 10, Controls { aim: [0.5, 0.0], ..Default::default() });
    assert_eq!(state(&g)["state"]["aim"], json!(0.5), "the right stick's position");
    ticks(&mut g, 50, Controls::default());
    let s = state(&g);
    assert_eq!(s["state"]["left"], json!(0), "spent after their life: {s}");
    assert_eq!(g.command_line("lua #world.find('fx')").unwrap(), "0");
    assert_eq!(g.command_line("lua world.get(state.box).pos.z").unwrap(), "3");
    assert_eq!(g.command_line("lua world.get(state.box).rot.y").unwrap(), "1.5");
}

#[test]
fn neon_siege_bots_clear_waves_and_replay_exactly() {
    let run = || {
        let mut g = LuaGame::load(&repo().join("games/neon-siege"), &Overrides { bots: Some(3), ..Default::default() }, None).unwrap();
        g.echo = false;
        for _ in 0..3000 { g.tick(&[Controls::default()]); }
        state(&g)
    };
    let s = run();
    assert_eq!(s["errors"], json!([]), "{s}");
    assert_eq!(s["cameras"].as_array().unwrap().len(), 1, "one shared view");
    let siege = &s["state"]["s"];
    assert!(siege["wave"].as_u64().unwrap() >= 3, "the bots clear waves: wave {}", siege["wave"]);
    assert!(siege["teamScore"].as_u64().unwrap() > 0 && siege["kills"].as_u64().unwrap() > 20, "{}", siege["kills"]);
    assert_eq!(siege["players"].as_array().unwrap().len(), 4);
    assert_eq!(run()["state"], s["state"], "the same seed plays the same siege");
}

const EXPOSURE_GAME: &str = r##"
local function init()
    light.set({ exposure = "auto", exposure_min = 4, exposure_max = 14, exposure_bias = 0.5, adapt_speed = 6 })
    state.bad = tostring(select(2, pcall(light.set, { exposure = "bright" })))
end
local function tick(dt, input)
    if game.tick() == 3 then light.set({ exposure = 11 }) end
end
return { init = init, tick = tick }
"##;

#[test]
fn exposure_can_be_automatic_and_tuned_or_fixed_again() {
    let dir = folder("exposure");
    std::fs::write(dir.join("main.luau"), EXPOSURE_GAME).unwrap();
    let mut g = game(&dir);
    let s = state(&g);
    assert!(s["state"]["bad"].as_str().unwrap().contains("\"auto\""), "{s}");
    let auto = g.core().light.auto.expect("exposure = \"auto\" turns it on");
    assert_eq!((auto.min_ev100, auto.max_ev100, auto.compensation, auto.speed_light, auto.speed_dark), (4.0, 14.0, 0.5, 6.0, 2.0));
    ticks(&mut g, 5, Controls::default());
    let light = g.core().light;
    assert_eq!((light.auto, light.ev100), (None, Some(11.0)), "a number fixes the exposure again");
}

#[test]
fn parented_entities_follow_rotate_collide_reparent_and_remove() {
    let dir = folder("parenting");
    std::fs::write(dir.join("main.luau"), r#"
local v = vector.create
local function near(a, b) assert(vector.magnitude(a - b) < 0.0001) end
local function init()
    local root = world.spawn("cube", {pos = v(9, 1, 5), size = 8, solid = false})
    local arm = world.spawn("cube", {pos = v(2, 0, 0), solid = false}, root)
    local tip = world.spawn("sphere", {pos = v(0, 0, 1), tag = "tip"}, arm)
    world.place(root, v(4, 2, 0), math.pi / 2)
    local at = v(5, 2, -2)
    near(world.get(tip).world_pos, at)
    assert(world.get(arm).parent == root)
    assert(world.overlap(at, 0.01, {tag = "tip"})[1] == tip)
    assert(world.raycast(at + v(0, 2, 0), v(0, -1, 0), 4).id == tip)
    assert(not pcall(world.set, root, {pos = v(999, 0, 0), shape = "missing.glb"}))
    near(world.get(root).pos, v(4, 2, 0))
    near(world.get(tip).world_pos, at)
    world.move(tip, v(1, 0, 0), 0.1)
    near(world.get(tip).world_pos, at + v(0.1, 0, 0))
    assert(not pcall(world.set_parent, root, tip))
    assert(not pcall(world.set_parent, tip, tip))
    assert(not pcall(world.set_parent, tip, 9999))
    assert(not pcall(world.spawn, "cube", {}, 9999))
    assert(world.get(tip).parent == arm and world.count() == 3)
    world.set_parent(arm, nil)
    near(world.get(arm).world_pos, v(2, 0, 0))
    world.remove(root)
    assert(world.exists(tip))
    world.remove(arm)
    assert(world.count() == 0)
    state.ok = true
end
return {init = init}
"#).unwrap();
    let g = game(&dir);
    assert_eq!(state(&g)["errors"], json!([]));
    assert_eq!(state(&g)["state"]["ok"], true);
}

#[test]
fn nested_local_libraries_reload_their_consumers_and_keep_the_last_good_version() {
    let dir = folder("local-libraries");
    std::fs::create_dir(dir.join("lib")).unwrap();
    let leaf = dir.join("lib/value.module.luau");
    write(&leaf, "return { n = 2 }");
    write(&dir.join("outer.module.luau"), r#"
        local value = require("lib/value")
        assert(value == require("lib/value"))
        return { read = function() return value.n end, tick = function() error("library ran as a hook") end }
    "#);
    write(&dir.join("main.luau"), r#"
        local outer = require("outer")
        return { tick = function() state.n = outer.read() end,
                 reload = function() state.reloads = (state.reloads or 0) + 1 end }
    "#);
    write(&dir.join("other.luau"), "return { reload = function() error('unrelated module reloaded') end }");
    let mut g = game(&dir);
    ticks(&mut g, 1, Controls::default());
    assert_eq!(state(&g)["state"]["n"], 2);
    write(&leaf, "return { n = 17 }");
    g.reload();
    ticks(&mut g, 1, Controls::default());
    assert_eq!(state(&g)["errors"], json!([]));
    assert_eq!(state(&g)["state"]["n"], 17);
    assert_eq!(state(&g)["state"]["reloads"], 1);
    write(&leaf, "return require('outer')");
    g.reload();
    assert!(state(&g)["errors"].to_string().contains("cyclic require"));
    std::fs::remove_file(&leaf).unwrap();
    g.reload();
    assert!(state(&g)["errors"].to_string().contains("unknown local library"));
    ticks(&mut g, 1, Controls::default());
    assert_eq!(state(&g)["state"]["n"], 17);
    write(&leaf, "return { n = 35 }");
    g.reload();
    ticks(&mut g, 1, Controls::default());
    assert_eq!(state(&g)["errors"], json!([]));
    assert_eq!(state(&g)["state"]["n"], 35);
    assert_eq!(state(&g)["state"]["reloads"], 2);
}

#[test]
fn save_slots_survive_restart_migration_conflicts_and_aborted_writes() {
    use warp::storage::Location;
    let dir = folder("storage-game");
    let user = folder("storage-user");
    let load = || LuaGame::load_with_storage(&dir, &Overrides::default(), None, Location::Directory(&user)).unwrap();
    write(&dir.join("main.luau"), "return {}");
    write(&dir.join("game.toml"), "storage=true\nid='storage-test'");
    let schema = dir.join("storage.json");
    write(&schema, r#"{"save":[{"type":"integer"}],"settings":[{"type":"object"}],"profile":[{"type":"object"}]}"#);
    let mut old = load();
    old.command_line("lua assert(storage.read('save','one') == nil); assert(storage.write('save','one',0,3) == 1)").unwrap();
    old.command_line("lua storage.write('settings','local',0,{xs=storage.array(),x=storage.null}); local d=storage.read('settings','local').data; assert(d.x==storage.null and #d.xs==0)").unwrap();
    old.command_line("lua assert(storage.write('profile','one',0,{name='Ana'})==1); assert(storage.read('profile','one').data.name=='Ana'); assert(storage.read('save','one').data==3)").unwrap();
    write(&schema, r#"{"save":[{"type":"integer"},{"type":"object","required":["points"],"properties":{"points":{"type":"integer","minimum":0}}}]}"#);
    let mut game = load();
    game.command_line("lua local d=storage.read('save','one'); assert(d.version==1 and d.data==3); assert(not pcall(storage.write,'save','one',d.revision,{points=-1})); assert(storage.write('save','one',d.revision,{points=d.data})==2)").unwrap();
    old.command_line("lua assert(not pcall(storage.write,'save','one',1,4)); assert(not pcall(storage.write,'save','one',2,4))").unwrap();
    let db = rusqlite::Connection::open(user.join("storage.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_update AFTER UPDATE ON documents BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
    game.command_line("lua assert(not pcall(storage.write,'save','one',2,{points=7}))").unwrap();
    drop(game);
    write(&dir.join("game.toml"), "storage=true\nid='other-game'");
    load().command_line("lua assert(storage.read('save','one')==nil)").unwrap();
    write(&dir.join("game.toml"), "storage=true\nid='storage-test'");
    load().command_line("lua local d=storage.read('save','one'); assert(d.version==2 and d.revision==2 and d.data.points==3)").unwrap();
    LuaGame::load(&dir, &Overrides::default(), None).unwrap()
        .command_line("lua assert(storage.read('save','one')==nil)").unwrap();
}

#[test]
fn gamenight_schema_validates_the_local_save_and_preserves_old_versions() {
    use warp::storage::{Location, Storage};
    let dir = folder("gamenight-schema");
    let user = folder("gamenight-schema-user");
    let open = || Storage::open(&dir, "score", Location::Directory(&user)).unwrap();
    let current = dir.join("save-schema.json");
    let v1 = json!({"$schema":"https://json-schema.org/draft/2020-12/schema",
        "$defs":{"Progress":{"type":"object","properties":{"wins":{"type":"integer"}},"required":["wins"]}}});
    write(&current, &v1.to_string());
    let mut store = open();
    assert!(store.write("save","one",0,json!({"wins":"wrong type"})).is_err());
    assert_eq!(store.write("save","one",0,json!({"wins":2})).unwrap(),1);
    drop(store);
    let history = dir.join("schema-history/save");
    std::fs::create_dir_all(&history).unwrap();
    std::fs::rename(&current,history.join("1.json")).unwrap();
    let v2 = json!({"$defs":{"Progress":{"type":"object","properties":{"victories":{"type":"integer"}},"required":["victories"]}},
        "x-gamenight-lenses":[{"from":"wins","to":"victories","op":"rename"}]});
    write(&current,&v2.to_string());
    let mut store = open();
    let old = store.read("save","one").unwrap().unwrap();
    assert_eq!(old.version,1);
    assert!(store.write("save","one",old.revision,old.data.clone()).is_err());
    store.write("save","one",old.revision,json!({"victories":old.data["wins"]})).unwrap();
    assert_eq!(store.read("save","one").unwrap().unwrap().version,2);
}

#[test]
fn luau_game_registers_nested_editor_pages_and_reloads_its_action() {
    use render::debug_ui::DebugAction;
    let dir = folder("editor-pages");
    let path = dir.join("main.luau");
    let src = r#"return {
        editor_pages = {
            { id = 1000, label = "Character" },
            { id = 1001, parent = 1000, label = "Grip", buttons = {{ id = 1002, label = "Apply" }} }
        },
        editor_action = function(id) state.last = id end
    }"#;
    write(&path, src);
    let mut g = game(&dir);
    let (pages, buttons) = g.debug();
    assert_eq!(pages[1].parent, Some(1000));
    assert_eq!(buttons[0].menu, Some(1001));
    g.debug_action(DebugAction::Activate { id: 1002 });
    assert_eq!(state(&g)["state"]["last"], 1002);
    write(&path, &src.replace("1002", "1003"));
    g.reload();
    g.debug_action(DebugAction::Activate { id: 1002 });
    assert_eq!(state(&g)["state"]["last"], 1002);
    g.debug_action(DebugAction::Activate { id: 1003 });
    assert_eq!(state(&g)["state"]["last"], 1003);
}
