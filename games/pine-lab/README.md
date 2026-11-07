# Pine lab

A row of Jeffrey pines at different ages, regrown whenever a growth rule
changes. Run it with `warp run games/pine-lab`.

- **Mouse and WASD** fly the camera, **Shift** (or E) goes up, **Ctrl** (or Q) down.
- **Arrow up / down** pick a parameter, **arrow left / right** change it
  (a step at a time; hold to keep going), **R** resets it to the default.
- Every tree shows its source meshes at once (a fraction of a second) and
  swaps to its full cook (voxel levels and proxies, as in a forest) when that
  is done; full cooks run one at a time in the background and are cached in
  build/cook-cache. **F** switches to previews only and back.
- **Tab** hides the panel.

Every rule you change is saved in `tuning.json` beside this file (only what
differs from the defaults) and loaded at the next start; delete it to go back
to the defaults. Copy the values you like
into `Species::jeffrey_pine()` (engine/crates/render/src/plant_growth.rs).
The rows of the panel come from `PARAMS` in main.luau: add one line there
to tune another field.

The full cook orders its ages so single-branch seedlings come last: as the
first variant of a GPU forest, a seedling hides every variant after it
(cause still open, see plants.rs).
