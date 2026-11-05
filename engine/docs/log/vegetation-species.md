# Vegetation species from shared primitives

Latest renderer experiment: [parallel brick visibility](vegetation-brick-visibility.md),
with shared support regions and a separate GPU visibility stage. The approved r4
source is unchanged; historical baseline timings below retain their original scope.

Status, 29 October 2026: the first woody species images were **rejected**. The
denser r4 source is now the approved baseline to preserve, particularly the
spruce. An opt-in assembly aggregate LOD fixture runs on that same source. This
is not acceptance of its distant representation, a live forest, or a completed
Nanite-equivalent pipeline.

**The largest rendering gap is still the consumer.** Warp ray-integrates branch
volumes and evaluates the BRDF at occupied samples. Epic documents separately
binned compact voxel clusters, front-to-back depth buckets and a normal
distribution sampled into the GBuffer before shading. Matching its brick size
does not implement that raster/shading pipeline. The experimental
`KERN_VOXEL_GLOBAL_INTERVALS=1` worklist changes interval scheduling inside
Warp's integrator; it does not turn it into Nanite voxel rasterization.
[Epic: Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)

The retained textured forest remains Sideways' default and visual reference.
`SIDEWAYS_TREE_SPECIES=mixed` now fails explicitly during construction: the
former complete-tree flattening route is disabled while the new aggregate and
pose scheduling paths lack forest-scale validation and source-shadow scaling. It must not be
used to expose the new detailed sources in a full stage. Mixed grass remains an
independent, opt-in feature.

## What changed after the rejected first version

V1 used three small meshes: a protected stem, an eight-triangle needle and a
closed twelve-triangle broad leaf. Its six recipes were flattened and merged by
material into complete-tree CLOD meshes. That shared each finished tree across
the forest, but discarded runtime reuse of the individual source parts.

The screenshots exposed inadequate source design as well as distance LOD
limits: oversized individual leaves, regular sparse branch whorls, angular
needles, and insufficient crown mass. The new trees were roughly 8–9 m tall,
where the retained raw tree sources were roughly 14–19 m tall before instance
scaling. Equal placement counts and transforms therefore did not imply equal
screen coverage, shadows or GPU work. Distant hills still contained sparse
brown sticks and dots. The user rejected the result; distinct silhouettes and
passing tests did not establish visual acceptance.

`SimplificationPolicy::ThinFoliage` added cumulative projected-area protection
to the existing orientation/UV constraints. It improves some intermediate-size
cuts, but cannot guarantee coverage for tiny crowns or supply an optical
aggregate. The synthetic disconnected-leaf check kept about 52% of its summed
projected triangle area at a 16 px radius versus about 6.5% under ordinary
textured simplification; at 8 px both still retained about 6.5%. This is a
geometric diagnostic, not a measurement of visible crown opacity or a proven
solution to the distant-forest defect.

## V2 source and assembly architecture

| Layer | Source | Current responsibility |
| --- | --- | --- |
| Detailed components | [`plant_spray.rs`](../../crates/render/src/plant_spray.rs) | Three needle sprays and three broadleaf sprays, each with separate real wood and foliage geometry |
| Woody library/recipes | [`plant_recipe.rs`](../../crates/render/src/plant_recipe.rs) | Fourteen immutable meshes, deterministic placements, branch hierarchy and validated recipe parameters |
| Assembly contract | [`foliage_assembly.rs`](../../crates/render/src/foliage_assembly.rs) | Stable part/branch identities, local transforms, materials and explicit construction limits |
| Shared source cook/attachment | [`vegetation_asset.rs`](../../crates/render/src/vegetation_asset.rs) | `CookedPlant::cook_source`, `SceneVegetation`, geometry/material/CLOD-policy deduplication and accounting |
| Optical aggregate cook | [`vegetation_aggregate.rs`](../../crates/render/src/vegetation_aggregate.rs) | Streamed shared placements, bounded 4³ bricks and sampled directional errors against a finer source-derived medium |
| Aggregate selection | [`aggregate_lod.rs`](../../crates/render/src/aggregate_lod.rs) | GPU per-view source-cut/volume choice and parent suppression; opt-in sampled-optics policy |
| GPU branch poses | [`part_wind.rs`](../../crates/render/src/part_wind.rs) | Parent-first rigid branch poses, current/previous state and shared primitive attachments |
| Sideways catalogue | [`plant_species.rs`](../../../games/sideways/app/src/plant_species.rs) | Six named recipes, colours, size/layout choices, seed and metadata |
| Live woody gate | [`plant_population.rs`](../../../games/sideways/app/src/plant_population.rs) | Explicitly rejects the obsolete flattened mixed-forest route |

