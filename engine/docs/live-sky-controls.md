# Live sky controls

The procedural sky with time of day and weather is an engine feature,
`render::sky`. Sideways uses it from its pause menu and Gamepad Party; Growing Guns
uses it from `--sky` and the `sky` console command (also reachable through the
live MCP server's `command` tool). The dated Sideways notes follow the engine
and Growing Guns sections.

## Engine API: `render::sky`

- `DayTime` (sunrise, morning, noon, afternoon, sunset, night) and
  `WeatherLook` (clear, haze, mist): `key`, `label`, `parse`, `step`.
  `DayTime::solar()` and `DayTime::exposure()` (EV100) give the authored sun
  and exposure; `WeatherLook::model()` gives the baked `Atmosphere` and
  `WeatherLook::fog()` the distance fog (start m, end m, density per m).
- `exposure_for_elevation(degrees)`: EV100 for any sun height, piecewise linear
  through the presets, for continuous cycles.
- `LiveSettings`: a time and weather (`LiveSettings::preset`), optionally with a
  custom sun and exposure (`with_sun`) or just an exposure (`with_exposure`).
  `changed("sky_time" | "sky_weather", value)` is the menu/GameNight vocabulary;
  `solar()`, `exposure()`, `metadata()` and `apply_presentation(light)` (fog,
  exposure, shadow strength).
- `bake_live(revision, settings, cache, quality) -> PreparedSky`: one
  synchronous bake (or a reuse from the `AtmosphereCache`), with metadata
  including `bake_cpu_ms` and `night_environment` (`night_metadata`).
- `PreparedSky::apply_scene(scene, light)`: sky texture, sun, skylight, fog and
  exposure together. For a running renderer also call
  `Renderer::update_sky(gpu, &prepared.sky, light)`; from an `app::Game`, reach
  the renderer through `Frame::renderer_mut()`.
- `LiveSky`: the background path. `LiveSky::new(initial_metadata, light)`
  starts from a world's existing sky (an unmatched sun becomes **Current**);
  `LiveSky::starting(settings, initial_metadata)` starts from known settings.
  `request(settings, now)`, then every frame `poll(now)`; hand a returned sky to
  the renderer and call `accepted` (or `failed`). `status()`, `busy()`,
  `metadata()`, `restore()` and `applied_snapshot()` cover menus and rebuilds.
  The worker thread is named `sky-bake`.

Nothing in the module bakes per frame or per camera.

## Growing Guns

Rendering only: the simulation never sees the sky, so replays, the network
and the bots are unchanged. `games/growing-guns/app/src/sky.rs`:

- `--sky random` (default) selects time and weather from the arena seed. `GG_SKY` can also select a sky.
- `--sky godot` uses Godot's gradient sky and fixed sun.
- `--sky TIME[:WEATHER]`, e.g. `noon`, `sunset:haze`, `night:mist` (a bare
  weather means noon).
- `--sky rounds[:WEATHER]`: the time of day steps each round, from morning.
- `--sky cycle[:WEATHER][:MINUTES]`: a slow day and night on the game clock
  (default 12 minutes a day) through `LiveSky`, off the frame loop.
- The same values at runtime: `sky night`, `sky godot`, ... in the console or
  through MCP `command`. `state` reports `sky.choice` and `sky.applied`.

A preset chosen with `--sky` or the `sky` command bakes synchronously (about
0.2 s, see below) and updates the renderer in place. The default and `rounds` select the sky at
the round boundary. Interactive matches prepare the round sky presets in memory during play. The first view uses the selected sky and direct light. Between rounds the renderer keeps its last complete indirect light volume while the next stadium’s volume is calculated in the background. The continuous
`cycle` uses `LiveSky` for background updates during play. Exposure never goes below EV 11: the lava, beacons and
tracers have fixed radiance tuned for Godot's day (EV 13.6) and wash out at
the night preset's EV 6. The stadium floodlights carry the night instead:
they scale from 1x in daylight to 16x below -4 deg of sun, so the floor, the
walls and the players read clearly. The banks point at the centre, so the crowd
in the stands falls dark at night.

`games/growing-guns/scripts/sky.json` captures every preset over the arena
and the stands:
`KERN_ADAPTER=software target/debug/growing-guns script games/growing-guns/scripts/sky.json --out DIR`.

### Bake cost

`bake_live` at the default `BakeQuality`, release build, CPU wall time on
the 2-CPU Linux build machine, 2026-10-28
(`cargo test --release -p render --lib sky::tests::bake_cost -- --ignored --nocapture`):

| Time | Clear | Haze | Mist |
| --- | --- | --- | --- |
| Sunrise (first of a weather, includes the transmittance table) | 307 ms | 273 ms | 227 ms |
| Morning | 193 ms | 268 ms | 205 ms |
| Noon | 197 ms | 208 ms | 207 ms |
| Afternoon | 217 ms | 251 ms | 236 ms |
| Sunset | 194 ms | 232 ms | 236 ms |
| Night | 138 ms | 132 ms | 138 ms |

About 0.2 s per sky, so baking is never per frame. Repeats are free: the
worker keeps the last eight skies, and Growing Guns' synchronous path shares
one `AtmosphereCache`.

The cycle is cheap enough to keep as an opt-in. The sun is quantised to
1/720 of a day (about 0.5 deg of sun per step), so a 12-minute day bakes once
a second on the `sky-bake` thread: roughly 20% of one core, and nothing on the
frame thread except the `update_sky` upload. A shorter day costs
proportionally more (at `cycle:3`, four bakes a second, close to a whole core),
so do not go much below a few minutes on a 2-CPU machine.

## Sideways notes (2026-10-24)

Implemented 2026-10-24. CLI/controller/HUD CPU checks pass. The parent task owns
the exclusive GPU check and restart; this task did not interrupt the playtest.

Delivered later that morning in `build/dev-runs/20261024T083137778Z-aa9fd83b`,
PID 269560. The actual pause panel was observed and live F12
`sideways-1792830832981-269560-0.json` records sunset revision 2 applied with no
pending request/error. The one-renderer GPU fixture passes five in-place sky
updates, identical return-to-noon pixels, rejected-update preservation and four
small views (`C:/dev/perf/warp-sideways-20261024/r16/sky-live-final`). A controlled
live paused-position pair and full atmosphere/controller matrix remain pending;
the user resumed driving, so interactive checks were stopped.

## Use

In standalone play, press **Start / Esc** for the compact translucent pause menu.
Use the d-pad, left stick or arrow keys to select a row and adjust it. **A /
Enter** selects; Start/Esc resumes. A held A button when opening the menu is
ignored until released. Controls remain accessible when the driving HUD is off;
the cached menu is not presented after resuming with HUD off.

- **Time of day:** sunrise, morning, noon, afternoon, sunset and night.
- **Atmosphere:** clear, haze and mist.
- Resume and New stage retain their separate meanings. Only New stage rebuilds
  a route. Time and atmosphere changes preserve the current race and positions.

Gamepad Party declares the same live choices as `sky_time` and `sky_weather`.
The existing `weather` setting is labelled **Road weather (next stage)** and
continues to control the next race's road/weather behavior. Visual atmosphere
changes do not silently change grip. A custom initial sun direction appears as
**Current**, and an atmosphere-only change preserves that direction/exposure.

The six times are authored solar elevation/azimuth presets, not a geographic
calendar calculation. Exposure changes with the chosen time and is recorded.
Night can be very dark: moon, stars, headlights and artificial fill are not
provided by this atmosphere foundation. Haze and mist change aerosol scattering
and distance fog. They do not claim volumetric clouds or rain.

## Update path

`render::sky::LiveSky` (formerly `procedural_sky::LiveSky`) owns a lazy background worker and a latest-wins
scheduler. Requests debounce for 150 ms. At most one bake runs and one newest
request waits; stale results are discarded before any live state is changed.
The worker retains at most eight completed skies, plus the bounded atmosphere
cache. There is no per-frame or per-camera bake, no synchronous UI bake and no
track rebuild for these settings.

Once the newest result arrives, `Renderer::update_sky` updates the shared sky
texture, raw SH and directional sun together. The CLI applies matching fog and
exposure in that same event, then stores the accepted sky for later ordinary
stage rebuilds. Sun and temporal histories invalidate through the renderer API.
GPU update errors, worker creation failure and a disconnected worker leave the
last applied sky intact and clear the updating state instead of quitting play.

Polling happens before the Gamepad Party render-quiet early return. A host-paused
game can finish/apply the requested sky without rendering the whole world or
waiting for Resume. The standalone menu displays **Updating sky...**, then
**Applied**, or a short error preserving the old sky.

Play state/F12 metadata has `live_sky_controls` with initial, desired, pending,
in-flight revision, applied revision/settings and error. Applied sky metadata
records solar direction parameters, sun RGB lux, horizontal sky RGB lux,
exposure, fog start/end/density, LUT dimensions, bake CPU time and cache reuse.

## Checked

- `cargo test -p cli procedural_sky -- --nocapture`: **6 passed**. Environment
  validation/cache behavior, latest-request debounce and stale-result rejection,
  preservation of a custom initial sun, menu edge/repeat behavior, and coherent
  sky/sun/fog propagation while retaining scene data.
- `cargo test -p cli live_ -- --nocapture`: **15 passed**, one pre-existing CPU
  residency benchmark ignored. Includes live host menu vocabulary and compact
  pause-panel geometry at 720p, 4K and a small output size.
- No GPU images or runtime input verification are claimed by these CPU tests.
  Next check: native and four-view noon → sunset → mist → noon, unchanged route
  and car positions, pending/latest behavior while paused, custom initial sun,
  invalid update preservation and F12 state. Check the HUD-off resume path too.

Main files: `crates/cli/src/procedural_sky.rs`, `play.rs`, `input.rs`, `hud.rs`,
`gamenight.rs`. The parent task supplies the renderer's shared sky update API.


## Visible sun and lens flare

Warp Sky enables `Lighting::sun_disc` and `lens_flare=0.5` when applying a
bake. The background draws a 0.266-degree-radius solar disc in the existing
sunlight direction, with a pixel-sized soft edge. It stays out of the baked
sky texture and diffuse irradiance; opaque foreground geometry hides it.
Radiance is capped before the fp16 HDR target. Existing terrain-horizon
visibility also attenuates the disc and its optical glare.

The tonemap compute pass adds soft glare, a streak and three faint coloured
lens ghosts. Sixteen depth taps across the solar disc run once per GPU tile,
not per pixel and without CPU readback. Partly covered discs reduce the flare;
fully covered, below-horizon and behind-camera suns produce none. Each split
uses its own camera projection and depth image. The flare fades out as the
disc leaves the viewport; it does not estimate visibility beyond the screen.
Transparent objects do not contribute to this opaque-depth visibility test.

`light lens_flare=0` disables the optical effect in Growing Guns;
`light sun_disc=0` disables both disc and flare. Legacy skies retain their
previous defaults. `render/examples/solar_flare_gpu.rs` captures clear, half
covered and hidden cases, checks hidden effects against disabled effects,
and compares each split with an independently rendered camera. This is a
visual/correctness fixture, not a four-player performance claim.


Games can tune their HDR highlight response through `Lighting::bloom_boost`,
`bloom_rounds` and `bloom_threshold`; renderer defaults are unchanged. The
existing bloom enable/strength controls and quality disable still apply.
Growing Guns builds a wider, stronger halo during Ion charge and impact,
then returns to its usual bloom when the effect expires. Three local samples along the charging column illuminate paving
and walls; they build with charge and expire at detonation. These local
samples use the engine's existing unshadowed local-light path.

Stadium lava return is baked once per arena geometry into the existing
irradiance grid, with static-box visibility against four emissive rectangles
around the arena floor. Mode 7 combines packed sky/sun transport and RGB9E5
emission without enlarging probes or adding light bindings. It follows the
source emission's exposure compensation. This is an approximation of static
illumination, not dynamic GI: later rubble, players and destruction do not
rebake the lava cache. `GG_LAVA_BOUNCE=0` disables the additional term for A/B.
