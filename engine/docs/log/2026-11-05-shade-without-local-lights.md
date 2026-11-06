# Shading without local lights, 5 November 2026

Commits 2a360aa and 296c9c3 (branch `claude/project-thread-lbfdp3`, PR #10).

## Why

On the Mac (Apple M5 Pro, `terrain_fly --town`, 2296x1440) the full-screen
shade passes were 3.2-3.5 ms (`shade`) and 2.3 ms (`shade terrain`) on the
wall view. Stubbing features one at a time showed that code which does nothing
at runtime still costs time through register pressure. The local light loop
(headlights, `set_local_lights`) runs for no pixel in a scene without lights,
but it is compiled into every shade pipeline.

## Change

- `FEATURE_LOCAL_LIGHTS` override in `shade.wgsl` guards the headlight and
  local light calls.
- `frame.rs` builds a second shade and shade terrain pair with it off, on a
  worker thread, the first time a frame has no lights. Frames keep the full
  pipelines until the pair is ready. `KERN_SHADE_UNLIT=0` turns it off.

## Measurements

Mac, 2296x1440, still camera, last 60 of 120 frames, median of 4 interleaved
runs, fc759df (inline build) against `KERN_SHADE_UNLIT=0`:

| View | `shade` | `shade terrain` | GPU frame | p50 |
|---|---|---|---|---|
| Wall, unlit | 2.40 | 1.95 | 17.43 | 20.95 |
| Wall, full | 2.73 | 2.04 | 17.86 | 21.30 |
| Hill, unlit | 0.15 | 7.30 | 58.43 | 64.50 |
| Hill, full | 0.16 | 7.67 | 58.85 | 64.75 |

Image: wall 0 pixels over 4 levels (max 3, same as run-to-run noise), hill
2 pixels over 4 (max 7).

RTX 5070 Ti, Vulkan, 2560x1440: at most 0.05 ms, which is noise. The gain is
specific to the Mac's register allocation.

The first launch of the fc759df binary on the Mac took 104.5 s on its first
frame, once, and could not be reproduced (later first frames 1.4-2 s in both
modes). It may have been a cold Metal compile of the new variants inside the
frame, which is why 296c9c3 moves the build to a worker thread. Headless on
lavapipe the worker's pipelines are in use from the third frame, and frames
0 to 5 match `KERN_SHADE_UNLIT=0` exactly.
