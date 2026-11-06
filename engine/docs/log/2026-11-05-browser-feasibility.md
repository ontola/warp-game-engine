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

## Packing toward 8 storage buffers per stage (2026-11-06)

Engine rule: every pass must fit 8 storage buffers per stage (WebGPU spec
minimum). Step 1, scene tables: materials, car materials and solid occluders
now live in one `scene_tables` buffer (`scene_tables.rs`) with a header vec4 of
offsets, read through `material(i)`, `car_material(i)` and `solid_occluder(i)`
in WGSL. Shade drops from 16 to 14 storage buffers, the scene group from 12 to
11. Native output is pixel-identical (lavapipe, alpine, with and without
trees). Cull, forward and portable still use the typed `materials` buffer.
Step 2: `Scene::aux` moved into the same buffer (header vec4 1 holds its
start and length; `aux_word(i)`, `aux_len()`). Shade is at 13, still
pixel-identical. Step 3: vertices, cluster vertices and triangles share one
`topology` buffer (`topology.rs`), with vertices at byte 0 and the other two
regions 256-byte aligned so writers (procedural terrain, skinning) and the
portable renderer bind the vertex region alone. The foliage source cache
allocates the same layout. Shade reads it through `vertex_f32(i)`,
`cluster_vert(i)`, `tri_word(i)`; the region starts sit in scene table vec4
1.z and 1.w. Shade is at 11, still pixel-identical.

Step 4: the world terrain hierarchy (built once on the GPU) and the foliage
sun page table (copied in each frame, 4 KiB) are copied into regions of the
scene tables; header vec4 2.x and 2.y say where. terrain_trace.wgsl and
foliage_sun_pages.wgsl now read through `terrain_world()`, `terrain_level(i)`,
`terrain_info()`, `terrain_bounds(i)` and `sun_pages_*()`, defined over the
scene tables in shade.wgsl; the standalone trace query pass gets typed
versions from `terrain_trace::TYPED`. The voxel lighting layout swaps
bindings 24 and 26 for 5. Shade is at 9. Pixel-identical, also with
`KERN_PAGED_FOLIAGE_SUN=1` and on all `world_sun_visibility` images (that
example's split-view check fails the same way on main before this change).

Step 5: the hidden meadow IDs and the sky tile list share one read-write
`ShadeScratch` buffer per view (`shade_scratch.rs`): the meadow's ID buffer
when it skips hidden shading, otherwise the renderer's own. Shade is at 8
storage buffers per stage. Pixel-identical, also with `KERN_SKY_TILES=1` and
on `meadow_shading_gpu` with `KERN_MEADOW_VISIBILITY=1
KERN_MEADOW_SKIP_HIDDEN_SHADE=1`, compared with main before step 1. That
check also caught a step 1 regression: the meadow lighting layout still
listed group 1 binding 12 (solid occluders), so legacy meadows failed to
create their bind group. Fixed here.

Step 6, raster: the draw args and the bucket draw map share one buffer per
view (`ViewTargets::draw`, args at byte 0 so the indirect draws keep their
offsets, the map from byte 256; cull still binds them as two sub-ranges).
The voxel crown replacement words are also in the scene tables (vec4 2.z),
so the main scene layout drops binding 11 (cull's reduced layout keeps it).
The view raster storage bindings are vertex-only, so raster fragments count
only the scene's. Raster now passes at `KERN_WEBGPU_LIMITS=spec`; still
failing there: the view cull layout (11) and shade's 20 sampled textures
(spec allows 16). Pixel-identical.

Step 7, cull (16 storage buffers per pipeline before): cull reads materials
and crown replacements through the scene tables (`cull_material(i)`,
`cull_voxel_replacement_at(i)`), and the counters, draw args and bucket draw
map are one `CullDraw` binding (frame.rs `ViewTargets::counter`: counters at
byte 0, args at 192 so the indirect draws add `ARGS`, the map from 512;
raster reads the same buffer). Cull is at 13. Pixel-identical; the ignored
occlusion GPU tests pass.

Step 8, cull to 10: the cluster BVH, the spatial group members, the sun
owner lists and the refitted spatial groups are copied into one
`cull_tables` buffer (`cull_tables.rs`; members and BVH once, groups after
each refit, sun owners each frame with a static sun map), read through
`cluster_tree_at(i)`, `spatial_member(i)`, `spatial_members_len()`,
`sun_owner(i)`, `spatial_header()`, `spatial_group(i)`. The foliage world
pass maps the same query accessors onto its own members binding.
`KERN_WEBGPU_LIMITS=mac terrain_fly --no-trees` now runs with no GPU
errors. At `spec`, cull is at 10 and shade has 20 sampled textures (16
allowed). Pixel-identical.

Browser rule after step 8: the portable renderer is used below 10 storage
buffers or 20 sampled textures per stage (`render::portable::needed`), so
most Macs (94 % offer 10 or more) get the full renderer without trees.
web3dsurvey.com (November 2026): sampled textures per stage are 44 or more
on 98 % of Macs and 92 % of all devices, so 20 is not a practical limit;
SwiftShader in headless Chrome offers only 16, so it still picks portable
and cannot show the full path here.