The fourteen-mesh library contains a protected trunk stem, three needle
wood/foliage pairs, three broadleaf wood/foliage pairs, and a tapered segment for
upper trunks and branch axes. A spray is detailed geometry with small needles
or leaves attached to modeled twigs. It is not one enlarged leaf standing in
for a branch and is not an alpha-masked card. Recipe changes arrange and scale
those shared parts; they do not rewrite the mesh bytes for every species.

| Catalogue ID | Intended form |
| --- | --- |
| `spruce` | Layered conical needle tree |
| `pine` | Open umbrella needle tree |
| `birch` | Columnar broadleaf tree |
| `broadleaf` | Rounded broadleaf tree |
| `juniper` | Spreading needle shrub |
| `shrub` | Rounded leafy shrub |

Names describe authored forms, not botanical scans. The r4 sources have been
reviewed as the baseline; later edits require renewed image review. Exact dimensions,
triangles and bytes must come from the current capture's report, because these
sources are being iterated. Do not reuse V1 counts as V2 results.

The current authoring budget is fourteen library parts, 64 branch groups and
8,192 placements per assembly. Separate arithmetic limits guard optional
expanded source callers: 32,000,000 vertices, 120,000,000 indices and 1,536 MiB.
Those limits are not a statement of actual allocation in `cook_source`.
The tree recipes retain the protected collision trunk at radius 0.22 m through
the local -0.5..5.5 m interval; shrubs have no protected collision trunk.
`BuildOptions` growth and season are static cook parameters, not a per-frame
biological simulation.

### Source-only execution

```rust,ignore
let assembly = plant_species::build(candidate_id, seed)?; // Game catalogue.
let cooked = CookedPlant::cook_source(&assembly)?;
let cook_stats = cooked.stats();
let mut vegetation = SceneVegetation::new(&mut scene, limits)?;
let handle = vegetation.append(&cooked, PlantInstance {
    id: stable_world_id,
    generation: asset_revision,
    root: plant_to_world,
    category: GeometryCategory::Trees,
})?;
let attachment_report = vegetation.finish_source()?;
```

`cook_source` clusters unique geometry/material combinations and retains compact
placements and branch descriptors. It does not flatten the complete tree or
invoke the voxel baker. Source-only `finish_source` adds no voxel assets,
representation groups or zero-cluster volume proxies. The separate optical
`cook(..., BakeConfig)` / `finish(Policy)` route still exists for bounded
experiments; it does not automatically become active in these previews.

Each source placement references a shared mesh and is attached to a branch pose.
The GPU evaluates the hierarchy and scatters its transforms to the source
instances. Candidate instances use `PART_WIND`; they do not additionally receive
the retained forest's `FOLIAGE` whole-tree shear. This is rigid branch animation,
not weighted leaf skinning or botanical dynamics. A single library part may be
reused many times, but each current placement still consumes a scene instance
and attachment entry. This is a real shared source path, not yet a lightweight
forest-wide hierarchy of assemblies.

The legacy admission defaults remain 256 branch poses and 65,536 attachments,
with 64 poses per assembly. The new runtime allocates buffers to submitted
counts under explicit `RuntimeLimits`: at most 262,144 poses, 1,048,576
attachments and 512 MiB of internal animation buffers, additionally constrained
by device binding limits. `AttachLimits.max_poses` and primitive-instance limits
must be raised explicitly; those maxima are not a validated forest size.
`Scene.part_wind_lod` enables GPU projected-motion scheduling across all supplied
player and dynamic shadow consumers. Assemblies can settle to rest and sleep;
active attachments are dispatched through pages of at most 256 entries.
Wake-up resets previous-pose history. This reduces scheduled work, but does not
stream instance/pose residency or allocate only visible parent chains.

`CookStats` and `AttachmentReport` distinguish shared source/resident geometry,
placements, poses and volume data. `expanded_source_*` is an arithmetic
counterfactual: what copied source geometry would cost, not memory allocated by
the source-only preview. Shared geometry bytes include resident CLOD data but
exclude instance, attachment and pose buffers. Scene sharing requires equal
geometry bytes, material bytes and concrete CLOD build policy (`ClosedSweep`,
`TexturedGeometry` or `ThinFoliage`). Identical source geometry requested through
different fallback policies must not accidentally share a differently cooked mesh.

### Opt-in aggregate execution

