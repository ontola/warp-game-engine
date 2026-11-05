# Foliage PR integration and trials — 26 October 2026

Status: the corrected local integration passes **517 renderer tests and 187 CLI
tests, with 1 ignored in each suite**. Its release build completed in **102 s**.
These checks cover CPU invariants, shader validation and compilation, not GPU
performance or visual acceptance. Eight original-PR GPU runs and five completed
corrected-integration runs are recorded below; the combined opaque case failed.
No experimental rendering default is promoted here. This report records the
trial branch state at that time; subsequent PR disposition and experiments are
tracked in the [ordered budget plan](four-view-budget-plan-r55.md).

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

The user's `C:/dev/warp-bench/run-bench.ps1` completed the **original PR branch
refs** in separate clones/worktrees and target directories under
`C:/dev/warp-bench`. This run wrote to `C:/dev/perf/claude-bench-20261026-2158` and does
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

## Original-PR GPU results

Evidence: `C:/dev/perf/warp-sideways-20261026/r54-pr-trials/` contains
`requested-bench-extracted.json` (all eight runs) and
`original-crowns-analysis.json` (matched crown counts, images and interpretation).
Every run exited **0** with **480/480 frames**. Each of its two sections has
**179 eligible synchronized GPU samples** after warmup and capture exclusions,
with no missing or invalid GPU samples. These are single ordered runs, not ABBA
repeats; there is no repeat-noise estimate. Times below are per-frame GPU interval
medians, not summed pass medians or gameplay FPS.

The five crown cases use the same original `cca3902` executable, SHA-256
`87c2c35926adf656db779107caa11d1b0430ff17bbdea6e2efe4357ceb644bef`, native
1920×1080, one view, noon and fixed wind time. They use legacy card grass rather
than the current live meadow, with 143,394 selected trees out of 319,179 roots,
size multiplier 1.5. These results do not describe the current four-player game.

| Original crown configuration | Held forest, ms | Canopy vista, ms |
|---|---:|---:|
| Cards, hardware | 9.204 | 10.311 |
| Opaque needles + proxies, hardware | 55.088 | 60.854 |
| Opaque + software threshold 16 | 35.355 | 38.055 |
| Opaque + software 16 + cluster traversal | 30.401 | 33.443 |
| Cards + software 16 | 8.779 | 9.439 |

Software raster and traversal reduce the opaque variant's regression, but its
best measured interval remains **3.30×/3.24×** the card reference. In held forest,
selected scene clusters grow 202,393→2,211,360 and shared-sun triangles
1,948,615→52,311,585. Hardware raster, culling and shadow geometry dominate this
increase. Cards + software suggests a modest reduction, but software excludes
alpha-tested needles: that case routes eligible opaque terrain/wood instead.
It is not evidence that card needles became cheaper, nor a repeat-validated gain.

The image review finds a conspicuously darker distant hillside in the opaque
variant before software routing is enabled. A fixed mixed crown/terrain ROI's
display luminance falls 39.92→17.02/255; this is not a segmented coverage or
physical-light measurement. Opaque hardware versus software differs at
1,558/5,977 pixels, with maximum channel differences 175/179; sparse differences
are not exact parity. Enabling traversal on the software variant produces exact
decoded RGB matches at both captures, with unchanged selected counts and sun
fingerprints. Cards hardware versus software still differs at 26/155 pixels.
These final RGB images cannot isolate normal error, coverage, shadow bias or
missing geometry, and do not clear the original routing defect.

The three route cases use original `8dd5d62`, executable SHA-256
`0e83da9007d4943da9a8821658fa1e000400db12f58eafaad08688040cbdd03d`, one
3840×2160 output with four actual 1916×1076 view rectangles. The scenario requests
DLSS Quality and follows the deterministic noon route.

| Original route configuration | First section, ms | Continued section, ms |
|---|---:|---:|
| Baseline | 32.557 | 34.752 |
| Static cache + 32 m guard | 32.855 | 34.786 |
| Cache + guard + wind freeze 50–70 m | 31.267 | 33.723 |

The freeze run is only about 1.29/1.03 ms below baseline. **The original camera-Z
packing defect invalidates performance acceptance:** GPU wind freezes around
different camera positions from those used to certify static shadow casters.
Completion and lower timing do not prove a correct cache or equivalent scene.
The corrected integration must be measured separately before any promotion.

## Corrected-integration GPU results

