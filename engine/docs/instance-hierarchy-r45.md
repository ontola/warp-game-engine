# Instance hierarchy — r45

Status: implemented behind `KERN_INSTANCE_GROUPS=1`, **off by default**. Repaired native, four-view, and sampled moving-wind/camera-cut diagnostics preserve exact selected geometry with zero fallbacks; a four-view ABBA measures a small net saving. Strict pixel equality remains failed. The exact tested binary is suitable for an explicit experimental playtest, with no global promotion. This is a two-level instance broadphase feeding the existing cluster selector, not a replacement for the renderer or a claim of complete Nanite functionality.

## Implementation

[`cull_groups.rs`](../crates/render/src/cull_groups.rs) builds a deterministic Morton ordering of every uploaded owner slot. Quantization uses one world-space scale across all axes; owner IDs break ties. A leaf contains up to64 owners and a parent up to8 leaves, giving at most512 owners per parent. Empty pool slots, moving cars, and animated owners remain in the ordering. A scene/renderer rebuild, including applying tree settings, reconstructs the ordering.

[`cull_groups.wgsl`](../crates/render/src/shaders/cull_groups.wgsl) refits leaf and parent bounds on the GPU after wind, procedural branch poses, and baked actor animation. This runs once per rendered scene frame and is shared by all views. Bounds use the current instance sphere plus the same wind envelope as the existing selector. They are not frozen at constructor positions. Moving or recycling slots can reduce spatial locality, but does not remove them from the hierarchy.

[`cull.wgsl`](../crates/render/src/shaders/cull.wgsl) first tests parent bounds, then surviving leaves, then individual owners. Rejected parents avoid leaf/owner reads; rejected leaves avoid owner reads. Surviving owner IDs are compacted into a separate per-view buffer and dispatched indirectly into the existing `cull_instance` path. That path retains material policy, projected cluster error, alpha geometry, and per-cluster visibility rules.

The two-phase tree path remains intact. Phase1 retains the same occluder membership, rasterizes real source depth, and builds its current-frame depth pyramid. Phase2 may reject an enclosing group using that pyramid before individual owner/cluster work. Group selection never becomes an occluder itself. Dedicated owner scratch preserves the phase1 visible-list prefix and bucket draw map while phase2 appends.

## Conservative rules and limits

- Group spheres enclose all current member spheres, including mixed categories and animated bounds. Empty groups are rejected only after the refit finds no active geometry. Invalid or extreme bounds fail open.
- Frustum and camera-fog tests operate on enclosing bounds. Group Hi-Z rejection is restricted to phase2. Near-plane/W crossings and invalid projections retain the group; the existing conservative pixel-footprint pyramid test includes depth openings.
- Every view uses its own projection, depth pyramid, compacted owners, and phase counters. No previous-frame camera visibility is reused.
- Sun, paged foliage, near-sun, and headlight shadow selection remain independent. An offscreen group rejected by a camera is still eligible as a shadow caster.
- Compaction overflow or an expected-versus-visited owner mismatch retains the complete owner-scan fallback and records it. The acceptance harness requires both fallback counters to be zero; fallback-protected image equality cannot validate the optimized traversal.
- This hierarchy does not yet adapt its topology after large movements, build a deeper BVH, stream geometry, or change source/voxel representation. It adds a shared refit and per-view selection cost; a net improvement must be measured.

`KERN_INSTANCE_GROUPS=0` preserves the flat comparison path. The r45 comparison also fixes `KERN_INSTANCE_BROADPHASE=0`, `KERN_TREE_OCCLUSION=1`, and cluster buckets on. The independent new static sun cache is forced off in this comparison so its gains cannot be attributed to instance grouping.

## Memory and diagnostics

For `N` owners, `L=ceil(N/64)` leaves, and `P=ceil(L/8)` parents, shared hierarchy storage is `16 + 32*(L+P) + 4*N` bytes, apart from minimum dummy allocations for an empty/disabled scene. It is approximately4.56 bytes per owner. Each camera adds `4*N` bytes of owner scratch plus its small indirect dispatch buffer. Shadow targets use dummy owner scratch. These are additional hierarchy allocations, not total scene memory.

`Renderer::debug_tree_occlusion` exposes `instance_groups`; the capture sidecar stores it under `render_diagnostic.tree_occlusion.views[i].instance_groups` and the game's tree-occlusion metadata. It reports:

