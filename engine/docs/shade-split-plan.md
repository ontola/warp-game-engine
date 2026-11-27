# Splitting the full shade pass

Status: plan, 27 November 2026. Nothing below is built yet.

## Why

Growing Guns at 2560x1440 on an M5 Pro (FULL, sky occlusion off): shade
10.9 ms of a 17.3 ms GPU frame. On 3 November it was 5.3 ms. An xctrace of
the bench shows the `shade` compute pipeline spilling 3168 bytes of
registers per thread. The spill grows over the whole kernel, not in one
feature (KERN_SHADE_PROBE stages on the full pass, M5 Pro):

| Stage | Shade ms | Spill |
| --- | --- | --- |
| 2 triangle and normal | 0.52 | 0 |
| 3 material | 1.07 | 1184 B |
| 4 sun shadow | 2.25 | 1776 B |
| 7 + SSAO, contact rays | 3.86 | 1856 B |
| 9 + ambient, irradiance | 6.49 | 2800 B |
| 10 + reflections, sun GGX | 6.77 | 2880 B |
| whole pass | 10.87 | 3168 B |

Compiling out one feature path (KERN_SHADE_FEATURES=-skin_hair,
-procedural, -foliage) saves at most 1.4 ms and 224 B. Removing SSAO,
contact rays and the irradiance volume (probe 11) saves 1.9 ms and 96 B.
The compact pass (KERN_COMPACT_SHADE=1) costs 5.9 ms but loses procedural
stone, wall decals and metal gloss.

## Plan

Two kernels from the same source, chosen by an override constant, the way
`shade_compact_surface` and `shade_compact_light` split the compact pass:

1. `shade` with SHADE_SPLIT=1 (surface): stages 1 to 3 (visibility,
   triangle, material, skin and hair shares) and nothing after. It writes
   two Rgba32Uint targets:
   - base rgb (f16 x3), metal, rough, ao, gloss (f16)
   - n and surface_n and geo_n (octahedral 16-bit each), tex_alpha, uv
   - surface emission (rgb9e5), flags: needles, face
2. `shade` with SHADE_SPLIT=2 (light): reads those instead of running
   stages 2 and 3, rebuilds p, dir and dist from depth, and runs the light
   stage and everything after it. Instance and material come from the
   visibility buffer, as now.

Scenes with car paint (coat, coat_n, glint, car_debug), terrain, grass or
voxel surfaces keep the single pass at first: their extra surface values
would not fit, and Growing Guns, the lobby and Spellstorm have none.

Switch: KERN_SPLIT_FULL_SHADE=1 while measuring; on by default only after
a Mac A/B shows it faster and the images match (`shot` frames compared).

## Status 27 November 2026

`KERN_SPLIT_FULL_SHADE=1` runs the full pass as `shade surface`
(material, `SHADE_SPLIT=1`) and `shade` (light, `SHADE_SPLIT=2`). The
surface result goes in `split_target` (rgba32uint: base, alpha, octahedral
normal, roughness/metal/ao/gloss) and the emission in a storage buffer
(binding 37/38), because a fifth storage texture breaks WebGPU's limit of
4 per stage. Headless llvmpipe at 640x360: max difference 1/255 against
the single kernel. Off by default until the Mac A/B (shade ms, GPU ms,
spill bytes, screenshots) shows it is faster.
