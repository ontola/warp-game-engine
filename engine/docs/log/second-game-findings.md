# A second game on Warp: findings

Growing Guns (`games/growing-guns`), a first-person arena shooter, was ported
as a vertical slice to see what Warp costs a game that is not Sideways. The
goal is a fast engine that is fast to iterate with. Numbers below are from a
2-core cloud container with llvmpipe unless stated; real-GPU numbers come
from `C:\dev\warp-bench\gg-bench.ps1` (headless, no window).

## What worked immediately

- Split screen: `split_rects` and one `try_render_texture` for all views.
- The HUD draw list (shapes and text) and the present path.
- Headless capture to PNG, which let every change be checked from images
  without a display.
- GPU timestamps per pass (`set_async_gpu_profiling`) and the startup
  profiler (`KERN_STARTUP_PROFILE=1`).

## Iteration speed

| Build | Change | Time |
|---|---|---|
| dev | edit in game code | 2 to 5 s |
| dev | edit in `render` | about 5 s |
| release (thin LTO) | any edit | about 3 min |
| dev | clean | 13.5 min |

The dev profile both builds and runs fast enough to play (CPU submit 1.5 ms
against 1.0 ms in release), so release is for final measurements only.

Startup dominated iteration: `Renderer::new` for a 256-instance scene took
4.3 s, of which 4.1 s was the shade pipeline, one uber-shader of about
2,700 lines that always included car paint, terrain layers, grass lighting,
needles, tracks and experimental sun visibility. Specialising it to the
scene (below) cut that to about 1.5 s. Every shader edit and, in Growing
Guns, every new arena pays this cost, because a new scene means a new
renderer.

## Changes made

- `render::shade_features`: the main shade pipeline sets WGSL override
  constants to false for paths the scene cannot reach (from its materials
  and construction-time resources). Other shaders that include `shade.wgsl`
  keep the defaults (true). Images are pixel-identical to
  `KERN_SHADE_FEATURES=all`; Sideways scenes keep every feature.
- `scene::EMISSIVE`: an untextured self-lit material (`tint * scale` nits).
- `GpuInstance::hide_in_views(mask)`: hide an instance from chosen camera
  views while it still casts shadows and shows elsewhere (first-person
  bodies). Uses free instance flag bits 8..15 and `View.grass.z`.
- `render::primitives`: unit cube, sphere and capsule with outward winding.
- `engine/crates/app`: the game shell (window, surface, fixed tick with
  interpolation, keyboard, mouse and gamepads as one gamepad-shaped
  `Controls`, split-screen frames with HUD, F11, F12 screenshots, smoke
  frames, and `capture` for the same frame path headless). Growing Guns'
  `play` and `shot` use it; the window code in the game went from about 300
  lines to none.

## Later rounds

- `render::slots::Slots`: reserve empty instances, then add, move,
  re-mesh and remove objects at runtime; Growing Guns now keeps one
  renderer for the whole session and refills the arena each round.
- GI marches are skipped (exactly) when there is no foliage or heightfield:
  shade pass 393 to 171 ms on llvmpipe at 640x360, pixel-identical.
  Contact shading loops over every SOLID box per pixel; Growing Guns stopped
  marking its arena SOLID. Frame 499 to 151 ms.
- `app::Headless` and `app::script`: JSON playtests with captures, state
  dumps and expectations; `Game::state()`.
- `primitives::gradient_sky`, `examples/hello.rs` and
  [making-a-game.md](../making-a-game.md).

## Still in the way

- New meshes or materials still need a new renderer (pipeline compile).
- Fixed per-frame work for features a scene does not use (exhaust, wheel
  particles, canopy filtering).
- No collision layer for simple games; Growing Guns uses its own box tests.
- The default look is pale (AgX, physical units) without tuning.
- `render` is about 50k lines, 26k of them vegetation; `frame.rs` refers to
  Sideways features more than 500 times.
- A clean dev build of `app` (hello example) takes 8.6 min on 2 cores
  (991 CPU-seconds). Largest units: naga 73 s, wgpu-core 73 s, render 46 s,
  wayland-protocols 43 s, x11rb-protocol 40 s, wgpu-hal 34 s. Fonts are
  built twice: cosmic-text 0.19 uses skrifa 0.40 / read-fonts 0.37 while its
  swash 0.2.10 uses skrifa 0.44 / read-fonts 0.41 (about 140 CPU-seconds).
  Pinning an older swash would remove that but may change HUD glyphs. Agents
  should keep `target/` between sessions.

## Destruction

Growing Guns' Godot destruction (pre-split chunks, health, GPU rubble) is
ported. What the engine gained for it:

- `kern::collide::BoxGrid` and the `Boxes` trait: a uniform-grid
  broadphase with removal, so movers and rays stay cheap over 6,000 to
  9,400 chunks. Results are identical to the plain scan (tested). The bots'
  simulation still runs at 0.07 ms per tick in release, a new arena of
  chunks included.
- `render::debris`: GPU-simulated shards in a reserved instance range,
  colliding with a one-bit occupancy grid; nothing is read back. 300
  shards fall, bounce and settle on llvmpipe in the GPU test.
- `primitives::hewn_block` (bevelled, optionally pitted bricks),
  `Physics::remove_static`, `Slots::is_used`.
- `Game::command` and the script `cmd` step: set up a scenario (give
  cards, teleport, aim, blast) instead of playing up to it.
- Fix: the shadow cluster capacity counted empty slots as zero, so a
  scene whose slots later filled up (a bigger arena next round) could hit
  the capacity assertion. Empty instances now count as the largest mesh.

Godot's running bond left gaps at 8 m tile borders (each tile had its own
edge seed); edges now depend on the parent block and the tile's position
along each axis, and a test checks walls and floors for holes.