- allocated parent/leaf counts, owner slots, shared bytes, and owner scratch bytes per view;
- actual GPU parent/leaf tests and rejections;
- owner tests, compacted owners, resident cluster references visited, full-scan fallbacks, and traversal-mismatch fallbacks.

Work counters accumulate the camera phases for that view. They are not unique world-owner counts, selected triangles, shaded pixels, or values to multiply by the number of views again. Shared allocation bytes are repeated as context in per-view metadata and must be counted once. Readback is explicit and blocking only on excluded diagnostic captures.

GPU timestamps expose `instance group refit` once per scene frame and `instance group select` for camera phases. Both are already inside the measured renderer GPU interval. Do not add them again to that interval or add CPU wall time to GPU time.

With `KERN_DIAGNOSTIC_HIDE=none`, each capture also records the selected `(owner, cluster)` multiset count and a sorted FNV1a64 fingerprint. Sorting removes append-order noise while retaining duplicates. Fingerprint equality is separate from image equality. A group may prove additional geometry fully hidden that the flatter test conservatively retained; a resulting fingerprint difference still fails the strict harness and requires retained-image and selection review, not automatic forgiveness.

## Validation and pending evidence

The root-coordinated render library run currently reports **410 passing tests**. Focused hierarchy tests cover stable membership including empty/nonfinite slots, isotropic spatial locality, bounds around moved/recycled/wind-swept members, enclosing-parent/frustum implications, composed WGSL validation, and the storage-binding budget. Existing tree phase/projection tests and shader-specialization tests also passed. These CPU checks do not establish GPU image parity or performance.

The external harness is `C:/dev/perf/warp-sideways-20261025/r45-instance-groups/run-hierarchy.py`; its preparation/validation suite now has **17 passing Python tests**, including first/last scenario tick and wind-time semantics. The later root-coordinated `render-tests-e.log` records **422 passed, 0 failed, 1 ignored**. It defaults to prepare-only and requires `--execute` to launch owned children:

```text
run-hierarchy.py --mode native --exe <frozen-exe> --out <native-run> --execute
run-hierarchy.py --mode four --exe <same-exe> --out <four-run> --native-report <native-run>/report.json --execute
run-hierarchy.py --mode temporal --exe <same-exe> --out <temporal-run> --native-report <native-run>/report.json --execute
```

Each full mode runs off/on/on/off with identical executable and shader provenance. Native covers two captured cameras, first frame, settled frames, and a camera cut:128 frames per camera,32 warmup,95 clean samples each. Four-view testing uses actual4K output with DLSS Quality,192 frames/64 warmup and127 clean samples. The optional144-frame native temporal subset uses deterministic scenario wind and a forest→synthetic upward sky→forest sequence. Native strict acceptance normally gates the subsequent modes for the same executable. `--pair-only` permits a limited native off/on repair check. Temporal off/on is allowed only with explicit `--diagnostic-only`; it cannot approve another mode. Diagnostic continuation records the review reason and verifies original evidence hashes, retains failed native pixel status, and always reports `accepted:false` and `promoted:false`.

Captures are lossless1280×720 images: downsampled previews for native/four-view performance modes, full native output for the temporal diagnostic. They are not continuous-motion proof. All capture/readback and warmup frames are excluded from timing. Camera/light/player inputs, actual internal/output dimensions, source counts, selected-pair fingerprints, group counters, and decoded previews are retained. Only `sky.bake_cpu_ms` timing telemetry is excluded from input identity; its original sidecar value remains available. CPU frame wall includes waits and may be CI-contended; ABBA summaries are descriptive, not confidence intervals or gameplay FPS.

## Repaired native and four-view results

Evidence root: `C:/dev/perf/warp-sideways-20261025/r45-instance-groups/`. The first `native-a` attempt was invalid: the divergent eight-lane leaf traversal visited only a fraction of the retained owners, causing large missing terrain/tree regions. The source now uses a uniform leaf loop with all 64 lanes and checks expected versus visited owner totals before consuming the compact list. The precise underlying compiler/control-flow mechanism was not independently established; the failing counters remain a regression fixture and the original failed captures are retained.

The frozen `sideways-r45-candidate-b.exe` SHA256 is `58367b0d9025237378115bda3074f32f4629f42c2e7ac96ab911af4d922f6538`. In `native-repair-pair-b`, all four input/selection signatures match and both fallback counts are zero. Near first/settled previews are byte-exact. Far first/settled previews differ at 553/463 pixels, at most 2 channel values, with mean absolute byte differences 0.000211/0.000175. The strict report remains failed. Two-run interval medians are 14.976→14.760 ms near and 16.870→16.628 ms far; these limited pairs do not control run-order noise.

