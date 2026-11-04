# Shared articulated forest poses

The normal GPU forest route now gathers the ancestry needed by its published
branch cut. A bounded frame-local hash table deduplicates `(root, branch)` pairs.
One root pose is evaluated per active world root; each requested non-root branch
is evaluated once, in cooked parent-before-child depth order. Source instances,
voxel transforms and their shadow consumers use the resulting same world pose.
No CPU visible-tree shortlist or GPU counter readback selects the animation work.

The recipe supplies only depth metadata at initialization. Per-depth GPU queues
have at most the selected-slot budget: each selected slot requests at most one
ancestor at a given depth. The hash capacity is the next power of two above
`2 * branch_slots * max(depth, 1)`. Entries are 64 bytes; root poses 48 bytes each;
queue storage is `4 * (128 + branch_slots * (depth + 1))` bytes. This does not
allocate the full world population times every recipe branch. Device bounds and
32-bit root/branch identity range are checked before resource creation.

Hash probes are bounded. An unavailable entry recomputes that chain from the
current root and wind instead of using an old pose or losing a branch. The cache
is cleared in command order each frame, so streaming generation changes cannot
reuse stale poses. The deliberately undersized-cache GPU test exercises this
fallback. Cache overflow is exceptional repeated work, not a CPU fallback.

## Indirect depth scheduling follow-up

The normal runtime now writes 64 indirect dispatch triples from the completed GPU
pose request counters. Each depth dispatch evaluates `ceil(min(count, slots)/64)`
workgroups instead of dispatching the full slot budget. Zero-demand depths launch
zero groups. All arguments are rewritten every frame, including frames that fall
back to root voxels after an asset fault. No CPU readback controls this work.

A separate preparation pass binds the argument buffer as writable storage; the
following pose pass only consumes it as INDIRECT. This avoids simultaneous
writable-storage/indirect usage of the same buffer. Storage overhead is 768 bytes.
Parent-depth dispatch order, hash-overflow serial fallback and shared transforms
are unchanged. Root preparation still scans reserved capacity; active-root
compaction remains open.

Release compilation and Naga validation of both materialization and dispatch
shaders pass. The numeric GPU oracle now checks every dispatch triple against queue counts and
checks that an asset-fault frame clears prior nonempty dispatches. **These new GPU
assertions and a performance repeat are pending** while other games occupy the
GPU. The measurements below predate this follow-up and do not measure it.

## Validation

A raw matrix GPU oracle compares selected output poses to independent CPU serial
ancestry composition at wind times 0, 17 and 119 seconds, nonzero wind, different
root yaw/scales and a shared two-level ancestor chain. The same test forces a
one-entry hash cache: transforms still match within 2e-5, with reported overflow.
The normal cache evaluates four unique non-root poses for two three-branch trees.

Per-pass GPU timestamps now separate selection/residency, shared pose scheduling
and materialization. Final tests and route measurements follow below. This does
not complete animated shadow-page caching, asset paging, shared grass migration
or consumer-aware upstream history-first publication.

## Measured normal-route result

All 111 foliage-filter tests pass, including streamed root reuse, multiple views,
teleports and the new numeric wind/overflow oracle. Three alternating runs per
unchanged executable, native 1713x964 one view, 80-frame alpine flight at 15 m/s:

| GPU timing (median of run medians) | Before ms | After ms |
| --- | ---: | ---: |
| Selection + residency + poses + materialization | 0.533 | 0.579 |
| Selection/residency alone | not separated | 0.484 |
| Shared poses alone | not separated | 0.038 |
| Materialization alone | not separated | 0.057 |
| Whole timestamp-covered frame | 17.313 | 16.938 |

This is **not a demonstrated speedup**: the forest preparation slightly increases
on this shallow-tree scene, while the small whole-frame difference is dominated
by other passes/run variation. The architectural benefit is bounded deduplicated
ancestry for deeper future assemblies and shared consumers. Final diagnostics:
4,020 requested unique poses, 4,020 evaluated, zero hash failures; the selected
cut and source/voxel counts match baseline. There are 62,274 active roots and
124,548 reserved roots. Current root processing still visits reserved capacity.

Inspected before/after images show no obvious pose discontinuity or missing trees.
RGB MAE excluding the top 180 HUD rows is 0.287-0.292/255; channel p99 difference
is 7. Images are not bit-identical; raw matrix correctness is verified separately.
Raw executables, images, logs and profiles are in `build/foliage-poses`.
[Machine-readable evidence](foliage-shared-poses-2026-11-04-evidence.json) includes
SHA256 hashes and every per-run/pass median. No interactive playtest opened.

Still required: consumer-aware upstream history/publication, actual asset page
streaming, cached animated shadow pages, unified meadow and far cell aggregates.
This shared-pose step does not finish those gates or establish 8.33 ms at 4K with
four players. The full goal remains active.
