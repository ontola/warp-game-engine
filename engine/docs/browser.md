# Running in a browser

`terrain_fly` builds for the browser: wasm32 with wgpu's WebGPU backend,
drawn into a canvas. It is a prototype: it loads, builds the world and runs
its frame loop in Chrome. The cull shader now compiles there, but a drawn
frame has not been confirmed since (see Open). It uses the same renderer as the native
build, without threads, gamepads, the F10 menu, screenshots, HDR or
adaptive quality.

## Build and open it

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # the version in Cargo.lock
python3 engine/tools/build-web.py --serve
```

Then open http://localhost:8790 in Chrome or Edge (WebGPU needs localhost or
HTTPS). `?style=alpine` and `?seed=2` pick the world; trees are off unless
`?trees=1` (cooking them freezes the page for minutes, see Open). Hosts
that pass only a fragment get the same from words in it: `#alpine-trees`.
Click to capture the mouse; WASD and the mouse fly, F2 to F9 work as
natively, Esc releases the mouse.

The page shows a progress bar until the first frame: the wasm download
(with bytes), the ground textures, building the world, WebGPU setup and
the first upload. The log sits in the corner.

`build/web` holds `index.html` (from `engine/crates/app/web`), `pkg/` (the
wasm and its JavaScript) and `assets/` (the ground textures). Any static
host serves it.

## What the browser needs

- Engine rule (Joep, 6 November 2026): every pass fits 8 storage buffers
  per shader stage, the spec minimum. Data a pass reads is packed into a few
  shared buffers read through WGSL accessor functions (`scene_tables.rs`,
  `topology.rs`, `shade_scratch.rs`, `cull_tables.rs`, the cull `CullDraw`
  binding); optional features keep their own bind group. Check a change with
  `KERN_WEBGPU_LIMITS=spec` ([renderer switches](renderer-switches.md)).
- Where it stands: shade, raster and cull fit 8 (cull leaves out the
  opt-in receiver shadow demand, which adds a ninth); shade needs 24
  sampled textures (the live bounce of clustered lights and the reflection
  probes added three; they belong in a bind group or texture array of
  their own before a browser build) (spec 16, but 98 % of Macs and 92 % of all WebGPU
  devices offer 44 or more, web3dsurvey.com, November 2026). The GPU forest
  (trees) still needs 16 storage buffers. Storage buffers on macOS: 94 %
  offer 10 or more, 29 % 16; Firefox (Zen) gives 9, Chrome's software
  adapter (SwiftShader) 10 with 16 textures.
- With fewer than 8 storage buffers or 24 sampled textures
  (`render::portable::needed`) the renderer switches to its portable path
  (`render::portable`, `KERN_PORTABLE=1` forces it natively): the
  procedural terrain drawn straight from its patch pool, textured with a
  simplified `field_surface`, a sky gradient and haze, with three storage
  buffers. No shadows, trees, grass, cars, GI or post-processing. The full
  renderer is still built next to it, so the console shows its layouts
  failing validation once at startup. `KERN_WEBGPU_LIMITS=mac` or `=spec`
  runs this natively.
- Optional features are used when there: timestamps; `primitive_index`
  (meadow visibility) has a fallback. There is no pipeline cache.

## How the build differs

- `app::web` is the shell: it takes the canvas `#warp` (or adds one),
  requests the adapter and device asynchronously
  (`Gpu::for_surface_async`), and draws on the browser's frame callbacks.
  The canvas has no sRGB format, so the shell renders into an sRGB view of it.
- No threads: `render::par::scope` is `std::thread::scope` natively and runs
  each spawned job at once in a browser. `terrain_fly` builds scenes,
  forests and sun bakes inline there (`background`), which stalls the page
  while it works; `app::web::yield_to_browser` lets the progress bar paint
  before each long step.
- Time comes from `web_time::Instant` (`std::time::Instant` natively).
- Files: the page fetches the textures first and hands them to
  `render::files::preload`; `render::files::read` and `open_image` find them
  by file name. Natively these read the filesystem.
- meshoptimizer's cluster LOD builder is C++ that needs parts of the C++
  standard library; `engine/vendor/meshopt` supplies them on wasm32 (see its
  `WARP.md`), and `render/src/web.rs` supplies `operator new`/`delete`.

## Open

- Trees: their cooked assets are cached on disk natively (`build/cook-cache`,
  about 260 MB for one forest); a browser cooks them at every load.
- Shaders are compiled by the browser's own WGSL compiler (Tint in Chrome),
  which runs WGSL's uniformity analysis; naga does not. In `cull.wgsl` lane 0
  decides whatever guards a barrier and shares it with
  `workgroupUniformLoad`. Other modules have not been checked the same way.
  Test in Chrome after changing a compute shader with barriers.
- Measurements and the feasibility check:
  [log entry](log/2026-11-05-browser-feasibility.md).