`CookedPlant::cook_aggregate(&assembly, vegetation_aggregate::Config)` keeps the
shared source and cooks branch-coherent volume candidates. A transformed
primitive is processed at a time; the complete expanded tree is not allocated.
The current hierarchy also has a static whole-plant aggregate with an explicit
relative-motion error allowance. `finish_aggregate(aggregate_lod::Policy)`
attaches this experimental selector; `allow_sampled_optics` defaults to false.

Each candidate level is limited to **128 bricks of 4×4×4 cells**. Packed levels
use brick-directory indirection and existing sparse optical cells, including
directional extinction and weighted normal lobes. The limit is enforced data
layout, not evidence that this is an optimal budget. The cooker compares volume
and triangle-cut optics against the same finer source-derived medium along
sampled directions. These are optical-model errors, not exact rendered triangle
coverage, a Hausdorff bound, or a guarantee under all lighting.

GPU selection combines projected spatial error with the sampled opacity and
colour thresholds, retains full source when no candidate passes, and suppresses
children when a containing aggregate is selected. A chosen aggregate uses its
exact measured mip, rather than treating that mip as a loose ceiling. Optional
finer source-cut candidates now run in the fixture. Their measured geometry
reduction does not establish optical or temporal acceptance; the results below
separate their two reference-grid configurations.

The volume pass projects each selected volume once into screen-tile bins, sorts
nonoverflow candidate IDs deterministically, and keeps exact per-ray entry
ordering. Tile, interval and active-cache overflow retains a complete-scan
fallback. This is separate voxel scheduling, not Epic-style cluster depth
buckets, compact voxel-cluster rasterization into a GBuffer, or a general
streaming hierarchy. Normal-lobe BRDF evaluation remains inside volume
integration. **Shadows still rasterize source
triangles**, including when camera colour selects a volume.

### Aggregate smoke evidence, not a performance result

[`vegetation_forest_lod.rs`](../../../games/sideways/app/examples/vegetation_forest_lod.rs)
uses one scene and renderer for source/auto, with identical source, poses,
camera, light and material. Source mode forces full source CLOD. The fixture
separates warmup and image/debug readbacks from asynchronous timestamp windows,
and records image differences plus native opaque-tree coverage unioned with
fractional volume alpha. Colour thresholds are not substituted for coverage.

`C:/dev/perf/warp-sideways-20261028/aggregate-lod/smoke-2/report.json` records one
spruce, one native 1920×1080 view, TAA off, two warmup and two measured frames
per case on RTX 5070 Ti/Vulkan. **Unrelated background GPU activity confounded
these timestamps. They are not isolated steady-state timings and establish no
performance win.** The fixture is offscreen, which does not isolate GPU use;
separate idle observations still showed roughly 40% competing GPU activity.
The case precedes later source-cut refinements and the global-interval option.

| Distance | Source/auto colour triangles, including ground | Auto decision | Captured result |
| --- | ---: | --- | --- |
| 50, 150, 400 m | 32,099,798 / 32,099,798 | All 61 groups stay source at error zero | RGB and native opacity match exactly |
| 1,000 m | 32,099,798 / 458 | 60 branch volumes at mip 0; one source group | Coverage integral 79.266 versus 68.000; fuzzy IoU 0.630; not optically accepted |

The 1,000 m image contains only a small crown; whole-image error would dilute
its difference in the background. The native opacity L1 divided by source
coverage is 0.492, and the tight crop's display-linear RGB MAE is 0.0266. These
diagnostics do not turn sampled cooker thresholds into a rendered-quality pass.
Both modes report 14,619,068 shadow triangles in the retained shadow maps.
The shared asset has 5,169 placements, 61 poses, 1,375,680 geometry bytes and
9,335,908 packed volume bytes. The 1,022,770,992 expanded-source bytes are only
an arithmetic counterfactual; instance, animation, texture and scratch memory
are additional.

Start further checks with one source/auto tree and a short smoke window, then
measure a longer isolated window. Four full-source trees are expensive; a larger
auto-only forest still needs shadow-capacity preflight. `MAX_VISIBLE` is
4,194,304 clusters per colour/light pass. Truncated or overflowed work must fail
validation, never become a speedup. The fixture's four-view path is implemented
but this single-view report does not validate it. No DLSS transition, moving
camera, multi-plant forest, or live Sideways acceptance follows.

### Latest aggregate experiments: neither promoted

`aggregate-lod/fine-global-1/report.json` uses a 0.025 m reference grid,
`triangle_refinements: true`, the global interval stream and TAA. The unchanged
0.08 optical policy rejects all alternatives at 50/150/400/1,000 m: every case
retains all 32,099,798 source triangles including ground. Preparation took
139.47 s and packed volume payload grew to 23,388,756 bytes (23.39 MB), without
reducing selected work. This configuration is rejected. Its source/auto captures
also used different TAA sampling phases. Their image and opacity differences
are **not LOD-quality evidence**; `reset_capture_sampling()` was added afterwards.

