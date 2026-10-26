# Sideways

A gravel rally game for GameNight, running on [Warp](../../engine/README.md).
Up to four players share a 4K output. The 120 FPS target is not yet reached.

`app/` contains the native application, HUD, visuals and GameNight adapter.
`sim/` contains stages, physics, AI and race state. `determinism/` retains
stable RNG/noise/math behavior without a Godot runtime. Existing Cargo package
names are retained for script compatibility.

Run from the repository root:

```powershell
cargo run --release -p cli -- play --views 1 --size 3840x2160
cargo run --release -p cli -- play --views 4 --size 3840x2160
games/sideways/tools/play-dev-windows.ps1 -Restart
games/sideways/tools/play-dev-windows.ps1 -BuildOnly
cargo run --release -p cli -- sim --ticks 45000 --seed 1 --km 2
```

The launcher builds while an owned playtest runs, then restarts it after a
successful build. Logs and executable manifests live in this game's
`build/dev-runs/`. F12 captures the displayed frame and camera/state metadata.
Old `engine/build/` runs remain historical evidence.

The latest measured developer optimization paths are an explicit opt-in:

```powershell
games/sideways/tools/play-dev-windows.ps1 -Restart -WorkReduction -Upscale dlss-quality
```

`-WorkReduction` enables one grass geometry submission, skipping opaque terrain
lighting hidden by fully opaque grass, and spatial headlight-caster selection.
The launcher records their effective settings and restores the parent process
environment afterward. These preserve the existing source appearance rather
than adding new visuals; exact full-image acceptance is still incomplete.
See [r47 measurements](../../engine/docs/meadow-work-r47.md) and
[r48 measurements](../../engine/docs/render-traversal-r48.md). Gains vary by
scene and must not be added together or presented as an 8.33 ms result.
The experimental world-volume trees remain disabled; see
[r50 rejection and next steps](../../engine/docs/world-foliage-r50.md).

Developer playtests default to fuller tree crowns (`-TreeFullness 1.25`, range
1..1.35) and tree occlusion culling. Use `-TreeFullness 1 -TreeOcclusion:$false`
for the reference settings. Explicit parameters take precedence over inherited
`SIDEWAYS_TREE_FULLNESS` / `KERN_TREE_OCCLUSION` (`0` or `1`), then launcher
defaults. The manifest records the resolved values and their source; the helper
restores the caller's environment after building or starting the game. Actual
constructed settings are in capture metadata; culling requires cluster buckets.

Controls: left stick steers, RT accelerates, LT brakes/reverses, A handbrake,
X reset, Y camera, right stick look around, Start menu.

Developer playtests also enable distant grass (`-MeadowAggregate`). Detailed
grass blends into a source-derived surface cache over60–80m; it fades into
matching terrain colour over160–180m. This is a directional surface atlas,
not voxel rendering. Use `-MeadowAggregate:$false` for the source-only reference.
An explicit switch overrides inherited `KERN_MEADOW_AGGREGATE`, then the launcher
default applies. `SIDEWAYS_MEADOW_SOURCE_RADIUS=100` retains the previous source
range for comparisons; valid values are60..100m. Captures report actual settings.
The current iteration improves coverage but retains a faint grid pattern on
some distant slopes; it is not a complete pop-in or120FPS solution. See the
[r39 results and limits](../../engine/docs/meadow-distance-r39.md).

Distributions place `assets/` and `data/` beside the executable.
`SIDEWAYS_GAME_ROOT` explicitly selects another game root; the source checkout
is a development fallback. Keep [CREDITS.md](CREDITS.md), asset licenses and
the font notice with distributions.

```powershell
python engine/tools/run-frame-scenario.py games/sideways/scenarios/fourplayer-route.toml --exe C:/path/sideways.exe --out C:/temp/new-run
games/sideways/tools/test-play-dev.ps1
```

See the [diagnostics guide](../../engine/tools/frame-budget.md) for measurement
contracts. GameNight acceptance tools are in `tools/`. The Godot implementation,
oracle comparison and export pipeline are removed.
