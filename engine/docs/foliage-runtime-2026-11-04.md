# GPU forest runtime, 4 November 2026

`terrain_fly` now has a real, opt-in route without `recipe_forest::Pool`:

```powershell
$env:WARP_TERRAIN_GPU_FOREST='1'
# Optional bounded branch output capacity; default 4096, maximum 16384.
$env:WARP_GPU_BRANCH_SLOTS='4096'
cargo run --locked --release -p app --example terrain_fly -- --style alpine
```

This interactive command is for a requested playtest. This development run used
only headless tests/captures. Default gameplay remains unchanged; integration is
implemented, but performance/visual acceptance is not implied by that fact.

## Architecture actually implemented

- CPU cooking/root placement once per scene. One shared recipe/template/voxel
  catalog per variant, one permanent root per world tree. No expanded branches
  per world tree, fixed full-tree roster, or CPU distance-driven slot allocator.
- GPU consumer-union frustum/error traversal, complete-cut demand counting,
  deterministic budget admission, persistent branch residency, articulated
  ancestor wind evaluation, and actual source/voxel draw data publication.
- Budget priority is projected size followed by stable root ID. Prefix sums use
  parallel 256-root blocks, followed by a small block-prefix pass. Queue arrival
  order cannot decide which tree keeps detail. A whole-root voxel needs no
  transient branch slot. Unadmitted cuts draw one resident coarse root.
- One frame-wide allocation domain covers cameras AND independent shadow
  consumers. This prevents shared instance/motion buffers from being reassigned
  between queued split-screen views. Existing per-view current-depth two-phase
  source cluster occlusion and cluster LOD run after publication.
- Source and voxels use the same branch pose. Slot changes invalidate motion
  history by epoch. Separate shadow choices retain the existing voxel caster
  policy instead of redrawing expensive source needles into fine sun maps.
- Authored crown bounds remain the canopy-lighting input. Enlarged animated
  selection bounds are visibility bounds, not extra foliage density.
- World root count is independent of the bounded branch-volume catalog. Tested
  attachment of 20,000 tiny-fixture roots and actual rendering of 62,274 trees.
- Diagnostics read GPU counters explicitly after the route, never to drive
  selection/allocation. `tree_pool_ms` must be zero on the new route.

## Validation

Release build and `cargo check --locked -p render -p app --all-targets` passed. Focused tests passed for shared selector validation,
current-depth disocclusion, queue overflow, deterministic near-first complete
cuts across multiple prefix blocks, GPU slot retention/reordering/generation
changes, duplicate requests and slot exhaustion. Actual rendered tests cover
source wind, voxel selection, fallback, teleports, four cameras and off-camera
sun casters. The small source-wind comparison matched every RGBA pixel at four
poses; fallback vs proxy MAE was below 2/255. These are focused fixtures, not a
claim of full-forest image equivalence.

The one-view forest route uses alpine seed 1, initial camera [0,691,0], yaw/pitch
0, speed 15 m/s, 80 rendered frames. Exclude frames before render sequence 12:
68 GPU samples/run. Native 1713x964, no DLSS. Three alternating old/new pairs
at each forest radius use the same executable. CPU Pool updates and GPU pass
times are separate; headless route wall time includes completion/readback and
is NOT gameplay FPS. The fixed-timestep HUD's 60 fps text is not a benchmark.

Raw artifacts: `build/gpu-forest-20261104-parallel/`; intermediate failed or
superseded runs remain under `build/gpu-forest-20261104*`. The final summary
and per-pass medians accompany this document. The 2.5 km forest run contains
62,274 trees; the 500 m run contains 4,841. These are not 4K/four-view benchmarks.

## Errors found and corrected during integration

1. Shared selector queue overflow initially let distant shadow demand displace
   near detail. Count complete cuts first and admit them deterministically on GPU.
2. Copying camera source choices to near sun maps caused about 33 ms of near
   shadow raster work. Publish a separate voxel shadow choice from the same pose.
3. The profiler reserved a voxel-composite query even when scene-surface rendering
   skipped that pass. Stale query data produced invalid-looking frame intervals.
   Reserve only scopes that execute. Early measurements are superseded.
4. Applying conservative selection spheres to canopy density inflated crowns and
   over-darkened the image. Preserve authored crown spheres instead.
5. Packed branch limits accidentally included all world roots. The proxy catalog
   is separate and does not consume branch group capacity.
6. A single serial priority-prefix scan cost too much at 62k roots. Use parallel
   blocks without changing stable admission order.

## Remaining limits / next architecture work

This is NOT full Nanite or Lumen. The standalone history-HZB selector is tested,
but the runtime uses a conservative shared cut followed by per-view source
occlusion; it is not yet a persistent load-balanced cluster work queue.

