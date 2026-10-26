# Distant mountains and forest

The background forest reuses the native source tree meshes, archetypes,
materials and wind. It measures actual native trees per eligible horizontal
square metre, then applies the same slope cutoff (`normal.y >= 0.75`) and
water clearance to the background's rendered triangles. Tree roots use those
triangles. This is not a new voxel tree renderer.

## Rolling mountain correction — 24 October

The live r23 capture `sideways-1792853644814-99004-0.png` has a conspicuous
bare triangular mountain and a steep bare ridge. Its log already matched
299 trees/ha in suitable habitat. The missing trees were principally a shape
problem: the old extension raised mountains by up to about 200 m over a
128 m transition immediately outside the native corridor. Its ridge function
also had an `abs(noise)` derivative cusp and short, steep shoulder detail.

`distant_forest.rs` now uses a 512 m mountain foot, rounded crests and broader
shoulder wavelengths. The exact distance to the native tile union is cached
per 64 m tile, avoiding a wide hash search for every terrain sample. Native
heights are unchanged, including their bit patterns at shared borders.
The background remains non-driving scenery; this change adds no colliders.

The tree placement seed, candidate jitter, native forest-gap rule, measured
density, archetype variation and wind are unchanged. Eligible land increases,
so the final selection contains more trees. The existing 320,000-tree cap,
tile/heightfield caps, and extent reduction before density reduction remain.

## CPU evidence

The regression reconstructs seed 1, 3 km, round 1 from the actual capture:
noise seed `683407368`, detail seed `2033395789`. It evaluates old and new
profiles on the same source fan triangles and all fixed background tree
candidates. It does not load textures or create a renderer.

| Measure | Previous shape | Rolling shape |
|---|---:|---:|
| Far land rejected for steepness | 6.808% | 0.420% |
| Steep land within first 128 m of native border | 23.954% | 0.000% |
| Median / p95 far slope | 18.87° / 44.28° | 15.12° / 32.00° |
| Highest sampled source vertex | 223.83 m | 214.71 m |
| Eligible background area | 8,577,168 m² | 9,165,072 m² |
| Trees at measured 298.95/ha | 256,417 | 273,993 |
| Background tiles / source triangles | 2,247 / 369,376 | unchanged |
| Background radius | 1,152 m | unchanged |

Native terrain in this stage has 1.32% steep land. The new far tree count is
6.85% higher and remains below the existing cap. Source triangle count being
unchanged does not establish unchanged GPU cost: selected cluster detail,
tree traversal and shadows still require measurement.

Run the focused CPU checks from `engine/`:

```powershell
$env:CARGO_TARGET_DIR = 'C:/dev/warp-target-20261023'
cargo test -p cli distant_forest -- --nocapture
```

13 tests passed, including native height preservation, mixed-resolution edge
stitching, exact tree roots, full-stage slope/peak/budget comparison, and the
expanded distance query against a brute-force native-union oracle.
The full-stage comparison took 2.56 s in the test profile; this includes both
shapes and is not a scene-build or gameplay timing.

Evidence and the exact previous source:
`C:/dev/perf/warp-sideways-20261024/r24-mountains/`.
Root subsequently reviewed `r24-mountains/replay-after/lit.png` at the exact
recorded camera: the bare pyramid is replaced by a wooded, rolling ridge.
This is a native1920x1080 reconstruction without cars/HUD, not the original
DLSS frame. The correction is running in the r24 playtest PID192948. Measure
tree/shadow cost before making a 4K/four-player performance claim.