`aggregate-lod/global-reference-1/report.json` restores the 0.075 m reference
grid with refinements and global intervals, and uses the corrected capture
sampling reset. Both modes accumulate 32 frames at the same fixed wind pose
with TAA enabled. These are stationary comparisons, not moving TAA or DLSS
transition validation. The source shape, seed and shared parts are unchanged.

| Distance | Source/auto camera triangles, including ground | Selection and native opacity |
| --- | ---: | --- |
| 50 m | 32,099,798 / 22,260,566 | Source cuts up to 0.009375 m; no volumes. Coverage 30,561 versus 31,649 (−3.44%); fuzzy IoU 0.966. |
| 1,000 m | 32,099,798 / 458 | 60 mip-0 volumes and one source group. Coverage 79.785 versus 71.000 (+12.37%); fuzzy IoU 0.656. |

These opacity measurements are the raw opaque/volume union before TAA, not the
accumulated output silhouette. Shadow maps retain 14,619,068 source triangles
in both modes. No tile, interval-stream, ray-cache or step overflow was reported
in the far capture. Only 272 rays were traced, but they performed 82,942 volume
samples: reducing camera triangles has not bounded aggregate shading work.

The eight-sample far window reports a median **14.56 ms for volume integration
alone**, versus **12.90 ms for the entire measured full-source GPU interval**;
auto's covered interval is 20.27 ms. Other GPU work remained uncontrolled, so
these are contended diagnostic observations, not isolated hardware costs or a
causal speedup estimate. The interval stream has demonstrated no performance
win and stays unpromoted. Packed volumes are 9,335,908 bytes in this run;
preparation took 67.57 s. Neither changing the source nor loosening the optical
gate is justified by these timings.

Review the [matched source/auto crops](C:/dev/perf/warp-sideways-20261028/aggregate-lod/global-reference-1/source-auto-review.png).

Two shading diagnostics preserve the opacity PNG byte-for-byte (SHA-256
`356b56fcf3a4cb5650e7d303328b819445319a6338ae0910779821fb1c9bef1c`) and every
reported traversal counter. `global-cheap-brdf-1` measures 13.44 ms integration;
`global-density-1` measures 15.25 ms, against 14.56 ms for full shading. Each has
eight samples with uncontrolled competing GPU work. Neither establishes a
material/lighting speed win or an accepted replacement; these are diagnostic
shader variants, not equivalent rendered appearance.

The larger native fixtures now execute, but remain far from usable performance:

| Report under `aggregate-lod` | Configuration | Camera triangles | Source shadow triangles | Contended median GPU observations |
| --- | --- | ---: | ---: | --- |
| `four-tree-four-view-1` | Four trees, four 960×540 views in one 1920×1080 output, TAA off, four samples | Source 513,596,744; auto 7,304, summed across views | 226,596,066 in both modes | Source covered interval 138.40 ms; auto 145.15 ms, including 101.30 ms integration |
| `forest-64-guard-1` | 64 trees, one 1920×1080 view, auto only, TAA off, two samples | 29,186; 3,840 volume groups and 64 source groups | 177,593,316 | Auto covered interval 135.14 ms; integration 98.73 ms |

**Shadow focus is deliberately pinned to the forest for both near and main
maps**, unlike the camera-relative gameplay setup. These are source-shadow
stress counts and offscreen execution diagnostics, not game FPS. The 64-tree
case uses a complete fallback for 25 overflowing tile candidate lists; step,
interval-pool and active-cache overflow stay zero. Its 5,088 rays perform
4,747,443 volume samples and 42,401,827 AABB tests. Completion within allocation
guards does not establish scalable rendering. Keep both triangle and volume
work visible in the report instead of presenting the camera triangle reduction
as a speedup.

The next architectural milestone follows Epic's documented boundary: compact
**spatial voxel clusters**, separate visibility/material resolve, then shading.
Further branch-integrator polishing is not the next milestone. Keep the current
path as a diagnostic reference while comparing cluster work, coverage, material
response and light-view costs against it.

`taa-null-2` forces the finest source in both modes with `--error 0.01`.
Native opacity matches exactly: coverage 31,649 in both, fuzzy IoU 1 and L1 0.
Only 24 of 2,073,600 output pixels differ in RGB (full-view MAE 0.00000916 on
the 0–255 scale). This validates corrected capture-phase parity for opacity,
not bit-exact RGB or a moving temporal transition.

