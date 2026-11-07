# Renderer switches

The renderer reads `KERN_*` environment variables at construction. They exist
for A/B comparisons and for experiments that never became the default. This
page lists every one the code reads (checked against the source on
5 November 2026), what it does by default, and what the switch changes. Common
developer variables (`KERN_ADAPTER`, `KERN_SHADER_DIR`, `WARP_HDR`, game
variables) are in [Building and running](building-and-running.md#environment-variables).

A variable only counts when it is set to exactly the value shown; anything
else keeps the default. Set them before the renderer is built (a running game
needs a restart or a new renderer).

## On by default: `=0` turns it off

Use these to compare against the old path. All of them are the normal route.

| Variable | What `=0` does |
| --- | --- |
| `KERN_OCCLUSION` | Turns off two-phase Hi-Z occlusion for every geometry category (clusters not visible last frame are tested against this frame's first-phase depth). `KERN_TREE_OCCLUSION=0` is the older name. |
| `KERN_OCCLUSION_REJECT` | Keeps the occlusion test but never rejects (`KERN_TREE_OCCLUSION_REJECT` is the older name). |
| `KERN_CLUSTER_BUCKETS` | Draws clusters without size buckets. Occlusion needs buckets, so this also disables it. |
| `KERN_RASTER_SPLIT` | Draws opaque clusters through the alpha-tested pipeline again (`discard` everywhere, no early depth). Also off with a game mask and with bounded draws. |
| `KERN_EARLY_GRASS_CULL` | Skips the early grass cull. |
| `KERN_SHADE_CLASSES` | One shade pipeline for terrain and everything else. |
| `KERN_SHADE_UNLIT` | Frames without spot or local lights keep the full shade pipelines instead of the ones built without the local light loop. |
| `KERN_TERRAIN_COARSE` | Turns off the coarse terrain footprint in shading. `KERN_COARSE_FOOTPRINT_M=<m>` sets its size. |
| `KERN_TILED_POST` | Untiled tone mapping and post (same image). |
| `KERN_SKY_VIS_CACHE` | No cached sky transmittance through the canopy. |
| `KERN_CAMERA_SUN_OCCLUSION` | Solar haze ignores terrain between camera and sun ([camera sun fog](camera-sun-fog.md)). |
| `KERN_CUTOUT_LINEAR_RGB` | Alpha-cutout textures mip in sRGB instead of linear RGB. |
| `KERN_GRASS_LEAF_BRDF`, `KERN_GRASS_SUN_OCCLUSION` | Plain grass lighting, no grass self-shadow from the sun. |
| `KERN_GRASS_DEPTH_PREPASS` | No depth prepass for transition grass. |
| `KERN_MEADOW_INDEXED`, `KERN_MEADOW_OCCLUSION` | Unindexed meadow geometry; no meadow occlusion. |
| `KERN_VOXEL_SURFACE`, `KERN_VOXEL_PASS`, `KERN_VOXEL_EMPTY_CEILING` | Voxel forest without its surface pass, separate voxel shade pass, or empty-space ceiling. |
| `KERN_REUSE_SUN_TEXTURE`, `KERN_ZERO_SUN_FAST_PATH` | Recreate the sun texture; shade a sunless scene the long way. |
| `KERN_WIND_FREEZE` | Trees keep moving in the wind at every distance (`off` works too). Default: frozen to the rest pose between 60 and 90 m, so distant crowns match cached sun shadows; `start,end` in metres sets the band. |
| `KERN_PIPELINE_CACHE`, `KERN_TEXTURE_CACHE` | No pipeline cache / no processed texture cache ([texture cache](processed-texture-cache.md)). `KERN_PIPELINE_CACHE_DIR` and `KERN_TEXTURE_CACHE_DIR` move them. |

## Chosen by the scene, forced either way

| Variable | Default |
| --- | --- |
| `KERN_FAR_SUN_SHADOW` | On for `Scene::stage_sun` scenes and (camera-anchored) for streamed GPU-forest worlds; `0` off, `1` forces the older foliage-oriented fit. `KERN_FAR_SUN_LOD`, `KERN_FAR_SUN_REACH` (m, default 1024) tune it. |
| `KERN_WORLD_TERRAIN_VISIBILITY` | `Scene::world_terrain_visibility`. `KERN_TERRAIN_NODE_BUDGET` caps its traversal. |
| `KERN_SIMPLE_LIGHTING`, `KERN_SIMPLE_CULL` | The scene's graphics settings (`terrain_fly` turns simple lighting on). |
| `KERN_BOUNDED_DRAWS` | On for Adreno (Quest) with simple culling. |
| `KERN_VIRTUAL_MEMBERS` | The game's `set_virtual_members` choice for recipe forests. |
| `KERN_SUN_SCENE_DEPTH` | On with a GPU forest. |
| `KERN_VOXEL_RASTER` | On when there are far voxel trees or proxies. |
| `KERN_CAMERA_CUT_FLOOR` | 16 | Lowest camera error-cut scale: voxels start about where a full forest's budget puts them, however few trees there are. 1 disables it. |
| `KERN_FOLIAGE_CUT=budget` | Default: the error-driven foliage cut (each root refines by error alone; the GPU loosens the camera and shadow targets when a cap binds). `budget` restores the budgeted collective admission for A/B. |
| `KERN_FOLIAGE_OCCLUDERS=terrain` | Default: any rigid opaque geometry hides the GPU forest. `terrain` restores land-only occlusion (`terrain_fly --no-building-occlusion`). |

## Tuning values

| Variable | Default | Meaning |
| --- | --- | --- |
| `KERN_SHADOW_REACH` | 180 | Metres of dynamic sun shadow. |
| `KERN_NEAR_SHADOW_LOD` | 1.0 | Scale on near shadow cluster error. |
| `KERN_SUN_VOXEL_M` | per scene | Voxel cell size in the sun map. |
| `KERN_SUN_CACHE_GUARD_METRES` | 0 | Guard band around the cached sun map (0 to 128 m). |
| `KERN_FOLIAGE_SUN_LOD`, `KERN_FOLIAGE_SUN_PAGE_METRES` | 0.5, fine pages | Paged foliage sun (only with `KERN_PAGED_FOLIAGE_SUN=1`). |
| `KERN_VOXEL_SELF_SHADOW_COARSER` | unset | Coarser voxel self-shadow mip. |

## Diagnostics

| Variable | Effect |
| --- | --- |
| `KERN_CPU_SPANS=1`, `KERN_STARTUP_PROFILE=1` | CPU time per frame phase; renderer construction profile. |
| `KERN_DIAGNOSTIC_HIDE` | Hide geometry categories in captures. |
| `KERN_MEADOW_CAPTURE_RAW=1` | Sideways: raw meadow buffers in the capture JSON. |
| `KERN_PORTABLE=1` | Draw with the portable renderer (`render::portable`: procedural terrain, sky and haze, three storage buffers per stage) as a device below 10 storage buffers or 20 sampled textures per stage does (`render::portable::needed`). Default off; such a device always uses it. See [Running in a browser](browser.md). |
| `KERN_WEBGPU_LIMITS=spec`, `=browser`, `=mac` | Native device with only what a browser's WebGPU gives: the timestamp and first-instance features, and the spec's default limits (`spec`), what about 80 % of WebGPU devices offer (`browser`: 16 storage buffers and 32 sampled textures per stage, 1 GiB buffers) or `browser` with the 10 storage buffers most Macs offer (`mac`), never above the adapter. GPU errors are printed once each and do not stop the run, so one run lists every layout over the limits. For checking a [browser build](log/2026-11-05-browser-feasibility.md) without a browser. |
| `KERN_DLSS_RUNTIME_DIR` | Where to find `nvngx_dlss.dll` ([DLSS](../crates/render/native/DLSS.md)). |

## Experiments: off by default, `=1` opts in

None of these is the normal route. Most were measured and not promoted; the
note linked says why. They are candidates for removal from the code.

| Variable | Experiment | Note |
| --- | --- | --- |
| `KERN_INSTANCE_GROUPS`, `KERN_SUN_INSTANCE_GROUPS`, `KERN_HEADLIGHT_INSTANCE_GROUPS` | GPU hierarchy of spatial instance groups | [r45](log/instance-hierarchy-r45.md) |
| `KERN_CLUSTER_TRAVERSAL` | Top-down cluster traversal (`terrain_fly` turns it on unless set) | [r46](log/cluster-traversal-r46.md) |
| `KERN_INSTANCE_BROADPHASE` | Compact colour-view instance selection | [broadphase](log/instance-broadphase.md) |
| `KERN_STATIC_SUN_CACHE` | Static/dynamic split sun cache | [r45](log/static-sun-cache-r45.md) |
| `KERN_RECEIVER_SHADOW_DEMAND` | Receiver-driven shadow tiles | [r49](log/lod-shadow-demand-r49.md) |
| `KERN_PAGED_FOLIAGE_SUN` | Paged foliage sun map (excludes the far sun map) | [world lighting](log/world-lighting.md), [distant tree light](log/distant-tree-lighting.md) |
| `KERN_ASYNC_SHADOW_VALIDATION` | Asynchronous shadow cache validation | [r61](log/frame-hitches-r61.md) |
| `KERN_SUN_RECEIVER_PLANE`, `KERN_SUN_TERRAIN_ERROR`, `KERN_SUN_SURFACE_SOURCE`, `KERN_ROCK_VARIATION` | Low-sun shadow and rock variants | [sun shadow diagnostics](log/sun-shadow-diagnostics.md) |
| `KERN_OWNER_RANGE_INDEX` | Indexed owner ranges | [terrain forest streaming](log/terrain-forest-streaming-2026-11-03.md) |
| `KERN_RASTER_LARGE_FIRST` | Raster large clusters first | [handoff log](log/handoff.md) |
| `KERN_SPLAT_BRICK_HZB` | Hi-Z culling of voxel splat bricks | [voxel brick culling](log/voxel-brick-culling-2026-11-03.md) |
| `KERN_VOXEL_BRICK_VISIBILITY`, `KERN_VOXEL_GLOBAL_INTERVALS` | Voxel brick visibility stream | [brick visibility](log/vegetation-brick-visibility.md) |
| `KERN_BRICK_EMPTY_SKIP`, `KERN_BRICK_EMPTY_JUMP`, `KERN_BRICK_SMOOTH_MIPS` | Optical brick empty-space skipping and mips | [r47](log/meadow-work-r47.md), [r48](log/render-traversal-r48.md), [r49](log/lod-shadow-demand-r49.md) |
| `KERN_MEADOW_AGGREGATE` (+ `_HALF_RES`, `_SAMPLE_PRUNE`), `KERN_MEADOW_SUBSTRATE` | Distant grass aggregate and its substrate | [r39](log/meadow-distance-r39.md), [r57](log/meadow-half-resolution-r57.md), [r58](log/meadow-sampling-r58.md) |
| `KERN_MEADOW_GROUP_CACHE` (+ `KERN_MEADOW_GROUP_REPLACE`) | Meadow world-page cache | [r45](log/meadow-group-cache-r45.md) |
| `KERN_MEADOW_VISIBILITY` (+ `KERN_MEADOW_SKIP_HIDDEN_SHADE`) | Meadow visibility pass | [r47](log/meadow-work-r47.md) |
| `KERN_MEADOW_SPATIAL_FILTER`, `KERN_MEADOW_CONTACT_AO`, `KERN_MEADOW_EDGE_SHADOW` | Grass filtering and contact shading | [r59](log/meadow-contact-ao-r59.md), [r64](log/meadow-edge-grain-r64.md) |
| `KERN_DIRECT_DRAWS`, `KERN_SKY_TILES`, `KERN_NEAR_SUN_SOURCE`, `KERN_SOURCE_CROWN_BOUNDS`, `KERN_CANOPY_COMPACT_SUPPORT`, `KERN_LEGACY_LIGHT_LOOKAHEAD`, `KERN_VOXEL_ANTIPODAL_NORMALS` | Direct draw arguments, tiled sky light, near sun from source geometry, crown bounds, legacy headlight lookahead, voxel normals | No note describes these; read the code before relying on them. |
