# Grain in the traced reflections of a moving surface (open), 12 November

Commit 76f067b9 (froxel fog), Mancave (RTX, Windows), `interior_rooms`,
room 2, the swinging pendant seen from below (camera 0.35,1.65,-8.4, yaw
pi, pitch 0.25), 1920x1080 stills, 4x crops of the shade's underside.

Joep's F4 (19:41) showed a green-grey grain on the inside of the pendant's
shade. It was taken with the old per-room fog, but the grain is not the fog:
it stays with the froxel fog and without fog.

## What causes it

| Switched off | Underside |
| --- | --- |
| nothing | grainy |
| `KERN_RSM=0` | grainy (darker) |
| `KERN_REFLECTION_PROBES=0` | grainy |
| `KERN_FROXEL_FOG=0` | grainy, greyer |
| `KERN_TRACED_REFLECTIONS=0` | smooth |

So it is the stochastic screen-space reflections (`sssr.wgsl`). And only while
the shade moves: frozen at the same pose (pendant time held at 3.333 s) the
underside is smooth, also with TAA's clip gamma at 1 (`taa_gamma=1`), so
TAA's wider box on still pixels is not what hides it.

## Ruled out (tried, not kept)

- **Camera-only reprojection of the reflection history.** The temporal pass
  reprojects through the virtual point at the hit distance, with the camera
  alone. Added the reflector's own motion (a motion-vector pass for the
  opaque surfaces before SSR, `MotionRenderer::opaque`, its delta against
  the camera's part): the delta on the shade is right (checked as a debug
  colour), the grain stays. Reprojecting by the surface motion alone: same.
- **Hit reprojection.** Hits on the moving shade read last frame's image by
  the camera alone; moved by the hit's own motion vector instead: same.
- **Box statistics.** The 3x3 statistics are already on the resolved signal
  (after the ratio-estimator reuse). YCoCg of the tonemapped colour with a
  clip towards the centre instead of a clamp: slightly less, still grainy.
- **Clamp off entirely** (box x100): smooth, but darker than the frozen
  shade, so the history it keeps is not the converged value either.

## What the debug colours show

The underside's rays mostly miss (march confidence low): its reflection is
the probe fallback mixed with a few hits, weighted by the neighbours' mean
confidence. Hits within 0.6 m (the shade on itself) are few. So each pixel
swings between the fallback and sparse hit radiance from frame to frame,
and the clamp follows that noise when the surface moves. Next to try: blend
by each sample's own confidence instead of the neighbourhood mean, and keep
the variance over time per pixel (SVGF moments) for the box.

The attempt's diff is not in the repository (only this note).
