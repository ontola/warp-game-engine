# terrain_fly in a browser: feasibility, 5 November 2026

Question: can `terrain_fly` run in a browser, built for wasm32 with wgpu's
WebGPU backend? Checked in the cloud container (no GPU; Mesa lavapipe for
the native runs), on main at 604821d plus the changes listed here.

## Compiling for wasm32-unknown-unknown

`cargo check --target wasm32-unknown-unknown -p app --example terrain_fly`
with `RUSTFLAGS='--cfg getrandom_backend="wasm_js"'`:

- `render` compiles as is, except that crates.io meshopt 0.7.0 leaves its
  cluster LOD builder (`clusterlod.h`, C++ with `std::vector`) out of wasm32.
  Fixed with a patched copy, `engine/vendor/meshopt` (see its `WARP.md`):
  small `<vector>`, `<algorithm>`, `<new>`, `<initializer_list>` and
  `math.h` stand-ins, and `operator new`/`delete` from `render/src/web.rs`.
  Native builds compile the same sources as before.
- `app` needed only its clipboard (`arboard`) and gamepads (`gilrs`) gated
  off on wasm32, the way Android already has them.
- What is left is in the example and at run time, not in compiling:
  `terrain_fly` builds its renderer on a thread, and GPU objects cannot
  cross threads on the web.

## GPU limits

`KERN_WEBGPU_LIMITS` (in [renderer switches](../renderer-switches.md))
gives a native device only WebGPU's optional timestamp and first-instance
features (no `primitive_index`, pipeline cache or timestamps inside
encoders) and browser limits. `terrain_fly --bench` at 640x360 on lavapipe:

| Mode | Result |
| --- | --- |
| native limits | runs, 0.1 fps (software) |
| `browser`: 16 storage buffers, 32 sampled textures per stage | runs, 0.2 fps; `--shot` image looks the same (forest, terrain, HUD) |
| `spec`: WebGPU's default limits | fails at the `scene` bind group layout: 12 storage buffers in one stage, limit 8 |

Lavapipe caps storage buffer bindings at 128 MiB, so the renderer already
fits WebGPU's default binding size. web3dsurvey.com (read 5 November 2026)
reports, across WebGPU devices: at least 10 storage buffers per stage on
98 %, 16 on 80 %; at least 44 sampled textures on 92 %; 1 GiB storage
bindings on 92 %. So the browser build reaches about 80 % of devices
without changes, and all of them only if the scene layout drops to 8
storage buffers per stage.

Not checked here: browsers compile WGSL with their own compiler (Tint in
Chrome, which wgpu does not run), which may reject shaders naga accepts.

## Work left for a running build

- Threads: about ten `std::thread::scope` sites in `render` (sun
  heightfield, forest recipes, sky occlusion, irradiance volume, texture
  chains) and the example's own `std::thread::spawn`s. On
  wasm32-unknown-unknown these panic; they need a fallback that runs inline,
  and the renderer must be built on the main thread.
- `std::time::Instant` (133 uses) panics on wasm32: use `web-time`.
- `pollster::block_on` around adapter and device requests: async on the web.
- `app::run` hands `winit` a borrowed game; the web event loop needs
  `spawn_app` with an owned one.
- File reads (`--overrides`, `--from`) and the F12 screenshot path.
- CPU cost while flying: about 64 ms per frame on Mancave at first, since
  cut to about 4.8 ms by the render benchmark work; in a browser it runs on
  one thread and slower than native.
