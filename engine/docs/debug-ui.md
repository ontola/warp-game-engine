# Live render debug panel

The engine reserves **F10** for this panel, **F11** for fullscreen and **F12** for a feedback screenshot in every build. Games receive F1–F9. The engine keys run before game menus, replay and text-entry handlers.

Press **F10** during a Sideways playtest to open or close the panel. Opening it
does not pause the race. To start with it open:

```powershell
games/sideways/tools/play-dev-windows.ps1 -DebugUi
# Already-built executable:
sideways.exe play --debug-ui
```

`-DebugUi` is optional and defaults off. The normal launcher still builds before
starting and requires `-Restart` to replace its verified owned playtest. The
resolved `--debug-ui` argument is recorded in the run manifest.

To keep a tested build available while the checkout is being edited, the
launcher also accepts `-PreparedExecutable <absolute.exe> -PreparedSha256 <hash>`.
This explicitly skips Cargo, stages and rechecks the exact artifact, then uses
the same owned-process checks and first-visible-frame proof. A hash mismatch
does not stop a running game. The manifest marks checkout provenance as launch
context, not proof that the prepared executable contains current source changes.

The panel is a top HUD layer above the pause/start menu, game HUD and screenshot
toast. Underlying menu navigation is suspended while it is open. Controller
Start closes the debug panel first without also resuming the underlying menu.
Keyboard driving can continue outside the pause menu; F11/F12 remain available.

The root links to **Vegetation**, **Advanced rendering**, **Weather / daytime**
and **Lighting / shadows**.
Vegetation has separate **Grass - live** and **Trees - apply changes** submenus.
Advanced rendering contains transparency, exhaust, wheel particles and geometry
LOD. Lighting contains headlights, exposure, sun intensity, shadow strength,
near sun shadows, environment visibility and contact/reflection occlusion.
Weather contains distance fog and the existing six time presets and three
atmosphere presets (Clear/Haze/Mist). These update sky and sunlight through the
existing background bake; they do not change road grip or simulate rain/clouds.
Click a menu or press Enter/Right to enter it.
Back or Esc returns to the parent and restores its selection and scroll.

Click a toggle, or click/hold the **− / +** buttons beside a number. Drag the
numeric value horizontally to adjust it (12 logical pixels per step). Holding
a button repeats after 350 ms, then every 75 ms, independent of frame rate.
Release, focus loss, resize or closing the panel cancels the interaction.
**Up / Down**
select a control, **Left / Right** adjust it, and **Enter** toggles the selected
switch or activates **Reset defaults**. Scroll the mouse wheel over the panel
to reach more controls; **Page Up / Page Down** also move the selection.
**F10**, the top-right **x**, or Esc at the root closes the panel. Changes are
process-local and never rewrite assets or saved defaults.

Grass size (50–200%), patch count (0–100%) and red/green/blue albedo multipliers
(0–200%) update immediately. They affect the source and distant representations
together, including their culling and wind bounds. Count selects a stable subset
of the allocated patches; it cannot increase allocation beyond 100%. The distant
surface atlas and terrain substrate remain approximations, not runtime voxels.
The separate **Grass LOD error** control (0.5–4 input pixels) changes the source
mesh simplification tolerance without reducing draw distance or patch count.
The general geometry LOD control does not affect grass. The coarsest available
source mesh remains the lower limit; increasing this value cannot simplify it
further.

Tree size (50–150%), count (0–100%) and crown RGB multipliers (25–200%) are staged.
Press **Apply tree changes** to rebuild the current forest's rendering, shadow
caches and native trunk collisions. A loading screen remains responsive during
this operation; race time, route seed and car poses remain unchanged. Tint affects
leaves/needles, not bark. Count chooses a stable subset of existing near and far
trees, without generating additional roots. **Discard pending changes** restores
the fields without a rebuild. Tree edits apply only to the current stage; a new
stage starts with its authored trees. Experimental packed voxel/branch scenes
reject this source-tree editor explicitly until their recipes can be edited too.

**Reset all defaults** restores every page; if rendered trees differ, it also
queues their restoration. Grass edits do not alter collision geometry; committed
tree size/count edits do update the native trunks.

| Control | Live effect |
|---|---|
| Grass | Toggles the procedural meadow source, distant representation and terrain substrate together. |
| Headlights | Toggles the car spotlight lighting and its shadow passes. |
| Transparency | Toggles the forward transparent-material pass, including car windows. |
| Exhaust | Toggles rendered smoke, fire and heat distortion. |
| Wheel particles | Toggles wheel dust and debris rendering. |
| Grass draw distance | Scales the existing source/distant transitions and outer fade from 25–100%. **Density and allocation do not change.** At the current 80/180 m configuration, 50% gives 40/90 m endpoints. |
| Geometry LOD error | Changes the cluster-mesh error threshold from 0.5–4 pixels; larger values permit simpler trees, rocks and other meshes. Meadow source LOD is separate. |
| Exposure offset | Adjusts the current time-of-day exposure by −3 to +3 EV. Positive EV darkens, negative EV brightens. |
| Distance fog | Toggles distance extinction and its visibility culling; keeps the procedural sky. |

Enabling a renderer feature does not create assets or buffers absent from the
scene. Reset restores the debug overrides, including 100% grass distance and
zero exposure offset, while retaining the chosen time of day and atmosphere.

