# Car headlights: bounded first implementation

Status: implemented, CPU/WGSL checked; isolated real-GPU fixture passed.
Live moving native/DLSS night acceptance remains pending. No measured
gameplay performance or 120 FPS claim.

`render::headlights::SpotLight` is a reusable world-space spotlight with a
position, direction, up vector, linear RGB intensity in candela, range and
inner/outer cone angles. `Renderer::set_headlights` validates the complete
list before replacing it; an empty list disables lighting. Maximum eight
(two physical front lights for each of four human cars).
The API changes neither sky/exposure nor physics.

Each light uses a 1024-square Depth32Float perspective map, refreshed once
per world frame before all cameras. Existing GPU CLOD selection, animated
source geometry and alpha cutouts supply occluders. A separate shadow scene
binding uses a dummy depth array to prevent sampled/attachment aliasing.
Lighting uses inverse-square attenuation, smooth cone and range falloffs,
depth comparison filtering, Lambert diffuse and GGX specular. Material paths
share incident-light sampling. Near/transition/far reference grass uses its
existing inclined-blade reflection/transmission distribution, not a world-UP
Lambert gate. Shadow/cone/range are sampled once per lamp outside that blade
loop. Experimental procedural leaf/voxel transmission is not covered by this
grass correction. Fog still attenuates the resulting surface radiance.

The game switches these on below five degrees solar elevation or below
5000 lux direct sun, for up to four human cars selected in view order. A
pair uses measured imported lens positions for Asti/MV6/Toyama, moved 2.5 cm
outside their lens surfaces. Unknown models use an explicitly recorded paired
collision-bound fallback. Ride is applied once and lamps follow the complete
body quaternion, including pitch and roll. Total pair intensity remains
80/76/68 kcd RGB, divided equally between the two lamps; 90 m range, 17/32 degree
inner/outer half-angle and a small downward aim. These are visual tuning
values, not a certified automotive beam pattern. F12 metadata records actual
light poses, intensity, range, cone and limitations.

Limits: no luminous lens meshes, AI headlights,
local indirect bounce or atmospheric beam volume. Existing low-grass
`NO_SHADOW` behavior is retained; grass receives the spotlight but does not
become a new shadow caster. The eight shadow textures reserve 32 MiB total,
plus lazily allocated per-light culling buffers. Extra cull/raster passes
are shared across views and included in GPU timestamps and shadow counters;
their production scene cost is unmeasured. Moving-light DLSS history quality
still requires a real drive test.

CPU checks passed:

- `cargo test -p render headlights`: two validation/layout/projection tests.
- `cargo test -p render shader_tests`: four production composition checks.
- `cargo test -p cli car_headlights`: imported lens triangles, full-pose
  attachment, daylight disabled, unchanged pair power and four-car cap.
- `cargo check -p render --example headlights_probe`.

Run only with exclusive GPU access:

```powershell
cargo run --release -p render --example headlights_probe -- --out FRESH_DIRECTORY
```

The fixture compares zero-sun/sky lamp-off darkness, a receiver lit by a
spotlight, the same receiver blocked by an actual wall, grass above the lamp,
and eight real light layers sampled by four cameras, including a last-layer-only
check. It saves native PNGs and a JSON report with
receiver brightness and GPU pass timings. After it passes, inspect the
procedural night preset in real one-player and four-player native/DLSS
driving, including hills, turning, grass boundaries and other cars, before
calling the feature delivered.

24 October r18, running in PID 231924: fixes the user's dark grass capture
`sideways-1792834206688-219656-0`, whose light points uphill at elevated tufts.
The old grass path rejected all light whenever the lamp was below the fragment.
The separate road-normal import issue is corrected only for the transferred
gravel map. Actual native replay at the captured camera now lights the tufts;
`r18/user-road/lit.png` removes the three dark stripes seen at the same camera in
`r17/afternoon-road/lit.png`. `r18/night-dlss.png` shows the updated paired lights
on the real road (1920x1080 output / 1280x720 input, held frame). These images
do not establish motion quality, 4K/four-player cost or grass shadow casting.

Final eight-light fixture `r18/headlights-final/report.json`: PASS; lamp off and
wall-blocked RGB 0, open 56.6, grass above the light 196.32, last light alone 4.4.
All four views have exactly zero pixel/channel differences against the same
eight-light reference. This test exposed a main split-view/shadow-pass marker
collision that incorrectly culled grass; perspective shadows now have their
own marker. Evidence root: `C:/dev/perf/warp-sideways-20261024`.

Earlier 24 October GPU evidence (predates r18 receiver and mount changes):
`C:/dev/perf/warp-sideways-20261024/r17/headlights/report.json` reports PASS,
no failures, lamp-off receiver RGB 0, wall-blocked RGB 0, unobstructed RGB
56.6. Four real light layers and four native camera views match their
single-view reference. Root inspected the actual wall-shadow image. These
small isolated-scene timings are not production-road costs. Shot setup now
derives and records the same car lights after all lighting overrides, and
`shot::run` uploads them before rendering.
