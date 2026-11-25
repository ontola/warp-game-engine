# Renderer switches

The renderer reads `KERN_*` environment variables at construction. They exist
for A/B comparisons and for experiments that never became the default. This
page lists every one the code reads (a test in `render/src/switches.rs`
checks that every variable declared there appears here or in
[Building and running](building-and-running.md#environment-variables)), what it
does by default, and what the switch changes. Common
developer variables (`KERN_ADAPTER`, `KERN_SHADER_DIR`, `WARP_HDR`, game
variables) are in [Building and running](building-and-running.md#environment-variables).

A variable only counts when it is set to exactly the value shown; anything
else keeps the default. Set them before the renderer is built (a running game
needs a restart or a new renderer).

## On by default: `=0` turns it off

Use these to compare against the old path. All of them are the normal route.

| Variable | What `=0` does |
| --- | --- |
| `KERN_SUN_CLIPMAP` | Restores the separate sun maps (shared, near levels, anchored far map) in streamed GPU-forest worlds instead of one cached clipmap of eight levels, 24 m to 3 km, first level growing with height (on since 9 November; [sun shadows](rendering.md#sun-shadows), [clipmap log](log/2026-11-08-sun-clipmap.md)). |
| `KERN_OCCLUSION` | Turns off two-phase Hi-Z occlusion for every geometry category (clusters not visible last frame are tested against this frame's first-phase depth). `KERN_TREE_OCCLUSION=0` is the older name. |
| `KERN_OCCLUSION_REJECT` | Keeps the occlusion test but never rejects (`KERN_TREE_OCCLUSION_REJECT` is the older name). |
| `KERN_CLUSTER_BUCKETS` | Draws clusters without size buckets. Occlusion needs buckets, so this also disables it. |
| `KERN_RASTER_SPLIT` | Draws opaque clusters through the alpha-tested pipeline again (`discard` everywhere, no early depth). Also off with a game mask and with bounded draws. |
| `KERN_EARLY_GRASS_CULL` | Skips the early grass cull. |
| `KERN_FAR_TREE_SHADOWS` | The coarse maps shade the far forest ground instead of per-tree ground shadows (`shade.wgsl`). |
| `KERN_VOXEL_STAMPS` | Voxel cooks sample every thin-foliage placement instead of reusing one stamp per part and material (`vegetation_aggregate.rs`). |
| `KERN_FAR_FOREST_MEASURED` | `terrain_fly` paints the far part of a pine forest with the generic biome canopy instead of the grown pines' measured crowns. |
| `KERN_SHADE_CLASSES` | One shade pipeline for terrain and everything else. |
| `KERN_SHADE_UNLIT` | Frames without spot or local lights keep the full shade pipelines instead of the ones built without the local light loop. |
| `KERN_TERRAIN_COARSE` | Turns off the coarse terrain footprint in shading. `KERN_COARSE_FOOTPRINT_M=<m>` sets its size. |
| `KERN_TILED_POST` | Untiled tone mapping and post (same image). |
| `KERN_LIGHT_TILES` | The simple and compact shade passes loop every local light per pixel instead of only those reaching the pixel's 8x8 tile (same image). |
| `KERN_SKY_VIS_CACHE` | No cached sky transmittance through the canopy. |
| `KERN_CANOPY_CLUMPING` | Crowns as a uniform fog at the coarse canopy levels again (a forest floor far away then sees a fifth of its sky; `canopy.rs` clumping_on). |
| `KERN_VOXEL_GAP_MIPS` | Coarse tree voxel levels add their children's leaf area again (a sparse branch cooks into a near-solid ball; `vegetation_aggregate.rs` gap_preserving_area). Part of the cook key, so it recooks. |
| `KERN_THIN_PIECES` | Needle brushes and other foliage made of many separate pieces simplify by edge collapse again: their first LOD is ~8 cm off, so near and mid trees draw every needle (~10M triangles per spruce) and fill the source cluster budget (`cluster.rs` piece_levels). Changes the cooked meshes, so it recooks. |
| `KERN_SOURCE_PER_TREE` | The camera's step from branch voxels to needles is judged per branch again instead of per tree, so near crowns mix needles and 4-16 px voxel branches (`foliage_refine.wgsl` fast_round). |
| `KERN_TRACED_REFLECTIONS` | No traced reflections: glossy surfaces reflect only the sky and the heuristic ground bounce ([dynamic-gi.md](dynamic-gi.md)). |
| `KERN_SSSR=0` | The single mirror-ray traced reflection instead of the stochastic one (sssr.rs): one ray per pixel blurred by mip, no spatial or temporal resolve ([dynamic-gi.md](dynamic-gi.md)). |
| `KERN_RSM=0` | No live bounce for clustered lights marked `with_bounce` (rsm.rs); `KERN_RSM=open` keeps it but drops its visibility walk, so it passes walls ([interior-light.md](interior-light.md#live-bounce-of-moving-lights)). |
| `KERN_FROXEL_FOG=0` | No froxel fog even where `Lighting::froxel_fog` is set ([volumetric-light.md](volumetric-light.md#froxel-fog)). |
| `KERN_PROBE_GRADIENTS=0` | Interior probes interpolate trilinearly even where gradients were uploaded (`Renderer::set_probe_gradients`, [interior-light.md](interior-light.md#noise-without-blur)). |
| `KERN_REFLECTION_PROBES=0` | No live reflection probes (`Renderer::set_reflection_probes`, reflection_probes.rs): no capture, glossy surfaces fall back to the sky and irradiance volume where screen-space reflections miss ([dynamic-gi.md](dynamic-gi.md#reflection-probes)). |
| `KERN_CAMERA_SUN_OCCLUSION` | Solar haze ignores terrain between camera and sun ([camera sun fog](camera-sun-fog.md)). |
| `KERN_CUTOUT_LINEAR_RGB` | Alpha-cutout textures mip in sRGB instead of linear RGB. |
| `KERN_GRASS_LEAF_BRDF`, `KERN_GRASS_SUN_OCCLUSION` | Plain grass lighting, no grass self-shadow from the sun. |
| `KERN_GRASS_DEPTH_PREPASS` | No depth prepass for transition grass. |
| `KERN_MEADOW_INDEXED`, `KERN_MEADOW_OCCLUSION` | `KERN_MEADOW_INDEXED=0` selects unindexed meadow geometry. `KERN_MEADOW_OCCLUSION=0` skips hierarchical depth culling. Bounded mown lawns with a draw range of 24 m or less skip it by default and keep the normal depth test. `KERN_MEADOW_OCCLUSION=1` forces hierarchical culling for those lawns too. |
| `KERN_VOXEL_SURFACE`, `KERN_VOXEL_PASS`, `KERN_VOXEL_EMPTY_CEILING` | Voxel forest without its surface pass, separate voxel shade pass, or empty-space ceiling. |
| `KERN_REUSE_SUN_TEXTURE`, `KERN_ZERO_SUN_FAST_PATH` | Recreate the sun texture; shade a sunless scene the long way. |
| `KERN_WIND_FREEZE` | With `0` or `off`, trees move in the wind at every distance. The default fade starts at 60 m and reaches rest at 90 m. Supply `start,end` in metres to change the band. Games can adjust it at runtime with `Renderer::set_wind_freeze`. Whole crowns and articulated growth rigs sample the same wind travelling downwind at 12 m/s. At high strength, irregular gusts change duration, pressure and direction across the planting. The local branch pose fades with distance while keeping the arrival time and direction of the same gust. Each planted root in a `Scene::tree_wind_batches` stand gets its own fade and shear, shared by colour, sun depth and motion vectors. Every flexible joint filters the recent wind pressure as a damped spring. The joint stiffness sets its bend and natural frequency, with a small frequency variation from branch identity. Fine branches respond to light wind and oscillate faster than their supporting boughs. Rigid growth roots stay anchored. |
| `KERN_TEXTURE_COMPRESSION` | Albedo, normal and material map arrays stay RGBA8 instead of BC7 ([materials](materials.md#compression)). |
| `KERN_PIPELINE_CACHE`, `KERN_TEXTURE_CACHE` | No pipeline cache / no processed texture cache ([texture cache](processed-texture-cache.md)). `KERN_PIPELINE_CACHE_DIR` and `KERN_TEXTURE_CACHE_DIR` move them. |

## Chosen by the scene, forced either way

| Variable | Default |
| --- | --- |
| `KERN_FAR_SUN_SHADOW` | On for `Scene::stage_sun` scenes and (camera-anchored) for streamed GPU-forest worlds; `0` off, `1` forces the older foliage-oriented fit. `KERN_FAR_SUN_LOD`, `KERN_FAR_SUN_REACH` (m, default 1024) tune it. |
| `KERN_WORLD_TERRAIN_VISIBILITY` | `Scene::world_terrain_visibility`. `KERN_TERRAIN_NODE_BUDGET` caps its traversal. |
| `KERN_SIMPLE_LIGHTING`, `KERN_SIMPLE_CULL` | The scene's graphics settings (`terrain_fly` turns simple lighting on). |
| `KERN_COMPACT_SHADE` | `1`: the compact shade pass (simple lighting plus game surfaces and sun shadows) even where the GPU binds enough textures for the full one. The Android phone host turns it on. |
| `KERN_SPLIT_SHADE` | `1`: the compact shade pass as two smaller kernels, the surface (triangle, material, albedo, sun) into an Rgba32Uint target and then the light (GI, lamps, styling, fog) from it. The Android phone host turns it on: one kernel's registers let an Adreno 740 run few pixels at a time (1.45x faster). `2` also splits the light half: sky, GI and sun into an Rgba16Float target, then the lamps and the rest. |
| `KERN_PHONE_GI` | `1`: the irradiance volume blends its eight probes by distance and facing only (no DDGI reach masks, probe visibility or gradients): about a third of the compact pass on an Adreno 740. The Android phone host turns it on; light can leak through a thin wall. |
| `KERN_CLUSTERED_LIGHTS` | `Graphics::clustered_lights` (off): clustered local lights, hundreds per frame, some shadowed ([rendering](rendering.md#clustered-local-lights)). `1` turns them on, `0` off. |
| `KERN_BOUNDED_DRAWS` | On for Adreno (Quest) with simple culling. |
| `KERN_VIRTUAL_MEMBERS` | The game's `set_virtual_members` choice for recipe forests. |
| `KERN_SUN_SCENE_DEPTH` | On with a GPU forest. |
| `KERN_VOXEL_RASTER` | On when there are far voxel trees or proxies. |
| `KERN_CAMERA_CUT_FLOOR` | 4 | Lowest camera error-cut scale: voxels start sooner than a few trees' budget alone would put them. 1 disables it; 16 (the old default) picks coarse per-branch voxel levels that show as smooth balls on grown pines at 10-30 m. |
| `KERN_COOK_THREADS` | machine threads | Worker threads for voxel cooks (`render::par`). A plant's branches and geometry groups cook in parallel; nested and concurrent cooks share the count. |
| `KERN_FOLIAGE_CUT=budget` | Default: the error-driven foliage cut (each root refines by error alone; the GPU loosens the camera and shadow targets when a cap binds). `budget` restores the budgeted collective admission for A/B. |
| `KERN_TEXTURE_STREAMING` | `Scene::texture_streaming` for scene textures, on for generated faces in native interactive sessions. `1` streams eligible layers and `0` uploads every mip at creation ([materials](materials.md#streaming)). |
| `KERN_FOLIAGE_OCCLUDERS=terrain` | Default: any rigid opaque geometry hides the GPU forest. `terrain` restores land-only occlusion (`terrain_fly --no-building-occlusion`). |

## Tuning values

| Variable | Default | Meaning |
| --- | --- | --- |
| `KERN_SHADOW_REACH` | 180 | Metres of dynamic sun shadow. Without it the shared map covers at least 96 m each way; a reach set here or by the game drops that floor (16 m steps). |
| `KERN_SUN_SKINNED_M` | 24 on Android, off elsewhere | Skinned bodies cast into the shared sun map only within this many metres of a camera (the near map still has them). A Warp Human is ~40k triangles with no coarser level: in Warp Theft Auto on a Tab S9+ they were 2.3 M of 2.5 M sun triangles. Phones also fit only a quarter of each human pool's optional slots (`humans::PHONE_SHARE`). |
| `KERN_CLUSTERED_SHADOW_LAYERS` | 4 | 1024-texel layers of 256-texel shadow tiles for clustered lights (16 tiles, 4 MiB each; at most 16). A point light takes 6 tiles, a spot 1. |
| `KERN_CLUSTERED_SHADOW_FACES` | 12 | Clustered shadow tiles redrawn per frame at most (moved lights first, then the oldest). |
| `KERN_NEAR_SHADOW_LOD` | 1.0 | Scale on near shadow cluster error. |
| `KERN_SUN_VOXEL_M` | per scene | Voxel cell size in the sun map. |
| `KERN_SUN_LEVELS` | 8 (clipmap), 4 with `KERN_SUN_CLIPMAP=0` | Sun clipmap levels round each camera (1 to 8; level k covers 24 m times 2^k, with `KERN_SUN_CLIPMAP` counted from a first level that grows with height). |
| `KERN_SUN_CLOSE` | on | `0` keeps the doubling ladder (24 m first level) in scenes with skin. By default such a scene's clipmap, seen from the ground, has levels of 3, 12, 24 m and doubling to 768 m, so a face a metre away gets 3 mm sun texels. |
| `KERN_SUN_STAGGER` | on | `0` redraws every clipmap level every frame. Normally level 1 refreshes every other frame, level 2 every fourth and the rest every eighth. In scenes using the 3 m close ladder, a shadow caster entering, moving within or leaving a view’s 12 m map also refreshes that map on intervening frames. The tiled cache updates damaged tiles. With `KERN_SUN_CLIPMAP`, a level also refreshes when its projection moves. |
| `KERN_SUN_CACHE_GUARD_METRES` | 0 | Guard band around the cached sun map (0 to 128 m). |
| `KERN_FOLIAGE_SUN_LOD`, `KERN_FOLIAGE_SUN_PAGE_METRES` | 0.5, fine pages | Paged foliage sun (only with `KERN_PAGED_FOLIAGE_SUN=1`). |
| `KERN_VOXEL_SELF_SHADOW_COARSER` | unset | Coarser voxel self-shadow mip. |
| `KERN_CUT_HYSTERESIS` | 1.25 | How much less error a tree the camera drew in detail last frame keeps that detail down to, so the cut does not flip each frame (`foliage_refine.rs`; values below 1 are ignored, 1 disables it). |
| `KERN_RASTER_FLOOR` | 8388608 | Lowest raster budget for the GPU forest (`foliage_runtime.rs`); the budget is the larger of this and the sum of the proxies' work, at most 16777216. |
| `KERN_VEGETATION_BUDGETS` | unset | Replaces the tier's steps for tuning: `error scale,source clusters,splats,slot share`, for example `1.5,2097152,8388608,1`. |
| `KERN_VEGETATION_QUALITY` | by adapter | Vegetation tier `low`, `medium` or `high` (`vegetation_quality.rs`): scales the forest's pixel error target and caps its voxel splat, branch slot and (on low) needle budgets. Unset: discrete GPU high, integrated medium, unknown low; `app` steps it down when adaptive quality runs out. |
| `KERN_SOURCE_CLUSTERS` | 2097152 | Source cluster budget that bounds the camera cut in a GPU forest (`foliage_runtime.rs`), at least 1. |
| `KERN_BRANCH_CELL_M` | 0.075 | Finest branch voxel cell in metres for aggregate cooks (accepted from 0.01 up to the default); each halving adds a level and quadruples the brick and voxel caps (`recipe_forest.rs`). |
| `KERN_FAR_TREE_SHADOW_FROM` | 0.35 | Where in the shared sun map (its clip-space radius) per-tree ground shadows start to take over (`shade.wgsl`). |
| `KERN_FAR_TREE_SHADOW_MIN_M` | 800 | Metres from the camera within which per-tree ground shadows never draw (`shade.wgsl`). |
| `KERN_FOREST_FLOOR_DARK` | 0.85 | How strongly the painted far ground darkens under forest stands (`terrain_surface.wgsl`). |
| `KERN_NEEDLE_BRUSH_OWN` | 0.42 | The share of the sun a drawn needle crown keeps (`shade.wgsl`). |
| `KERN_VOXEL_OWN_IN_CLIPMAP` | 0 | Share of a voxel cell's own sun march kept where the sun clipmap covers it; 1 keeps all of it (`shade.wgsl`). |
| `KERN_VOXEL_LEAF_PARITY` | 1 | Voxel leaves take the needle brushes' own shade (`KERN_NEEDLE_BRUSH_OWN`) on sun and sky and read sky visibility at the surface, like mesh needles; 0 = the old voxel lighting (`shade.wgsl`). |
| `KERN_VOXEL_DISC_DEPTH` | 1 | Voxel splat alpha uses the optical depth over the drawn disc's mean chord (cell/1.487) for camera and sun splats and the splat booking estimate; 0 = the old cube chord cell/L1 (`voxel_forest.wgsl` `volume_splat_append`). |
| `KERN_FAR_CROWN` | unset | Comma-separated look values for the painted far forest in the order base, area, sun_loss, sky_loss, opacity, filter, gap, size, shadow_opacity, shadow_size; any may be left empty (`frame.rs`, `terrain_surface.wgsl`). |
| `KERN_GRASS_PROXY` | 0.6,0.85,0.5 | `r,g,b` colour the ground takes past the grass disc in `terrain_fly`. |

## Diagnostics

| Variable | Effect |
| --- | --- |
| `KERN_CPU_SPANS=1`, `KERN_STARTUP_PROFILE=1` | CPU time per frame phase; renderer construction profile. |
| `KERN_DIAGNOSTIC_HIDE` | Hide geometry categories in captures. |
| `KERN_FOOTPRINT_LOD=1` | The simple and compact shade passes pick the albedo mip from the pixel's footprint through the triangle's UVs instead of from distance (mip 0 nearer than 33 m). Off: on an Adreno 740 it made the compact pass 3.4x slower (2026-11-25, cause open). |
| `KERN_ALBEDO_LOD_BIAS=<mips>` | Added to the simple/compact passes' albedo LOD (bandwidth experiment). |
| `KERN_SHADE_PROBE=N` | The simple/compact shade pass stops after stage N and stores what it has, so a device's pass times show what each stage costs: `1` visibility and depth reads, `2` + triangle and normal, `3` + material, game surfaces and sun, `4` + albedo texture and ambient/GI, `5` + local lights, `6` + the game's light styling (the rest is flash, glow and fog); `7` the whole pass without the albedo texture. The image is a debug view. |
| `KERN_SUN_DAMAGE_REPORT=1` | Prints, every frame, how many of the 4096 tiles of each cached sun map were marked changed, and how many casters changed by snapshot row (blocking readback). |
| `KERN_FAR_CROWN_DEBUG` | Debug colours on the painted far forest: `1` red for the averaged share and green for single-crown cover, `2` white noise in 2 m world cells, `3` red where the stand field reaches the ground (`terrain_surface.wgsl`). |
| `KERN_SPLIT_SELECTION_TIMING=1` | Times the GPU forest selection's parts (seed classify, refine, compact and finish, residency) separately (`frame.rs`). |
| `KERN_MEADOW_CAPTURE_RAW=1` | Sideways: raw meadow buffers in the capture JSON. |
| `KERN_PORTABLE=1` | Draw procedural terrain, sky and haze with the three-buffer portable renderer. Devices with eight storage buffers and sixteen sampled textures can use the GPU forest with compact material lighting. Default off; such a device always uses it. See [Running in a browser](browser.md). |
| `KERN_WEBGPU_LIMITS=spec`, `=browser`, `=mac` | Native device with only what a browser's WebGPU gives: the timestamp and first-instance features, and the spec's default limits (`spec`), what about 80 % of WebGPU devices offer (`browser`: 16 storage buffers and 32 sampled textures per stage, 1 GiB buffers) or `browser` with the 10 storage buffers most Macs offer (`mac`), never above the adapter. GPU errors are printed once each and do not stop the run, so one run lists every layout over the limits. For checking a [browser build](log/2026-11-05-browser-feasibility.md) without a browser. |
| `KERN_DLSS_RUNTIME_DIR` | Where to find `nvngx_dlss.dll` ([DLSS](../crates/render/native/DLSS.md)). |

## Experiments: off by default, `=1` opts in

None of these is the normal route. Most were measured and not promoted; the
note linked says why. They are candidates for removal from the code.

| Variable | Experiment | Note |
| --- | --- | --- |
| `KERN_REFRACTION` | Screen-space refraction for materials with a `Refraction` (same as `Scene::graphics.refraction`); default once checked on a real GPU | [refraction](log/2026-11-08-refraction.md) |
| `KERN_GPU_PARTICLES` | GPU particle pools take emitters (sparks, embers, grit in Growing Guns and Spellstorm blasts). Opt-in until timed on a real GPU; read by `GpuParticles::new`, so a running game needs a restart | [GPU particles](gpu-particles.md) |
| `KERN_INSTANCE_GROUPS`, `KERN_SUN_INSTANCE_GROUPS`, `KERN_HEADLIGHT_INSTANCE_GROUPS` | GPU hierarchy of spatial instance groups | [r45](log/instance-hierarchy-r45.md) |
| `KERN_WATER` | Water optics on terrain water (the bed through refracting, absorbing water) as the `Lighting` default; read whenever a `Lighting` is made | [water](water.md) |
| `KERN_CLUSTER_TRAVERSAL` | Top-down cluster traversal (`terrain_fly` turns it on unless set) | [r46](log/cluster-traversal-r46.md) |
| `KERN_INSTANCE_BROADPHASE` | Compact colour-view instance selection | [broadphase](log/instance-broadphase.md) |
| `KERN_STATIC_SUN_CACHE` | Static/dynamic split sun cache | [r45](log/static-sun-cache-r45.md) |
| `KERN_SHADOW_DISTANCE_LOD` | `=F`: shared sun map casters may be F x coarser than one screen pixel of the nearest camera spans at their distance (2 is the candidate). Not with `KERN_STATIC_SUN_CACHE` | [Sideways GPU cost](log/2026-11-11-sideways-gpu-cost.md) |
| `KERN_RECEIVER_SHADOW_DEMAND` | Receiver-driven shadow tiles | [r49](log/lod-shadow-demand-r49.md) |
| `KERN_PAGED_FOLIAGE_SUN` | Paged foliage sun map (excludes the far sun map) | [world lighting](log/world-lighting.md), [distant tree light](log/distant-tree-lighting.md) |
| `KERN_ASYNC_SHADOW_VALIDATION` | Asynchronous shadow cache validation, far-sun redraws included; on by default since 10 November, `=0` restores the explicit wait | [r61](log/frame-hitches-r61.md) |
| `KERN_VIRTUAL_SERIAL_CULL` | `=1` restores the old serial per-template loop for virtual branch owners in the cull (the default gives each lane its own template; was ~10 ms at 1440p) | |
| `KERN_SPLIT_CULL_TIMING` | `=1` times the camera cull's finalize and cluster scatter in their own passes (diagnostic) | |
| `KERN_CULL_PROFILE` | `=1` prints the cull's owner, cluster and virtual branch distribution at startup | |
| `KERN_SUN_RECEIVER_PLANE`, `KERN_SUN_TERRAIN_ERROR`, `KERN_SUN_SURFACE_SOURCE`, `KERN_ROCK_VARIATION` | Low-sun shadow and rock variants | [sun shadow diagnostics](log/sun-shadow-diagnostics.md) |
| `KERN_OWNER_RANGE_INDEX` | Indexed owner ranges | [terrain forest streaming](log/terrain-forest-streaming-2026-11-03.md) |
| `KERN_RASTER_LARGE_FIRST` | Raster large clusters first | [handoff log](log/handoff.md) |
| `KERN_SPLAT_BRICK_HZB` | Two-pass Hi-Z culling of voxel splat bricks; on by default since 9 November, `=0` turns it off | [voxel brick culling](log/voxel-brick-culling-2026-11-09.md) |
| `KERN_VOXEL_BRICK_VISIBILITY`, `KERN_VOXEL_GLOBAL_INTERVALS` | Voxel brick visibility stream | [brick visibility](log/vegetation-brick-visibility.md) |
| `KERN_BRICK_EMPTY_SKIP`, `KERN_BRICK_EMPTY_JUMP`, `KERN_BRICK_SMOOTH_MIPS` | Optical brick empty-space skipping and mips | [r47](log/meadow-work-r47.md), [r48](log/render-traversal-r48.md), [r49](log/lod-shadow-demand-r49.md) |
| `KERN_MEADOW_AGGREGATE`, `KERN_MEADOW_AGGREGATE_HALF_RES`, `KERN_MEADOW_AGGREGATE_SAMPLE_PRUNE`, `KERN_MEADOW_SUBSTRATE` | Distant grass aggregate and its substrate | [r39](log/meadow-distance-r39.md), [r57](log/meadow-half-resolution-r57.md), [r58](log/meadow-sampling-r58.md) |
| `KERN_SUN_DITHER_TEXEL` | `=1` anchors the sun map's coverage pattern for voxel splats to the map's texels instead of to each splat (`voxel_splat_raster.wgsl`) | |
| `KERN_SUN_TERRAIN_LOD_DAMAGE` | `=1` lets terrain LOD changes mark cached sun clipmap tiles as changed again (`shadow_damage.wgsl`) | |
| `KERN_MEASURED_ERRORS` | Any value but `0` measures grown plants' voxel errors with rays instead of estimating them, which takes minutes for an old tree (`recipe_forest.rs`) | |
| `KERN_MEADOW_GROUP_CACHE` (+ `KERN_MEADOW_GROUP_REPLACE`) | Meadow world-page cache | [r45](log/meadow-group-cache-r45.md) |
| `KERN_MEADOW_VISIBILITY` (+ `KERN_MEADOW_SKIP_HIDDEN_SHADE`) | Meadow visibility pass | [r47](log/meadow-work-r47.md) |
| `KERN_MEADOW_SPATIAL_FILTER`, `KERN_MEADOW_CONTACT_AO`, `KERN_MEADOW_EDGE_SHADOW` | Grass filtering and contact shading | [r59](log/meadow-contact-ao-r59.md), [r64](log/meadow-edge-grain-r64.md) |
| `KERN_DIRECT_DRAWS`, `KERN_SKY_TILES`, `KERN_NEAR_SUN_SOURCE`, `KERN_SOURCE_CROWN_BOUNDS`, `KERN_CANOPY_COMPACT_SUPPORT`, `KERN_LEGACY_LIGHT_LOOKAHEAD`, `KERN_VOXEL_ANTIPODAL_NORMALS` | Direct draw arguments, tiled sky light, near sun from source geometry, crown bounds, legacy headlight lookahead, voxel normals | No note describes these; read the code before relying on them. |
| `KERN_LIGHT_DEBUG` (terrain_fly) | Lighting debug view for crops: 1 sky visibility, 2 albedo, 3 direct, 4 indirect, 5 specular | terrain_fly.rs `draw` |
| `KERN_REFINE_WAVE_PUBLISH`, `KERN_REFINE_ROUNDS` | Foliage refine: workgroup-aggregated record publishes (0 = one global atomic per task); fixed refine round count (diagnostic) | foliage_refine.rs / foliage_refine.wgsl `fast_round_wave` |

## KERN_FORWARD

`KERN_FORWARD=1` renders plain mesh scenes with the low tier: the forward
renderer (`forward.rs`) with per-pixel light (`shaders/forward_lit.wgsl`).
One render pass, no compute per pixel: the material colour and albedo
texture, the baked room probes (`set_irradiance_volume`) as one 3D texture
(three filtered taps), every local light and spot unshadowed, emission,
then exposure, AgX, contrast and saturation straight into the display
target. `Lighting::volumetric` beams scatter per pixel along the view ray
(the analytic integral of `volumetric.wgsl` with three samples, nearest 16
lights). Glow (`shaders/forward_bloom.wgsl`) works on the finished 8-bit
image at an eighth of its size: bright pass, separable blur, screened over
the image, strength from the bloom options. No shadows or TAA. Made for tile GPUs
(phones, tablets, Quest); see the plan "Mobiel renderpad voor Warp". A
game's own `Graphics::minimal_forward` keeps its vertex-lit minimum profile.
Game surfaces (`SURFACE`) run there too, with `SURFACE_LOW_DETAIL` true; a
surface that does not compile there leaves the scene to the medium tier.
`KERN_FORWARD=auto` (the Android default) picks the low tier wherever the
scene has no terrain, grass, foliage, crowds or debris; the
log line `graphics tier: ...` says which tier draws and why. See
`graphics-tiers.md`.
