# Foliage visibility test fixture

`authored_pine_albedo.png` is a pinned, byte-identical copy of the retained
foliage texture used by the renderer's source-crown visibility regression.
It accompanies `../fir_a_near.kmesh`. Keeping this explicit test asset here
allows engine tests to run without the Sideways game assets or working directory.

The texture is derived from **Pine Tree** by **evolveduk**:

- Author: https://sketchfab.com/evolveduk
- Source: https://sketchfab.com/3d-models/pine-tree-d45218a3fab349e5b1de040f29e7b6f9
- License: Creative Commons Attribution 4.0 International; see
  `LICENSE-CC-BY-4.0.txt` and the retained `UPSTREAM-LICENSE.txt`.
- Existing modification: texture resolution capped at 1024 pixels. No further
  texture modification was made for this fixture.

No endorsement by the original author is implied. The fixture preserves the
actual alpha mask; substituting an opaque or synthetic mask would weaken the
clear-ray and bare-stem regression.
