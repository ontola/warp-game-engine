# Warp

Reusable Rust systems for procedural outdoor worlds, developed through
[Sideways](../games/sideways/README.md). Cargo runs from the repository root;
the game owns its simulation, assets, tuning and integration.

| Crate | Responsibility |
| --- | --- |
| `kern` | Simulation interfaces, deterministic digests, traces and timings |
| `render` | Cluster LOD, GPU visibility, shading, shadows, vegetation, atmosphere, HUD and reconstruction |
| `audio` | Synthesis, mixing and explicit caller-provided sound configuration |
| `app` | Game shell: window, fixed tick, input, split screen, headless frames and scripted playtests |
| `physics` | Rigid bodies on Rapier: boxes, spheres, capsules, impulses, blasts, joints, rays |
| `assets` | glTF import with materials and textures; baked weighted characters |
| `net` | Online play over iroh: typed host/client messages behind polled channels |
| `script` | Luau modules: sandbox, hot reload, deterministic step budget |

`assets/` contains licensed default fonts, reusable audio resources and
explicit renderer test fixtures. Game runtime meshes belong to Sideways.

Focus: realistic, performant procedural nature, vehicle physics APIs,
AI-driven workflows and fast iteration. A human editor UI, complete indoor
rendering, water, buildings, people and general animation remain missing.
Procedural plant wind is not general character animation.

```powershell
cargo check --locked --workspace
cargo test --locked -p render --lib
cargo test --locked -p audio --lib
python engine/tools/frame-budget.py --help
python engine/tools/render-category-report.py --help
```

New game? Read [making a game](docs/making-a-game.md) and run
`cargo run -p app --example hello`. The [documentation index](docs/README.md)
has guides for every crate (`physics`, `assets`, `net` and `script` too)
and lists the dated engineering notes.

See the [diagnostics guide](tools/frame-budget.md),
[live render debug panel](docs/debug-ui.md),
[forest roadmap](docs/forest-rendering-roadmap.md),
[shared vegetation API and validation](docs/unified-vegetation.md),
[four-view grass timing and visual tradeoffs](docs/meadow-half-resolution-r57.md),
[full-resolution sampling and raw-buffer diagnostics](docs/meadow-sampling-r58.md),
[analytic grass contact AO](docs/meadow-contact-ao-r59.md),
[atmosphere notes](ATMOSPHERE.md), [handoff](HANDOFF.md) and
[category measurements](docs/render-categories-2026-10-24.md).
Old development notes are preserved in the game's historical documentation.
