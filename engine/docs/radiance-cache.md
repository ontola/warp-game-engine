# Shared world-space diffuse radiance cache

Status, 24 October 2026: `render::radiance_cache` is an original CPU transport
reference. `render::radiance_cache_gpu` consumes its packed SH and visibility
data in an isolated GPU sampling fixture. **Neither is used by the live renderer.**
The existing renderer still combines global sky SH with canopy visibility and a
heuristic green ground-bounce term. Calling that term a radiance cache would be
incorrect: it has no material or illumination sampled at a ray hit.

This foundation is the first bounded step toward actual cached diffuse
transport. It does not implement Lumen, virtual shadow maps, mesh distance
fields, a surface cache, multiple diffuse bounces, or glossy reflections.

## Principles taken from primary sources

Epic's [Lumen technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine)
and [SIGGRAPH 2022 presentation](https://www.realtimerendering.com/advances/s2022/SIGGRAPH2022-Advances-Lumen-Wright%20et%20al.pdf)
describe separate problems: scene intersection, material/lighting lookup at the
hit, and amortized lighting reuse. The surface cache supplies the hit's material
and lighting (slides 7–8, 56–57); a world-space radiance cache reuses distant
lighting with lower spatial and higher directional resolution than the screen
gather (slides 161–169). Cached data must be updated, not treated as permanently
correct after geometry or lighting changes.

This implementation borrows those architectural principles, not Unreal source
code or its exact representation. In particular, its SH9 probes and directional
distance moments are **not a claim about Lumen's storage format**. Visibility
moments follow the general idea described by the authors of
[Dynamic Diffuse Global Illumination with Ray-Traced Irradiance Fields](https://www.jcgt.org/published/0008/02/01/paper-lowres.pdf).
Our low-resolution interpolation is an approximation and does not guarantee
that every narrow wall is resolved.

Epic's cited performance examples use a 1080p internal render with temporal
upscaling for 4K output and 30/60 Hz quality targets. They do not establish a
native-4K, four-player, 120 Hz budget for this engine.

## What is computed now

For every probe ray, `Transport::trace` returns one of:

- `Hit`: actual distance, outward geometric normal, linear diffuse albedo and
  emitted radiance. A second ray through the **same transport backend** tests
  the sun. The hit contributes emission plus shadowed Lambertian sun reflection
  `albedo * sun_irradiance * max(dot(normal, sun), 0) / pi`.
- `KnownSky`: the backend has certified escape into the environment. The ray
  contributes the supplied linear sky radiance, excluding the separately
  sampled solar disc.
- `Unknown`: missing geometry, unavailable material, insufficient traversal
  budget, or unproven escape. This does not contribute unoccluded sky.

The gathered incoming radiance is projected into real SH9 and cosine-convolved
to irradiance. Surface shading will eventually multiply the sampled irradiance
by the receiving material's diffuse albedo divided by pi. That receiver shading
is not wired into production yet.

This is one actual sun-lit diffuse surface bounce plus directly visible sky
and emission. It does **not** yet compute sky light reflected from the first
hit, cache feedback, or multiple diffuse bounces.

## Shared residency and work budget

`RadianceCache::set_views` takes one to four camera positions together. Probe
keys are fixed integer world cells, independent of camera orientation. Duplicate
or overlapping views reuse the same probes. Moving within a cell does not
reorder a capacity-limited residency set. Requests beyond capacity are reported
as dropped; there is no silent per-view budget multiplication.

Default configuration:

| Item | Bound |
| --- | --- |
| Grid spacing | 4 m |
| Neighborhood per camera | 9 x 5 x 9 probe requests |
| Shared capacity | 2,048 probes |
| Updates per call, across all views | 4 probes |
| Directional samples per probe | 96 (six 4 x 4 cube faces) |
| Maximum rays per update | 384 primary + 384 sun |
| Primary distance budget | 64 m |
| Secondary sun distance budget | 1,024 m |
| Packed GPU storage per probe | 176 B SH/metadata + 1,536 B moments |
| Full default packed storage | 3,506,176 B, about 3.34 MiB |

These are work and storage limits, not measured GPU performance. Each trace
backend must also enforce its own traversal budget. The reference scheduler
currently allocates temporary CPU vectors. The GPU consumer uses bounded binary
search over shared, sorted integer world keys; GPU transport/update scheduling
is still required before a production integration.

An update prioritizes invalid probes and then oldest attempts, so a permanently
unresolved first probe does not block the rest of the residency set. The first
valid result requires all 96 directions to resolve. Geometry, material, sky or
sun changes invalidate old energy before the next budgeted refill. There is no
temporal blending yet; gradual refill may therefore be visible if enabled in a
game without further policy.

## Visibility and validity contract

Each ray stores distance and squared distance. The lookup interpolates these
directional moments and uses a one-sided variance bound to weight the eight
spatial probe neighbors. Missing neighbors remain explicit in
`Sample::known_weight`; `visibility_weight` separately reports the surviving
visibility weight. No arbitrary minimum-light term is added. SH truncation,
coarse directional bins, normalized visibility weights and cube-face boundaries
remain possible leakage/smoothing sources and need scene tests.

Moment interpolation removes variance only within a conservative floating-point
roundoff bound (`8 * f32::EPSILON * max(abs(second_moment), mean_squared)`).
Without this, filtering identical constant-distance moments could invent tiny
positive visibility, which normalized interpolation could turn into full light
behind a blocker. The correction is shared by CPU and GPU and has an explicit
constant-distance regression; it does not add a visibility or brightness floor.

A back-facing geometric hit rejects a probe as inside opaque geometry. Such a
probe, any unresolved ray, invalid material data or non-finite transport makes
the probe invalid and exports zero SH with `valid=0`. Thin two-sided leaves and
fractional transmittance are not supported by this opaque reference yet. They
must be introduced as explicit material/visibility behavior, not misclassified
as opaque enclosure or empty sky.

`terrain_trace::Status::Clear` certifies only a finite segment, and must not be
automatically converted to `KnownSky`. `AboveTerrainBounds` can certify escape
from the represented terrain only; all other represented scene geometry still
needs testing. A complete-loaded-terrain policy may explicitly declare terrain
outside the field absent. Unknown heights, an unbounded missing world, or an
exhausted trace remain unknown. A terrain adapter also needs an actual
position/normal-to-material lookup: a constant green albedo would reproduce the
existing heuristic under a new name.

## Public integration boundary

```text
RadianceCache::new(Config)
set_views(&[world_camera_positions]) -> residency counts
update(&Transport, DirectLight, Revision) -> ray/validity counts
sample(world_position, world_normal) -> irradiance + explicit coverage
packed() -> Vec<GpuProbe>, Vec<GpuMoment>
```

`GpuProbe` is a 176-byte storage record: position/spacing, nine RGB SH
coefficients in vec4s, then validity/unresolved-count/moment-offset/ray-count.
`GpuMoment` is a 16-byte record: mean distance, mean squared distance, known bit,
reserved. Packed order is stable world-key order. There is no per-view buffer or
camera-dependent illumination in this contract.

The isolated GPU slice now uploads that contract, plus one 16-byte integer key
per probe. At 2,048 probes the cache buffers total 3,538,944 bytes (about 3.38
MiB). The consumer checks the actual device buffer limits and rejects an
oversized cache instead of truncating it. Empty caches use harmless dummy
buffers with a zero logical count. Query batches are independently capped at
1,048,576 pixels and checked against texture and dispatch limits.

```text
GpuRadianceCache::new(&Gpu, &RadianceCache) -> bounded shared upload
encode(&Gpu, &mut encoder, &[Query], [width, height], exposure) -> Batch
queue.submit(...)
Batch::read(&Gpu) -> raw HDR samples, receiver PNG pixels, validity pixels
```

This is a GPU **consumer**, not GPU ray transport. The next production step
requires actual scene intersection and material lookup, including offscreen
meshes and foliage. A terrain-only backend cannot certify that those objects
are absent. A separate bounce-only channel could retain incomplete coverage
without replacing the current sky term, but it must not reinterpret unknown
paths as known empty space or manufacture missing energy by normalization.

## CPU verification

Run `cargo test -p render radiance_cache --lib`. Tests cover constant-radiance
cosine energy, a red Lambertian floor and sun blocker, unknown primary/secondary
rays, shared four-view residency and update budgets, geometry revision
invalidation, distance-moment bounds, constant-distance filtering, opaque
enclosure rejection and malformed world/configuration input. Consumer tests
also validate WGSL, upload layouts, negative world identities and rejection of
more than 2,048 probes. These tests do not constitute GPU or live-game acceptance.

## Isolated GPU acceptance fixture

Build with `cargo build -p render --example radiance_cache_gpu --release`.
Run the resulting executable only when the shared GPU is available:

```powershell
$env:KERN_ADAPTER = 'NVIDIA'
$env:WGPU_BACKEND = 'vulkan'
& C:/dev/warp-target-20261023/release/examples/radiance_cache_gpu.exe --out C:/dev/perf/radiance-cache-proof
```

The output directory must not exist. The fixture constructs complete analytic
worlds on the CPU: a sun-lit red Lambert floor, a blocking black plane, a
red/green floor, constant sky, unknown transport, an occupied sphere and an
emissive cavity. Actual geometry, material albedo and shared sun-visibility rays
produce the probe energy. GPU sampling produces the PNGs and raw numeric
results. Nine cases compare every query against the CPU reference, including
missing neighbors and negative world coordinates. Independent checks cover
Lambert energy, constant-sky `pi * radiance`, black blocked/invalid cases and
distance-moment occlusion. Four simultaneous query panels reuse one uploaded
cache and must match the individual panel byte-for-byte. This is not a four-view
game performance result. An empty-cache case must remain black and unknown.

`report.json` records all failures, maximum raw HDR/coverage errors, shared cache
bytes and CPU-bake/dispatch-plus-readback wall times. These wall times are **not
isolated GPU timings**. Each `*-validity.png` uses red for known weight, green for
visibility weight and blue for missing coverage. Receiver images use white
Lambert reflectance, fixed exposure and Reinhard solely for display; numerical
acceptance uses unclipped HDR values.

GPU acceptance is pending an actual fixture run. No live indirect lighting is
enabled by this work, and this does not establish a frame-rate budget.
