# History-first source cluster visibility

The normal GPU forest route now seeds camera depth with source clusters that
contributed actual alpha-tested pixels in the preceding frame. Previously it
used a fixed 32 m owner-distance split. Remaining source clusters are tested
against the current first-pass depth pyramid and recovered in the same frame.
Shadow consumers remain independent of camera history.

History uses a 32 KiB two-hash Bloom payload in a 64 KiB uniform per camera over `(member ID, cluster)`.
It schedules work; it does not assert that stale geometry or stale depth is a
valid occluder. A collision draws extra current geometry first. The complementary
second pass still covers every other current cluster. New/reused slots, teleports,
changed LOD and an empty initial history therefore need no special permission to
recover. The gather runs after actual source visibility/depth tests, with small
workgroup-local hash coalescing to avoid one global atomic per covered pixel.

The first pass appends currently valid but deferred clusters to a bounded GPU
queue. The second pass dispatches that queue indirectly rather than re-traversing
all source owners. A full queue invokes the complete complementary scan instead.
The normal budget is 262,144 records (6 MiB per camera, including current validated bounds), stored after
the visible prefix in the same storage allocation; this retains the 16-storage-binding
device contract. The queue never aliases the published visibility prefix.

This is **not** full history-first assembly traversal. Shared branch demand and
residency still conservatively union cameras and shadows before rasterization.
Source cluster history reduces downstream work without dropping off-camera
casters, but cannot yet avoid upstream assembly traversal/pose work. Voxel
visibility is also still a separate downstream consumer. Those remain explicit
architecture gaps, not completed by this pass.

The first owner-level history version shifted work between passes with little
total benefit. It is retained as measurement evidence in `build/foliage-history`
(`after-*` captures), not used as the final design. Cluster-level measurements
and image checks are recorded separately from that first attempt. The initial
cluster implementation still scanned the whole world twice and regressed; the
bounded recovery queue addresses that duplicated work. Those intermediate
`cluster-final-*` captures are not evidence for the final queued version.

Validation includes a raw GPU bitset oracle with hash/workgroup collisions,
virtual-member IDs and clearing; actual renderer tests verify first-frame
recovery, next-frame history usage, streamed root reuse, four views, teleport
and off-camera shadow requests. No interactive playtest is started.

Recovery carries current validated bounds from phase one. Its workgroup combines
output reservations, triangle totals and bucket counts before touching global
atomics. This removes repeated pose/LOD/frustum work and per-record contention.
Hi-Z starts at the first potentially usable footprint mip; an odd-dimension CPU
oracle verifies it chooses the same first covering mip as a scan from level zero.

The cached-bounds attempt **still regressed**, roughly19.3 ms covered GPU time
versus17.1 ms. Moving work between two raster passes is not itself an optimization.
Retain those `bounded-bounded-*` runs as rejected intermediate evidence, separate
from the workgroup-aggregated recovery measurements below.

An automatic GPU scheduling policy handles sparse alpha coverage: after a frame
with zero cluster occlusion rejections, the complete currently valid cut is
rendered in phase one. Every32nd frame probes history-first recovery again.
This only draws more valid geometry; it never trusts old depth or skips a newly
visible object. Probe cadence affects potential savings, not image completeness.
The frame epoch and last rejection count live beside the history payload and
are updated/copied on GPU. No CPU diagnostic readback controls this decision.
The depth pyramid/gather still execute; eliminating those dispatches would need
further GPU work scheduling, not a CPU count round-trip.

## Final validation and measurements

Three alternating runs per executable, same80-frame alpine flight, native
1713x964 one view,62,274 active roots/124,548 reserved,4,096 branch slots.
Discard first10 GPU profile samples; median each run, then median of three.

| Timestamp-covered pass | Baseline ms | Final ms |
| --- | ---: | ---: |
| Whole covered GPU frame |17.206|17.047|
| First source cull |1.295|1.678|
| First source raster |5.676|5.367|
| Depth pyramid |0.084|0.084|
| Deferred cull/recovery |1.151|0.075|
| Second source raster |0.916|1.172|
| History gather |—|0.046|

Pass medians do not sum to the median frame. The0.16-ms whole-frame difference
is too small for a strong speedup claim; the architectural change is roughly
performance-neutral here. Initial history-only variants regressed to19.2–19.3 ms.
The final frame has230,442 phase-one clusters and no deferred clusters; actual
alpha coverage yielded no occlusion rejections in the inspected probe evidence.
This is a limitation of this scene/representation, not proof occlusion is solved.

All110 foliage-filter tests and10 history-filter tests pass (filters overlap).
The forced one-record queue fixture retains exact RGBA output. Real route
captures have RGB mean absolute differences0.378–0.380/255 excluding the top180
HUD rows, channel p99=9. Inspected before/after images show no obvious holes;
small draw-order differences remain. These captures are not exact equivalence.

See [machine-readable evidence](foliage-history-2026-11-04-evidence.json) for
per-run/per-pass medians, executable SHA256s and failed intermediate measurements.
Raw images, logs, executable copies and GPU samples remain in
`build/foliage-history`. CPU work and readbacks cause wall-time hitches; this is
not a gameplay FPS claim or evidence for4K/four-player8.33 ms. No playtest opened.

Next: move visibility scheduling into shared assembly traversal/publication
without losing shadow demand, then actual bounded asset pages, shared ancestry
poses, animated shadow cache, meadow migration and far cell aggregates. None of
those gates is marked complete by this downstream camera step.
