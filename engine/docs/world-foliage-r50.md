# World foliage replacement — 26 October 2026

Experimental and disabled by default. The normal game retains source trees.
This is a shared whole-tree volume provider with world-grid ray candidates and
GPU group admission before source cluster traversal/raster. It still visits
source owners in retained broadphase leaves; it does not yet remove that work.

## Publication and verification

The engine/game separation, Godot removal, documentation and accumulated engine
work were pushed to main in `79cb3395217c0c9339d2a9d77eab31171831949a`.

Evidence: `C:/dev/perf/warp-sideways-20261026/r50-world-foliage/quick-a`.
The immutable `forest-group-a.exe`, SHA-256 sidecar and source patch relative to
that commit are saved in its parent directory. This run predates the subsequent
mip-parent validation guard; the guard has separate CPU regression coverage.

Quick fixture: 64 existing trees, three shared source assets, native 128x128,
one camera, clear sky, fixed wind at 1.9 seconds. One warmup and two measured
frames per mode. This is a functional diagnostic, not a stable benchmark or
live-game performance claim.

- All four replacement groups were selected; 64 source owners and 12,740
  selected source triangles became zero in the camera pass.
- Solar source selections stayed identical. Switching off the replacement and
  invalidating its provider both restored exact source pixels and selection.
- The volume had 1,286 traced rays, 100,635 iterations, 506,668 volume samples
  and 664,091 occupied cell reads. Zero failed rays or candidate-cache overflows.
- Shared provider memory was 1,133,328 bytes; one retained view target was
  131,232 bytes. New targets are reused unless dimensions change; fresh camera
  selection buffers are rebound without recreating textures/counters.
- Median integration cost was 23.779 ms. The full source interval was 0.313 ms,
  versus approximately 24.1 ms with replacement. The prototype is rejected for
  deployment. The source-tree pixel region also differs visibly: the volume
  is too soft, with mean absolute RGB difference 31.65/255 in that tiny view.

Mechanical success is not optical or performance acceptance. No density,
shadow quality or source default was reduced. No 16K/65K scale run was performed:
the small case already showed an unacceptable cost. No 120 FPS claim is made.

## Required next work

Investigate repeated overlap traversal, per-sample lighting and local ray-cache
pressure. Prefer measured structural reduction over lowering density. Group
aggregation must preserve directional opacity, illumination and current wind.
Before production use, add complete provider/reference validation, a proven
work bound/fallback before source suppression, and temporal correspondence.
Shader reload currently refuses a world provider to avoid mixed lighting
versions. Non-dyadic or missing mip parents are rejected before publication;
the existing conservative hierarchy validator is reused.

## Current session

The user requested an interactive playtest after this diagnostic. GPU fixtures
are stopped. The validated r49 candidate E game was launched with one player,
4K output, DLSS Quality and the existing grass/tree settings. The experimental
world foliage backend is not in that playtest. Active launch evidence is under
`games/sideways/build/dev-runs/20261026T160601393Z-ef7609c3/iteration.json`.
Do not run competing GPU fixtures or restart this playtest without user intent.
