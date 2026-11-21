# Voxel rasterization and brick culling

Baseline f0efc8cc on main. The tests ran headless on Windows, Ryzen 9 5900X,
using D3D12’s Microsoft Basic Render Driver 10.0.26100.9818. The RTX 5070 Ti
was occupied. Times below are medians from isolated passes on the software
adapter. They cannot be added to predict the cost of a hardware frame.

## Camera worklists

Splat generation clears 160 bytes at the start of each camera view. These are
the 144 bytes of counters and draw arguments, followed by the 16-byte compact
tree-list header. Only entries appended this frame can be read. The host sizes
the list for at most one entry per eligible tree, retaining the existing view
cap and the direct-dispatch fallback on overflow. Ray integration still
reserves and clears its tile candidates, masks and job counters.

| View | Baseline clear | New clear |
| --- | ---: | ---: |
| 1920 × 1080 | 0.661 ms | 0.002 ms |
| 3840 × 2160 | 2.488 ms | 0.002 ms |

For 4,096 eligible trees in a 4K view, the counter/worklist allocation falls
from 8,553,872 to 16,544 bytes. The old per-frame clear covered the
8,424,000-byte tile array in addition to the headers. The comparison timed
the command-encoder clears for an active-tile, compact-list scene, with
four warm-ups and 47 alternating timestamp pairs per resolution. The
selection pass and readback ran outside the timed region. Starting from
nonzero buffers, the fixture checked 224 lists and indirect headers across
full, partial, empty and overflowing views. Overflow used a 16 × 16 view.

## Brick occlusion

The camera tests the same eight world-space corners in four pairs along the
z axis. A brick that reaches the near plane stays visible. While testing
the reversed-depth pyramid, the first sample at or behind the brick makes
the brick visible and ends the search.

For 65,536 transformed brick bounds, median time fell from 0.992 to 0.740 ms
over 61 interleaved samples. All visibility decisions matched the prior
shader and an independent CPU calculation. Of the 26,991 hidden bricks,
21,817 lay outside the viewport. The depth test rejected the other 5,174.
The CPU calculation counted 129,791 texel reads before and 52,623 after.
Cases included rotated, sheared and nonuniform transforms, four camera
matrices, sky holes and an odd-sized 1025 × 577 depth pyramid.

## Drawing and shading splats

Camera and shadow draws submit four strip vertices per splat. The two
triangles keep their previous diagonal and winding. The separate extinction
lookup selects the three direction bins with scalar min/max operations and
retains stable x, y, z ordering for ties.

| Raster workload at 640 × 384 | Splats | Baseline | Four vertices |
| --- | ---: | ---: | ---: |
| Subpixel | 40,960 | 78.53 ms | 79.69 ms |
| Near | 15,360 | 30.53 ms | 29.43 ms |
| Four overlapping layers | 61,440 | 117.73 ms | 115.41 ms |

The raster comparison used three warm-ups and 25 interleaved pairs. Every
visibility ID, splat ID and depth word matched in all three workloads. A
separate 256 × 256 comparison matched all ten camera and sun configurations,
including the second indirect draw, both sun dither patterns, and full,
partial and empty shadow damage masks.

On batches of 131,072 packed cells, the extinction lookup took 3.634 to
3.416 ms for varied directions and 3.635 to 3.367 ms for coherent camera
directions (41 interleaved pairs). All indices, float weights and extinction
values matched bit for bit over the 262,144 inputs, including axis ties,
zero components and changes of sign.

## Production checks

The 29 focused voxel-forest tests passed. The assembled forest shader, splat
raster shader and composite shader passed Naga validation. The raster budget
example built with the same topology as the renderer.

The stock full-forest and compact-layout scene checks on this D3D12 adapter
stopped while compiling the terrain shader. DXC rejected the 2 × 2 rotation
matrix array used by the terrain hex tiler. The isolated pass measurements
and buffer comparisons above used the production voxel shaders.
A compile-only check reproduced the DXC failure with the exact terrain code
from the baseline and no voxel code. The compact-layout fixture then rendered
a 64 × 64 scene with 519 occupied cells using a test-only terrain override
that stored each rotation as its sine and cosine. The voxel pipelines and
renderer worklist code were the production versions. The longer multi-scene
replay with that override was stopped after the compact check passed.
