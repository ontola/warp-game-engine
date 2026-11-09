# Clustered shadowed local lights, 8 November 2026

Gap from the UE5 comparison: Warp had 8 shadowed headlights and 32
unshadowed local lights per frame, nothing in between. This adds clustered
local lights (opt-in) and shows them in Spellstorm.

## What

- `render::clustered_lights`: up to 1024 lights per frame
  (`Renderer::set_clustered_lights`), binned on the GPU into a world grid
  (`shaders/clustered_bin.wgsl`), looped per pixel in shade
  (`shaders/clustered_lights.wgsl`). Details in
  [rendering](../rendering.md#clustered-local-lights).
- Shadows: 256-texel tiles in extra layers of the headlights' shadow array,
  cube (6 tiles) or spot (1 tile). Tiles are cached per light id and
  redrawn when the light moves or round robin, 12 per frame by default. A
  tile is drawn with the normal cull and raster (a headlight shadow pass)
  under a viewport, after a full-tile triangle at depth 1 clears it.
- Engine rule (8 storage buffers per stage): shade already read 8 (with the
  wind coefficients), so the arena (lights, faces, cells) is a region at
  the end of the scene tables buffer and only the grid uniform is new, in
  its own bind group 2. A test checks the `shade` entry point's storage
  buffers after the overrides.
- Off (the default), `FEATURE_CLUSTERED_LIGHTS` is 0 and shade compiles
  none of it; the scene tables grow by nothing and the shadow array keeps
  its 8 layers.
- Spellstorm, with `KERN_CLUSTERED_LIGHTS=1`: every lamp (84 in the chapel)
  shadowed, every staff, shrine and spell light (no more top-12 spell cut,
  no 32 nearest lamps). Each lamp excludes its own kit's instances from
  its shadow.

## Checks (cloud container, lavapipe, not a real GPU)

- `cargo test -p render --lib clustered`: grid fit and CPU binning, cube
  face coverage inside the tile guard, atlas tile keeping, redraw order and
  budget, naga validation and the storage buffer count.
- `clustered_lights_probe` (640x360): one clustered light matches the same
  `set_local_lights` light in every pixel (0 differ); 400 lamps light the
  plaza; 24 shadowed lamps (10 get tiles at 4 layers) darken 1823 pixels
  and brighten none; the floor behind a pillar from a shadowed lamp reads 0
  against 126 in front; no light leaves no lit pixel.
- Spellstorm shots, 1280x720, biomes 0, 3, 5: only darker pixels where
  walls now block lamps (chapel: 10382 pixels darker by more than 15/765,
  none brighter).

Not measured: GPU time on a real GPU. Each shadow tile is a full cull and
raster pass, so 12 tiles per frame is the main cost; check on Mancave before
turning it on by default.

## Open

- More shadowed lights: smaller tiles for distant lights, one cull for
  many faces, or caster movement tracking instead of round robin.
- Soft shadows (the headlights' PCSS) for clustered tiles.
- Transparency, volumetric light and the forward renderer ignore clustered
  lights.

## Mancave, 9 November 2026

RTX 5070 Ti, driver 621.14, 2560x1440. Probe passes (0 pixels differ for
one light; 400 lights 0.58 ms shade, with 24 shadowed 0.61 ms plus 0.06 ms
of shadow tiles). Spellstorm chapel, median of 3 interleaved runs: frame
5.94 -> 5.54 ms, shade 5.05 -> 4.52 ms (fewer lights per pixel than the old
flat 32-light loop), clustered shadow cull 0.09 and raster 0.05 ms. Images:
only darker where lamp light used to pass through walls; no acne, seams or
missing lamps. A hard edge reported in the probe's one-lamp image is a
pillar's unlit face (identical without the shadow); the probe now checks
that open floor is not darkened by the shadow. Spellstorm turns clustered
lights on from here.
