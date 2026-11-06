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

## Prototype in headless Chrome

Same evening, with the browser shell (`app::web`, `engine/tools/build-web.py`)
in headless Chromium (Playwright's build) on SwiftShader, flags
`--enable-unsafe-webgpu --use-gl=angle --use-angle=swiftshader`. (Chrome's
headless shell with `--use-vulkan=swiftshader` loses the device at the
first canvas present: "Could not find SharedImageBackingFactory"; a bare
WebGPU canvas page does the same, so it is the setup, not Warp.)

- `terrain_fly_bg.wasm` is 7.5 MB; the page also fetches 24 MB of ground
  textures. Wasm memory 379 MB after the first frame.
- `?trees=0`: world built and first frame submitted 2.1 s after load; the
  frame loop ran 1024 frames without a crash.
- With trees the page was still building after 4 minutes: the tree assets
  are cooked from scratch on one thread (natively they come from
  `build/cook-cache`, 264 MB for this forest).
- SwiftShader offers 10 storage buffers per stage, so the `scene` (12) and
  `view cull` (11) layouts fail and nothing is drawn here. A real GPU in
  Chrome offers 16 or more.
- Chrome's WGSL compiler (Tint) rejected, one error per module:
  - `x*a ^ y*b` without parentheses (5 shaders): fixed.
  - Barriers after branches on plain workgroup reads (`cluster_traversal`,
    `cull`): fixed with `workgroupUniformLoad`.
  - Still open: in `cull.wgsl`, barriers in `cluster_traverse` follow early
    returns on per-instance values (`instance_policy`, the traversal's
    sanity checks) that are the same for the whole workgroup but which
    WGSL's uniformity analysis cannot prove. Naga does not run that
    analysis, so native builds never see it. Fixing it means deciding those
    in one lane and sharing them with `workgroupUniformLoad`. Other modules
    may have more: Tint stops at the first error per module.

## Storage buffer limits and the portable renderer (6 November 2026)

- Joep's Mac: Chrome 156 offers 10 storage buffers per stage, Zen
  (Firefox 159) 9. web3dsurvey, macOS: 94 % offer at least 10, 29 % at
  least 16.
- Per pipeline, from a wgpu trace of a native run with trees: shade,
  raster, terrain coarse and cull 16; canopy clipmap 14; voxel forest 15
  and 16; foliage selection 14, refinement 16, materialization 16, branch
  residency 11. Everything else 8 or fewer.
- Portable renderer (`render::portable`, three storage buffers): native at
  `KERN_WEBGPU_LIMITS=mac` (lavapipe) draws the terrain with no errors of
  its own; the full renderer's layouts fail validation 21 times at
  startup. Headless Chrome (SwiftShader, 10): Tint compiles it, frames run
  at about 1.3 s each in software; the canvas cannot be read back there,
  so pixels in a browser still need a real GPU.
