# Making a game on Warp

For people and coding agents. To make a game without Rust, in Luau
scripts and sound patches only, use the [`warp` executable](warp-cli.md)
instead. For a Rust game, start from
[`engine/crates/app/examples/hello.rs`](../crates/app/examples/hello.rs)
(about 150 lines); [Growing Guns](../../games/growing-guns/README.md) is a
complete example with split screen, bots and online play.

## The loop

1. Write the change.
2. `cargo build -p your-game` (dev profile: seconds; release is only for
   final timing).
3. Look and check without a display:
   `KERN_ADAPTER=software your-game script s.json --out dir`, which saves
   frames and state dumps and fails on a broken expectation or an image
   that drifts from its reference (`compare`, written on the first run).
   `cargo run -p app --example hello -- --script
   engine/crates/app/examples/hello.json --out /tmp/hello` takes about 2 s
   on a CPU rasteriser and is deterministic (difference 0.0 between runs).
4. Measure with GPU timestamps when performance matters (`bench` in Growing
   Guns shows how).

## Structure

- Implement `app::Game`: `local_players`, `tick(&[Controls])` (fixed
  rate), `draw(&mut Frame) -> Draw` (cameras, `Lighting`, HUD list) and,
  for scripts and agents, `state()` as JSON.
- `app::run(Config, &mut game)` opens the window: fixed tick with
  interpolation (`frame.alpha`), keyboard, mouse and gamepads as
  `Controls` (one per local player), split screen for several cameras,
  F10 engine debug panel, F11 fullscreen, F12 screenshot. These reserved keys run before game input, including menus and text fields. Games receive F1–F9; F3 a game does not use opens the engine stats overlay, and `Game::debug_stats` adds the game's own lines to it. `app::Headless` and `app::capture`
  render the same frames to memory; `app::script` drives a game from JSON.
- Keep the simulation in its own crate without rendering. Deterministic
  ticks make scripted tests and online play straightforward.

## Scene

- Meshes: `render::primitives::{cube, sphere, capsule}(material).mesh()`
  or your own `ClusterMesh::build`. Materials: tint (linear), roughness,
  `flags` (`EMISSIVE` with `scale` as nits for glowing things).
- Models: `assets::gltf::load(&mut scene, path)` reads .glb or .gltf
  (node transforms, materials, base-colour and normal textures) into one
  mesh; `cargo run -p assets --example view_model -- model.gltf --out
  m.png` shows it from four sides without a window. Skinning and animation
  are not read yet; convert FBX to glTF first.
- Static things: `scene.add_instance(mesh, transform)`.
- Effects: `render::particles::Particles` (sparks, dust, drops, debris on
  slots, with a collision callback).
- Sound: `audio::sfx` (WAV clips, voice pool, spatial placement for one or
  more listeners) with `app::sound::start` for the speakers; scripts record
  it (`audio` step) so a test can check that a sound played.
- Online: the `net` crate (iroh: hole punching, relays, tickets) carries
  your own message types between a host and clients through channels the
  game loop polls; Growing Guns' `session.rs` shows host-authoritative
  play with input sequence numbers and look prediction, and `growing-guns
  nettest` runs a host and a client over loopback.
- Physics: `kern::collide` for characters (yaw boxes, rays, move and
  slide) and the `physics` crate (Rapier) for rigid bodies, blasts, joints
  and ragdolls. Movers and rays take any `kern::collide::Boxes`: a slice
  for a few hundred boxes, `BoxGrid` (uniform grid, `remove`/`restore`,
  stable indices) for thousands, such as a destructible level.
- Destruction: pre-split level pieces into chunks (no mesh rebuilds), keep
  them in a `BoxGrid` and in `Slots`, and remove both when a chunk breaks.
  `render::debris::Debris` throws GPU-simulated rubble (thousands of shards,
  colliding with a one-bit `Occupancy` grid you keep in step with the
  level); `primitives::hewn_block` gives bevelled and worn bricks. Growing
  Guns' `destruct.rs` and `view.rs` show the whole path, including sending
  breakage to clients.
