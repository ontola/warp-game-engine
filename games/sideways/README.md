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

Sideways loads its saved vegetation baseline from
[`data/render-baseline.toml`](data/render-baseline.toml): trees at 150% size,
45% population and crown RGB 100/115/90%; grass at 200% size, 55% patch count,
RGB 130/90/130%, 4 px source LOD error and 100% draw range. These are the last
applied settings from the 26 October playtest, not Warp engine defaults.
The earlier 15% grass count is retained in the file as a named reference only.

The preset applies at startup before graphics resources are created. Applied
tree edits carry to subsequent stages, including matching native trunk
collisions, canopy and shadow resources. Reset restores the saved preset;
pending tree edits require Apply and do not become the next stage's settings.
Interactive edits are session-local and do not overwrite the file. To change
the persistent preset, edit the game's data file; bundles use the copy beside
the executable through the normal `SIDEWAYS_GAME_ROOT` locator. Weather,
time of day and lighting are not part of this vegetation preset. Historical
benchmarks used their recorded settings; do not compare them as identical
configurations to this new baseline.

For an authored-source fixture, explicitly set `SIDEWAYS_RENDER_BASELINE=authored`
(`saved` or unset selects the saved preset). Captures record the mode and all
resolved values. Authored mode keeps neutral grass/tree settings and leaves
packed voxel/branch/world experiments untouched; tree editing is unavailable
there. A non-neutral saved tree preset fails explicitly for such scenes, rather
than changing their source while retaining stale packed assets.

Controls: left stick steers, RT accelerates, LT brakes/reverses, A handbrake,
X reset, Y camera, right stick look around, Start menu.

The engine debug Lighting submenu includes **Highlight bloom** and a draggable
**Bloom strength** control (4% default, 0-20%). Bloom softens bright lamp cores
after reconstruction and before AgX; it does not increase how strongly the
lamps illuminate the road. Rear/brake/reverse lights and exhaust flashes now
provide short-range environmental light without additional shadow maps.
See [r51 evidence and limits](../../engine/docs/local-lights-and-vegetation-r51.md).

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