The release executable is `r54-pr-trials/sideways-trials.exe`, SHA-256
`028dd855a9fca56159ba4506c034916f23fd5d61f18858a23a06e37c3a4b01f7`.
Source snapshot: `1d6a233f0ed5cfbd05765d65f9b50a7318d4d46a` on
`codex/try-foliage-prs-20261026`. Unlike the original script, these runs explicitly
use the current saved vegetation baseline, meadow field/aggregate rendering,
tree occlusion and paged foliage shadows. All flags and shader/executable hashes
are in each run manifest. The GPU is an NVIDIA GeForce RTX 5070 Ti.

The following are **mean covered GPU intervals**, not the medians above, not
visible presentation FPS, and not typical gameplay. Every completed case has
480 rendered frames and 358 measured samples: 60 warmup frames per section and
the captured frames are excluded. Each variant ran once in order; shared-host
CI load is uncontrolled. No compilation overlapped these corrected GPU runs.

| Current-preset case | Resolution / views | Mean GPU ms | p95 ms |
|---|---|---:|---:|
| `integrated-crowns-a`: cards | Native 1920×1080, one view, two held cameras | 19.398 | 21.921 |
| `integrated-crowns-b`: cards + corrected software raster 16 | Same | 19.636 | 21.908 |
| `integrated-route-a`: baseline | 3840×2160 output, four DLSS Quality views | 48.800 | 55.797 |
| `integrated-route-b`: freeze + cache + guard | Same route/output | 47.870 | 55.119 |
| `integrated-route-sw`: cards + corrected software raster 16 | Same route/output | 46.450 | 52.963 |

The combined opaque + software + traversal + freeze case
`integrated-crowns-all` **fails before its first rendered frame**, exit 1:
`foliage page exceeds visible-cluster capacity; refusing truncated shadows`.
It has no valid GPU performance or output image. The paged-shadow guard was
retained; no incomplete-shadow workaround was enabled to manufacture a pass.
The 256 m page's summed resident caster cluster counts exceed the existing
4,194,304-cluster visible-list capacity. This is an admission limit, not a
measurement of post-LOD selected triangles.

Four-player freeze/cache changes the shared shadow half-extent from 112 to
176 m and texel size from 0.0546875 to 0.0859375 m. Thus the small 1.91% mean
change includes 57% coarser shared texels, frozen animation and caching; it is
not an isolated equal-quality cache saving. Five JPEG pairs show no obvious
gross stale shadows, while all 480 recorded camera/car/light identities match.
Four measured repins add 1.38–1.49 ms static cull/raster work. The cache uses
about 121.3 MB and seeds about 67.1 MB/frame. Temporal and raw-depth parity need
an identical-freeze/guard cache-off/on comparison before promotion.

Software raster on existing cards is mixed: a small one-view regression and a
4.82% lower four-view mean in this ordered trial. It still excludes alpha-tested
needles; it cannot be credited with eliminating their raster cost. These results
need repeated order-reversed runs and motion/image acceptance before changing
defaults. The four-view baseline's major mean scopes remain tree remaining
raster 12.65 ms, meadow aggregate/visibility/resolve about 13.27 ms, shade
9.03 ms and DLSS 3.05 ms. Scope costs are descriptive, not additive projected
optimization gains.

Corrected card HW/SW PNGs differ at 516/2,394 pixels in the two held views,
with maximum channel differences 44/175 bytes. Selected owner/cluster identities
and captured inputs match. Preview review shows no obvious large coverage shift,
but those sparse differences are not exact parity. The route JPEG pairs also
show no obvious large shift at preview scale; compression prevents using them
as pixel-exact evidence. Software remains off by default.

Evidence under `C:/dev/perf/warp-sideways-20261026/r54-pr-trials/`:
`summary.json`, `fixed-wind-analysis.json`, `fixed-crowns-analysis.json`, complete
run manifests, raw frame profiles and captures. The default frame-budget report
correctly excludes hidden diagnostic samples; the separate extraction explicitly
reports their synchronized GPU intervals instead.

| Evidence stream | Exact source / executable / run path | Visual and correctness result | GPU timing and scope |
|---|---|---|---|
| Original PR refs, external user script | `claude-bench-20261026-2158`; heads/hashes above | Complete; limitations and known original defects above | Eight cases above; single ordered run each |
| Corrected integrated source | `r54-pr-trials/sideways-trials.exe`, SHA-256 above | Five completed cases; combined opaque/paged-shadow case fails | Table above; experimental, no default promotion |

Decision: keep all three experimental modes opt-in. The integration fixes and
tests are retained on the trial branch; main and the remote PR merge states are
unchanged. The 8.33 ms frame-budget target is not achieved. Next work is reducing
opaque proxy/source geometry and shadow capacity pressure, measuring projection-
matched cache behavior, and repeating the software-raster four-view comparison.
