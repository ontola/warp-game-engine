# Nanite-style vegetation handoff — 6 November 2026

For Claude taking over vegetation and cached sun shadows from Codex.

## Start here

**The requested result is NOT achieved.** Infrastructure exists, but normal-scene
voxels are still visibly oversized and performance is unacceptable. Do not treat
passing fixtures, implementation checkboxes or smaller storage as visual success.

- Repository: `ontola/warp-game-engine`.
- Windows worktree: `C:\dev\warp-terrain-playtest` (detached HEAD, working directly
  toward main). All implementation work described here is committed and pushed.
- Latest upstream integrated for this handoff: `9773c67`. My last implementation
  commits are `7803bc9` (compact cells) and `154b249` (split sun correctness), with
  documentation through `e846294`. No implementation edits are pending.
- Newer upstream commits after my measurements include browser/WebGPU support,
  terrain-streaming CPU work avoidance and phase-two queue capacity fixes. **The
  measurements below do not validate those newer builds. Rebuild and baseline.**
- Authoritative checklist: [vegetation-architecture.md](../vegetation-architecture.md).
  Current defaults/open issues: [status.md](../status.md).
- Use the new [render suite](../render-suite.md) and
  `engine/tools/render-suite.json` for comparable cameras and repeat runs.
- Read `AGENTS.md`. Direct main, no force pushes, preserve other agents' work.
  Physics stays120Hz/four substeps. No Godot comparison. Headless measurements;
  do not open an interactive playtest unless requested.
- `engine/HANDOFF.md` is retired. Logs belong here; update current guides in place.

## What Joep actually wants

A unified GPU-driven vegetation architecture for grass, shrubs and trees:
shared procedural assemblies, species/growth variation, articulated wind,
fine geometry nearby and approximately pixel-scale aggregate voxels farther
away. No CPU-selected near-tree pool. Stable transitions and realistic lighting.

The overall performance target remains4K output, four players,120FPS/8.33ms.
Suggested frame budgets: vegetation3.5ms (selection0.5/raster1.5/shadows1.5),
terrain1.5ms, lighting2ms, upscale/post1.3ms. These are targets, not achieved results.
Joep explicitly rejected solving performance by accepting large visible spheres.

The immediate missing result is **affordable fine voxel detail**, not another
coarse fallback, a tighter source cap, or merely a smaller cell storage format.
Shadow work is also a major outstanding architectural task (see below).

## Current implementation and code map

Under `engine/crates/render/src/`:

| Files | Current responsibility / limitations |
| --- | --- |
| `foliage_select.rs`, `foliage_refine.rs`, `shaders/foliage_refine.wgsl` | GPU incremental hierarchy selection, complete replacement cuts, reservation/prefix admission, dormant retries, consumer masks. No CPU near-tree shortlist. |
| `foliage_runtime.rs` | Shared species/population, source and voxel representations, asset dependencies, materialization, shared poses and diagnostics. |
| `foliage_pages.rs`, `foliage_page_cache.rs`, `foliage_source_pages.rs` | Bounded relocatable asset storage, asynchronous demand/readback, partial uploads, publication generations, ancestor fallback. CPU services asset bytes, not per-tree visibility. |
| `foliage_volume_data.rs`, `shaders/foliage_volume.wgsl` | Logical optics and packed GPU cells; shared reconstruction for providers. |
| `voxel_forest.rs`, `shaders/voxel_forest.wgsl` | Camera/sun splat generation and rasterization, current voxel rendering. |
| `vegetation_asset.rs`, `foliage_voxel*`, `foliage_groups*` | Cooking, hierarchy/groups, optics and voxel levels. Fine reference grids are retained when they fit the cook limits. |
| `shadow_damage.rs`, `shaders/shadow_damage.wgsl` | Main-map exact source/voxel history and damaged tiles. Raster reuse exists, but cull/splat generation still repeats. |
| `static_sun.rs`, `frame.rs` | Static/dynamic split and normal renderer integration. Split correctness repaired, default-off experiment remains. |
| `foliage_sun_pages.rs` | Existing experimental rest-pose, mesh-only page route; NOT the finished shared voxel/source page architecture. |
| `occlusion.rs`, `shaders/cull.wgsl`, `far_sun.rs` | Other agent's two-phase visibility and far-map work. Preserve it. |

Trees and terrain grass share the route in `terrain_fly`. Sideways still has a
separate meadow path. Shared assembly/pose infrastructure is not proof that all
games, far aggregates or consumer-specific detail are complete.

### Source work accounting (`7fe7539`, `fbd48ce`)

Source leaves used to cost zero splats and could bypass the splat budget into
millions of triangles. This is a real failure, not an imagined optimization.

- Count repeated source templates, not just unique resident meshes.
- `Node.links[3]`: low8 bits flags; upper24 bits conservative source cluster work.
- Fourth reservation dimension counts cluster visits for each visible camera or
  shadow consumer. Includes collective admission, refunds and retries.
