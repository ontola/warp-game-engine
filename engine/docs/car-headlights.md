# Car headlights: bounded first implementation

Status: implemented, CPU/WGSL checked; isolated real-GPU fixture passed.
Live moving native/DLSS night acceptance remains pending. No measured
gameplay performance or 120 FPS claim.

`render::headlights::SpotLight` is a reusable world-space spotlight with a
position, direction, up vector, linear RGB intensity in candela, range and
inner/outer cone angles. `Renderer::set_headlights` validates the complete
list before replacing it; an empty list disables lighting. Maximum four.
The API changes neither sky/exposure nor physics.

Each light uses a 1024-square Depth32Float perspective map, refreshed once
per world frame before all cameras. Existing GPU CLOD selection, animated
source geometry and alpha cutouts supply occluders. A separate shadow scene
binding uses a dummy depth array to prevent sampled/attachment aliasing.
Lighting uses inverse-square attenuation, smooth cone and range falloffs,
depth comparison filtering, Lambert diffuse and GGX specular. Opaque, near
grass, distant grass, transparent and voxel material paths call the same
helper. Fog still attenuates the resulting surface radiance.

The game switches these on below five degrees solar elevation or below
5000 lux direct sun, for up to four human cars selected in view order. A
single combined headlamp pair per car starts 12 cm beyond the front collision
bound and follows the complete body quaternion, including pitch and roll.
The pair is a first approximation: 80/76/68 kcd RGB, 90 m range, 17/32 degree
inner/outer half-angle and a small downward aim. These are visual tuning
values, not a certified automotive beam pattern. F12 metadata records actual
light poses, intensity, range, cone and limitations.

Limits: no individual left/right lamps, luminous lens meshes, AI headlights,
local indirect bounce or atmospheric beam volume. Existing low-grass
`NO_SHADOW` behavior is retained; grass receives the spotlight but does not
become a new shadow caster. The four shadow textures reserve 16 MiB total,
plus lazily allocated per-light culling buffers. Extra cull/raster passes
are shared across views and included in GPU timestamps and shadow counters;
their production scene cost is unmeasured. Moving-light DLSS history quality
still requires a real drive test.

CPU checks passed:

- `cargo test -p render headlights`: two validation/layout/projection tests.
- `cargo test -p render shader_tests`: four production composition checks.
- `cargo test -p cli car_headlights`: source outside bumper, full-pose
  attachment, daylight disabled and four-car cap.
- `cargo check -p render --example headlights_probe`.

Run only with exclusive GPU access:

```powershell
cargo run --release -p render --example headlights_probe -- --out FRESH_DIRECTORY
```

The fixture compares zero-sun/sky lamp-off darkness, a receiver lit by a
spotlight, the same receiver blocked by an actual wall, and four real light
layers sampled by four cameras. It saves native PNGs and a JSON report with
receiver brightness and GPU pass timings. After it passes, inspect the
procedural night preset in real one-player and four-player native/DLSS
driving, including hills, turning, grass boundaries and other cars, before
calling the feature delivered.

Verified 24 October GPU evidence:
`C:/dev/perf/warp-sideways-20261024/r17/headlights/report.json` reports PASS,
no failures, lamp-off receiver RGB 0, wall-blocked RGB 0, unobstructed RGB
56.6. Four real light layers and four native camera views match their
single-view reference. Root inspected the actual wall-shadow image. These
small isolated-scene timings are not production-road costs. Shot setup now
derives and records the same car lights after all lighting overrides, and
`shot::run` uploads them before rendering.
