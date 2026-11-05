# Unified vegetation: shared primitives and branch poses

## Parallel brick visibility - 29 October 2026

The new opt-in `KERN_VOXEL_BRICK_VISIBILITY=1` path builds shared, disjoint
support regions, projects them with the existing poses, constructs visibility
in parallel and sorts bounded ray intervals front to back before integration.
It preserves the source and optical data. It is still fractional integration,
not opaque Nanite rasterization, and source triangles still cast shadows.
See [implementation, budgets and measured evidence](vegetation-brick-visibility.md).
The following measurements describe the preceding branch-volume baseline.

## Assembly aggregate baseline - 29 October 2026

**The current volume consumer is not Nanite voxel rasterization.** Warp still
ray-integrates branch volumes with BRDF evaluation at occupied samples. Epic
documents compact voxel clusters in separate raster bins, front-to-back depth
buckets and a normal distribution sampled into the GBuffer before shading.
Warp's tile bins and experimental `KERN_VOXEL_GLOBAL_INTERVALS=1` worklist
optimize its existing integrator; neither implements that pipeline. Compare
each proposed fix with the documented mechanism and state the remaining gap.
[Epic: Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)

The approved dense r4 woody source is preserved through the shared primitive
path. [`vegetation-species.md`](vegetation-species.md) is the current source,
aggregate API and evidence guide. The new opt-in engine flow is:

```rust,ignore
let cooked = CookedPlant::cook_aggregate(&assembly, vegetation_aggregate::Config::default())?;
let mut vegetation = SceneVegetation::new(&mut scene, explicit_attach_limits)?;
vegetation.append(&cooked, plant_instance)?;
vegetation.finish_aggregate(aggregate_lod::Policy {
    allow_sampled_optics: true, // Explicit experiment, default is false.
    ..Default::default()
})?;
```

The cooker streams transformed shared primitives into pose-coherent branch
aggregates and a static subtree-root candidate. Each candidate level uses at
most 128 bricks of 4³ cells. Triangle cuts and volumes are compared against the
same finer source-derived medium using sampled directional opacity/material
errors. GPU per-view selection combines those errors with projected spatial
error; a parent aggregate suppresses its children. This is a small aggregate
hierarchy, not unrestricted Nanite assembly streaming or exact rendered-error
measurement. Finer source-cut candidates now execute in the fixture, with
single-camera evidence below; their optical and temporal acceptance remains open.

The volume renderer now bins each selected volume to tiles once, restores a
deterministic source-ID order, and retains exact per-ray entry ordering. Any
tile/interval/active-cache overflow uses a complete fallback. Selected aggregate
mips are sampled exactly; legacy volume consumers keep their footprint/ceiling
behavior. Tile bins are not cluster depth buckets. Source triangles continue
to cast all shadows.

Rigid branch animation has dynamic, device-checked buffer sizes, all-consumer
projected-motion scheduling, rest/wake history and indirect attachment pages.
The legacy 256-pose admission default is no longer the runtime allocation ceiling;
64 branches per assembly remains. The explicit runtime limits are 262,144 poses,
1,048,576 attachments and 512 MiB internal animation payload, with per-buffer
device limits. These are ceilings, not measured supported forest populations.
Source scene-instance residency still grows with placements; sleeping or
dispatching fewer attachment pages does not remove that memory.

The single-tree native 1080p `aggregate-lod/smoke-2` fixture ran successfully.
At 50/150/400 m auto retained full source and matched source RGB/opacity exactly.
At 1,000 m 60 branch volumes reduced colour triangles from 32,099,798 to 458
including ground, while shadow maps retained 14,619,068 source triangles.
Native coverage increased from 68.000 to 79.266 pixel-equivalents (fuzzy IoU
0.630); the distant optical result is not accepted. Only two measured frames
followed two warmups, and unrelated background GPU activity confounded timing.
Offscreen rendering did not isolate the GPU. No speedup, moving-camera/DLSS transition, four-view forest result or
live Sideways switch is established. Keep the retained forest default; the old
flattened `SIDEWAYS_TREE_SPECIES=mixed` route remains explicitly disabled.