The timing values have different meanings:

- **GPU measured intervals** is the union of measured pass intervals: overlap
  counts once and unmeasured gaps do not count. It is not a hardware utilization
  or active-cycle counter.
- **GPU render interval** runs from the first measured GPU start to the last
  measured end, including any intervening gaps. GPU HUD and presentation are
  outside this renderer instrumentation.
- **CPU main thread** is actual Windows user + kernel execution time consumed
  by the game/window thread during the frame, measured with `GetThreadTimes`.
  Sleeping, waiting and time when the scheduler runs another process are excluded.
  Other worker threads are excluded too: this is not total process CPU usage or
  a hardware-speed benchmark. Counter resolution is coarser than its 100 ns units;
  the bounded average is more useful than one frame. Unsupported platforms/API
  failures show unavailable, never a substitute wall-clock number.
- **Frame wall** is elapsed time of the frame call, including waits. It can
  follow GPU time when CPU submission waits for the GPU; that does not mean
  the CPU spent that time executing. Adding CPU and GPU time is misleading.

Largest-pass rows show independent pass durations, not additive percentages of
a frame total. The panel uses a bounded 32-sample timing window and refreshes
text at 4 Hz. GPU samples arrive asynchronously; their age, pending samples and
drops are shown. Missing samples remain unavailable rather than becoming zero.
Changing a control clears the prior timing window so it does not describe the
old configuration. The timing area reserves a fixed number of rows even while
samples are unavailable, so controls never move when a measurement resets.
The panel does not infer FPS or certify the 8.33 ms target.

The reusable engine UI is `render::debug_ui`, using existing `HudDrawList` and
`TextRun` objects. It consumes a caller-supplied model and emits typed actions;
it performs no GPU readback. `render::debug_settings` owns renderer gates and
`render::debug_timing` owns interval accounting and `render::cpu_time` reads the
calling thread's OS execution counters. Sideways maps platform input
and controls in `games/sideways/app/src/engine_debug.rs`.

The panel scales from 1280×720 to 4K and shows a bounded set of largest passes
above scrollable controls. Its CPU tests cover scaled hit regions, input
consumption, clamping, disabled controls and timing labels. Visual/runtime
acceptance is separate from those tests; this document makes no measured
performance claim.

Validated 2026-10-25: 373 render and 151 game tests passed (one existing ignored
test in each suite); launcher contract passed. A visible 256-frame native
1920x1080 night fixture exercised actual UI key events for grass on/off/on and
headlights off/on. Async pass-name checks confirmed removal and restoration,
with 39/39/39/23/23 clean matched samples across the five sections. Evidence:
`C:/dev/perf/warp-sideways-20261025/r42-debug-ui/check-a/report.json`.
The panel was visually inspected in JPEG captures 128, 192 and 256. This fixed
camera validates controls and instrumentation, not representative performance.

Current limits: six largest pass rows; no vegetation population above its allocated source;
long help/status text can clip at the panel edge. No claim that every graphics
subsystem has a toggle yet.

Vegetation/environment update, 2026-10-25: 392 renderer tests and 162 game tests
passed (one existing ignored test per suite), plus four native tree-collider
tests. The actual GPU fixtures are recorded under
`C:/dev/perf/warp-sideways-20261025/r43-vegetation-controls`:

- `candidate-a/report.json`: 160 paused native1080 scene frames, live grass
  size/count/RGB, staged tree size/count/RGB, Apply and Reset. Tree edits
  selected159403 of319179 source trees; Reset restored319179. Both graphics
  rebuilds preserved the route, camera, cars and race clock. Four JPEGs reviewed.
- `environment-a/report.json`: 160 paused native1080 scene frames, real menu
  input selecting Morning/Mist through the asynchronous sky worker, sun/shadow
  strength changes, near-shadow disable/restore and Reset. Actual GPU passes
  and accepted lighting metadata matched the controls. Two JPEGs reviewed.
  Measured CPU main-thread execution was about3.6–3.9ms while frame wall was
  about20ms in those captures. This verifies that execution and waiting are
  separately reported; it is not a four-player performance benchmark.

For repeatable UI fixtures, `--start-paused` shows the real pause menu and
`--debug-ui-script path.json` accepts a bounded array of input events. Every
event has a rendered `frame` and exactly one action: `key` (for example `enter`),
`pointer` (`[x,y]` in physical output pixels), `pointer_down: true` or
`pointer_up: true`. The script uses the same widget handlers as live input;
the normal frame delta drives holds. It does not bypass input widgets to write
renderer values directly. Scenario runs do not test real controller routing.

Overlay interaction update, 2026-10-25: 380 renderer tests and 152 game tests
passed, plus the focused final-pointer-release regression test and launcher
contract. The 128-frame paused-menu fixture passed at native 1920x1080:
grass distance dragged from 100% to 75%, holding exposure minus repeated, its
value remained unchanged for 31 frames after release, Advanced expanded and
Reset restored defaults. All three final JPEGs were visually inspected; the
opaque top layer hides underlying menu text. Evidence:
`C:/dev/perf/warp-sideways-20261025/r42-debug-ui/priority-final/report.json`.
The earlier `priority-a` fixture used incorrect pointer coordinates and failed;
`priority-b` passed interactions but exposed translucent menu bleed-through,
which the final opaque panel corrects. These are UI checks, not FPS claims.