Validation logs in the same `aggregate-lod` directory record 672 passing render
tests (15 ignored), one separately executed GPU selector hierarchy/cut test,
189 passing Sideways tests (one ignored), workspace all-target checking and
developer-launcher contracts. These establish those code contracts, not optical,
performance, forest-scale or DLSS acceptance. See `render-tests-5.log`,
`selector-gpu-2.log`, `cli-tests.log`, `workspace-check.log` and
`launcher-tests.log`.

## Actual GPU source previews

The examples use the native Warp renderer. Image composition only downsamples
complete GPU captures; it does not generate, repaint or replace the plants.
Successful compilation is not evidence that a screenshot was rendered, and a
rendered screenshot is not visual acceptance.

[`plant_spray_gallery.rs`](../../../games/sideways/app/examples/plant_spray_gallery.rs)
shows three variants of each component family as ordinary source mesh instances.
It writes six native 1280×1080 PNGs and `component-gallery.png` at 1920×1080:
needle variants 0–2 above, broadleaf variants 0–2 below. A common camera within
each family preserves relative size. Family cameras can differ. The source
palette comes from the current spruce/broadleaf recipes; LOD is forced to zero
and the components remain in their rest pose.

[`tree_species_compare.rs`](../../../games/sideways/app/examples/tree_species_compare.rs)
uses `CookedPlant::cook_source` for the candidate. The retained Canopy tree at
fullness 1.25 remains a visual reference with its existing textures, normal maps,
alpha-test material and whole-tree wind. Candidate foliage keeps the opaque
assembly palette and branch animation. This compares complete source
appearances; it is not an isolated material, shader or LOD experiment.

Reference and candidate use separate scenes/renderers, identical union-bounds
cameras, light, ground and wind field. This avoids an inactive plant contributing
shadows, and avoids CPU updates to instances owned by the attachment pass.
Close views use `lod_pixels=0`; far views use `lod_pixels=1` at eight times the
close camera distance. The native images are 1920×1080. `comparison-close.png`
and `comparison-far.png` place the reference left and candidate right;
`comparison.png` stacks those views. `detail-candidate.png` fits only the
candidate and reports its different camera scale, allowing shrubs to be
inspected without pretending that they match a reference tree's height.
`far-source-candidate.png` additionally retains source CLOD at the identical far
camera, isolating simplification loss from subpixel rendering loss.

`--time` sets deterministic wind time in seconds, default 4.25. Separate runs
at different times can expose animation defects. Two different still frames
alone do not establish smooth motion, stable shadows or valid motion vectors.
`--taa 0` is the default raw diagnostic, with two warmup frames. `--taa 1` uses
32 warmup frames after resetting history, with the camera and wind pose fixed
throughout. Both comparison sides use the same setting. Static TAA accumulation
can reduce edge aliasing; it does not establish temporal acceptance under moving
cameras or wind. Each capture reports TAA, warmup and any available GPU timestamp
scopes for that single frame with forced shadow redraw. Those scopes are not a
steady-state benchmark or gameplay FPS.

Run these commands **serially, with no simultaneous GPU game or fixture**, from
the repository root. Choose fresh output directories; the commands below are
reproduction instructions, not a claim that this exact run has completed:

```powershell
$gameRoot = Join-Path (Get-Location) 'games/sideways'
$evidenceRoot = 'C:/dev/perf/species-v2-source-review-new'
cargo build --locked --release -p cli --example plant_spray_gallery --example tree_species_compare
cargo run --locked --release -p cli --example plant_spray_gallery -- --root $gameRoot --out "$evidenceRoot/components"
foreach ($species in @('spruce', 'pine', 'juniper', 'birch', 'broadleaf', 'shrub')) {
    cargo run --locked --release -p cli --example tree_species_compare -- --root $gameRoot --out "$evidenceRoot/$species" --candidate $species --time 4.25 --taa 1
}
cargo run --locked --release -p cli --example tree_species_compare -- --root $gameRoot --out "$evidenceRoot/spruce-time-5" --candidate spruce --time 5.0 --taa 1
foreach ($species in @('spruce', 'shrub')) {
    cargo run --locked --release -p cli --example tree_species_compare -- --root $gameRoot --out "$evidenceRoot/$species-raw" --candidate $species --time 4.25 --taa 0
}
```

Keep `report.json` beside every image. Record the built revision/binary hash,
GPU/backend, source bounds, source and resident bytes, placements/poses, cameras,
wind time, lighting and submitted triangles. Counts include ground where noted.
The report has no FPS claim. `plant_species_preview` is now a retired entrypoint
that exits with an error and directs callers to the examples above. Its
historical flattened V1 images remain evidence of that rejected version; the
old whole-tree viewer cannot be used to render V2 detailed sources.

