# Terrain normals follow the rendered surface

The r20 close cliff capture exposed a real normal/geometry mismatch. Vertex
positions used native triangle height plus clamped, weighted rock relief,
while shading normals differentiated an unbounded curved relief field.
Those were different surfaces. On the captured seed-1 stage, 478 of 25,554
local cliff triangles had an average normal in the opposite hemisphere;
4,409 had at least one such vertex. Source and selected CLOD matched there,
so the defect existed before simplification.

`visuals::terrain_surface_normals` now accumulates area-weighted normals
from the final triangles across all terrain chunks before cluster creation.
Shared XZ vertices receive one common result including both sides of chunk
borders. At sharp non-convex joints, the smooth normal is moved only as far
as necessary towards the heightfield's feasible +Y direction to remain in
every incident face's outward hemisphere. The road/shoulder protection band
retains its existing normals. Positions, topology, material UVs and collision
are unchanged; rock relief is not flattened and no brightness mask is added.

Validation on 2026-10-24:

- All nine `visuals::geometry_tests` passed, including exact shared-edge
  normals/segments, bit-identical positions/UVs/indices and protected road
  normals, plus 21,894 road contact checks over three generated stages.
- The actual captured-region audit now has zero opposite average normals
  and zero opposite vertex normals, both at source and selected CLOD.
- The native source still has zero folded/degenerate XZ triangles. The four
  selected CLOD faces with non-positive XZ area are vertical slivers: two
  have exactly zero XZ area; two have signed areas −5.72e−6 and −1.91e−6 m²
  from rounded f32 coordinates whose intended 1/12-metre grid points are
  collinear. Their 3D areas are 0.00195–0.00859 m². All four project wholly
  above/outside this capture (y −266 to −628 px), with projected triangle
  areas 1.02–19.93 px², and have outward-facing shading normals. These are
  not large inverted source faces. Other cameras/LOD cuts remain outside
  this bounded audit's certification.
- The shared normal pass took 22.14 ms for nine chunks / 27,535 shared detail
  vertices. Its conservative temporary hash-table bound is 3.25 MiB. This is
  CPU stage construction, not per-frame GPU cost or full-stage build timing.
  The production build log also records retained source-buffer allocation.

Evidence: `C:/dev/perf/warp-sideways-20261024/r20/cliff-geometry-audit.json`
and `C:/dev/perf/warp-sideways-20261024/r21/cliff-geometry-after.json`.
`cliff-geometry-after-classified.json` in the same r21 directory adds exact
sliver positions, signed/3D areas and projection measurements.
The latter includes an asserted exact production function snapshot, its
SHA256, selected-CLOD results and rays through the actual black pixels.
The audit is `cli/examples/cliff_geometry_audit.rs`; it fails if its embedded
function snapshot differs from production (line endings are normalized).

The root's separate GPU diagnostic using flat geometric normals removed the
black sky-visibility pits without changing geometry, heightfield or bias.
The subsequent matched production captures `r21/wall-after.png` and
`r21/wall-after-sky.png` were inspected: the artificial black pits disappear
with the smooth normals, while the shaded rock wall and its geometric relief
remain. This accepts the reported capture case; it does not claim every
remaining angular rock feature or distant LOD has been eliminated.

The full CLI regression run passed 110 tests. The captured complete stage's
normal pass processed 170,608 shared detail vertices in about 279 ms during
construction, with a conservative temporary-table bound of 26 MiB and
39 MiB of retained source buffers. It adds no per-frame normal reconstruction
or GPU pass. These are stage construction figures, not frame timing or FPS.
