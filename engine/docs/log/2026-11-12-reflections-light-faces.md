# Reflections: light faces out of the mips, a real crossing for hits, 12 November

Machine: mancave, RTX 5070 Ti. Scene: `interior_rooms`, spot
`r2_pendant_table` (Joep's F4: room 2, pendant only, room 3 dark beyond the
doorway): yellow-green speckles over the table top and on room 3's floor
just past the doorway, under the bulb on screen.

## Diagnosis

- `KERN_TRACED_REFLECTIONS=0`: gone, so the traced path, not the analytic
  specular, the live bounce or the miss fallback.
- `KERN_SSSR=0` (single mirror ray): a uniform yellow band straight under
  the bulb over the table and the floor beyond the doorway.
- A stricter crossing test alone (below) left both. The band and speckles
  came from the reflection mips: hits on the wall and shade next to the
  bulb read a coarse level, where the bulb's face (EMISSIVE, its light
  stands in as an analytic light) was averaged into everything near it.
  `light_face_at` only rejected a hit exactly on the face's pixel.
- New diagnostic: `Lighting::debug` 26 shows each pixel's reflection hit
  (screen position, confidence); `interior_rooms --debug N`.

## Changes

- Shade writes alpha 0 on light faces; the mip chain stores colour times
  the share that is not a face, readers (shade, sssr resolve) divide it out.
- The march's bisected candidate counts only where the ray meets the depth
  buffer's surface (gap within the ray's depth change, the surface's depth
  slope across the pixels between the ends, and 0.2 % of the depth) and the
  surface faces the ray; otherwise it marches on. Before, a fixed 0.2 m or
  3 % behind was accepted after bisection.

## Measurements (4K)

| pass | before | after |
| --- | --- | --- |
| reflection mips | | 0.15 ms |
| sssr resolve | | 0.88 ms |

`--pan 240` at r2_pendant_table: mean frame-to-frame change 0.212 (before
0.221). Suite: r2_pendant_table and r2_box clean (the table and cabinet
speckle the earlier suite noted are gone); room 3 lamp views change a little
(the panels reflect through their analytic specular only); the rest within
0.1 of 255.
