# GPU foliage work queues

The production `Selector::union` now traverses refining subtrees through two
bounded GPU queues. This replaces the serial per-root demand and emission walks;
it is not an environment-controlled alternative renderer.

CPU work is limited to cooking immutable child spans and submitting a fixed
number of levels from the shared recipe depth. All active task counts, indirect
dispatches, frustum/error decisions, complete-cut demand and publication remain
on the GPU. No per-frame population readback or CPU near-tree selection exists.

## Task granularity and memory

The initial GPU pass handles culled and coarse roots directly. A queued item is
`(root ID, refining subtree ID)`. One workgroup processes its immediate children
in parallel, consulting a shared per-asset child-span table. Leaves emit/count
directly; only refining internal children enter the next frontier. This distributes
large trees across workgroups without storing every selected needle in a queue.

Each task is eight bytes. Two fixed allocations hold the frontiers. Default
capacity is max(8 * roots, 4 * record capacity, 4096), capped at 1,048,576 unless
more root seeds are required. Device binding and dispatch limits are checked.
The child-span table costs less than 12 bytes per shared node, independent of
world population. Dispatch boundaries publish tasks; no persistent-kernel spin
loop assumes GPU workgroup scheduling guarantees.

If a frontier overflows, the affected whole tree receives fallback flag 3 and
its partial records must not publish. The existing root proxy stays visible.
Slot-budget rejection remains flag 2, selected deterministically by projected
priority and root ID. Emergency queue-overflow victim order can vary with GPU
scheduling; normal operation must avoid this overload. These are separately
observable through `frontier_overflow_requests` and `peak_frontier_requests`.
Peak counts refer to requested child frontiers, not initial root seeds.

The former per-node frontier design was rejected: the 62,274-root route demanded
857,999 items and dropped 359,807 requests at the smaller allocated capacity.
Its lower GPU time was caused by lost detail and is not a valid improvement.
Raw failed-run evidence is retained as `build/foliage-queues/after-0.*`.

## Correctness checks

- GPU cuts equal the serial reference on a multi-level 193-root scene.
- Deliberately undersized queue, coarse recovery frame, then detailed frame:
  no stale fallback flags, partial published trees or duplicated root draws.
- Deterministic near-first complete-cut budgeting across 1,025 roots.
- Two-dimensional indirect dispatch across 32,769 refining roots: every root
  publishes its two leaves, including the second dispatch row.
- Existing historical-depth disocclusion and output/deferred overflow tests.
- Actual renderer: source/voxel selection, overflow, teleport, four cameras and
  off-camera shadow consumers.

History-first traversal is still a separate selector fixture; the shared runtime
union must not use one camera's HZB to reject another consumer's casters. Incremental
cell updates, asset paging, animated shadow caches and shared meadow integration
remain open. This change completes the work-queue portion, not the entire roadmap.

## Integrated measurement

Three repeated runs of the same release executable, alpine seed 1, 62,274 roots,
4,096 branch slots, 2,500 m population radius, 80 moving frames at 15 m/s. One
native 1713x964 view, no DLSS; exclude render sequences below 12 (68 GPU samples
per run). Median of run medians:

- Selection, residency and poses: **0.474 ms**.
- Covered GPU interval: **17.015 ms**. This is not the 8.33-ms target.
- Largest pass: tree occluder raster **5.568 ms**; voxel splats **2.422 ms**.
- Each final frame: 4,090 selected branches, 1,751 source / 2,339 voxel slots,
  4,082 direct roots, 14,447 branch-budget fallback roots. All three runs match.
- Final diagnostic snapshots: zero work-queue, slot or hash failures. Peak child frontier: **3,048** tasks.
- Two frontier buffers: 498,192 tasks each, 7,971,104 bytes total including headers.

[Per-pass medians, counters and executable hash](../evidence/foliage-wavefront-2026-11-04.json).
Raw data and inspected final images: `build/foliage-queues/grouped-{0,1,2}.*`.
The old captured executable predates the removal of CPU root sorting; equal
priority roots therefore differ in budget tie-breaking. Its image is a visual
sanity reference, not an isolated performance or pixel-equivalence baseline.
The multi-level GPU fixture compares identical inputs to prove selection parity.
The actual images retain foreground grain and coarse distant crowns; no visual
polish or temporal-stability claim is made. Headless wall times include capture
and readback, and are not quoted as gameplay FPS.

Validation: six selector tests and three runtime tests pass, including explicit
GPU fixtures. Release build and render/app all-target checking pass.
