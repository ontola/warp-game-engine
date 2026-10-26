# Foliage PR integration and trials — 26 October 2026

Status: the corrected local integration passes **517 renderer tests, with 1
ignored**. This covers CPU invariants and shader validation, not GPU performance
or visual acceptance. No experimental rendering default is promoted here.

## Scope and defaults

| PR / reviewed head | Purpose | Activation and unchanged default |
|---|---|---|
| [#4](https://github.com/ontola/warp-game-engine/pull/4), `0d8e81f` | Replace alpha-tested crown cards with opaque needle geometry, then voxel-derived tetrahedron LODs. | `SIDEWAYS_TREE_CROWN=opaque`; default remains `cards`. Opaque mode uses `SIDEWAYS_TREE_VOXEL_SWITCH=0.08` metres by default; `0` isolates needle geometry without proxies. |
| [#5](https://github.com/ontola/warp-game-engine/pull/5), `8dd5d62` | Fade distant tree wind to rest and admit conservatively pinned frozen trees to the existing static sun cache. | `KERN_WIND_FREEZE=50,70` is an example trial, not an accepted distance. Freeze is off when unset. Cache trials additionally use `KERN_STATIC_SUN_CACHE=1` and `KERN_SUN_CACHE_GUARD_METRES=32`; both remain opt-in. |
| [#6](https://github.com/ontola/warp-game-engine/pull/6), `cca3902` | Rasterize eligible small opaque triangles in compute for camera visibility and orthographic sun depth; order voxel proxies spatially. | `KERN_SOFTWARE_RASTER=16` is an example threshold for bounding-square pixels per triangle, not a 16-pixel cluster diameter. Unset means hardware only. Camera software raster requires supported 64-bit atomic features; unsupported devices retain hardware. |

PR #6 includes PR #4's source changes. `KERN_CLUSTER_TRAVERSAL=1` is a separate
experimental variable and must be pinned explicitly when comparing these paths.
The engine supplies reusable cluster switching, wind/cache policy and raster
routing; Sideways supplies the tree assets, material settings and trial scenes.
The tetrahedra are **voxel-derived raster geometry**, not the shared volumetric
optics/integration path and not a complete Nanite implementation.

## Three root-cause corrections in the integration

1. **Perspective source/proxy overlap.** `ClusterMesh::replace_beyond` originally
   enclosed vertices but not the existing local error spheres. An off-centre
   camera could therefore accept a proxy while a finer source branch still
   passed its old cut. The common switching sphere now encloses all original
   parent/own error spheres, with double-precision construction and an outward
   rounded radius. A non-planar two-branch regression reproduces the old overlap
   and checks exact leaf coverage around perspective transition thresholds.
   Empty replacement levels are rejected before changing source data.

2. **Wind field and cache disagreement.** The host packed cameras as
   `[x,z,0,0]`, while WGSL read `.xz`, making GPU camera Z zero. Uploads now retain
   actual XYZ. Related invalidation fixes ensure camera movement changes combined
   sun depth even at fixed wind time, transformed frozen owners return to the
   dynamic partition until recertified, and disabling freeze for more than four
   views clears old pinned membership. Tests inspect uploaded bytes and cache
   transitions, not just the independent CPU distance formula.

3. **Software routing could silently discard geometry.** A radial-distance
   diameter estimate understated off-axis perspective bounds; a later independent
   triangle-box cap could drop geometry already removed from hardware. Routing
   now projects the wind-padded AABB, certifies the complete depth slab, and bounds
   the inclusive pixel rectangle to 64×64. Uncertain clipping stays on hardware;
   the conflicting raster-side box rejection is removed. Double-precision
   projection regressions cover off-axis, near-plane and orthographic cases.

The integration also increases timestamp capacity for the additional camera and
sun software scopes. These are correctness and observability repairs, not
measured speedups.

## Remaining acceptance risks

- Crown proxies fit summed silhouette coverage over six directions. The PR's
  CPU checks report unseen-view coverage ratios of **0.75–1.32**. They bake from
  the original cards, use one averaged normal per tetrahedron, and do not establish
  lighting or shadow equivalence to the opaque source. Low sun and backlighting
  are important checks. Cell size alone is not a proven conservative geometric
  error for the jittered, scaled tetrahedra.
- Fine proxy levels can contain more triangles than the source cut they replace.
  Resident clusters, software atomic contention, shadow raster and CPU proxy
  fitting must be measured separately; fewer distant triangles do not prove a
  cheaper whole frame.
- Wind freezing deliberately changes distant animation. Test split-screen camera
  movement and cache repinning, including the full static redraw spike. Near sun
  maps still perform dynamic work.
- Software edge rules, equal-depth ties and emulated sun depth bias still need
  matched image/depth inspection. A valid shader is not evidence of hardware
  raster parity or performance.

## Separate evidence streams

The user's `C:/dev/warp-bench/run-bench.ps1` is running the **original PR branch
refs** in separate clones/worktrees and target directories under
`C:/dev/warp-bench`. It writes to `C:/dev/perf/claude-bench-<timestamp>` and does
not test the corrected integration described above. Its branch names are
`software-raster` and `wind-freeze-sun-cache`; record the resolved commits and
executable hashes from each run rather than assuming branch names are immutable.
Original-PR findings remain useful, but cannot certify these fixes.

For the corrected integration, pin every feature flag, camera, wind time and
light state. Start with native crown source/proxy transitions and low sun; isolate
software raster and traversal afterward. Wind/cache trials need moving cameras
and one/four-view coverage. Report actual input/output resolution, clean sample
count, GPU interval and individual scopes, CPU execution separately, memory,
selected geometry, image/depth differences and any fallback. Exclude warmup and
diagnostic readback from timing summaries.

## Pending GPU results — to be filled by the root runner

| Evidence stream | Exact source / executable / run path | Visual and correctness result | GPU timing and scope |
|---|---|---|---|
| Original PR refs, external user script | Pending run manifests | Pending | Pending |
| Corrected integrated source | Pending immutable snapshot and executable | Pending | Pending |

No hardware GPU result, accepted visual equivalence, default promotion or
8.33 ms frame-budget achievement is claimed in this document yet.
