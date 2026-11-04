# Stable GPU forest population updates

The normal terrain route now streams loaded roots without rebuilding Renderer.
This implements population identity/lifecycle from vegetation-architecture.md;
it does not complete history-first selection, asset paging or animated shadow
page caching. There is no alternative backend switch.

## Contract

`plant_population` installs the shared species catalog and reserves root capacity
separately from branch output slots. `WorldRoot.id` comes from deterministic world
placement, not distance ordering. Unchanged roots keep slots and generations.
Removed roots become inactive; reused/moved/species-changed roots increment their
generation. GPU branch residency and root motion epochs consume that generation.
Empty populations can recover without rebuilding pipelines or geometry catalogs.

`Renderer::gpu_forest_population_seed` cheaply snapshots immutable metadata.
Its `prepare` operation runs on a CPU worker and validates/plans the population
delta. `commit_gpu_forest_roots` rejects stale transactions before publication,
updates renderer bookkeeping and submits a bounded GPU scatter upload. Each
record contains selector root, voxel proxy and source/canopy owner data. Only
changed records are uploaded; chunks contain at most 16,384 records. Submission
order puts publication before selection/materialization in the following frame.
The CPU does not read GPU counts or choose detailed trees.

The demo separately recenters forest population at 300 m, meadow ground at 150 m,
and baked terrain-sun coverage at 4 km. Preparing world data is background work;
changing world configuration still deliberately rebuilds the scene.

## Validation and measurement

GPU regression covers reordered snapshots, replacement retaining other roots'
branch slots, capacity rejection, empty population, re-addition and a stale
transaction prepared on another thread. CPU tests cover stable identity,
generation changes, duplicate IDs and invalid input.

Headless traversal fixture: alpine seed 1, initial position [0,691,0], yaw/pitch
zero, 62,274 initial roots, capacity 124,548, 4,096 branch slots, one native
1713x964 view, 300 frames at 240 m/s. This deliberately crosses three population
boundaries. Artifacts are in `build/foliage-stream`; no interactive game opened.

The first synchronous implementation spent 36–61 ms publishing a population.
Moving planning to a worker alone still left 18–40 ms on the render thread:
thousands of small sparse uploads remained. The final implementation batches
those records and scatters them on GPU. Final measurements are recorded in the
[evidence JSON](evidence/foliage-population-2026-11-04.json). Three final
updates cost 5.20, 4.19 and 4.36 ms CPU including renderer metadata/publication.
These are three streaming events, not three paired benchmark runs. The final
GPU-covered median was 10.113 ms; selector/residency/pose timings and every pass
are retained in the JSON. All 109 foliage-filter tests passed. Final images were
inspected: scene remains visible; the worker/scatter captures are not pixel
identical (RGB MAE 2.44/255), so no zero-visual-delta claim is made.

An additional 300-frame no-tree traversal at 2,000 m/s, 640x360, crossed
4 km coverage boundaries and completed without errors or renderer rebuilding.
There is no per-recenter event counter in that capture; it is a lifecycle smoke
test, not visual validation of distant shadow coverage.

## Limits

- Root capacity is reserved at construction. Exceeding it returns an explicit
  error; there is no automatic resize or asset page eviction yet.
- Population planning is a loaded-world snapshot, not yet incremental cell
  commands. Canopy metadata still recomputes its mass/height budget at publication.
- GPU scatter publication is a separate submission. Its GPU time is not included
  in the ordinary frame's covered-pass interval. CPU enqueue time is not GPU time.
- The fast moving route performs terrain work and synchronous image readbacks.
  Its wall percentiles are not gameplay FPS. The HUD's 60 FPS reflects simulated
  ticks in this headless path, not measured presentation.
- A single view here does not validate 4K output/four players or the 8.33 ms target.
- Existing callers using compatibility vegetation APIs are not silently migrated.

Next architecture gate: history-first traversal with current-frame recovery and
safe publication for camera and shadow consumers, followed by asset paging.
