# Windswept grass integration — 28 October 2026

The other local agent's uncommitted grass work was preserved as `ac29b0f`
on `codex/grass-rounded-shading-20261028` in `C:/dev/warp-game-engine`.
Its base `2df8717` was already an ancestor of main. The integration combines
that snapshot with main `83b289b` and CPU optimization commit `09639b6`.
The source checkout and its benchmark evidence remain intact.

## Included work

- Curved windswept blades, per-fragment pigment and authored root darkening,
  continuous source LOD morphing and travelling wind.
- Compact homogeneous blade storage and a 250 m source radius with a
  200–250 m fade. This remains source geometry, not a completed distant
  voxel/filtered terrain representation.
- Optional depth-aware distant RGB filtering and accepted-root contact
  shading. `KERN_MEADOW_SPATIAL_FILTER` and `KERN_MEADOW_EDGE_SHADOW` remain
  off by default. Filtering disables itself with temporal reconstruction.
- Capture scenarios, launcher/debug-script support, regression fixtures,
  evidence reports and the upstream Quick_Grass MIT notice.

Procedural meadow defaults to the source agent's `windswept` profile;
`SIDEWAYS_MEADOW_PROFILE=classic` selects the previous recipe. The earlier
one-view native-4K source measurements remain about 23–26 ms. Importing this
work is not evidence of a four-player or 120 FPS performance improvement.

## Semantic merge repairs

Main already used scene binding 28 for procedural noise. Grass contact uses
29; both Rust layout and WGSL declarations agree. The full-scene replacement
buffer at binding 11 is restricted to raster vertices (compute culling has
its own reduced layout), keeping shade compute storage at the existing
16-buffer device limit. The contact buffer is fragment/compute only.

Main's one-based view index still drives per-view hiding; the contact receiver
converts it to the producer's zero-based view slot. The meadow shader feature
gate includes contact even when substrate shading is disabled. Existing
resource caches, shader feature specialization, tiled shading, noise,
irradiance volumes, bloom/fog fields and the CPU optimization work are retained.

## Validation

The combined merge passed:

- `cargo test --locked -p render --lib`: 611 passed, 12 ignored.
- `cargo test --locked -p growing-guns-app --bin growing-guns`: 53 passed,
  one ignored.
- `cargo test --locked -p cli --bin sideways`: 186 passed, one ignored.
- `cargo check --locked --workspace`.
- The three frame-scenario Python tests and Sideways launcher checks.
- The separately enabled accepted-root contact and spatial-filter GPU tests,
  run serially on the local GPU. They check empty-habitat rejection, offscreen
  sources, view isolation, coverage preservation and foreground rejection.

Logs are in `C:/dev/warp-bench/cpu-spikes-20261028-175917/merged-*.log`.
The real `meadow_wind_gpu` fixture also passed with both optional flags enabled
at native 1280x720, two wind frames per profile, zero-wind checks and detail/LOD
stills. Its `merged-grass-gpu/report.json` records the mechanical results.
The windswept field and blade-detail captures were inspected: source geometry
and pigment render, with fine distant edge grain still visible. This short
fixture is not a gameplay quality acceptance or a frame-time benchmark.

See [r63](meadow-windswept-r63.md) and [r64](meadow-edge-grain-r64.md) for
the source agent's separate visual evidence and remaining limitations.
The historical r64 playtest has been stopped; importing does not start a game.