## Comparison with Epic's documented architecture

Epic describes assemblies, voxels and skinning as cooperating systems. Its
voxel builder can prefer a voxel representation when it has lower simplification
error than triangles, with at most 128 bricks of 4×4×4 cells per cluster. Voxel
clusters use separate raster bins and front-to-back depth buckets. Its documented
GBuffer path samples the normal distribution before shading; the Witcher demo's
direct shading of that distribution is a separately described implementation.
The system also disables animation below a screen-size
threshold. Alpha masking or detailed triangles alone do not supply this
pipeline. [Epic: Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)

Epic's assemblies put repeated part clusters into the containing hierarchy
without duplicating the source geometry. Coarser assembly levels combine parts,
so a distant assembly can reduce to a single cluster instead of retaining a
root cluster for every twig. [Epic: Nanite Assemblies](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-assemblies)

| Concern | Current Warp preview | Missing for the intended forest |
| --- | --- | --- |
| Detailed source reuse | Approved r4 shared sprays; branch aggregates and one coarse subtree candidate | Deeper aggregate hierarchy, streaming and forest validation |
| Distance representation | GPU per-view triangle/voxel choice; refined source cuts measured at 50 m | Accepted optical/temporal cuts and useful trade-offs across ordinary viewing distances |
| Voxel rendering | Enforced 128×4³ brick levels; per-volume tile bins, ray integration and per-sample BRDF | Compact voxel-cluster raster bins/depth buckets, GBuffer integration, bounded world-scale work and accepted transitions |
| Animation | Dynamic buffers, all-consumer GPU motion LOD and indirect attachment pages | Visible-chain residency allocation, weighted skinning and transition/history proof |
| Memory/work | Shared geometry and aggregate payload accounting; source instances still scale with placements | Forest residency, light-view costs and overflow budgets |
| Evidence | Single-tree source/auto fixtures with corrected TAA phase and raw opacity; r4 source gallery | Isolated timings, moving camera, shadows, DLSS and four-player forest acceptance |

The new aggregate path now uses that brick ceiling and compares candidate
triangle/voxel errors. The older `foliage_representation` planner still uses a
footprint bound and external optical gate; its evidence is separate. Neither
path is equivalent to Epic's full system. In particular, the current optical
oracle is a finer source-derived medium, source-shadow work remains, and tile
binning plus per-ray sorting is not a cluster depth-bucket raster pipeline.
The global-interval option remains an optimization of this ray integrator,
not delivery of that missing renderer. For each new problem or proposed fix,
record how Epic's documented pipeline addresses it, which mechanism Warp
actually implements, and the remaining difference before claiming equivalence.
Epic's demo timing and memory examples describe its content and hardware; they
are not estimates or acceptance thresholds for Warp.

### Ordered next steps

1. **Preserve the approved r4 source.** Keep its recipes, shared components and
   reports together in a verified rescue snapshot before further source edits.
   New species edits still need common-camera and component review.
2. **Implement the missing voxel renderer boundary.** Following Epic's compact
   voxel-cluster model, build spatially compact clusters and separate visibility/
   material resolve from shading. Compare against the current branch integrator
   using unchanged source and optical gates. Keep full source as the oracle;
   measure visited clusters/bricks, samples, coverage, holes and material response.
   More worklist or BRDF polishing of branch volumes is not this milestone.
3. **Make hierarchy selection and residency bounded at world scale.** Traverse
   visible assembly nodes on the GPU, account for unique resident pages and
   selected work, and specify overflow/source-retention behavior. Avoid an
   always-resident scene instance and visible root cluster for every twig in
   every distant tree. Preserve stable plant identities across streaming.
4. **Bound colour and light-view work together.** Extend the compact cluster
   path to depth-ordered raster work and light consumers; address source shadows.
   Carry material statistics, normals, coverage, motion and lighting through the
   same selection. Validate shadows and the transition under motion; do not
   assume separately correct colour and shadow passes compose correctly.
5. **Validate motion scheduling, then residency.** Exercise all player and
   shadow consumers, rest/wake transitions and camera cuts with the dynamic
   pose buffers. Validate previous poses and cache invalidation before adding
   visible-chain residency allocation or streaming. Existing indirect pages
   reduce attachment work; they do not remove resident source instances.
