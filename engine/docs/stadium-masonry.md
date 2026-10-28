# Clustered stadium masonry prototype

Growing Guns now uses real staggered stone courses on the inner facade and
stepped seating. Columns, the outer wall and canopy retain their prior meshes.
`GG_STADIUM_BRICKS=0` restores the old flat sections in the same executable.

`render::masonry::panel(material, columns, rows, seed)` cooks a unit panel from
shared quarried stone shapes and a recessed solid core. It is intended to be
cooked during scene creation, never each frame. The core closes the joints
without separate mortar objects. Nonuniformly scaled stone normals use the
inverse scale; instance placement and physics keep the existing stadium envelope.

The game reserves two panel types per quarry palette, instanced once per stadium
section. Its 56 facade sections and 1,008 stepped sections retain the same
instance count. There are no per-brick rigid bodies or additional shadow casters.
The shared stone texture is reused. The normal GPU cluster hierarchy selects
geometry by projected error for each view; this is actual geometry LOD.

| Shared panel | Source triangles | Coarsest root triangles | Levels |
| --- | ---: | ---: | ---: |
| Facade, 4 columns x 8 courses | 2,732 | 42 | 7 |
| Step, 6 columns x 1 course | 468 | 58 | 4 |

These are hierarchy endpoints, not a claim that all visible panels render at
the coarsest level. Across all sections the corresponding bounds are 624,736
source / 60,816 root triangles, before culling. Old boxes total 12,768 triangles.
Render costs and appearance must be checked separately.

Reproduce images without opening a window:

```powershell
target/debug/growing-guns.exe script games/growing-guns/scripts/stadium-masonry.json --out masonry-new --players 1 --seed 52 --sky noon
$env:GG_STADIUM_BRICKS = '0'
target/debug/growing-guns.exe script games/growing-guns/scripts/stadium-masonry.json --out masonry-old --players 1 --seed 52 --sky noon
Remove-Item Env:GG_STADIUM_BRICKS
```

The existing stone surface is still a prototype with overly regular fine
fracture patterns. This change adds architectural block geometry; it does not
resolve that remaining material issue.

## Measurement, 28 October 2026

Same executable, alternating old/new three times, RTX 5070 Ti / Vulkan,
3840x2160 native output, four views, seed 52, stock weapons, full lighting.
Each run discards 30 warmup frames and measures 120 frames; all six runs have
120/120 GPU samples and no dropped/error samples. No interactive game was running.
The benchmark advances bots and effects; this is a short controlled arena
scenario, not an explosion stress test or a general 120 FPS guarantee.

| GPU frame median (ms) | Repeat 1 | Repeat 2 | Repeat 3 | Median of runs |
| --- | ---: | ---: | ---: | ---: |
| Flat sections | 3.49 | 3.52 | 3.59 | 3.52 |
| Masonry | 3.99 | 3.93 | 3.90 | 3.93 |

Added covered GPU time is approximately 0.41 ms in this scenario. Instance
count remains 55,249 for the complete scene. Selected pass costs below are
**medians of the three per-run means**, because the existing bench reports
per-pass means, not per-pass sample medians:

| Pass (ms) | Flat | Masonry |
| --- | ---: | ---: |
| Colour raster | 0.441 | 0.625 |
| Shade | 1.309 | 1.355 |
| Colour cull | 0.227 | 0.257 |
| Near shadow raster | 0.330 | 0.333 |
| Far shadow raster | 0.088 | 0.088 |

Serialized CPU-submit-plus-GPU-wait frame medians vary between 8.63–8.94 ms
for old and 8.76–9.87 ms for new. Do not equate covered GPU time with gameplay
FPS; CPU scheduling and background CI can affect wall time.

Command: `growing-guns bench --views 4 --size 3840x2160 --frames 120 --seed 52 --build none`.
Evidence: `C:/dev/warp-bench/masonry-20261028/bench-{0,1}-{1,2,3}.log`,
matching `view-0/` and `view-1/` captures at three fixed viewpoints.
Executable SHA256: `580314684a8fbd47d33e8432ec70dcc0996a823a85a9d1bba1d7f618ad35433d`.
Tested source base: `ff03637` plus this masonry change.
Mesh winding, finite/unit normals, bounds and hierarchy reduction passed;
the existing stadium orientation/clearance tests also passed.

The additional bevels read clearly in the close facade capture. At arena
distance they retain the large masonry courses without replacing the overall
stadium silhouette. Public rows obscure much of the tread detail. Outer walls
and columns remain plain in this first bounded implementation. No interactive
playtest was launched; the user assigned that to Claude.
