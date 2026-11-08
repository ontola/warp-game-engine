# F3 stats overlay

Press **F3** in any game on the app shell (`terrain_fly`, `hello`, Growing
Guns menus that leave F3 alone) to see what a frame costs while you play.
**Shift+F3** cycles the depth of the GPU tree. `WARP_STATS=1` (or 2, 3 for
the depth) starts with it open; headless captures (`terrain_fly --fly N
--fly-shot out.png`) draw it too.

F3 is the game's first: a game whose `Game::function_key(3)` returns true
keeps the key (Growing Guns opens its cheat menu there). F2 stays the
render debug panel ([debug-ui.md](debug-ui.md)) for switching things;
F3 only shows numbers and takes no input.

## What it shows

- **Frame**: time between frames, frames per second and the worst frame of
  the last 120.
- **CPU**: what the main thread spent, from the shell's own spans.
- **GPU**: the measured GPU time of a frame (union of all timed passes,
  so overlaps count once) and its worst recent value.
- **Device, Render, Window**: adapter and backend, each view's rendered
  size and upscaler, the window size when the picture is scaled up.
- **GPU memory**: the allocator report (Vulkan, DX12; Metal and WebGPU give
  none), asked for once a second.
- **Forest pages**: voxel forest pages resident and pending, and their bytes.
- **GPU tree**, sorted by cost at every level, like Unreal's GPU profiler:
  1. feature group (Shadows, Terrain, Vegetation, Geometry, Lighting,
     Post-processing, Effects, Other; `render::debug_timing::pass_group`),
     the union of its passes;
  2. pass, summed over views;
  3. sub-scopes a pass authored (`gpu_profile::Envelope`, e.g. the meadow
     aggregate's depth, material and illumination).
  Rows under 0.02 ms fold into one "smaller passes" row. The bar is the
  share of the GPU frame.
- **CPU tree**: window and input, game ticks, game draw, render encode and
  submit, HUD, acquire and present, GPU timestamp readback, and the rest of
  the frame as waiting (vsync, GPU, OS).
- **Game**: `Game::debug_stats()` lines. `terrain_fly` shows world, camera,
  terrain patches and streaming, and the quadtree update time.
- Its own CPU cost, on the last line.

## Cost

Closed, the overlay does nothing and GPU timestamps stay off (unless F2, a
frame log or a probe turned them on). Open:

- Each timed pass writes two GPU timestamps. They resolve into a four-slot
  readback ring that is mapped a few frames later; a busy ring drops the
  sample instead of waiting, so the CPU never blocks on the GPU and the
  numbers are 2 to 4 frames old.
- Values smooth every sample; the text changes four times a second, so the
  HUD reshapes only a few runs per refresh.
- The GPU memory report lists every allocation, so it runs once a second.

## Adding to it

- A new GPU pass: write timestamps with `timestamp_pair` and push its name in
  `frame.rs` as the other passes do; give it a group in `pass_group` if its
  name does not match one.
- Sub-scopes: an `Envelope` over child timestamp pairs nests them under a
  parent row without extra timestamp writes.
- A game: implement `Game::debug_stats`. It is only called while the overlay
  is open, at most four times a second.
