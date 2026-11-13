# Five storage textures in one stage: terrain_fly would not start, 13 November

Mancave (RTX 5070 Ti, Windows), `main` from 76f067b9 (froxel fog) to
e87a45e7.

## What failed

`terrain_fly` stopped at startup in `create_pipeline_layout`: "Too many
bindings of type StorageTextures in Stage COMPUTE, limit is 4, count was
5". The froxel fog put the cells `fog_inject` writes (binding 28) in
shade's view group (group 1), next to the shade output (8), the
reflection rays (22) and the live bounce's gather (24). The
`terrain_coarse` pipeline adds its own output (group 3, binding 32): five.
`interior_rooms` has no procedural terrain and never built that pipeline,
so it ran.

## The change

The fog cells moved to a group of their own (group 3 of `fog_inject`, the
only pass that writes them). Shade's view group carries three storage
textures again. `frame::shade_storage_textures` counts, for every compute
pipeline layout built from shade's module (shade, terrain coarse, fog
inject, sky visibility bake), the storage textures each stage sees; a test
in `portable.rs` keeps it within wgpu's default of 4, which the renderer
requests on every device.

## Checked

- `cargo test -p render --lib`: 913 passed.
- `terrain_fly --bench 60` at 4K starts and runs (terrain coarse 0.44 ms).
- `interior_rooms` with fog, room 2 under the pendant: renders as before;
  fog inject 0.82 ms at 4K.
