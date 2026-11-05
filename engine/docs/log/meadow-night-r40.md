# r40: grass distribution and a faint night sky

The reported F12 image `sideways-1792953438414-242232-0` shows close grass with
square bare holes. Its exact stage and camera are replayed in the r40 fixture.
The reproduction does not restore the original car input or tyre marks.

## Grass source

The previous generator could reject an entire 45 cm cell, including up to 12%
of otherwise suitable woodland cells. Each surviving recipe filled a square,
used only quarter-turn yaw and independently randomized its height. Those
choices exposed the placement grid even before the distant representation.

The new source keeps all 360 blades and its 1800/1080/600 triangle cuts. Roots
follow an irregular rounded footprint with a full centre and smoothly declining
edge density. The density is built into actual blade positions by inverting a
radial area CDF at cook time. Nominal support is 36 cm, bounded by 38 cm; patches
overlap across the unchanged 45 cm cell spacing. This is neither screen dither
nor an opacity multiplier pretending to be the same density.

Patch centres have stable jitter of at most 1.8 cm per axis and full-angle yaw.
Growth follows smooth 9 m and 3 m world fields, with only 5% independent root
variation. Every suitable patch is retained after physical eligibility tests.
The source-derived distant atlas and substrate recook from the same recipe;
there is no separate far-only grass layout. The atlas remains a directional
surface cache, not a voxel volume.

Road/terrain safety remains conservative. A patch spanning native triangle
boundaries can still be excluded as a whole near the road. The larger support
can widen that bare margin. This change does not claim exact smooth habitat
density across those safety boundaries, grass self-shadowing, or continuous
temporal parity of the distant representation.

## Night

The requested night treatment adds a small adjustable sky illumination floor
and stable procedural stars. This is an artistic night sky, not a simulated
moon or a new multi-bounce GI implementation. Surface response continues to use
the existing sky visibility, materials and SH integration; headlights remain
the primary local light source. Stars belong only to the background and must
respect foreground depth and atmospheric attenuation.

## Validation status

Candidate A is running for user playtest. Evidence and immutable executables are stored under
`C:/dev/perf/warp-sideways-20261025/r40-grass-night/`.

350 render tests and149 CLI tests passed, with one existing ignored test in each
suite. The initial radial sampler test found an actual omitted outer feather
ring; the fix samples all90 radial strata in each quadrant and retains the
independent density/spacing checks. The fog regression now checks the shared
attenuation wrapper and explicitly forbids stars on opaque surface fog.

Matched F12 JPEGs show the square holes removed without obvious replacement
round holes. The far inspection remains continuous, while stepped road edges
are still visible. An independent review and hashes are in
`grass-visual-review.json`. Stills do not prove continuous motion stability.

The clear night sky shows stars behind foreground trees. The ambient floor
reaches the shared sky/SH pipeline, but the ground remains very dark at EV6.
The user requested immediate playtest; further brightness tuning is deferred.
Candidate A was not described as a fully tuned night lighting result.

New live feedback: terrain appears to change colour while approaching from
about200m to150m. The renderer's aggregate fades out over160–180m horizontal
distance. Beyond180m only substrate-tinted terrain remains. This substrate
blends a top-view coverage fraction into terrain RGB but keeps terrain normals,
normal maps and opaque material shading. The aggregate instead uses directional
source coverage, source normal moments and two-sided thin-leaf illumination.
Matching pigment alone therefore does not match apparent colour/brightness.
These code facts explain a likely transition mismatch; the dominant lighting
term in the user's specific view has not yet been isolated with paired captures.
Next correction should retain coarse grass coverage and lighting at the last
representation, not merely move the same mismatch farther away. Live candidate A
was left running and unchanged during diagnosis.

Eight one-view4K/DLSS Quality sections and three four-view sections completed.
Actual one-view input is2560x1440; four views use1277x717 inputs for1916x1076
output panes in one3840x2160 output. Four-view covered GPU means are48.410ms noon,
57.686ms sunrise,52.767ms night,71 clean samples each. These controlled figures
do not meet120FPS. Full old/new numbers are in `compact-timings.json`.

Live executable SHA256:
`28f1a1798492357df1a256079be2e7f1162a3b88654ce1b8ce2084225893539d`.
Manifest: `../games/sideways/build/dev-runs/20261025T190811046Z-a344157b/iteration.json`.

The r39 reference completed all seven 4K/DLSS Quality visual sections and both
four-view sections. Its first harness attempt lacked the external DLSS runtime
path and rendered no gameplay frames; the second completed rendering but its
postprocessor incorrectly demanded far-grass draws in a fully occluded close
hillside view. Existing results were revalidated without rerendering. These
harness failures are retained rather than counted as game rendering failures.

CPU CI activity remains an external source of contention. Report GPU timestamp
samples separately from CPU wall time. One 4K output with four players at
120 FPS (8.33 ms) remains the target, not a claim about current performance.