- Things that come, go or move: `render::slots::Slots::reserve(&mut scene,
  n)` before the renderer exists, then `add`/`set`/`remove` and one
  `flush` per frame. Any mesh already in the scene can go in any slot.
  New meshes or materials need a new renderer (`frame.set_scene`), which
  recompiles pipelines (seconds); reserve up front instead.
- First person: `scene.instances[i].hide_in_views(1 << view)` hides a body
  from its own camera; it keeps its shadow.
- Sky: `scene.sky = Some(primitives::gradient_sky(zenith, horizon, ground))`.
- Explosions: `render::fx::Effects` (Growing Guns' Godot blast: lumpy fire
  and smoke billows, flame tongues, ember streaks, heat shimmer, flash and
  fireball lights), drawn in HDR after TAA with soft depth fades; spawn
  with `explosion`, then `flush` per frame and add `lights` to your local
  lights. Local lights take a `falloff` exponent (2 physical; Godot's
  `omni_attenuation` for a stylised long reach).
- Debug commands: implement `Game::command` (`give 0 explosive`,
  `teleport 0 1 2 3`) and scripts can set up any scenario with a `cmd`
  step instead of playing up to it.
- Up to 32 unshadowed point or spot lights per frame:
  `renderer.set_local_lights` (an omni light is `outer_cos: -1.0`).

## Costs to know

- The shade shader only compiles what the scene's materials reach. Car,
  terrain, grass, road or foliage material flags pull in their paths.
- `scene.mark_solid()` adds near-field contact shading. It loops over
  every SOLID instance per pixel: keep it to a handful.
- `Lighting::gi` marches foliage and terrain volumes. Without foliage or a
  heightfield the marches are skipped exactly; with them they are the
  largest shading cost.
- Units are physical: sun and sky in lux, exposure in EV100. The defaults
  (`Lighting::default()`) give a sunny day; start there.

## Gotchas

- `Controls::aim` preserves raw right-stick position (x right, y up), and
  `Controls::triggers` preserves left/right analog pressure in 0–1. Seat routing
  merges both. Headless steps accept `"triggers": [0.25, 0.8]`; digital LT/RT
  still work.
- Look in `Controls` is right- and up-positive radians per tick; convert to
  your yaw convention once.
- `Frame::set_scene` replaces the whole renderer; call it once, not per
  frame.
- Render loops outside `app` should use `Renderer::try_render_into` with a
  texture from `Renderer::output_texture`, so no output is allocated per
  frame; `KERN_CPU_SPANS=1` shows where a frame's CPU time goes.
- Pipe test output with `set -o pipefail` when you filter it, or a failing
  `cargo test` looks green.


## Desktop launch policy

Windows games open fullscreen by default, after their first complete frame.
Use `--windowed` or F11 for a window. During startup a minimized taskbar marker
shows the game icon without taking focus; there is no terminal or white loading
window. Gamepad Party warm launches stay invisible until the host requests them.
Sideways keeps its initial stage preparation hidden too; subsequent in-game
stage changes retain their loading presentation.

Native binary entry points use `#![cfg_attr(windows, windows_subsystem = "windows")]`
and call `app::startup::init("Game title", Some(include_bytes!("icon.png")))`
before loading assets. The shared shell installs that icon on the real window
and removes the marker after presenting the first correctly sized frame.
`None` uses the engine icon. Custom shells must call `app::startup::ready()`
after their first complete frame. Use `app::startup::fullscreen(&args)` for the
same explicit `--windowed` policy. This is enabled in all current desktop games.

The Luau runner reads the game title from its manifest/header and checks
`icon.png`, `assets/icon.png`, then `assets/branding/logo.png` in the game folder.
Headless tools create no loading marker. Redirected stdout/stderr are preserved;
console-launched tools attach to the parent console when needed. Unconnected
Windows GUI launches write diagnostics under `%TEMP%/warp-logs/`.

The Windows startup fixture verifies minimized state, installed icon, no focus
steal and cleanup. The complete fullscreen transition still needs a desktop
playtest; compilation/headless checks do not establish visual timing.
