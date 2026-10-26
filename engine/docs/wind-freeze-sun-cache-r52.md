# Optional distance-based tree wind freeze and static sun cache

Status: opt-in only. No default promotion or large performance gain is claimed.
This feature does not depend on opaque tree crowns, software rasterization or
the experimental voxel representation. Existing source trees and LODs remain.

## Configuration and behavior

All three settings default off: absent/invalid `KERN_WIND_FREEZE` disables
freezing; `KERN_STATIC_SUN_CACHE` must be exactly `1`; absent/invalid
`KERN_SUN_CACHE_GUARD_METRES` gives zero guard. Explicit example files live in
`games/sideways/scenarios/wind-freeze.{cache-only,on}.json`; no launcher or saved
game defaults are changed. `KERN_WIND_FREEZE=50,70` is an experimental setting,
not an accepted quality preset.

Tree sway fades smoothly between the two horizontal distances to the nearest
camera and becomes exactly zero beyond the second. Up to four cameras share
one coefficient buffer, consumed by color, shadows and motion. Grass and
PART_WIND retain their existing behavior. More than four views disables this
freeze field and clears old frozen-tree static membership.

With static caching and a positive guard, eligible trees beyond `end + guard`
from every pinned camera join the static caster list. Crossing the guard or
changing the camera count re-pins the partition. The shared 4096-square depth
map copies cached static depth, then renders current dynamic casters with the
same projection/LOD/alpha/raster policy. Near 2048-square maps remain dynamic.
Shadow lists do not discard off-camera casters. Guarding enlarges the shared
projection and therefore makes its world-space texels larger.

## Correctness fixes included with the isolated landing

- Pack actual XYZ cameras: WGSL reads `.xz`; the initial PR packed XZ into XY.
- Changed camera/freeze uniforms invalidate combined sun depth even when wind
  time is fixed; previous/current motion snapshots retain their existing order.
- A changed frozen-owner transform demotes it to dynamic until another explicit
  pin certifies its new position. The old static image is invalidated.
- Disabling freezing through the view-count fallback clears static certificates.

Focused CPU regressions cover asymmetric nonzero-Z one/four-camera packing,
empty/disabled fields, fixed-time camera updates, moved frozen trees and the
existing guard-distance proof. The isolated main-based landing passed 509
renderer and 183 game tests (one ignored in each suite). Logs are retained in
`C:/dev/perf/warp-sideways-20261026/r54-pr-trials/pr5-isolated-v2/`.
These are CPU invariants and compilation checks, not new GPU acceptance.

## Retained evidence and limits

The original lavapipe and route results used the incorrect camera-Z packing;
their reported triangle reduction is not correctness or performance acceptance.

Corrected evidence is retained under
`C:/dev/perf/warp-sideways-20261026/r54-pr-trials/fixed-wind-analysis.json`.
The one ordered A/B used the same executable and 480 matching deterministic
frame states, four DLSS Quality views in one 3840x2160 output, with 358 timing
samples per run after warmup/capture exclusion. Mean GPU interval was
48.800 to 47.870 ms (1.91%); five JPEG pairs show no obvious gross stale shadow.
This is not presentation FPS or a controlled repeatability result.

That A/B changed freeze, caching and guard together. Shared extent changed
112 to 176 m and texel size 0.0546875 to 0.0859375 m (57.14% larger). Four steady
static redraws each cost 1.38-1.49 ms for cull plus raster. Reported extra GPU
storage is 121306068 bytes, with a 67108864-byte depth seed per shared-sun draw.
There is no same-quality cache-only acceptance, raw-depth parity proof or
continuous-motion acceptance. Validate cache off/on with identical freeze and
guard, moving/fixed-time nonzero-Z cameras, guard crossings, separated views,
camera cuts and moved owners before promoting any setting.
