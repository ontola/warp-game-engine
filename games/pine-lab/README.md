# Pine lab

A row of Jeffrey pines at different ages, regrown whenever a growth rule
changes. Run it with `warp run games/pine-lab`.

- **Mouse and WASD** fly the camera, **Q / E** go down and up.
- **Arrow up / down** pick a parameter, **arrow left / right** change it
  (a step at a time; hold to keep going), **R** resets it to the default.
- **F** switches between the fast preview (source meshes, a fraction of a
  second) and the full cook (voxel levels and proxies, minutes for old trees).
- **Tab** hides the panel.

Every change is saved in `tuning.json` beside this file and loaded at the
next start; delete it to go back to the defaults. Copy the values you like
into `Species::jeffrey_pine()` (engine/crates/render/src/plant_growth.rs).
The rows of the panel come from `PARAMS` in main.luau: add one line there
to tune another field.

Known: with the full cook, only the first age of a mixed row shows (seen on
lavapipe; a single age shows). The preview is not affected.
