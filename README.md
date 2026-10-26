# Warp + Sideways

[Warp](engine/README.md) is a Rust engine for realistic, performant procedural
outdoor worlds. [Sideways](games/sideways/README.md) is its rally game for
[GameNight](https://gamenight.ontola.io), with up to four local players.

One Cargo workspace and lockfile, with separate engine and game directories.

| Location | Responsibility |
| --- | --- |
| `engine/crates/` | Reusable rendering, audio and simulation interfaces |
| `engine/tools/` | Frame accounting and iteration diagnostics |
| `games/sideways/app/` | Native app, HUD and GameNight integration |
| `games/sideways/sim/` | Physics, stages, AI and race state |
| `games/sideways/determinism/` | Stable RNG, noise and math |
| `games/sideways/assets/`, `data/` | Game assets, licenses and tuning |
| `games/sideways/tools/`, `scenarios/` | Launchers, acceptance and replays |

From the repository root:

```powershell
cargo run --release -p cli -- play --views 1
games/sideways/tools/play-dev-windows.ps1 -Restart
```

Current focus: procedural nature, trees and grass, sky, vehicle physics,
AI-driven workflows and short iteration loops. Missing general systems:
a human editor UI, indoor scenes, water, buildings, people and general animation.
These priorities can change.

Target: four players in one 4K output at 120 FPS; not yet reached.
See the [forest roadmap](engine/docs/forest-rendering-roadmap.md) and
[category measurements](engine/docs/render-categories-2026-10-24.md).

Current work (26 October 2026): reduce foliage traversal/raster cost, preserve
appearance across vegetation LODs, and measure full GPU frame intervals.
The latest controlled four-player 4K-output route measured about 38–44.5 ms
with DLSS Quality; this is not native 4K or a typical-gameplay guarantee.
See [the latest measured iteration](engine/docs/lod-shadow-demand-r49.md).
World-grid traversal of shared plant volumes and early whole-owner replacement
are experimental, disabled by default, and still require GPU/visual validation.

The Godot implementation and export workflow are retired. Historical
measurements remain as evidence, without ongoing comparisons.
[Third-party asset credits](games/sideways/CREDITS.md).