`four-diagnostic-a` deliberately continues from that failed pixel prerequisite without changing it to PASS. All first/settled signatures match across all four independent views and all four runs, with zero overflow or traversal fallbacks. Actual output is 3840×2160; each viewport is 1916×1076 with 1277×717 DLSS Quality input. There are 127 clean GPU samples per run after 64 warmup frames and capture/readback exclusion. The world remains seed 1, tick 6000, fixed wind, 120-blade meadow, tree occlusion on, and the independent static-sun cache off.

| Run order | Instance groups | GPU interval median |
|---|---|---:|
| Off A | Off | 43.234ms |
| On A | On | 42.539ms |
| On B | On | 42.610ms |
| Off B | Off | 43.285ms |

The mean of the two per-run medians changes 43.259→42.574 ms: **0.685 ms / 1.58% lower** in this diagnostic scene. Shared refit costs 0.0224 ms and all per-view/phase group selections total 0.250–0.251 ms, already included in the interval. This remains far above the 8.33 ms whole-frame target. It is a descriptive ABBA result under possible CI contention, not gameplay FPS, a confidence interval, or a sum of independent category costs.

At the settled On A capture, the 321,840 owner slots form 5,029 leaves and 629 parents. Shared storage is 1,468,432 bytes, plus 1,287,360 bytes of scratch per view (about 6.31 MiB combined for four views, plus small indirect records). Actual owner visits per view are 89,600 / 213,952 / 224,576 / 304,576; compacted owners are 37,103 / 57,148 / 66,227 / 141,725. These count work across each view's two phases, not unique objects or selected triangles.

All eight four-view PNGs were visually reviewed. Forest silhouettes, trunk placement, terrain, road, and meadow coverage remain aligned between off/on; the previous wholesale omissions are absent. Strict pixel equality still fails: A/B mean absolute byte errors are 0.00971–0.01114, comparable to same-mode A/A and B/B errors 0.00942–0.01101. First-frame maxima are 56–57; settled maxima are 15–25 for A/B and 13–15 for same-mode repeats. Similar repeat noise is evidence context, not proof of pixel equivalence; no tolerance was added. The 1280×720 previews and pause overlay limit fine-detail assessment. The separate temporal diagnostic below covers evolving wind and camera cuts in one native view; the feature remains opt-in.

## Moving wind and camera cuts

`temporal-pair-b` runs the same frozen binary off/on at native 1280×720 for 144 frames: 48 in the forest, 48 after a synthetic upward camera cut, then 48 at the original forest camera. Simulation ticks advance 6000–6143; wind uses those 120Hz simulation ticks, ending at 51.1916667 seconds with strength 0.6. The corrected harness treats rendered frame 1 as the section's start tick, not start tick+1. The earlier failed `temporal-pair-a` evidence is retained unchanged; its completed off run was revalidated on CPU before the new pair.

All 144 recorded frame input hashes match. All 12 captured input/selected-geometry signatures match, and every capture reports zero overflow and traversal-mismatch fallbacks. All 24 PNGs were visually inspected and their hashes checked against the report. No missing tree/terrain regions, changed coverage, or stale geometry after the camera cuts were observed. The upward view still includes branch/crown fragments around its borders, so this is not a fully geometry-free sky test.

The four upward-view pairs are pixel-exact. The eight forest pairs retain 536–584 changed pixels, maximum channel differences of 1–7/255, and mean absolute byte differences of 0.000203–0.000224. Strict pixel parity therefore remains **failed**, with `accepted:false` and `promoted:false`; no threshold was relaxed. A same-mode temporal repeat was not run, so those residuals are not attributed to noise as an established cause. The hashed review is `temporal-pair-b/visual-review.json`.

This supports a bounded **experimental interactive playtest** with `KERN_INSTANCE_GROUPS=1` on the exact 58367b0d… binary. It does not establish every-frame motion correctness, four-view temporal correctness, a default promotion, or the 8.33ms target. The root-coordinated launch is recorded in `games/sideways/build/dev-runs/20261025T224631837Z-b411005b/iteration.json` with hierarchy on and the independent experimental caches off; runtime source and the strict reports remain unchanged.
