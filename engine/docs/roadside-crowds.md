# Roadside crowds prototype

Status: opt-in source foundation, CPU and WGSL validated on 2026-10-24.
No GPU image, timing, motion-oracle or playtest acceptance has been performed.
The existing live session was not interrupted. This is procedural placeholder
quality, not a finished character-art or 100,000-person crowd system.

## Enable and reproduce

Set `SIDEWAYS_CROWDS=1` before constructing a stage with `play` or `shot`.
Omitting the variable keeps the feature off. The Windows development launcher
records the variable in its manifest. Play snapshots (including F12 state) and
shot metadata record actual actor/part/clip counts, atlas size and quality.
The current release candidate intentionally leaves this feature disabled.

The first application creates groups of 6–12 candidates and occasional groups
of 32–48. It searches nearby route positions and both verges, preferring curves,
and can shrink or reject a group if there is no safe space. A single shared world
pool is capped at 256 people / 3,840 rigid parts, independent of player count.
Groups are deterministic from the stage's existing seeds and never consume the
simulation RNG. Height, three torso widths, skin/clothing palettes, facing,
idle/cheer clip, phase and playback rate vary deterministically.

Placement samples the native terrain triangles with their actual diagonal.
Both footprints must have support, a height spread no greater than 5.5 cm,
sufficient road clearance (at least 4.5 m outside the drivable width), no water,
and no nearby source tree/rock/stump or other spectator. The bodies remain
upright; this prototype has no foot IK. Spectators cast ordinary shadows but
are not marked as forest/solid GI proxies and have no gameplay colliders.

## Reusable renderer API

`render::baked_animation::BakedAnimations` contains shared `Clip` atlases,
`Actor` root/phase/rate data and `Part` bindings to ordinary scene instances.
`Scene.baked_animations` is optional. `Renderer::set_animation_time(seconds)`
sets one presentation clock for the world. It is independent of wind strength.

Clips contain periodic frame-major model-space rigid poses, with quaternion
rotation and translation (32 bytes per pose). The crowd application bakes two
48-frame, 15-part clips once: idle and cheer. They share 46,080 pose bytes across
all actors. Rotation uses shortest-arc normalized interpolation; scale is
uniform at the actor root. Body dimensions are baked into reusable mesh assets,
so animation does not introduce sheared normal bases. A future importer can
supply the same rigid pose format; smooth skeletal skinning is not implemented.

One bounded compute dispatch writes each part's live 3×4 instance matrix and
current conservative sphere before shadow and camera passes. Existing source
cluster LOD, per-view frustum/fog culling and visibility-buffer shading are used.
There is no independent animation evaluation per camera and no CPU per-person
update. Animation computes the entire bounded pool even when some people are
offscreen; GPU worklist/animation LOD and distant impostors are future work.
The generic API rejects over 384 actors, 8,192 parts, 16 clips or 65,536 poses.

Parts are appended before the contiguous CPU-updated car instances. The
renderer rejects CPU range updates overlapping animated owners and rejects
owners also driven by foliage wind. Actual poses feed shadow culling and
invalidate cached direct shadows when time changes; that cache cost needs
measurement before choosing a production population budget.

Ordinary rigid motion already uses the current instance inverse and the
previous instance matrix in `motion.wgsl`. Those buffers are snapshotted after
all cameras. The new pass runs before every view and writes the same instance
buffer, so it requires no new motion format. CPU correspondence passes, but
this is not evidence of GPU DLSS correctness: compare animated source pixels
and motion vectors in a dedicated GPU fixture before promotion.

## Verified checks

- `cargo test -p render baked_animation -- --nocapture`: **3 passed**. WGSL/Naga
  and byte layouts; loop continuity; interpolation bounds; rigid current/old
  correspondence; shared pose over four views; invalid/duplicate owner,
  double-deformation, malformed atlas and capacity rejection.
- `cargo test -p cli roadside_crowds -- --nocapture`: **3 passed**. Native-ground
  placement and road clearance, deterministic output and unchanged stage data,
  fixed feet and clip continuity, outward body geometry winding.
- Real generated seed-1, 1,600 m CPU fixture: **81 actors in 7 groups**, group
  sizes 7, 6, 6, 12, 6, 10 and 34; **1,215 shared parts**. This is a placement
  result, not a render-performance measurement.
- Main CLI compiled with the feature's optional hooks. No physics/simulation
  core source was changed. No GPU test or game restart was run by this task.

## Next exclusive-GPU check

The production-path fixture is now `crates/cli/examples/crowds_gpu.rs`:

```powershell
cargo run -p cli --release --example crowds_gpu --features render/dlss -- FRESH_DIRECTORY --cpu-only
cargo run -p cli --release --example crowds_gpu --features render/dlss -- FRESH_DIRECTORY --four-views
```

It enables crowds only in its own process. CPU-only mode constructs the real
placement/animation data and verifies that both mixed idle/cheer and large
groups have usable camera targets; it does not create a GPU. GPU mode uses
`visuals::build_live`, the production meshes/materials/animation pass, and the
actual actor roots to select close, medium and large-group cameras. Native
1920x1080 and DLSS Quality capture the same 16-frame actor-motion sequence;
a second sequence holds actors still and moves only the camera. The optional
four-view capture is 3840x2160 output with four 1920x1080 panels sharing the
same world pool. DLSS failure is an error, never a silent native fallback.

Each saved frame checks the actual post-compute instance matrices, bounds,
identity and planted feet against the CPU quaternion reference. Torso picks
also check that real crowd owners are visible. `report.json` records images,
actual render sizes, per-pass timings and numeric acceptance separately from
visual acceptance. Image inspection remains required; this fixture does not
independently read motion-vector textures or establish a crowd-off/on cost.

1. Construct a fresh stage with `SIDEWAYS_CROWDS=1`; log population and camera
   positions. Inspect a close idle/cheer group, medium group and large group.
2. Read back actual post-compute matrices at times 0, 0.25 and 1.0 and compare
   against `BakedAnimations::sample`, including spheres and planted feet.
3. Render identical one-view and four-view poses: appearance and animation must
   match for equal cameras and world time; check all shadow passes.
4. Freeze time, move only the camera; then freeze the camera and move only the
   actors. Compare native and DLSS motion/edges, especially hands and heads.
5. Measure paired off/on scenes with matched cameras and routes. Report crowd
   compute, raster and shadow cost separately; do not infer 120 FPS support
   from the small atlas or CPU tests. The 4K / four-player / 120 FPS target
   remains unproven.

Full skinning/VAT import, richer faces/clothing, collisions/reactions, crowd
avoidance, impostors, animation LOD and migration to a moving actor pool remain
outside this bounded first implementation.