- Proposal has six vec4s/96bytes; parked task+proposal112bytes. Publication mode
  is separate from the fourth cost component. Do not revert just one stride.
- Refiner state69 is reserved source work,70 snapshot,71 wake observation,
  72 source-budget blocked count. Diagnostics expose these costs.
- `Spec.source_cluster_budget` / `Selector::set_source_cluster_budget` control it.
  Current safety default is1<<30, deliberately nonbinding in the tested scene.
  **This is not a useful calibrated performance cap.** Tighter defaults failed
  visually. The setter rejects integers not exactly representable by its f32 ABI.
- Counts cover the full source CLOD catalog conservatively; they are neither
  actual raster triangle counts nor GPU milliseconds. The final capture reserved
  about25.7M such visits. Do not present this as25.7M rendered clusters.
- Original refinement priority is preserved. A source-cost priority penalty was
  also rejected because it changed where detail was admitted.

### Compact cells (`7803bc9`)

- Logical/cooked `GpuCell` / `VolumeCell` remains176bytes, full float data.
- `PackedCell` / `PackedVolumeCell` is128bytes: colour and directional extinction
  remain f32; six normal-centroid/weight vec4s become snorm16 pairs.
- Preserve centroid length and weight; do NOT normalize moments before packing.
  Maximum component error approximately1/65534 plus float rounding.
- Cache, uncached voxel and world-provider uploads all use the packed layout.
  GPU decode is tested against CPU. The separate dense brick prototype still
  uses logical cells; do not blindly change every176-byte constant.
-64MiB cell arena holds524288 rather than381300 cells. Splat arena remains512k
  references, index arena4M words, upload budget8MiB/eight pages per frame.
- This changes storage, not projected voxel size. Normal scene still fails1px.

## Latest measurements and their limits

RTX5070Ti/Vulkan on Windows. Diagnostic stationary lowland scene, seed1,
151058 active roots,32768 branch slots,16M splat budget, native1280x720,
ONE view/TAA,1200frames. GPU medians are the last120 frames. These were individual
runs, not repeated controlled acceptance. Do not extrapolate to4K/four views.

| Capture stem | Covered GPU median | Largest submitted cell | Decision |
| --- | ---: | ---: | --- |
| `pr8-retained-1200` |36.504ms|5.482px|Incomplete baseline |
| `source-budget-1200` (262144 cap)|20.636ms|18.993px|REJECTED; faster by losing detail |
| `source-budget-retained-1200` (16M cap + cost priority)|32.885ms|14.448px|REJECTED |
| `source-budget-final-1200` (high ceiling, original priority)|33.287ms|5.482px|Accounting retained, quality still fails |
| `packed-cells-1200`|33.287ms|5.482px|Storage retained, no speedup claim |

Packed capture:935 resident pages versus712 in the previous capture, roughly
88.19MB cache allocation in both, no missing raster records or refinement faults.
Upstream raster/fog changes also occurred between these captures: not an isolated
packing A/B. Page misses are cumulative demand-admission misses, not missing
rendered pixels. Still pictures visibly retain coarse foliage in some regions.
HUD60FPS is simulation-related; use GPU timing JSON, not the HUD or readback wall time.

Committed raw summaries/per-pass medians:
[vegetation-fine-endpoint-measurements.json](../vegetation-fine-endpoint-measurements.json).
Local images/logs/timings: `build/playtest-unified/<stem>.*` (ignored, not portable).
The build executable in the shared target directory may have been overwritten
by another agent: never assume its contents from the filename.

## Failed approaches: do not repeat without a new reason

- More refinement waves: same oversized cells, much slower selection. Reverted.
- More source slots: more geometry, still not pixel-scale; not accepted.
- PR8's screen-height/child-error gates around source bypass did not solve the
  issue. Both were removed in6a4002f. Do not restore the10%-height gate.
- Publishing only one selected mip and restricting all-source bypass
  (`90d51dd`) produced31.660px cells,602862 page misses and visible giant spheres.
  Reverted by`62a5779`. Passing tests did not establish visual correctness.
- Tight source caps expose the fine voxel deficit rather than solve it.
- Shrinking drawn splats without finer coverage would make holes. Not a fix.

## Working diagnosis and next architectural work

Verified: normal adaptive voxel representations require whole mip bundles;
`Spec::voxel_page_dependencies` retains the volume range. Raster reservation
uses conservative maxima over potentially accessed mips. Fine requests can be
blocked by both residency and splat capacity, leaving coarse ancestors. Source
bypass hides some of that deficit at excessive geometry cost.

**Recommended direction, not yet implemented/validated:** independently
selectable and streamable voxel clusters/bricks; visibility rejection before
large splat expansion; accurate per-consumer footprint/work reservation;
complete parent/child coverage and atomic publication through eviction. Avoid
requiring every mip of an articulated volume to be resident just to use one
bounded cluster representation. Investigate exact pressure per asset/cluster,
not only aggregate page-miss counts.

