# Warp

Reusable Rust systems for procedural outdoor worlds, developed through
[Sideways](../games/sideways/README.md). Cargo runs from the repository root;
the game owns its simulation, assets, tuning and integration.

| Crate | Responsibility |
| --- | --- |
| `kern` | Simulation interfaces, deterministic digests, traces and timings |
| `render` | Cluster LOD, GPU visibility, shading, shadows, vegetation, atmosphere, HUD and reconstruction |
| `audio` | Synthesis, mixing and explicit caller-provided sound configuration |

`assets/` contains licensed default fonts, reusable audio resources and
explicit renderer test fixtures. Game runtime meshes belong to Sideways.

Focus: realistic, performant procedural nature, vehicle physics APIs,
AI-driven workflows and fast iteration. A human editor UI, complete indoor
rendering, water, buildings, people and general animation remain missing.
Procedural plant wind is not general character animation.

The latest grass experiment adds opt-in native spatial RGB filtering and
source-accepted basal edge contact. Grass alpha, wind and source geometry stay
intact; distant shading variance is reduced in a static crop, with a measured
brightness shift. Final native 4K captures and selected wind poses completed;
user acceptance remains pending. See
[r64 implementation, measurements and limits](docs/meadow-edge-grain-r64.md).

```powershell
cargo check --locked --workspace
cargo test --locked -p render --lib
cargo test --locked -p audio --lib
python engine/tools/frame-budget.py --help
python engine/tools/render-category-report.py --help
```

See the [diagnostics guide](tools/frame-budget.md),
[live render debug panel](docs/debug-ui.md),
[forest roadmap](docs/forest-rendering-roadmap.md),
[shared vegetation API and validation](docs/unified-vegetation.md),
[four-view grass timing and visual tradeoffs](docs/meadow-half-resolution-r57.md),
[full-resolution sampling and raw-buffer diagnostics](docs/meadow-sampling-r58.md),
[analytic grass contact AO](docs/meadow-contact-ao-r59.md),
[rounded grass blade shading](docs/meadow-rounded-shading-r62.md),
[curved grass, travelling wind and r63 measurements](docs/meadow-windswept-r63.md),
[distant grass filtering and edge contact](docs/meadow-edge-grain-r64.md),
[atmosphere notes](ATMOSPHERE.md), [handoff](HANDOFF.md) and
[category measurements](docs/render-categories-2026-10-24.md).
Old development notes are preserved in the game's historical documentation.
