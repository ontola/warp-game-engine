# Shared terrain visibility foundation

This backend answers visibility and first-hit queries against the loaded native
heightfield. It is camera-independent, shared between views/lights, and contains
no screen-space data, fog colour, light brightness or baked sun direction.
It is not a mesh signed-distance field, a foliage optical field, or a complete
Lumen implementation. Trees, cars, buildings and overhangs are not represented.

## Why this representation

Epic describes Lumen's shared ray-tracing representation separately from its
surface cache and amortized lighting updates. That separation is the useful
architectural precedent here: establish truthful visibility before caching
radiance. [Lumen technical details](https://dev.epicgames.com/documentation/unreal-engine/lumen-technical-details-in-unreal-engine).

Epic's distance-field documentation also explicitly handles landscape terrain
with heightfields rather than per-mesh SDF resolution. Our conservative min/max
hierarchy skips empty space using that simpler surface representation; it does
not provide signed Euclidean distances or SDF cone tracing.
[Mesh Distance Fields](https://dev.epicgames.com/documentation/en-us/unreal-engine/mesh-distance-fields-in-unreal-engine).

The eventual radiance-cache and update-budget work is separate. The performance
guide describes cached global visibility, separate static/movable updates and
world-space irradiance probes with occlusion-aware interpolation. None of its
performance figures transfer to this engine.
[Lumen performance guide](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-performance-guide-for-unreal-engine).

## Query contract

`terrain_trace::Ray` contains a world position/direction and a finite metre range
`t_min..t_max`. Directions are normalized internally. A trace returns:

- `Clear`: only the complete requested finite segment is known clear.
- `Hit`: first terrain/solid hit, distance, world position and upward normal.
- `Unknown`: outside coverage, unknown source heights, exhausted work budget,
  invalid input or exhausted traversal stack. This is never silently lit.
- `AboveTerrainBounds`: the upward helper has proved a clear path above the
  maximum represented terrain. This is terrain-only evidence, not visibility
  through other geometry or a declaration about future streamed terrain.

The default `Coverage::UnknownOutside` treats field exits and invalid/missing
height samples as unknown. `CompleteLoadedTerrain` is an explicit caller promise
that finite sentinel samples <= -1e8 and everything outside the field contain no
terrain **in this loaded scene**. It ignores absent triangles, preserves a valid
half-cell triangle, and still treats NaN, infinity and unexpected extreme values
as unknown. Power-of-two allocation padding is never part of the real domain.
The game must not select this policy for a partially streamed field without
tracking that change in its coverage contract.

## Data and traversal

Each native cell uses exactly the x+z=1 diagonal already used by `ground_at`.
Base bounds enclose those triangles; each following level contains conservative
min/max bounds of four children. Traversal visits intersected children nearest
first. A node is skipped only when the complete ray interval is above its maximum
height, with a rounding margin. Leaves solve the actual affine triangle planes.
No fixed 48m, 50m or 400m maximum distance is present.

GPU storage is one buffer: 304-byte header, then tightly packed `vec2<f32>` bounds
for all levels, followed by zero padding to 16-byte struct alignment. A single
2x2 heightfield therefore requires 320 bytes, not 312. The header carries native origin/cell size, dimensions, sixteen
level descriptors and the coverage policy. The shared WGSL module uses read-only
group 0 binding 24 and the caller's existing raw heightfield texture. It does not
consume another sampled texture. `GpuHierarchy::new` can reuse an existing height
view; the standalone fixture owns a separate texture for isolation.

`encode_build` builds the base and reduces each level in a separate compute pass.
The hierarchy is built once for immutable terrain and must be rebuilt when that
terrain changes. Neither camera motion nor sun movement invalidates it. CPU
`Hierarchy::packed_bytes` supplies an independent packed reference.

## Bounded work and limitations

The default traversal budget is 256 nodes, with a hard maximum 4096. Exhaustion
returns `Unknown`. The GPU traversal stack is 64 IDs / 1 KiB per ray. The isolated
batch API accepts 1..8192 queries. Maximum allocated base cells are 16,777,216;
allocation also checks the actual device buffer and texture limits. A 1024x1024
base hierarchy uses about 11.2 MB, plus the raw heights if not already shared.

This is suitable for measuring a future shared visibility/radiance update
schedule. Running the maximum budget for every pixel is a correctness experiment,
not an established 4K/four-player performance solution. Grazing terrain rays may
visit many cells. There is no guessed brightness fallback for that case.

Visual mesh relief may differ from the native heightfield. The integration must
handle that declared representation error at the receiver; the trace itself does
not add a large arbitrary sun bias or modify authoritative collision heights.

## Verification

Eleven focused CPU/layout/WGSL tests passed on 2026-10-24. They include a 1500m
distant blocker, a 4000m clear ray requiring one node, actual diagonal geometry,
slopes, translated worlds, missing data, explicit coverage, padding, zero budget,
and packed ABI validation. An independent brute ray/triangle oracle covered 256
rays through a non-planar field: 148 hits, matching first-hit distances, maximum
18 hierarchy nodes. These are correctness observations, not FPS measurements.

The initial five-case GPU run passed on an RTX 5070 Ti. GPU-built hierarchy bytes
matched CPU bounds exactly; status/distance/normals matched the reference, including
strict and complete coverage, zero-budget queries and four repeated query batches
sharing one field. Evidence: `C:/dev/perf/warp-sideways-20261024/r25-world-visibility/terrain-gpu/report.json`.
These small batches took 0.037-0.067ms to query, excluding hierarchy construction;
that is not a per-pixel scene cost or a gameplay FPS claim.

The separate integrated renderer fixture passed a 600m ridge with three camera
approaches/orbits, low and high roof receivers, and four views. Evidence:
`C:/dev/perf/warp-sideways-20261024/r25-world-visibility/world-sun-fixture/report.json`.
The current fixture adds the newly fixed single-cell 320-byte binding and malformed
direction inputs; those additions have passed CPU/WGSL checks but await the next
GPU run. Overflowing direction length and non-finite derived field extents are
explicitly rejected rather than creating different CPU and GPU rays.

## Proposed next step: certified world-space sun ceiling cache

This is a design, not an implemented or timed cache. Keep the existing hierarchy
as its exact fallback and acceptance oracle. A ground-only binary shadow mask is
insufficient: a low car roof and a high tree crown at the same XZ need different
terrain visibility.

For a sun above the horizon, let `u` be its normalized horizontal direction and
`m = sun.y / length(sun.xz)`. Define a terrain shadow ceiling:

```
C(p) = sup over s >= 0 of [h(p + s*u) - m*s]
```

A receiver at horizontal position `p` is clear of terrain when its height exceeds
`C(p)`, and blocked below it. Thus a two-dimensional WORLD field can answer all
receiver heights without flattening roofs or crowns to the ground. It still says
nothing about trees/cars blocking one another. An almost vertical sun uses the
local terrain query instead; the no-direct-sun case needs no ceiling query.

### Store intervals, not interpolated scalar heights

Store a certified `[lower, upper]` ceiling interval for the entire footprint of
each world tile, plus terrain/coverage and sun-key validity. Do not bilinearly
interpolate sampled ceilings as if they were exact: the maximum envelope can
contain a narrow ridge shadow that falls between samples. No opacity blend or
brightness adjustment should conceal that uncertainty.

For a tile with sun coordinates `a = dot(p,u)`, `b = dot(p,perpendicular(u))`:

* An **upper bound** comes from all hierarchy nodes intersecting the tile's
  sunward beam. For a node, `node.max_height - m*max(0,node.min_a-tile.max_a)`
  bounds its contribution. Reject nodes outside the beam's transverse interval;
  retain conservative overlap otherwise. The maximum of these bounds encloses
  every possible blocker for every receiver in the tile. Refining nodes tightens
  the bound. If work stops early, include the bounds of all unvisited frontier
  nodes, so an unfinished update remains conservative instead of becoming clear.
* A useful **lower bound** requires an occluder that every ray from the tile must
  cross. Choose a sunward cross-section at `a=A >= tile.max_a`. If known terrain
  covers its entire transverse interval, then
  `min_height_on_section - m*(A-tile.min_a)` is a valid lower bound for every
  receiver. Take the largest certified section bound. Minima are evaluated on
  the actual piecewise-linear terrain/triangle intersections. A node minimum
  alone is NOT this proof unless the node covers the complete cross-section.
  Missing/unknown parts invalidate that lower-bound witness, not all other
  independent witnesses.

Known-empty triangles contribute no blocker. Unknown nodes cannot certify an
upper bound; an independently proven lower blocker can still certify occlusion.
Strict coverage retains its outside-world uncertainty. Complete-loaded coverage
only uses the explicit scene promise already established by the trace backend.

At lookup, height above `upper + numeric_margin` is certified clear, height below
`lower - numeric_margin` is certified blocked, and the interval between them uses
the exact hierarchy query. Equality is deliberately unresolved. Cache miss,
coverage mismatch or exhausted exact work retains the existing explicit Unknown
result. Receiver offset/visual-error policy is applied BEFORE this query, exactly
as in the uncached path; it must not become an additional global shadow bias.

### Bounded shared scheduling

Use world-aligned sparse tiles shared by all four views. Native-cell tiles are the
finest level; start with larger tiles and refine only where certified intervals
remain too wide. Priorities may follow visible receivers, but answers and cache
addresses must not depend on the camera. A fixed entry count and global hierarchy
node budget bound memory and update work. For example, 65,536 entries at 16 bytes
are 1 MiB of result payload, excluding hash/page metadata and pending work. This
is a sizing example, not a validated default or a GPU time estimate. Allocating
another dense native-resolution field over the whole current stage would be a
large avoidable cost.

The key includes terrain geometry, coverage policy and sun direction. Cars,
foliage wind, camera movement and view count do not invalidate a terrain cache.
On a sun change, stale cells become misses, not temporarily wrong shadows. A
future continuous day/night implementation may certify a whole angular sun bin,
using beam bounds over that angular interval; reusing an exact-angle cache under
an unbounded direction change is not allowed. That extension needs its own
proof and may widen the fallback band.

### Acceptance before enabling

Compare cached decisions with the exact triangle oracle for random receivers
at multiple heights, ridge edges narrower than a tile, valleys, translated worlds,
grazing sun, missing coverage, budget exhaustion and sun changes. There must be
zero false certified-clear or certified-blocked results; uncertainty is a valid
outcome. Validate actual GPU parity, four-view cache sharing, camera motion and
static-frame reuse. Measure cold construction, bounded per-frame updates, warm
lookup cost, exact-fallback percentage, Unknown percentage, memory and node visits
separately. Then test native 4K/four-player output. Only those measurements can
justify a cache size/update budget and whether it improves the 120FPS target.
