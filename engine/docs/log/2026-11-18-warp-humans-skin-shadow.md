# Warp Humans: sun shadow on skin (18 November 2026)

Scene: `games/human-lab`, `scripts/portraits.json`, 1280x720, lavapipe in the cloud container (not a real GPU). Base: main fc531d64.

Joep pinned the stepped edge of the shadow on a cheek ("dit moet ook fixed").

## Cause

Two things together. The first sun level has 2.3 cm texels (24 m half extent at 2048), and a face is looked at from a metre. And the sun test on skin was lit-or-dark against the flat triangles, with taps clipped to the receiver triangle, while the skin's diffuse term wraps past the terminator: the wrapped light was cut off along triangle edges. Finer texels alone did not change the edge (parked experiment, same day).

## What changed

- Scenes with skin get sun levels of 3, 12, 48 m and doubling (`KERN_SUN_CLOSE`), the fine ones centred ahead of the camera. Without the clipmap the four levels are 3, 12, 48, 96 m instead of 24, 48, 96, 192 m.
- `sun_through_skin` in shade.wgsl measures the skin in the sun's way over sixteen taps; skin uses it for the sun's visibility where the map's texel is under 4 mm (fading out to 10 mm). See `engine/docs/skin-and-hair.md`.
- `SKIN_SPREAD` (diffusion lengths times four) is gone: with texels fine enough to measure a cheek it lit whole cheeks orange.

## Tried and dropped

- Phong lift of the receiver, receiver-plane off, finer texels with the old test: no change.
- One comparison with a B-spline blend of the depths: no false self-shadow, but a hard edge.
- B-spline of sixteen lit-or-dark answers with the receiver plane: soft, but triangles edge-on to the sun climb out of a cast shadow (lit triangles in the shade); with the slope capped, lit slopes shade themselves.
- Residual of each texel against its blurred neighbourhood: contour bands.
- Taps closed up along the slope on a square lattice: four bands (four distinct places across the slope); the spiral has sixteen.

## Cost and open

64 texel reads per close skin pixel for the front, 64 more where the surface faces away. Not measured on a real GPU. Between 12 and 24 m the sun map is coarser than before in scenes with skin (4.7 cm against 2.3 cm a texel without the clipmap). A little texel-sized blockiness is left in the edge at 3.5x zoom. Hair strands cast no shadow yet.