The shared cut satisfies the most demanding consumer, which can over-refine
other views. Limited branch budgets intentionally produce coarse root fallbacks;
optical equality and transition stability still need wider acceptance coverage.
Coarse roots are not a claim that every requested optical bound was met.

Root records are still per tree. Spatial-cell aggregates without per-tree
instance data (as described in the user's forest-demo reference) remain future
work. GPU traversal still examines roots each frame; shared ancestor pose caching,
hierarchical world culling and streamed virtual pages are not implemented here.

Sun cache invalidation is conservative when GPU slots/wind change. Incremental
animated shadow page updates remain separate work. Existing voxel headlight
shadow support is not added by this integration. Root movement/repopulation
rebuilds the scene; no new incremental root-update API is claimed.

The 4K output / four players / 120 Hz (8.33 ms) target remains unmet. Lighting's
2.5 ms goal, hybrid screen-space/voxel GI, and far instanceless proxies remain
separate milestones. Do not switch defaults based solely on this report.

Reference: https://www.youtube.com/watch?v=uR5EvBWxN14 . Architecture description
was supplied by the user. Fetching the video was throttled; no video imagery,
hardware specification or performance claim has been independently verified.

## Final measurements

Per-condition median of the three run medians. Same executable SHA256
`c6ef5e7ab5c0a7c207f179392da4fa6575dad9ea03e5222780c6fdfd2ebec7e1`.

| Forest | Route | GPU covered interval (ms) | GPU selection/poses (ms) | CPU Pool update (ms) |
|---|---|---:|---:|---:|
| 500 m | CPU Pool baseline | 16.078 | 0.000 | 0.003 |
| 500 m | GPU residency | 15.927 | 0.702 | 0.000 |
| 2500 m | CPU Pool baseline | 16.347 | 0.000 | 0.003 |
| 2500 m | GPU residency | 17.335 | 0.789 | 0.000 |

No overall speedup is established: the large-scene GPU route remains slightly
slower than the CPU Pool baseline. It removes the CPU allocation dependency and
keeps full-world branch output bounded. Different cuts also change pixels.

All per-run/pass medians, conditional-pass presence and CPU measurements are in
[the evidence JSON](evidence/foliage-runtime-2026-11-04.json). Per-frame pass medians
include zero for frames where a cached pass did not run; conditional medians
are provided separately. Do not add medians and call that an exact frame time.

| GPU pass, 2.5 km forest | CPU Pool baseline (ms/frame median) | GPU residency (ms/frame median) |
|---|---:|---:|
| GPU forest selection residency poses | 0.000 | 0.789 |
| aggregate error selection | 0.010 | 0.000 |
| bloom | 0.046 | 0.046 |
| canopy sky transmittance | 0.000 | 0.000 |
| canopy volume | 0.000 | 0.000 |
| exhaust | 0.001 | 0.001 |
| filter canopy | 0.000 | 0.000 |
| motion vectors | 0.059 | 0.060 |
| near shadow cull | 0.116 | 0.110 |
| near shadow raster | 0.135 | 0.152 |
| shade | 0.310 | 0.304 |
| shade terrain | 0.709 | 0.755 |
| shade voxels | 0.118 | 0.118 |
| shadow cull | 0.131 | 0.114 |
| shadow raster | 0.232 | 0.230 |
| taa | 0.054 | 0.054 |
| terrain coarse | 0.209 | 0.213 |
| tonemap | 0.044 | 0.044 |
| tree occluder cull | 1.332 | 1.256 |
| tree occluder raster | 6.724 | 5.667 |
| tree occlusion pyramid | 0.083 | 0.084 |
| tree remaining cull | 0.861 | 1.139 |
| tree remaining raster | 0.513 | 0.951 |
| vegetation branch poses | 0.095 | 0.000 |
| voxel near sun splat raster | 0.054 | 0.043 |
| voxel near sun splats | 0.326 | 0.287 |
| voxel splat raster | 0.456 | 1.013 |
| voxel splat resolve | 0.341 | 0.383 |
| voxel splats | 1.905 | 2.496 |
| voxel sun splat raster | 0.210 | 0.180 |
| voxel sun splats | 0.258 | 0.206 |
| wheel particles | 0.000 | 0.000 |

## Image review

Reviewed matched 500 m old/new route images, the large-forest image, and all
eight 960x540 pan frames (1 degree/tick). The inflated dark crown effect is
corrected. Trees, source branches and terrain remain present during the short
pan. Root-budget substitutions still change nearby crown texture/coverage;
stochastic grain is visible, particularly before temporal history settles.
This eight-frame sequence does not establish shimmer-free transitions or broad
lighting/weather correctness. Keep opt-in; no visual-parity default switch.
