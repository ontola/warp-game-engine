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
games/sideways/tools/play-dev-windows.ps1 -Views 4 -Size 3840x2160 -Restart
games/sideways/tools/play-dev-windows.ps1 -Restart
games/sideways/tools/play-dev-windows.ps1 -BuildOnly
cargo run --release -p cli -- sim --ticks 45000 --seed 1 --km 2
```

The launcher builds while an owned playtest runs, then restarts it after a
successful build. Logs and executable manifests live in this game's
`build/dev-runs/`. F12 captures the displayed frame and camera/state metadata.
Old `engine/build/` runs remain historical evidence.

For a managed GameNight preview, register the package with the graphics profile
from a recorded developer playtest. Copying only the executable is insufficient:
`SIDEWAYS_PROCEDURAL_GRASS=field` selects the new GPU meadow; without it the
legacy photographed grass is constructed. `KERN_MEADOW_AGGREGATE=1` alone does
not select the meadow.

The GPU meadow defaults to `SIDEWAYS_MEADOW_PROFILE=windswept`: curved blades,
yellow-green tips and travelling noise wind. Six segments smoothly morph into
one triangle over growth-scaled 3–6 m; the cooked three-segment cut is unused.
Current r63e code defaults to source geometry through 250 m, fading over
200–250 m. `SIDEWAYS_MEADOW_WINDSWEPT_RADIUS` accepts 60–256 m and ignores
classic source-band/radius flags. This is an interim source comparison; a
source-derived, terrain-following far layer is proposed but not implemented.
Windswept forces the aggregate off and rejects group cache. Use
`SIDEWAYS_MEADOW_PROFILE=classic` for the previous recipe, the aggregate
experiments below and r62 rounded shading.
See [r63 source equations, runtime choices and measurements](../../engine/docs/log/meadow-windswept-r63.md).

Detailed woody species are now built from reusable twig/foliage assemblies.
`tree_species_compare` renders each candidate beside the retained textured tree
at the same metre scale; `plant_spray_gallery` exposes all six component variants.
The source path shares meshes and evaluates branch/tip poses on the GPU.
`SIDEWAYS_TREE_SPECIES=mixed` currently fails explicitly: the rejected v1
whole-tree flattening path has been removed, and forest-scale assembly LOD/pose
selection is not ready. The default live forest retains its reference assets.
`SIDEWAYS_MEADOW_SPECIES=mixed` still selects three windswept grass forms in
coherent world patches. See [species architecture and evidence](../../engine/docs/log/vegetation-species.md).

The current [r64 grass experiment](../../engine/docs/log/meadow-edge-grain-r64.md)
adds two opt-in flags: `KERN_MEADOW_SPATIAL_FILTER=1` filters distant rendered
RGB while preserving grass alpha exactly; `KERN_MEADOW_EDGE_SHADOW=1` adds
source-accepted basal contact on terrain/road. Filtering runs only on native
source grass without TAA/DLSS. Wind and current lighting remain live; far
geometry cost and silhouette aliasing remain. A static far crop shows less
contrast and a brightness shift. Combined native 4K captures and selected wind
poses completed; user acceptance is pending.

For the matched r64 preview, pass
`-DebugUiScript games/sideways/scenarios/windswept-grass-edge-settings.json`
to `play-dev-windows.ps1` with one native 3840×2160 view. The launcher copies
and hashes this script. It sets session grass size 1.6/density 1.0 through the
debug UI and closes the panel; the saved size 2.0/density 0.55 baseline stays
unchanged. Verify the resolved values in capture metadata after launch.

```powershell
games/sideways/tools/register-gamenight-windows.ps1 -PackageDirectory C:/path/to/package -ReferencePlaytestManifest C:/path/to/dev-run/iteration.json -DebugUi
games/sideways/tools/test-register-gamenight.ps1
```

The package must include its verified executable, game assets and DLSS runtime.
The helper copies the complete preview graphics profile, remaps packaged asset
paths, preserves other games and backs up registration changes. `-ValidateOnly`
reviews without writing. It does not launch or stop GameNight. Its default is
4K output with DLSS Performance; GameNight supplies the actual player count.
These flags remain scoped to the registered preview, not engine defaults.
After launch verify the log identifies `GPU meadow`, `full terrain field` and
`no photographed grass or CPU tuft pool`; process startup alone is insufficient.

Four-player developer launches now select **DLSS Performance** unless `-Upscale`
is explicitly supplied. One 4K output contains four panels: the measured input
is 958x538 per panel, reconstructed to 1916x1076 after divider margins. The HUD
uses output resolution. Actual dimensions are reported by DLSS capture metadata.
Use `-Upscale native`, `dlss-quality` or `dlss-balanced` to override; one to three
views keep their previous native fallback. This launcher validates the optional
NVIDIA SDK/runtime and builds with `render/dlss`; missing support is an error,
not a silent native fallback. Generic packaged builds still require explicit
DLSS support before requesting it. The launch manifest records mode and source.

This follows the user's 27 October request to use DLSS for four players. It does
not accept other graphics experiments or reduce the output resolution. The
[r55 sweep](../../engine/docs/log/resolution-sweep-r55.md) and
[r56 grass trial](../../engine/docs/log/meadow-band-r56.md) already used DLSS;
their 27-33 ms Performance results are not native-1080p-per-player measurements.

An experimental earlier-grass tier is available for controlled comparisons:
`KERN_MEADOW_AGGREGATE=1`, `SIDEWAYS_MEADOW_SOURCE_BAND=25,35`, and
`KERN_MEADOW_AGGREGATE_HALF_RES=1`. The last flag reduces only the aggregate
depth/material/lighting targets to half their internal width and height. Source
grass and the final merge stay at full internal resolution; world placement,
density, color, wind and the 160-180 m outer fade remain unchanged. The merge
uses full-resolution opaque depth and reconstructed motion. This is an optical
approximation and stays off by default. The [r57 measured comparison](../../engine/docs/log/meadow-half-resolution-r57.md)
saves 1.89/3.87 ms total GPU interval, but visibly smears distant grass/ground.
It is not visually equivalent or accepted for default use; temporal acceptance
also remains open. Moving the existing aggregate closer alone did not save time.
It cannot be combined with `KERN_MEADOW_GROUP_CACHE`. Captures record the actual
aggregate target dimensions, ratios and allocated target bytes for every view.

`KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE=1` is a separate full-resolution experiment:
skip zero-weight directional reads and fetch the selected surface payload once.
It reduces aggregate time 15–21% in the [r58 comparison](../../engine/docs/log/meadow-sampling-r58.md),
with exact captured depth/motion and coverage masks and no newly observed blur.
It stays off by default and is incompatible with the experimental group cache.
`KERN_MEADOW_CAPTURE_RAW=1` saves pre-DLSS layers alongside scenario captures;
it performs synchronous readback and is for explicit diagnostics only.

`KERN_MEADOW_CONTACT_AO=1` separately enables soft grass-root contact footprints
in terrain diffuse skylight. It follows canonical root placement, live size and
density, and adds no shadow pass. Direct sun and headlights remain unchanged.
The [r59 OFF/ON comparison](../../engine/docs/log/meadow-contact-ao-r59.md) found no
large cost increase above run variation; the daylight effect is subtle.
It remains off by default, without claiming directional grass shadows.

`SIDEWAYS_MEADOW_BLADE_SHADING=rounded` is an opt-in grass appearance experiment:
smooth blade normals along the curve and round them across the width, including
every source LOD and the rebuilt distant atlas. It requires the GPU meadow
(`SIDEWAYS_PROCEDURAL_GRASS=field`). Unset or `flat` retains existing shading.
Placement, geometry, colour and wind stay the same. The launcher and captures
record the mode; replays reject a mismatch. See [r62 details and validation](../../engine/docs/log/meadow-rounded-shading-r62.md).

The latest measured developer optimization paths are an explicit opt-in:

```powershell
games/sideways/tools/play-dev-windows.ps1 -Restart -WorkReduction -Upscale dlss-quality
```

`-WorkReduction` enables one grass geometry submission, skipping opaque terrain
lighting hidden by fully opaque grass, and spatial headlight-caster selection.
The launcher records their effective settings and restores the parent process
environment afterward. These preserve the existing source appearance rather
than adding new visuals; exact full-image acceptance is still incomplete.
See [r47 measurements](../../engine/docs/log/meadow-work-r47.md) and
[r48 measurements](../../engine/docs/log/render-traversal-r48.md). Gains vary by
scene and must not be added together or presented as an 8.33 ms result.
The experimental world-volume trees remain disabled; see
[r50 rejection and next steps](../../engine/docs/log/world-foliage-r50.md).

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

The [r60 handling trial](docs/history/handling-grip-r60.md) selects
`tyre_grip_scale = 1.12` and `lateral_peak_scale = 0.80` in the `[base]` section
of [`data/cars.toml`](data/cars.toml), for more grip and earlier lateral bite.
Both accept finite values from 0.5 to 1.5; `1.0/1.0` restores the original
tuning. With an r60-capable executable, edit the game's data copy and relaunch
through GameNight to apply changes; New Stage does not reload the file.
Flat-gravel tests show about 10% less sideways travel, with subjective driving
acceptance still pending. Physics remains 120 Hz with four substeps.

The engine debug Lighting submenu includes **Highlight bloom** and a draggable
**Bloom strength** control (4% default, 0-20%). Bloom softens bright lamp cores
after reconstruction and before AgX; it does not increase how strongly the
lamps illuminate the road. Rear/brake/reverse lights and exhaust flashes now
provide short-range environmental light without additional shadow maps.
See [r51 evidence and limits](../../engine/docs/log/local-lights-and-vegetation-r51.md).

For the explicit classic profile, developer playtests also enable distant grass
(`-MeadowAggregate`). Detailed
grass blends into a source-derived surface cache over60–80m; it fades into
matching terrain colour over160–180m. This is a directional surface atlas,
not voxel rendering. Use `-MeadowAggregate:$false` for the source-only reference.
An explicit switch overrides inherited `KERN_MEADOW_AGGREGATE`, then the launcher
default applies. `SIDEWAYS_MEADOW_SOURCE_RADIUS=100` retains the previous source
range for comparisons; valid values are60..100m. Captures report actual settings.
For an opt-in earlier handoff, `SIDEWAYS_MEADOW_SOURCE_BAND=40,50` sets independent
start/end metres and takes precedence over an inherited source-radius override.
It requires the aggregate, accepts finite `0 <= start < end` with `10 <= end <= 100`,
and leaves the 160–180 m outer fade unchanged. Unset it to restore the existing
radius-override/default behavior (normally 60–80 m). Captures record the authored pair and its origin; the live grass
distance control still scales both the source band and outer fade together.
This comparison setting has no visual or performance acceptance yet.
The current iteration improves coverage but retains a faint grid pattern on
some distant slopes; it is not a complete pop-in or120FPS solution. See the
[r39 results and limits](../../engine/docs/log/meadow-distance-r39.md).

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