6. **Measure and accept the integrated forest before enabling it.** Match stage,
   source sizes, cameras, settings, wind and resolution. Check pop-in, distant
   opacity/colour, grain, motion and occlusion alongside CPU/GPU timing, memory
   and selected work. Include native and temporal-upscaled output, then four
   views. The target remains one 4K output, four players and 120 FPS; these source
   fixtures establish none of those performance or presentation results.

## Grass species remain a separate bounded feature

`SIDEWAYS_MEADOW_SPECIES=mixed` selects three recipes in the existing windswept
GPU field. The CPU cooks three small source recipes and uploads terrain/settings
at construction. World roots, habitat/species selection, visibility, LOD, wind
and rendering use the GPU field. There is one source pool, the existing 12-word
plant payload and three draw lists, not three full-radius vegetation layers.

| Species ID | Height multiplier | Width multiplier | Linear base-and-tip tint |
| --- | --- | --- | --- |
| `short_grass` | 0.50 | 0.90 | 0.98, 1.04, 0.96 |
| `broad_grass` | 0.70 | 1.40 | 0.87, 0.96, 0.94 |
| `tall_grass` | 1.15 | 1.00 | 1.06, 1.02, 0.94 |

[`WindsweptRecipe`](../../crates/render/src/meadow_windswept.rs) validates dimensions
and tint. [`cook_windswept_recipes`](../../crates/render/src/meadow_data.rs) takes
exactly three recipes. Sideways uses coherent 12 m world patches; the ABI is not
an unbounded species registry. The source retains its authored roots, 6/3/1
segment ordering, existing six-to-one morph, blade budget and shared wind. The
three-segment cut remains cooked but unused by appearance LOD. Tint multiplies
base and tip together; arbitrary tip colour and per-species wind stiffness are
not supported by this recipe contract.

Mixed geometry recomputes conservative wind bounds and contact area. With the
optional native spatial filter, distinct widths use the narrowest basal blade
for the shared threshold. Neither filtering nor contact is automatically enabled
by the species flag. Grass still has subpixel aliasing and its existing source
range/outer fade; mixed species do not establish an accepted far-volume solution.

For an opt-in grass playtest with the retained reference forest:

```powershell
$env:SIDEWAYS_PROCEDURAL_GRASS = 'field'
$env:SIDEWAYS_MEADOW_PROFILE = 'windswept'
$env:SIDEWAYS_MEADOW_SPECIES = 'mixed'
$env:SIDEWAYS_TREE_SPECIES = 'reference'
$env:KERN_MEADOW_SUBSTRATE = '0'
games/sideways/tools/play-dev-windows.ps1 -Views 1 -Size 3840x2160 -Upscale native -MeadowAggregate:$false -Restart
```

Unset/`off` meadow species retains reference grass; unset/`reference` tree
species retains existing trees. Mixed grass requires the windswept profile.
The launch above disables the older average-colour substrate; that substrate
does not model species patches. Check resolved capture metadata, including
species mode and patch size, rather than relying on shell variables.

## Evidence status and historical baseline

The V2 r4 captures were generated and visually inspected on 29 October 2026,
at `C:/dev/perf/warp-sideways-20261028/species-v2`. All six candidates rendered
beside the retained reference at identical camera and metre scale. The native
captures are 1920x1080 on an NVIDIA RTX 5070 Ti, Vulkan, driver 617.14. The
`all-species-reference-r4.png` montage uses the same central crop for both
members of every pair; `all-species-detail-r4.png` fits each candidate separately.
`components-r3/component-gallery.png` shows the shared component geometry,
which is unchanged in r4. The r4 change increases placements at fixed part size.

| Species | Placed source triangles | Shared resident geometry MiB | Counterfactual expanded source MiB | Placements | Poses |
| --- | ---: | ---: | ---: | ---: | ---: |
| spruce | 32,099,796 | 1.312 | 975.39 | 5,169 | 61 |
| pine | 29,795,060 | 1.312 | 905.38 | 4,809 | 61 |
| birch | 4,171,770 | 0.283 | 120.02 | 3,009 | 61 |
| broadleaf | 4,521,216 | 0.283 | 130.03 | 3,257 | 57 |
| juniper | 9,116,558 | 1.312 | 277.13 | 1,529 | 41 |
| shrub | 695,136 | 0.283 | 20.14 | 569 | 41 |

These source counts exclude the ground and do not describe the selected runtime
triangle count. Shared bytes include CLOD geometry but exclude instance,
attachment and pose buffers; expanded bytes are arithmetic only, not an actual
allocation. These single-plant source counts are not a validated forest budget.
The per-capture reports preserve the actual submitted colour/shadow triangles
and single-frame timing scopes. No steady-state speedup or frame-rate claim is
made from these fixtures.

