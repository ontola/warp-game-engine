# Warp + Sideways

[Warp](engine/README.md) is a Rust engine for realistic, performant procedural
outdoor worlds. [Sideways](games/sideways/README.md) is its rally game for
[GameNight](https://gamenight.ontola.io), with up to four local players.

One Cargo workspace and lockfile, with separate engine and game directories.
[Growing Guns](games/growing-guns/README.md), an arena shooter, is the second
game on the engine. [Hello Warp](games/hello-warp/README.md) is the third: a
game made only of Luau scripts and sound patches, run by the
[`warp` executable](engine/docs/warp-cli.md) without compiling anything. Documentation starts at
[engine/docs/README.md](engine/docs/README.md); [`website/`](website/README.md)
builds it into a static site.

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

Current work (27 October 2026): follow the [ordered four-view budget plan](engine/docs/four-view-budget-plan-r55.md).
The repeated four-player 4K-output route measures 32.16/40.35 ms with DLSS
Quality and 27.67/33.93 ms with Performance in its two sections. These are
medians of three run medians, not native 4K or typical-gameplay guarantees.
See [the resolution sweep and per-pass costs](engine/docs/resolution-sweep-r55.md).
The subsequent [opt-in grass handoff trial](engine/docs/meadow-band-r56.md)
measures 26.95/33.35 ms with Performance, a 2.3-2.4% same-executable improvement.
Both the 25 ms decision gate and 4 ms grass budget remain unmet; the next
quality/target decision is pending. Renderer timing now identifies each view
and the aggregate grass depth/material/illumination costs separately.
The subsequent [earlier/half-resolution grass experiment](engine/docs/meadow-half-resolution-r57.md)
measures 25.94/30.23 ms against 27.83/34.10 ms in the same executable.
It reduces grass cost to 5.00/5.78 ms, but visibly smears distant grass/ground:
**opt-in only, not accepted as the default**. The earlier transition alone
does not save time. Full-output comparisons and per-pass CSVs are in the report.
The [full-resolution sampling follow-up](engine/docs/meadow-sampling-r58.md)
removes redundant aggregate reads: 15–21% less aggregate time and 0.64/0.76 ms
less total GPU interval in a six-run comparison. Raw depth/motion and coverage
masks match; image review found no new blur. It remains opt-in. The larger
cost is overlapping per-root pixel work, which still needs structural reduction.
An [opt-in grass contact-AO experiment](engine/docs/meadow-contact-ao-r59.md)
adds bounded root footprints to terrain skylight, without a new render pass.
Six repeats show no large cost increase above run variation; the visual effect
is subtle in the daylight fixture. It is not directional grass shadowing.
World-grid traversal of shared plant volumes and early whole-owner replacement
are experimental, disabled by default, and still require GPU/visual validation.

The Godot implementation and export workflow are retired. Historical
measurements remain as evidence, without ongoing comparisons.
[Third-party asset credits](games/sideways/CREDITS.md).

Growing Guns stadium lighting now has [cached diffuse sky return](engine/docs/stadium-sky-occlusion.md),
including invalid-probe rejection. Native 4K/four-view fixture: +0.43 ms GPU;
covered rows retain more detail. Direct-sun bounce remains missing.
