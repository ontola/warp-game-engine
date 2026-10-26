# Dense meadow — r36

The user rejected r35 as far too sparse and requested roughly ten times the
grass density. This replaces the field source, rather than multiplying CPU
plant instances or widening the old 36-blade tuft.

## Source and GPU integration

- `MeadowPatch` has 360 narrow opaque blades distributed over each fixed
  0.45 × 0.45 m patch, versus 36 in r35. Nominal capacity is 1778 versus
  178 blades/m², before habitat, slope, road and visibility rejection.
- A jittered 20 × 18 basal lattice fills the patch. Three shared ribbon
  shapes have actual geometric normals, 6–14 mm authored widths and
  12–40 cm authored heights. Per-patch vigor/growth scales blades about
  their individual roots; it never scales the fixed patch footprint.
- Four GPU wind groups supply current/previous angles. Every blade rotates
  about its own basal anchor. Depth, colour and motion share those poses.
  GPU world-cell generation, culling and indirect drawing remain; there is
  no per-frame CPU blade/root stream.
- Ribbon CLOD retains every authored blade, root pair and tip. It preserves
  the finest source bytes and rejects reductions that inflate projected
  coverage. Accepted source counts are 1800 / 1080 / 600 triangles per patch.
  One source variant correctly cannot reduce to one triangle; the runtime
  coverage gate was not weakened to force a lower count.
- The shared source vertex now stores its fixed anchor (64 bytes/vertex).
  Shared recipes occupy 2,004,480 bytes. The 205,209 candidate slots/view
  and 25,445,916-byte root/owner pool/view remain unchanged.
- Stable fresh/olive/dry material patches remain independent of camera,
  wind and illumination. No distance tint or frame-random alpha mask was
  introduced to make the field appear fuller.

## Road boundary refinement

The first actual r36 capture fills the slopes, but exposes square patches
at the verge. Within a single native terrain triangle, the generator derives
the exact local road-clearance gradient from its already loaded samples.
The renderer tests each fixed blade anchor against that plane, with a small
world-stable positive threshold variation and conservative blade/wind margin.
The result is identical across LODs, depth, colour and current/previous pose.

Packed road gradient/clearance reuse two unused words in the 112-byte patch
record. Fully interior patches skip the per-blade hash. No per-vertex terrain
reads or extra GPU bindings are required. Half-float gradient rounding is
bounded over the anchor footprint. Patches crossing a native triangle edge
retain the previous whole-patch rejection; some square steps therefore remain.
Whole-patch habitat/slope rejection can also produce visible boundaries.

Runtime triangle counts describe submitted source geometry, not surviving
rasterized blades. Multiplying selected patches by 360 gives an authored-blade
upper bound before the road filter, not measured grass coverage or visible
blades per square metre. Metadata and the comparison runner state this scope.

## Validation and limits

Sixteen focused meadow/source/anchor/bounds/ABI/shader tests and three ribbon
CLOD tests pass. Dense two-way surface samples check the conservative error
bounds; exact root anchors are checked against independent source transforms.
Small-growth wind bounds retain the unscaled basal lattice on steep terrain.
The final release build completed in 55.23 seconds under concurrent CI load.

Evidence is in `C:/dev/perf/warp-sideways-20261025/r36-dense-meadow`.
The serial comparisons use the exact staged r35 executable and r36 with the
same seed, two cameras, tick 6000, noon light and fixed wind. Each section has
32 frames, 20 warmup frames and one excluded capture, leaving 11 clean GPU
samples. Output is one 3840 × 2160 view with DLSS Quality from 2560 × 1440;
previews are 1600 × 900 JPEGs. CPU wall time and GPU intervals stay separate.

Final `comparison-b` inputs match and both actual camera images were reviewed.
The open slopes are now covered by dense grass instead of separated tall
tufts and large bare gaps. Green/olive/dry areas remain visible. The road-edge
refinement improves some boundaries; the close cliff still shows patch steps.
Accepted for the requested density playtest, with those visible limits.

| Frozen camera | r35 source triangles | r36 source triangles | r35/r36 meadow GPU ms | r35/r36 covered GPU ms |
| --- | ---: | ---: | ---: | ---: |
| Driving chase | 12,025,656 | 14,629,800 | 11.776 / 15.261 | 32.382 / 35.498 |
| Close cliff | 104,544 | 269,280 | 0.782 / 1.302 | 15.714 / 16.363 |

Chase selects 1997 / 5030 / 9338 patches in the three source LODs.
Source counts include each triangle once; depth and colour submit it twice.
These short measurements under concurrent CI are not a clean performance
certification. In particular, ten times the nominal blades did not mean ten
times the submitted triangles, but grass GPU cost still increased materially.

The tested release SHA256 is
`9c5e31b6bec09b1e7bba125eb52a4e189d290597c97234445736b779f79337b5`.
It was staged and launched at
`games/sideways/build/dev-runs/20261025T151030601Z-r36-play/iteration.json`.
The first scene frame was confirmed after 10.075 seconds. Playtest uses one
4K-output/DLSS Quality view with live wind 0.6 and unchanged 120 Hz/four substeps.

The field still uses source ribbons out to 100 m, fading from 80 m. Scalable
far voxel aggregation and grass self-shadow casting remain absent. One local
terrain plane is applied to the patch: roots that cross a native triangle
boundary can still float or become buried on sharp height changes. The new
road-plane guard does not fix that separate terrain-support approximation.
The four-player 4K/120 FPS target remains unmet. Density success is not an
optical-parity, full-route, four-view or performance acceptance claim.