The r4 source crowns are the accepted baseline to preserve; native subpixel
grain and distant LOD remain unresolved. At normal 1 px far CLOD, the spruce loses much of its
crown. `spruce-r4-lod-oracle` isolates that defect: with the same distant camera
(4.681 pixels/metre at target), pose, light and TAA, source CLOD submits
32,099,798 triangles including ground and retains a green crown. Normal 1 px
CLOD submits 189,998 and loses most foliage. The identical crop/enlargement in
`spruce-r4-far-lod-diagnostic.png` makes the difference inspectable. This proves
substantial loss from simplification for this view; it does not validate a voxel
replacement or establish a general optical error metric.
Source reuse does not fix that distance representation. The six TAA
captures accumulate 32 stationary frames; the extra `spruce-r4-raw` run exposes
native aliasing. `spruce-r4-wind` at time 6.5 differs from time 4.25 in 117,236
detail-image pixels, establishing a pose response, not smooth motion or valid
motion vectors. Source approval does not approve the far CLOD or aggregate transition.

Full render-library tests passed 649 with 13 ignored; full Sideways binary tests
passed 189 with one ignored before the final density-only r4 revision. After r4,
the three focused recipe tests, the game species catalogue test and the workspace
all-target check passed. The launcher contract test also passed; no interactive
playtest was started. Logs retain scope and exclusions. The complete GPU meadow
fixtures were not rerun for this source-only tree revision.

The working files remain at `C:/dev/warp-sideways-plant-species-20261028`.
Its original common Git directory disappeared during the task. No commit, push
or merge to main has been made. Recovery evidence contains the tracked patch,
new files and hashes; do not relink or reset this checkout blindly.

Retain both accepted and rejected iterations with their report/binary provenance;
do not overwrite their meaning.

The following results are **historical V1**, recorded on 29 October 2026 under
`C:/dev/perf/warp-sideways-20261028/species-v1`. They do not validate later spray
geometry, source-only attachment or forest performance:

- Render library: 638 passed, 13 ignored; Sideways binary: 189 passed, one ignored.
- Workspace/all-target check and launcher contract tests passed.
- Four capture construction-guard tests passed.
- Serial GPU grass selector oracle checked 65,536 cells: species counts
  19,605/23,008/22,923 and 4,621 neighbour changes out of 130,560, versus 86,870
  for independent reference hashes. This checks selection, not image quality.
- GPU contact oracle passed classic, reference windswept and mixed windswept
  sources; other ignored fixtures were not run.

`reference-visible/` and `mixed-final-visible/` completed the same 216-frame
scenario at **1920×1080 native, one view**, seed 1, clear noon, wind 0.8/time 4.25,
blade budget 120 and tree fullness 1.25. They enabled the existing contact/filter
experiments and disabled aggregate/substrate. Each section excludes 24 warmup
frames and capture readback. The final section has 45 matched GPU samples
because the last three asynchronous records were unavailable at shutdown.

| Section | Samples per run | Reference GPU median / p95 ms | Rejected V1 mixed GPU median / p95 ms |
| --- | --- | --- | --- |
| `forest-close` | 48 | 18.368 / 23.713 | 22.358 / 28.385 |
| `grass-and-road` | 48 | 16.960 / 23.945 | 16.550 / 20.313 |
| `distant-hillside` | 45 | 23.911 / 27.840 | 17.498 / 21.554 |

These are covered GPU intervals, not pure grass costs or an additive pass sum.
The smaller V1 trees changed occlusion and visible work; this pair establishes
neither a general speedup nor constant overhead. The forest-close candidate was
about 4 ms slower. It does not test the 4K/four-player/120 FPS target.

`captures/frame-000024.jpg` is the useful forest/horizon comparison; frames 96
and 168 show the grass/road and the route among trees. `catalogue-final/` forced
source CLOD and did not validate distant geometry. Its two wind poses differed
in 71,971 pixels, demonstrating a response rather than smooth animation.
`budget/report.html`, `budget/summary.json`, `run.json` and
`matched-timings.json` preserve measurements. Both stage runs used
`sideways-species-final.exe`, SHA-256
`8fb393cf86b3a6b0371d9eb2283b21b2fe2808a8e4d21a79133f5450a5a733ef`.

The [windswept source](meadow-windswept-r63.md),
[contact/filter](meadow-edge-grain-r64.md) and
[bounded shared-vegetation prototype](unified-vegetation.md) documents describe
their own recorded states and limits. They do not imply acceptance of V2 distance rendering or
approval of the rejected V1 woody visuals.
