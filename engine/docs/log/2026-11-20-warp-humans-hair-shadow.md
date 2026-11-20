# Warp Humans: hair casts a shadow

2026-11-20, cloud container (lavapipe), human-lab at 30, all eight people
from four sides.

Playtester: shadows are missing, and can it be cheap. Strands received the
sun map but were not in it, so a fringe or an afro left no shadow on the
forehead.

- A quarter of each head's grown strands, as ribbons across the sun's
  direction at least one texel wide, drawn depth only into the sun levels
  with an extent up to 12 m (texels 3 mm and 12 mm), right after the shadow
  raster. The density pass that picks strand counts for the camera picks
  them for the map too (slot 16), so a head small in the 12 m level casts
  fewer. No new buffers or textures.
- Result: fringes, partings and the afro shade the forehead and temples.
  Hair itself changed little: strands sample the map 3 cm out along the
  head's normal, which skips most of their own shadow.

Not measured on a real GPU. Worst case is a quarter of the camera's strand
vertices again per level drawn (16,384 strands a head: 220,000 vertices a
head and level), with no fragment shader. Open: time it on Mancave; depth
inside thick hair (smaller sample offset); other grooms cast by their mesh.
