# Workspace and crates

One Cargo workspace and one lockfile at the repository root. Cargo always
runs from the root. Engine crates live under `engine/crates/`, games under
`games/`. Keep engine APIs independent of any game: the game supplies its
assets, settings and tuning.

```text
Cargo.toml, Cargo.lock      the workspace
engine/
  crates/                   the engine crates (below)
  assets/                   licensed default fonts, reusable audio resources,
                            renderer test fixtures
  tools/                    Python diagnostics: frame budgets, scenarios
  docs/                     these guides and the engineering notes
games/
  sideways/                 rally game: app/, sim/, determinism/, assets/,
                            data/, scenarios/, tools/
  growing-guns/             arena shooter: app/, sim/, mods/, scripts/,
                            assets/, tools/
website/                    the static site built from these docs
```

## Engine crates

| Crate | What it does | Main dependencies |
| --- | --- | --- |
| `kern` | The core: the `Sim` trait for a deterministic fixed-tick simulation, state digests, first divergence between runs, traces of queried values, timings per system, and simple collision (`kern::collide`: yaw boxes, rays, a mover with slide, the `BoxGrid` broadphase). | serde, sha2, glam |
| `render` | The renderer: GPU-driven clusters with LOD, a visibility buffer, HDR shading in physical units, sun and sky light, shadows, vegetation, atmosphere, particles, effects, HUD, temporal reconstruction and optional DLSS. Headless by design. | wgpu, glyphon, meshopt |
| `app` | The game shell: window and presentation, a fixed tick with interpolation, keyboard, mouse and gamepads as one `Controls` type, split screen, F11 fullscreen, F12 screenshots, headless frames (`Headless`, `capture`), JSON scripted playtests (`app::script`), menus (`app::ui`) and speaker output (`app::sound`). | render, audio, winit, gilrs, cpal |
| `audio` | Engine-sound synthesis, tyre sounds, sample playback for effects (`sfx`), WAV and Ogg Vorbis decoding, spectrograms, and the procedural synth (`audio::synth`). | lewton, serde, toml |
| `physics` | Rigid bodies on Rapier with enhanced determinism: static boxes, dynamic boxes, spheres and capsules, impulses, radial blasts, ball joints and rays. | rapier3d, kern, glam |
| `assets` | Asset import: static glTF models with materials and textures (`assets::gltf`), and baked weighted characters (`assets::skinned`). | render, gltf, image |
| `net` | Online play over iroh (QUIC with hole punching and relays): a host and clients exchange the game's own typed messages through channels the game loop polls. | iroh, tokio, postcard |
| `live` | Live control over MCP: a Streamable HTTP server on localhost with a bearer token, off the game thread; games add tools through the `LiveGame` trait. The `warp-mcp` stdio bridge. See [live control](live-control.md). | tokio, hyper, image |
| `script` | Luau scripting: sandboxed modules from a folder, hot reload, a step budget per call and a memory cap. | mlua (Luau) |

How the crates depend on each other:

```text
kern  <-  physics
render  <-  assets
render, audio  <-  app
net, script, live: standalone
```

A game picks what it needs. Sideways uses `kern`, `render` and `audio`
directly, with Rapier in its own simulation crate. Growing Guns uses all of
them.

## Game packages

| Package | Path | Notes |
| --- | --- | --- |
| `sideways` | `games/sideways/sim` | Stages, car physics (Rapier), AI, race state. No rendering. |
| `gdcompat` | `games/sideways/determinism` | Stable RNG, noise and math. Not a Godot runtime dependency. |
| `cli` | `games/sideways/app` | Builds the `sideways` executable: play, shot, sim, bench, sound, gpu-run. |
| `growing-guns` | `games/growing-guns/sim` | 60 Hz gameplay: arena generator, movement, weapons, rounds, bots, wave survival. Implements `kern::Sim`. |
| `growing-guns-app` | `games/growing-guns/app` | Builds the `growing-guns` executable: play, shot, bench, sim, script, nettest, serve, mods-api. |

Package names are kept stable for scripts and tools, which is why the
Sideways app package is still called `cli`.

## Build profiles

From the root `Cargo.toml`:

- `dev`: workspace crates at `opt-level = 1` with line tables, every
  dependency at `opt-level = 3` without debug info. This is the iteration
  profile; it plays at a usable frame rate.
- `release`: `opt-level = 3`, thin LTO, four codegen units. For final
  timing and packaging.

On Linux, `.cargo/config.toml` links with clang and lld.
