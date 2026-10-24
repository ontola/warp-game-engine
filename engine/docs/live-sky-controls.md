# Live sky controls

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

GameNight declares the same live choices as `sky_time` and `sky_weather`.
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

`procedural_sky::LiveSky` owns a lazy background worker and a latest-wins
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

Polling happens before the GameNight render-quiet early return. A host-paused
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