Validate the normal route throughout. Demonstrate pixel-scale detail and
coverage before calling it done, then establish repeated pass budgets. A new
fixture alone must not become another substitute for the normal forest.

Epic reference checked: https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage
Describes error-selected triangle/voxel hierarchy, clusters up to128 bricks of
4x4x4 voxels, dedicated depth-bucketed front-to-back voxel raster, and distributions
of normals. It does not promise exactly one voxel per screen pixel everywhere.

## Cached sun-shadow handoff and latest fix

Joep wants world-anchored static pages rendered once, moving casters composited
on top, and camera motion paying primarily for newly exposed pages.

Other agent's handoff measurements (Mac M5Pro, bbc3095, town2296x1440, last60/120
still frames) reported4.70ms sun work at wall and4.22ms at hill. terrain_fly has
no wind unless explicitly set. Main-map raster is cached but culls/splat
generation repeat; near map uncached; far map redraws on anchor/sun changes but
remains a forest selection consumer every frame. These are their measurements,
not my Windows results.

**Correctness prerequisite fixed in154b249:**

1. Static/dynamic path omitted `voxel_forest::render_sun` entirely. It now renders
   voxel depth after either shared or split source passes, with matching selection.
2. CPU partition skipped empty GPU source slots. Reserved forest owners now remain
   dynamic candidates; GPU culling checks their live geometry/NO_SHADOW state.
3. `gpu_static_split_preserves_forest_sun_depth` compares raw depth with the normal
   renderer for still, wind, root removal and restoration: exact match.
4. Existing cached/full redraw depth test and all9 static-sun tests pass.

This DOES NOT implement page-level work avoidance. The split stays opt-in.
No real-scene performance win or full render-suite image acceptance is claimed.
Cross-owner edits are `frame.rs` and `static_sun.rs`; no occlusion/cull/terrain
algorithm was changed by this fix.

Remaining shadow tasks:
- Independent shadow cuts at page texel scale, not tied to camera refinement.
- World-anchored pages covering voxel and source casters plus static world geometry.
- Unchanged pages skip selection/cull/splat generation, not just raster writes.
- Bounded invalidation for moved casters, population/assets, terrain and sun changes.
- Near-map reuse and explicit wind policy; no silent freezing of wanted animation.
- Raw cached/full depth after camera movement, zero static-page work counters
  when still, one moved caster damaging only affected pages; render-suite views.

## Ownership and integration

The other agent owns terrain/regions/footprints, occlusion (`occlusion.rs`, renamed
from `tree_occlusion.rs`), cull.wgsl, seed occluders and far_sun. Coordinate changes
there and record cross-boundary edits. Preserve seed visibility removing only
the camera role; hidden trees can still cast shadows. Preserve PR8's adaptive
selection-round scheduling and sun-splat target key `(selection, depth view)`.

No subagent or background implementation job was started for this handoff.
Claude is taking over; this document is not a claim that the overall goal is complete.

## Reproduce and validate

PowerShell, from the Windows worktree:

```powershell
$env:CARGO_TARGET_DIR='C:\dev\warp-gg\target'
$env:NVIDIA_DLSS_SDK='C:\dev\tools\nvidia-dlss-sdk'
$env:VULKAN_HEADERS='C:\dev\tools\vulkan-headers-dlss'
$env:WGPU_BACKEND='vulkan'
$env:KERN_ADAPTER='NVIDIA'
$env:KERN_DLSS_RUNTIME_DIR='C:\dev\tools\nvidia-dlss-sdk\lib\Windows_x86_64\rel'
cargo test --release -p render --lib foliage -- --include-ignored --test-threads=1
cargo test --release -p render --lib voxel_forest -- --include-ignored --test-threads=1
cargo test --release -p render --lib static_sun -- --include-ignored --test-threads=1
cargo build --release -p app --example terrain_fly --features render/dlss
```

Keep build environment consistent; changing SDK variables can trigger avoidable
recompilation. Do not compile/edit the same sources concurrently or run GPU tests
beside another GPU workload. Preserve other agents' processes.

Before the latest upstream pull:166 foliage +29 voxel tests passed for packing;
new static-split depth oracle, cached/full oracle and9 static tests passed for
shadow correction. Those counts belong to those commits, not latest main.

For the older diagnostic, camera JSON is:
`{"state":{"style":"lowland","seed":1,"pos":[0,8,0],"yaw_deg":34.3774677,"pitch_deg":0,"speed":1}}`.
Run terrain_fly with `--style lowland --grass --stationary --branch-slots 32768`, `--splat-budget 16777216 --fly 1200
--speed 1 --size 1280x720 --from <camera.json> --voxel-raster-profile
--require-pixel-vegetation --fly-times <output.json> --fly-shot <output.png>`.
The strict pixel check exits with failure AFTER saving artifacts; keep that failure.
For new work prefer the render suite, repeated same-resolution/mode comparisons,
per-pass medians and its camera/coverage/image checks. Its shared-world scenario
is different from the older lowland diagnostic; do not compare their totals.
