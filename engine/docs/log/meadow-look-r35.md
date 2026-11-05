# Meadow appearance — r35

User feedback on r34: grass is too uniform in colour and leaves too much ground
uncovered. This iteration changes the actual source silhouette and material
variation, while retaining the GPU field and its candidate/per-tuft triangle
budgets. Actual selected triangles can change with the source bounds/LOD.

- Each group now has two tall, three middle and four low outward-arching blades.
  The compact root collar, 36 blades, four wind groups and three reusable closed
  meshes remain. The finest source remains 792 triangles per tuft.
  The ribbon cross-section is rotated across its arch (a proper rotation,
  preserving winding and exact local-Y rings). Low tips sit 9–14 cm high.
- Root jitter is constrained to 0.95 of its existing 0.45 m cell. The previous
  1.6-cell range could move neighbouring roots together and leave accidental
  gaps. Habitat rejection and the maximum candidate count are unchanged.
- The GPU computes coherent fresh/olive/dry material patches from stable world
  coordinates once per root. Small root and blade-height pigment variation
  preserves local detail. It does not modify light visibility, normals or
  opacity, and does not introduce a view-distance tint.
- RGB10 pigment and the terrain X/Z gradient use formerly available fields in
  the existing root record. No extra root bytes, CPU plant updates or
  per-vertex noise/heightfield evaluation.
- The GPU applies the root terrain triangle's vertical shear to the full
  current/previous wind pose, with inverse-transpose normals. Conservative
  wind bounds and source LOD error include the shear's exact operator norm.
  This keeps the low blades above the local plane on a hillside.

The source silhouette check clips actual triangles below 18 cm and measures
their union against retained r34 geometry and placements. The final independent
seven-seed geometry audit measures oblique low coverage at 10/20/35 degrees:
gains are 1.5–14%, 23–42% and 75–100% respectively. Top-down low coverage rises
10.7–14.2 times. These are shape measurements, not terrain-screen coverage or
performance claims. The horizontal tall-fan silhouette is deliberately reduced;
tests now gate oblique low coverage and retention relative to ordinary grass,
not unchanged r34 side silhouette. All 11 focused meadow/source/bounds/shader
checks pass for the final candidate (`tests-meadow-canopy.log`).

Each tuft follows one local terrain plane; blade tips crossing a differently
sloped neighbouring triangle can still intersect terrain. Grass receives
scene shadows but does not cast its own. Source mesh LODs and the 100 m field
remain; scalable far voxel aggregation is still open. Larger silhouettes can
cost more pixels even when their triangle count is unchanged.

Evidence: `C:/dev/perf/warp-sideways-20261025/r35-meadow-look`.
The comparison uses the staged r34 and rebuilt r35 executables with the same
seed, stage, cameras, fixed wind and lighting. It records actual GPU counts,
DLSS input/output sizes and per-section GPU/CPU scopes. There is deliberately
no pixel-equality expectation for this appearance change. Concurrent CI load
and the short sample counts prevent a clean performance improvement claim.

Final `comparison-c` was reviewed at both real cameras and accepted for an
interactive iteration with visible limits. Low ribbons now overlap more of
the foreground ground, and green/olive/dry patches are visible. Sunlit basal
cover still looks pale/flat and upright/cliff tufts remain discrete. Own grass
shadows are absent; this is a known contributor, not a proven sole explanation
of the remaining appearance. This is not final dense-carpet/optical parity.

The actual chase view selects 15,878 roots / 12,025,656 source triangles; the
cliff view selects 132 / 104,544. Counts include each source triangle once;
depth and colour submit them twice. Eleven clean GPU samples per camera:
chase meadow render 11.685 -> 11.698 ms, covered GPU interval32.802 ->32.328 ms;
cliff meadow .761 ->.732 ms, GPU15.650 ->15.692 ms. These are short A/B
observations under concurrent CI, not a performance win or120FPS result.

The final release build completed in54.49s. Playtest first visible frame was
confirmed in9.844s and PID/path/start ticks verified. Manifest:
`games/sideways/build/dev-runs/20261025T121139630Z-r35-play/iteration.json`.
Executable SHA256:
`ccdc8d52fc36799c22e10080098565969f0ea15f6eb822ce06ffd02d7eb3c120`.
One player,3840x2160 output, DLSS Quality, live wind0.6. Final captures are
1600x900 JPEG previews of4K output; fixed lighting/wind isolate appearance.

The first `comparison-a` is explicitly an intermediate visual failure: colour
variation appeared, but low blades disappeared into sloping ground. It is
retained as evidence, not labelled an accepted final result. This prompted the
terrain-plane change above and a fresh `comparison-b` on the same cameras.

`comparison-b` also remains an intermediate visual failure: terrain alignment
improved and material patches are visible, but the chase view still shows broad
soil gaps. It selected 15,850 roots / 12,005,856 source triangles versus r34's
15,169 / 11,455,056. The extra roots did not solve the source's coverage. A
source inspection found ribbon width aligned with the arch plane: lowering
those blades mostly exposed their thin edges from above. The next comparison
corrects the cross-section orientation and reduces the soil inset from 25 to
5 mm, instead of increasing field density. No final acceptance is implied by
either intermediate run.
