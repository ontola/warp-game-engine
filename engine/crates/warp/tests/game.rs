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
    assert_eq!((s["state"]["score"].clone(), s["state"]["grounded"].clone()), (json!(0), json!(true)));
    assert_eq!(s["tags"], json!({ "crate": 3, "floor": 1, "orb": 6, "player": 1 }));
    // Hold forward and jump: the player moves and leaves the ground.
    ticks(&mut g, 20, Controls { mv: [0.0, 1.0], ..Default::default() });
    g.tick(&[Controls { pressed: Buttons(Buttons::SOUTH), held: Buttons(Buttons::SOUTH), ..Default::default() }]);
    ticks(&mut g, 5, Controls::default());
    let s = state(&g);
    assert!(s["state"]["vel"][1].as_f64().unwrap() > 3.0, "jumping: {s}");
    assert_eq!(s["sounds"]["recent"].as_array().unwrap().last().unwrap()["sound"], "jump");
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
