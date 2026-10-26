# Wheel dust and debris

The runtime path is `sideways::wheel_fx::Emitter` → CLI `wheel_effects` adapter →
`Renderer::update_wheel_particles` → shared `WheelParticles` pool. The renderer
accepts engine-independent births, not physics objects. Existing surface and
slip rules therefore also cover spinning driven wheels while the car is stationary.

Each birth retains its fixed-tick simulation timestamp. GPU vertices evaluate
analytic drag, gravity and age; four views share the same records and clock.
Pause does not advance the effects. Stage replacement/reset discards the pool.
The CPU uploads births, expires/culls indices and sorts each view; this is **not**
a compute simulation or GPU-generated wheel contact system. None of this mutates
car physics, the stage RNG or race determinism.

## Shapes and light

- Stones, soil, mud and water use small rotating, irregular octahedral meshes.
  Their hidden faces are rejected, preventing back sides painting over fronts.
- Grass pieces are narrow bent ribbons with deterministic tumbling.
- Dust and tyre smoke use smooth lobed, low-opacity puffs with continuous edges;
  their two triangles have no visible rectangular cutout boundary.
- Existing far and per-view near sun maps supply directional shadowing. Sky
  illuminance supplies a cheap ambient term. With zero sun and sky illumination,
  particles emit no light.
- Opaque scene depth uses the actual jittered/internal-resolution projection,
  independently of the reconstructed output camera. Intersection edges soften.

Lighting is deliberately bounded: sky colour currently uses a fixed blue ratio
times `sky_lux`, not the baked nine-coefficient sky irradiance. No particle
self-shadow, headlight illumination, volume integration, light bounce, water
refraction or shadow casting is implemented. Water is a translucent lit fragment.
The underlying shadow-map coverage limitations still apply. Ballistic fragments
disappear below their **local initial contact plane**; they are not rigid bodies
and do not query the terrain or other cars after birth.

## Bounds and temporal ownership

The shared ring holds at most 4,096 births (80 bytes each, 320 KiB GPU storage).
New births replace oldest ring slots when saturated. Each view selects at most
2,048 records and limits the approximate sum of projected bounding-box coverage
to two screen areas, then draws back-to-front. Dust and grass use two active
triangles; debris uses eight. The current single pipeline submits 24 vertices per
selected record, degenerating surplus vertices for puffs/ribbons: at most 49,152
vertices per view. There is no new shadow-rendering pass. Birth uploads are split
into at most two contiguous queue writes for a ring wrap.

The approximate coverage/culling estimate uses projection column norms. It can
underestimate a rotated camera's exact sphere bounds near screen edges; it is
not a hard GPU-time guarantee. Small particles below the pixel threshold and
particles beyond 100 m are rejected. This is a small transient effect budget,
not a world vegetation LOD strategy.

Particles compose after native TAA or DLSS and before tone mapping. If exhaust
has already made a disposable output, particles reuse it with no additional
copy. Otherwise the particle pass copies the resolved HDR image once into a
cached output. It never writes into TAA/DLSS history. Only selected particle views
allocate output textures. Per-view index/uniform storage is about 8.4 KiB; the
optional full-resolution HDR target is eight bytes per output pixel.

## Verification

CPU/shader tests: five focused renderer tests passed on 24 October 2026 in
`C:/dev/perf/warp-sideways-20261024/r22/wheel-particle-tests.log`: WGSL validation
and shared layouts, bounded wrap/expiry, paused time/reset, delayed birth age,
invalid-batch atomicity, and distinct analytic motion/material classes. The
independent adapter has separate actual-car and unchanged-digest tests.

`render/examples/wheel_particles_probe.rs` is the executable acceptance fixture.
It saves all seven material kinds and reports actual changed-pixel counts. It
checks empty/reset parity, four same-camera views, foreground occlusion, a
sun-only occluder, no-light non-emission, exhaust-output reuse and temporal-history
parity. `--dlss-quality` explicitly requests DLSS for the temporal subtest; an
unavailable backend is an error. Root owns exclusive GPU execution and visual
inspection. Root ran the fixture successfully on the RTX5070Ti, including
DLSS, on 24 October 2026: `r22/particle-gpu/report.json`. Actual
`trail-front-on/off.png` captures confirm dust behind the cars and the native
4K/four-player pair renders successfully. The tested executable is running
in the r22 playtest. See `wheel-effects.md` for evidence and remaining limits;
these checks do not establish gameplay FPS or the 4K/four-player/120 FPS target.