Evidence: `C:/dev/perf/warp-sideways-20261028/aggregate-lod/smoke-2/report.json`.
This evidence predates subsequent source-cut refinements and must not certify
unmeasured code.

Later evidence under the same `aggregate-lod` root:

- `fine-global-1`: the 0.025 m reference grid plus refinements is rejected.
  The 0.08 optical policy retains full source even at 1,000 m, after 139.47 s
  preparation and 23,388,756 packed volume bytes. Source/auto TAA sampling phases
  differed in this run; its image/opacity differences cannot judge LOD quality.
- `global-reference-1`: 0.075 m plus refinements/global intervals uses the later
  `reset_capture_sampling()` fix and 32 frozen-pose TAA frames. Camera triangles
  fall from 32,099,798 to 22,260,566 at 50 m, using source cuts. At 1,000 m,
  60 volumes leave 458 camera triangles, but raw coverage exceeds source by
  12.37% and fuzzy IoU is 0.656. Shadows still retain 14,619,068 source triangles.
  There is no reported volume overflow. The eight-sample far median is 14.56 ms
  integration alone versus 12.90 ms full-source covered interval; auto totals
  20.27 ms. Competing GPU work remains uncontrolled: these diagnostics establish
  no performance win. The interval stream is not promoted.

Shading diagnostics `global-cheap-brdf-1` and `global-density-1` keep identical
opacity PNG hashes and traversal counters, but measure 13.44/15.25 ms integration
against 14.56 ms full shading in eight contended samples. No material/lighting
win is established. The four-tree/four-view fixture reduces summed camera
triangles from 513,596,744 to 7,304, yet source/auto covered intervals are
138.40/145.15 ms; 226,596,066 source-shadow triangles remain. The 64-tree auto
guard run also completes, with 3,840 volume groups, 98.73 ms integration and
135.14 ms total covered interval. Its 25 tile-list overflows take the complete
fallback; step/pool overflow is zero. These runs deliberately pin near/main
shadow focus to the forest, unlike camera-relative gameplay. They are tiny,
contended stress tests, not game FPS or a ready forest implementation. See the
[full counts and review image](vegetation-species.md#latest-aggregate-experiments-neither-promoted).

Source shapes are unchanged. Corrected stationary TAA accumulation is not
motion/DLSS proof. The next boundary follows Epic's compact spatial voxel
clusters with separate visibility/material resolve and shading; further
branch-integrator polishing is not the next milestone. `taa-null-2` forces the
finest source with `--error 0.01` and matches native opacity exactly (IoU 1,
L1 0, coverage 31,649); 24 RGB pixels differ slightly. Capture-phase opacity
parity is established, not bit-exact RGB or motion parity.
The latest code checks pass: 672 render tests, one separate
GPU selector test, 189 Sideways tests, workspace all-target checking and launcher
contracts. Ignored tests and scope are recorded in the source/evidence guide;
these passes do not accept appearance or forest performance. The following
sections retain the earlier r30/r34 context.

## Historical meadow and mixed-patch status

Status, 25 October 2026: the shared grass recipe now also drives an opt-in
full-terrain GPU meadow field in Sideways, with three variants, per-branch
wind, source LODs, native/DLSS motion and current-frame Hi-Z rejection.
See [r34 architecture, actual GPU evidence and limits](meadow-field-r34.md).
This field replaces photographed grass in that playtest configuration; it
does not yet use far voxel aggregation or cast its own grass shadows.
The original source/volume path described below remains a bounded mixed
grass/sapling prototype and does not replace the full forest.

Current follow-up: [r32 source LOD and volume work](vegetation-optimization-r32.md)
adds source-derived intermediate blade/needle geometry and corrects empty-space
skipping at the selected voxel mip ceiling. It records actual per-view triangle
reductions, 72 per-plant image checks and controlled GPU timings. Full-forest
cost and source/voxel optical parity remain open. The [r31 distance and work
budgets](vegetation-distance-budget.md) remain the historical baseline; the
acceptance evidence and timings below describe the original r30 prototype.

## Historical r30 data and execution

`foliage_assembly::generate` produces a deterministic assembly: reusable opaque
primitive meshes, material bindings, stable placement IDs, branch hierarchy and
local transforms. The existing recipes are grass tufts and conifer twigs.
Growth variants are cooked parameters, not a runtime botanical growth simulation.

`vegetation_asset::CookedPlant::cook` keeps unique primitive geometry. The voxel
baker expands each branch only during cooking, to derive the volume from the
same geometry and materials. Runtime attachment does not flatten blades or
needles into copied branch meshes. The current implementation requires exact
geometry, material **and concrete CLOD build policy** bytes to deduplicate
across variants and plants; different materials currently require separate meshes.

`SceneVegetation` attaches plants to an existing scene transactionally. It returns
stable plant/group identities and explicit instance, pose and volume ranges.
Invalid input or unfinished attachment rolls back the scene additions. Shared
mesh bytes, placement counts and packed volume bytes are reported separately.

Each occupied branch has a zero-cluster proxy and a validated list of primitive
members. The per-view representation decision belongs to that branch. Selecting
its volume suppresses every member in the colour pass, rather than leaving both
representations visible. Source geometry still casts shadows.

The GPU computes the hierarchy once per frame, before all views. A second pass
scatters `branch_pose * primitive_rest` to the primitive instances. There is no
second wind-wave calculation per needle. Current/previous branch poses, volume
inverse transforms, source transforms and conservative bounds share this pose.
Nonuniform positive root scaling is supported; shear/reflection roots are rejected.

Minimal engine-side flow:

```rust,ignore
let source = foliage_assembly::generate(parameters, Limits::default())?;
let cooked = CookedPlant::cook(&source, bake_config)?;
let mut vegetation = SceneVegetation::new(&mut scene, AttachLimits::default())?;
let handle = vegetation.append(&cooked, PlantInstance {
    id: stable_world_id,
    generation: asset_revision,
    root: plant_to_world,
    category: GeometryCategory::Trees, // Grass uses the same path.
})?;
let report = vegetation.finish(Policy::default())?;
```

`Policy::default()` keeps source geometry until optical evidence is accepted.
`allow_unvalidated_optics` is only for explicit voxel experiments. It must not
silently become the production default because source poses pass validation.

## Applied Sideways slice

`games/sideways/app/src/grass_voxel_asset.rs` cooks three grass layouts and two
conifer-sapling growth variants through the same engine API. A fixed 8×4 native
grass-cell rectangle replaces at most 32 existing clumps; at most four nearby
saplings are added on supported ground with road clearance. Saplings are
decorative and do not change driving physics. Native grass roots and exclusion
cells keep their existing identities. Saplings use a separate ID namespace.

Enable explicitly with `SIDEWAYS_PROCEDURAL_GRASS=source`, or `auto` for the
unaccepted source/voxel switch, and `SIDEWAYS_FAR_GRASS=0`. Optional
`SIDEWAYS_PROCEDURAL_GRASS_CELL=x,z` freezes the patch while changing cameras.
Without it, placement searches near the track roughly 70 m ahead of the initial
camera. These historical flag names now control both grass and saplings.

The repeatable scene is
`games/sideways/scenarios/unified-vegetation.toml`. Its comments specify the fixed
cell and required environment. It uses one native 1600×900 view and four held
camera distances; this is a real-stage preview, not a driving FPS benchmark.
Use the same wind time, seed, weather and environment for source/auto pairs.

## Acceptance and evidence

Evidence root: `C:/dev/perf/warp-sideways-20261024/r30-unified-vegetation`.

- Renderer CPU/WGSL tests: 267 passed, zero failed, one ignored.
- Sideways app tests: 142 passed, zero failed, one ignored. Includes exact native
  roots, mixed-species bindings, seasonal placement stability and maximum capacity.
- GPU fixture: `cargo run --release -p render --example unified_vegetation_gpu --
  --out FRESH_DIR`. Run serially without a playtest on the GPU.
- `gpu-final/report.json`: six plants, 30 branch poses, 315 primitive attachments,
  27 branch volumes. Images of shared and independently expanded source geometry
  match byte-for-byte at three wind states. GPU pose, primitive, volume inverse
  and bounds errors stay below 3e-7. Animated vertex/sphere checks pass.
- Four views choose 0/0/26/27 voxel branches at 3.8/45/120/240 m. GPU readback
  confirms selected branches no longer rasterize their primitive members. Source
  shadows remain. No step/candidate/active-cache overflow occurred in this fixture.
- A subsequent camera cut moves all four populated views back to source geometry.
  Each view again matches the expanded reference byte-for-byte, traces zero volume
  rays, and has a completely clear raw volume layer. No stale voxel image remains.
- Final locked all-target workspace check and native release fixture build pass.
- Actual seed-1 stage: 23 grass plants + two saplings, 125 poses, 102 volume
  owners and 966 primitive instances. Unique clustered geometry is 40,928 bytes;
  the unclustered per-placement expanded-source comparison is 2,297,376 bytes.
  Packed volumes add 973,540 bytes. These are different payload categories, not
  a total-memory ratio. Cook/attachment preparation was about 263–285 ms in the
  observed runs. `mixed-source.png.json` contains exact plant provenance.

The near recipe still looks sparse and differs visibly from the neighbouring
photo grass. The conifer recipe is a small twig/sapling prototype, not an adult
tree species. Source/voxel optical equivalence is **not** accepted.

## Historical r30 remaining work and performance gate

Mesh sharing is not a frame-time win by itself. A grass tuft has 36 primitive
instances instead of four flattened branch instances; a conifer twig has 69
instead of five. Joint coarse hierarchy traversal, compact visible work and
animation LOD are needed before expanding this to the forest.

The initial warmed four-view fixture (`gpu-b`, 12 alternating samples after three
warmup pairs) spends a median 3.371 ms on voxel integration for only 55 + 16
traced rays in the two far views. Source views trace zero rays. The complete
measured pass sum is 3.639 ms versus 0.192 ms for CPU-posed expanded source.
That reference has no GPU branch pass and retains far triangles: it is a workload
comparison, not an isolated optimization or equal-quality proof. The volume
cost must be fixed before a default switch.

The final post-rollback run (`gpu-final`, same 12-sample protocol) measured
3.018 ms median integration and 3.292 ms total instrumented GPU passes, versus
0.188 ms for expanded source. Shared branch-pose/scatter work was 0.0158 ms.
This run confirms the remaining volume bottleneck; differences between separate
runs are not attributed to a new speedup. Output is 1600×900 with four 800×450
native views on an RTX 5070 Ti, not the four-player 4K target.

A pixel-compaction candidate was measured and **rejected**. It reduced heavy
invocations from 2,048 scheduled tile pixels to the 71 actual rays, with identical
decoded RGBA images and no overflow, but the ABBA run medians were 3.152/3.304 ms
off versus 4.151/3.862 ms on: approximately 24% slower on this RTX 5070 Ti fixture.
The comparison includes the added compaction kernel. The candidate also required
an extra pixel worklist. Its runtime path was removed; useful active-tile and
scheduled-pixel diagnostics remain. Source and evidence are archived under
`rejected-pixel-compaction/` and `compaction-decision.json` in the evidence root.
The result disproves the proposed scheduling optimization for this case; it does
not establish whether register spilling, divergence or another kernel cost is
the underlying bottleneck. Do not reintroduce it as an assumed optimization.

The r30 path's limits were 256 branch poses/volume owners, 64 poses per plant, and 65,536
primitive attachments at the renderer boundary. Sideways deliberately uses a
smaller patch. There is no world streaming, shared coarse assembly hierarchy,
GPU habitat placement or full-forest animation LOD in this path yet. DLSS is
explicitly rejected for part-wind/voxel scenes; previous pose storage alone does
not provide correct fractional-volume motion vectors. Default foliage remains
unchanged. The 4K/four-player/120-FPS target remains unmet.
