# Making a game on Warp

For people and coding agents. Start from
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
  F11 fullscreen, F12 screenshot. `app::Headless` and `app::capture`
  render the same frames to memory; `app::script` drives a game from JSON.
- Keep the simulation in its own crate without rendering. Deterministic
  ticks make scripted tests and online play straightforward.

## Scene

- Meshes: `render::primitives::{cube, sphere, capsule}(material).mesh()`
  or your own `ClusterMesh::build`. Materials: tint (linear), roughness,
  `flags` (`EMISSIVE` with `scale` as nits for glowing things).
- Static things: `scene.add_instance(mesh, transform)`.
- Things that come, go or move: `render::slots::Slots::reserve(&mut scene,
  n)` before the renderer exists, then `add`/`set`/`remove` and one
  `flush` per frame. Any mesh already in the scene can go in any slot.
  New meshes or materials need a new renderer (`frame.set_scene`), which
  recompiles pipelines (seconds); reserve up front instead.
- First person: `scene.instances[i].hide_in_views(1 << view)` hides a body
  from its own camera; it keeps its shadow.
- Sky: `scene.sky = Some(primitives::gradient_sky(zenith, horizon, ground))`.
- Up to 16 unshadowed point or spot lights per frame:
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

- Look in `Controls` is right- and up-positive radians per tick; convert to
  your yaw convention once.
- `Frame::set_scene` replaces the whole renderer; call it once, not per
  frame.
- Pipe test output with `set -o pipefail` when you filter it, or a failing
  `cargo test` looks green.
