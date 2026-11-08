# Material maps, weathering, BC7 and texture streaming, 8 November 2026

Gap 5 of the UE5 comparison (materials and textures). What landed is
described in [materials](../materials.md); this note keeps the numbers.

Machine: cloud container, 4 cores, no GPU, lavapipe (llvmpipe, LLVM 20.1.2)
through Vulkan. Lavapipe times are CPU rasterisation, not GPU estimates.

## BC7 encoder presets

`block_compression` 0.11 CPU encoder on `mossy_rock_albedo.jpg` and
`authored_pine_albedo.png` resized to 1024 square, one core:

| Preset | ms per megatexel | PSNR |
| --- | --- | --- |
| opaque_ultra_fast | 84 | 41.2 dB |
| opaque_very_fast | 1519 | 43.2 dB |
| opaque_fast | 2302 | 43.3 dB |
| alpha_ultrafast (pine) | 406 | 42.0 dB |
| alpha_very_fast (pine) | 2013 | 44.7 dB |
| alpha_fast (rock) | 4047 | 43.2 dB |

Chosen: the ultra-fast presets. With `fast` presets the material probe's
renderer took 32.5 s to build cold against 4.0 s for RGBA8.

## Material probe

`material_probe --size 640x360`, five Sideways textures plus the stump's
packed map, renderer creation including pipeline builds:

| Arrays | Cache | Renderer |
| --- | --- | --- |
| RGBA8 | cold | 4.0 s |
| BC7 (ultra-fast) | cold | 5.6 s |
| BC7 | warm (mips and blocks cached) | 2.5 s |
| BC7, streaming | warm | 2.2 s |

The 2x2 dry/wet/snow/moss grid rendered RGBA8 and BC7 at 49.8 dB PSNR to
each other. Holding every layer at the 64-texel streaming tail renders
soft, never black (`stream-tail.png`).

## Sideways

`sideways shot --size 960x540`, seed 1, against `main` at 310872a:

- Clear weather: 38.9 dB PSNR to main. The 24 texture layers become BC7.
  Texture upload ("gpu upload", which includes building the mips): main
  13.5 s cold; this branch 21.5 s cold (BC7 encoding on 4 cores), 8.5 s
  warm with blocks cached.
- Wet weather now sets `Lighting::wetness` 0.85, which also wakes the road
  and verge wet look the Godot port already had in shade.wgsl: darker gravel
  and ruts.

Not measured: real GPU frame cost of the extra material map sample and the
LOD floor lookups, and texture memory on a real device.
