# Seeded stadium crowd from stands to face

Review build 5f9b0351, NVIDIA GeForce RTX 5070 Ti / Vulkan, Growing Guns seed 52,
8,353 occupied seats. Each of two views renders at 960 by 540 native.
Three independent 121-frame runs per candidate, after five warmup draws on the
wide camera. Both cameras share time, seat identities, and GPU geometry.
The travel camera reaches the first seat and returns; the second stays wide.

| Far representation | Fixed wide view GPU ms | Both views GPU ms | Both during close-up GPU ms | Resident buffers wide, bytes | Resident buffers close, bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| cells | 0.285 | 0.418 | 0.437 | 925,620 | 1,475,892 |
| mesh | 0.183 | 0.303 | 0.328 | 920,940 | 1,471,212 |
| impostor | 0.112 | 0.213 | 0.251 | 909,420 | 1,459,692 |

Times are the median of run medians. The two-view metric sums compute selection
and indirect raster timestamps. Close-up uses frames 54 through 66 inclusive.
These are headless crowd and stadium pass costs. The fixture serializes views
for counters and excludes readback from GPU timestamps. All three candidates
classified the entire population as distant in the fixed view, including while
the travelling camera used face geometry. A 155 by 190 pixel crop of the focal face at frame 60 was byte-identical across all three far representations. The two targets use 4,147,200
additional bytes for colour and depth, reused between views in this fixture.

The first frame requesting close detail drew the medium fallback. The next frame
used the uploaded mesh in every run. See the [measurement summary](../evidence/warp-humans-stadium.json)
for request frame numbers and wall times. The page is cold on the GPU, with
reference data embedded in the executable. This benchmark does not measure file
I/O, texture streaming or eviction.

The test exercises a shared anatomical head, procedural face proportions, skin
and scalp shading, clothing colour, simple geometric garments and waving. The
close-up still shows shared topology and a limited wardrobe. The distant
impostor candidate uses articulated camera-facing ellipses, and cells are
articulated boxes. An animated atlas or sparse voxel traversal would be a
different experiment. [Reproduction and review guide](../warp-humans.md)
