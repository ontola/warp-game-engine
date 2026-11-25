# Low tier on a Samsung Tab S9+, 25 November

Samsung Galaxy Tab S9+ (SM-X810, Adreno 740, 120 Hz), `fps=120`,
`players=1`, 20 s after the first frame. Frame and GPU in ms, render lines.

## Game Room

| Build | Tier | Lines | GPU | fps |
| --- | --- | --- | --- | --- |
| before the low tier | medium | 722 | 14 | 60 |
| 54212a8 | low (`KERN_FORWARD=1`) | 756 | 5.8 | ~111 |
| 54212a8 | low | 1172 | 7.7 | ~104 |
| 7eb072c | low (`auto`) | 722 | 5.2 | ~116 |

## Games, 7eb072c

Default (medium), `KERN_SIMPLE_LIGHTING=1`, and `KERN_FORWARD=1` (which in
7eb072c drew game surfaces as plain tint, so its images were white or
dark: the GPU time is what matters here).

| Game | Default frame (GPU) | Simple | Forward |
| --- | --- | --- | --- |
| Spellstorm | 55.9 (52.0: shade surface 20.7, shade 9.9, shadow raster 6.0) | 34.5 (30.7) | 18.0 (13.7 at 918 lines) |
| Space Racer | 69.1 (65.3: shade surface 29.1, shadow raster 7.1) | 15.7 (10.9) | 10.7 (2.7) |
| Warp Theft Auto | 100 (106.9: shadow raster 55.1, shade surface 10.1, skinning 9.6, near shadow 8.4) | 54.9 (52.0) | not applied (skinned crowd) |
| Dead Miles | 91.4 (86.8: shade surface 60.2, skinning 9.9) | 26.6 (24.1) | not applied (skinned) |

Space Racer picks its biome at random per start, so its rows differ in
biome (forward ran the forest).

## What changed after

e0bd163 runs game surfaces, clustered lamps, blended cards, car paint and
the sky on the low tier, and `auto` keeps game surfaces there: Spellstorm
and Space Racer's city and desert now take the low tier on the phone
hosts. Checked by desktop shots only (lavapipe, image only); tablet
numbers follow. Next levers for the medium tier: shadow rasters (55 ms in
Warp Theft Auto) and skinning (~10 ms).
