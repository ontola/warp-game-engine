# Analytic grass-ground ambient occlusion — r59

The user's request is inexpensive grounding beneath grass clumps. The opt-in
`KERN_MEADOW_CONTACT_AO=1` adds soft, world-anchored basal footprints using the
same root seed, retained-cell decision and root jitter as source and aggregate
grass. It attenuates **terrain diffuse skylight only**. Direct sunlight,
headlights, other local lights, emission, specular, fog and terrain pigment
are unchanged. This is an ambient-light approximation, not grass casting
directional or headlight shadows.

Four neighbouring canonical root cells suffice for the bounded radial kernel.
Shared deterministic helpers moved into `meadow_field.wgsl` without changing
their seed/hash mathematics. Density selects the same retained roots instead
of darkening every cell uniformly. Source rest top-coverage controls footprint
weight, and existing habitat plus conservative road/field bounds gate it.
The radius is capped below one root-cell pitch, with a world-coordinate
floating-point error allowance so distant coordinates cannot introduce a
fifth nonzero neighbour. The CPU support oracle checks this against a wider
neighbourhood and checks continuity across grid boundaries.

This adds a bounded arithmetic loop to terrain shading, no texture fetches,
new rendering pass, per-root CPU streaming or shadow-map update. The existing
substrate uniform grows from 32 to 64 bytes. AO and substrate albedo have
independent switches. Disabling grass or setting zero density disables AO;
live size/density controls are applied once. No camera distance, source/aggregate
LOD or render scale participates in the world-space AO definition.

The base strength is 0.35 at size 1, scaled by `2*size/(1+size)`; the saved
size-2 fixture therefore uses strength 0.46666667. The reported maximum radius
is the configured upper bound; the local floating-point safety allowance may
reduce it slightly. Capture metadata reports actual constructed
`meadow.contact_ao.{enabled,strength,max_radius_m,scope}`.

Limitations: this is a source-weighted radial rest footprint, not a bake of
individual blade silhouettes or dynamically bent grass. Receiver habitat and
rectangular bounds do not reproduce every neighbour's terrain-support validity
check. A conservative road setback intentionally omits contact near the road
edge rather than casting darkness onto the track. Visual benefit must be
checked; low arithmetic cost alone does not establish good appearance.

Runtime source: `e8634da173abc1293e02edcd122d8102e4da3a0b`.
Frozen binary: `C:/dev/perf/warp-sideways-20261027/r59-meadow-contact-ao/sideways-r59.exe`.
SHA256: `48652f1686222145b2c1216a610406b955df2080b9b65c4f74c12f0ff17674b5`.
528 renderer and 185 CLI tests passed, one ignored each. Tests cover shader
consumers, CPU/GPU uniform layout, independent switches, empty/density endpoints,
root support and lighting scope. Launcher contracts passed; DLSS release build
took 89 seconds.

Six alternating OFF/ON timing runs and separate all-four-view PNG comparisons
completed. Both use one 3840×2160 output, four 1916×1076 panels,
958×538 actual DLSS Performance input, full-resolution aggregates, a 25–35 m
source band and 160–180 m outer fade. Sample pruning is OFF in both variants,
so the AO cost is isolated. The same seed, 480-frame route, lighting, camera
traces and 120 Hz/four-substep physics are retained. No default changed.

## Repeated timing

Median of three run medians, with 179 clean frames per section per run
(537 per variant/section). All 2,880 GPU records validated. These are hidden,
synchronized route diagnostics, not gameplay FPS or CPU execution time.

| Measurement | First section OFF / ON (ms) | Continued OFF / ON (ms) |
| --- | ---: | ---: |
| GPU interval | 28.007 / 27.997 | 34.110 / 34.203 |
| Opaque shading | 4.092 / 4.085 | 3.920 / 3.936 |
| All meadow root scopes | 7.312 / 7.463 | 9.511 / 9.654 |

The continued shading median increases 0.016 ms; its run ranges overlap
(OFF 3.916–3.935, ON 3.933–3.940 ms). First-section shading ranges also
overlap (OFF 4.086–4.135, ON 4.055–4.146 ms). Total GPU intervals vary
more between runs than the OFF/ON median differences. There is no resolved
large cost increase here, but neither zero cost nor a speedup is established.
AO executes in opaque shading, not the meadow passes. The 8.33 ms total and
4 ms meadow targets remain unmet.

Per-pass medians: [CSV](data/meadow-contact-ao-r59-passes.csv).
Root groups: [CSV](data/meadow-contact-ao-r59-group-costs.csv).
Local evidence root: `C:/dev/perf/warp-sideways-20261027/r59-meadow-contact-ao/`.
`trial-a/report.json` retains run validation; `analysis-a/` retains full
ranges and all four views. The inherited canned budget report excludes
`diagnostic_sync` and reports zero samples; it is not the source of these
numbers. The explicit analyzer selects and validates 179 clean frames.

## Visual scope and state

Separate full-output PNG runs capture frames 60, 240 and 480 in both modes.
`visual-qa/contacts-a/manifest.json` identifies all 12 view/frame crop boards,
their source images and hashes. Sparse stills do not establish temporal or
all-weather acceptance. The daylight fixture also makes skylight-only contact
subtle; it is not evidence of a strong new grass-shadow effect.

All 12 boards were reviewed. No obvious new stipple, halo, road-edge black
fringe or silhouette loss was observed. Improved grounding cannot confidently
be identified in every crop. Existing tall grass walls, abrupt bare-ground
edges/pale gaps and broad colour bands remain in both variants. The hashed
review is `visual-qa/contacts-a/visual-review-agent.json`. This is a small,
opt-in grounding aid, not acceptance of those broader grass issues.

The interactive r58 playtest was closed at the user's request. Both r59 QA
runs have ended. No new interactive session is scheduled; AO remains opt-in.
