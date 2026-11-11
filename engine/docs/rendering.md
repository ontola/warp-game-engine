# Rendering overview

The `render` crate is the largest part of Warp (about 50,000 lines, more
than half of it vegetation). This page is a map of what exists and how a
frame is built. [Status](status.md) says what is open,
[renderer switches](renderer-switches.md) lists the `KERN_*` A/B switches,
and the measurements and dead ends are in the [development log](log/README.md).

## Shape of a frame

- **Headless by design.** The renderer draws into textures and reads them
  back. A window is only the `app` crate presenting one of those textures.
  The same path runs on a desktop GPU and on a CPU rasteriser.
- **Cluster geometry with LOD.** Every mesh becomes clusters of up to 128
  triangles in a LOD DAG (meshoptimizer's clusterlod). Groups are
  simplified with locked borders, so any cut through the DAG is crack-free.
  A cluster is drawn when its own error projects to at most a pixel and its
  coarser replacement would project to more (`cluster.rs`).
- **GPU culling and selection.** Culling and LOD selection run per cluster
  on the GPU (`cull.wgsl`). Clusters are sorted into size buckets.
- **Occlusion culling for every category** (`occlusion.rs`). History first,
  in two phases: clusters visible last frame draw first and build a depth
  pyramid; the rest are tested against it and only drawn if they show. A
  probe re-checks hidden clusters every 4 frames. The GPU forest is also
  tested before its selection, against the depth of rigid opaque geometry
  only (terrain, buildings, cars; not deforming or see-through materials),
  so a forest behind a wall is not selected at all.
- **Opaque before masked.** Opaque clusters raster through a fragment stage
  without `discard`, so they keep early depth testing; alpha-tested clusters
  (masked materials, fading grass cards) draw after them through the
  alpha-cut pipeline. A game mask or bounded draws (Quest) use the single
  pipeline.
- **Visibility buffer, compute shading.** Clusters raster into a visibility
  buffer; the main shade pass lights each pixel once, in compute, in HDR
  (`shaders/shade.wgsl`).
- **Shader specialisation.** The shade pipeline compiles only the paths the
  scene's materials and resources can reach (`shade_features.rs`); images
  are pixel-identical to compiling everything. A pipeline cache shortens
  later starts.
- **Reconstruction and output.** Temporal anti-aliasing, or upscaling from a
  lower render resolution: Warp's portable temporal upscaler on every GPU
  (`temporal-*` modes) or DLSS (the `dlss` cargo feature), both fed by
  motion vectors that cover skinning and wind
  ([temporal upscaling](temporal-upscaling.md)). Then subtle bloom and AgX
  tone mapping. A requested mode never falls back silently to another path.
- **Split screen.** `render::split_rects` lays out up to four views; one
  call renders all of them into one output, sharing work such as skinning
  and wind.
- **HUD.** Shapes, text (glyphon) and pictures in a draw list, drawn in
  every view and in headless captures. Barlow and Barlow Condensed ship as
  the default fonts.
- **Runtime pictures.** `render::picture::decode` turns PNG, JPEG, WebP and
  SVG bytes (sniffed by content, bounded) into RGBA; `HudRenderer::set_image`
  uploads one under a `HudImage` id (mipmapped linear, or nearest for pixel
  art) and `HudDrawList::image` draws it with a uv rectangle (negative to
  mirror) and a tint. Uploading needs no renderer rebuild.
- **Screens in the world.** `Scene::add_screen_texture` reserves an albedo
  layer; `screen::Screen::draw` renders a HUD list (2048 square) into it
  and rebuilds its mips on the GPU, after the renderer exists. Put it on
  `primitives::quad` (uv top-left is the list's top-left) with an EMISSIVE
  material for a glowing TV, or a plain one for a printed sign or cover.
- **Sprites.** `Renderer::set_sprites` draws textured quads (billboards
  that turn to each camera, or fixed axes) in HDR after reconstruction:
  pixel art stays crisp, the opaque scene's depth hides them, back to
  front per view. Images come from `set_sprite_image` (pictures, nearest or
  mipmapped) or `set_sprite_texture` with a `sprites::Canvas` (a HUD list
  drawn offscreen: name tags, 3D text, button prompts). Colour is radiance;
  `sprites::irradiance` lights a sprite from the scene's local lights.
  `Sprite.depth_test` defaults to `DepthTest::Fragment`. Optical halos can use
  `DepthTest::Source`: test visibility at the sprite centre, then spread the
  glow over nearby surfaces without slicing the quad against their depth.
  A hidden source still hides the whole halo. Keep normal geometry/particles
  on fragment depth; pair optical halos with `Facing::Screen`.
  `Scene.graphics.sharp_sprites` composes these effects after world motion blur,
  before bloom/tonemapping. This avoids using the background's velocity for
  electrical arcs and tracers; it also leaves exhaust/particle composition sharp.
  The default order is unchanged for other games.

- **Blended surfaces and refraction.** Materials given
  `Scene::set_material_blend` (glass, decals) or `set_material_additive`
  skip the visibility buffer and draw afterwards in a small forward pass
  (`transparency.rs`, `shaders/transparency.wgsl`), sorted back to front per
  view, lit with the same sun, sky and shadow code as opaque pixels.
  `Scene::set_material_refraction` adds a `Refraction` (index of
  refraction and thickness: `Refraction::WINDOW` for a 4 mm pane,
  `Refraction::SOLID` for a solid reaching the background). With refraction
  on (`Scene::graphics.refraction` or `KERN_REFRACTION=1`, off by default
  until checked on a real GPU), a view with refracting glass first finds
  the nearest refracting surface per pixel (a depth-only draw over a copy
  of the opaque depth), draws the glass behind it, copies the HDR, then
  draws that surface over the copy, bent, and finally the glass in front.
  The copies, the opaque depth (a read-only attachment) and the front depth
  are an optional fourth bind group with no storage buffers; the phase is
  in the top bits of each sorted entry, so it stays one pipeline. A pane
  bends the ray through its thickness and lets it out parallel; a solid
  bends it once to the background (its inside faces are not drawn). Light
  through the glass loses its Fresnel reflection, takes the glass's hue and
  blurs with roughness; the reflection is kept whole. Only the nearest
  refracting surface bends: glass behind it moves with the background but
  does not bend again. A view without refracting glass draws in one pass.
  Not refracted: whatever lies beyond the screen edge (clamped) and anything
  in front of the glass (falls back to the straight view).
  `refraction_probe` renders a glass ball, slab and pane, also split screen,
  with it off and on.

## Light

### Sun shadows

The sun is shadowed by depth maps that every representation (source
geometry, voxel splats, proxies) casts into:

- **Clipmap levels** around each camera (`KERN_SUN_LEVELS`, 4 by default):
  2048-texel maps in one depth array, level k covering 24 m times 2^k, so a
  texel spans about the same share of the screen at every distance. Shading
  takes the finest level that covers a point and blends finer partial levels
  over it. Outer levels redraw on a stagger (`KERN_SUN_STAGGER`).
- **The shared map** (4096 texels, about 180 m around the focus), behind the
  levels. In streamed GPU-forest worlds it is cached: its projection snaps to
  whole texels, so camera movement scrolls the retained depth, and
  `shadow_damage.rs` keeps one history of every caster (source instances and
  voxel trees) and marks the 64 x 64 tiles whose casters changed. Only those
  tiles are cleared and redrawn.
- Past it, a far map (below) or the coarse canopy transmittance.

**One clipmap (default in GPU-forest worlds since 9 November; `KERN_SUN_CLIPMAP=0` restores the separate maps).** Eight
levels per view (24 m to 3072 m from the ground) replace the shared map and
the anchored far map. From high up the first level grows with the camera's
height over its lighting focus (`frame.rs` `clipmap_base`: half extent at
least half the height, at most 384 m, kept within a band), so the levels
cover the visible ground instead of texels far below a pixel, and the
outermost reaches past the drawn trees. Every level is cached like the shared map: the same caster history marks
damaged tiles in every level (one pass for all maps, `shadow_damage::MAX_MAPS`
slots), projections scroll, and only damaged tiles redraw. Level 0 redraws
every frame; outer levels keep collecting damage between their staggered
redraws and redraw at once when they move. Their centres snap to whole tiles
(32 texels: 3 m at level 1, 96 m at level 6), so that happens rarely. The
outermost level stands in for the shared map in shading (binding 5,
`shadow_view_proj`); past the outermost level the coarse transmittance applies as past the
far map. Levels span the scene's whole sun depth, so shading reads only the
covering level (no extra shared-map tap up the sun ray). While the camera is
still, the GPU forest still reassigns thousands of voxel slots per frame,
which marks about half of every map's tiles (the shared map's too); that work,
not the maps, sets the cost. `KERN_SUN_DAMAGE_REPORT=1` prints the counts.

The optional static/dynamic split (`KERN_STATIC_SUN_CACHE=1`) includes both
GPU-published source branches and voxel casters. Reserved branch slots stay
in the dynamic candidate list even while their CPU placeholders are empty;
GPU culling evaluates their current contents. Voxel depth is drawn after the
static-depth seed and dynamic source pass. The split is not a world-page
cache: forest culling and voxel-splat generation still run on redraws. Raw
split/full depth is tested for still frames, wind and population changes.


Games can set `Scene::stage_sun = true` for a camera-independent sun-shadow
cache of the generated level. It renders source geometry/alpha once before the
first view, fitting the complete static scene (not only vegetation), and shares
one 4096-square depth map across all views. The map adds 64 MiB plus bounded
caster buffers. Live `Slots` ranges register in `Scene::dynamic_instances` and
are excluded even when empty at creation; direct dynamic instance users must
register their ranges before constructing the renderer. Skinning, rigid-part
animation and car categories are excluded too. Nearby moving shadows remain
in the existing dynamic maps. Distant foliage uses its rest pose.

Camera movement, wind and adaptive camera LOD do not rebake this map. A static
source transform, sunlight direction or alpha threshold change does; replacing
the scene for a new seed creates a fresh cache. `KERN_FAR_SUN_SHADOW=0` disables
it for A/B captures. `KERN_FAR_SUN_LOD` controls the bake's fixed source error.
This is direct sun visibility, not baked indirect GI or local-lamp shadows.
Near-map detail transitions and far-map resolution remain finite. The older
experimental `KERN_FAR_SUN_SHADOW=1` opt-in retains its foliage-oriented fit
unless the scene also opts in; paged foliage sun is a mutually exclusive mode. Streamed GPU-forest worlds
(`terrain_fly`) get a camera-anchored far map by default, and the forest has
its own shadow tile cache that only redraws tiles whose casters changed.

### Clustered local lights

Opt-in with `Graphics::clustered_lights` or `KERN_CLUSTERED_LIGHTS=1`
(off by default until checked on more GPUs). `Renderer::set_clustered_lights`
takes up to 1024 lights a frame, each a `LocalLight` (the same model as
`set_local_lights`) with an optional `LightShadow { id, exclude }`. They add
to the headlights and local lights, which still work.

- Binning: a compute pass sorts the lights into a world-space grid of at
  most 16384 roughly cubic cells, fitted around the lights and clipped to
  600 m around the cameras. Each cell keeps its first 62 lights in the
  order given, so put the important ones first. Shade loops over its
  pixel's cell only.
- Shadows: 256-texel tiles in extra layers of the headlights' shadow array
  (`KERN_CLUSTERED_SHADOW_LAYERS`, 4 layers, 64 tiles by default). A spot
  light up to a 120 degree cone takes one tile, anything wider six (a cube).
  The lights nearest a camera, relative to their range, get tiles; the rest
  stay unshadowed (`clustered_light_stats().dropped_shadows`). A tile keeps
  its depth while its light keeps its `id` and does not move: moved lights
  redraw first, then the oldest tiles, at most
  `KERN_CLUSTERED_SHADOW_FACES` (12) a frame, so a caster that moves past a
  still lamp is a few frames late. `exclude` names the lamp's own
  instances. Lights with a `source_radius_m` of 0.3 m or more (a bounce
  light standing in for a lit patch of floor) get soft shadows: PCSS, an
  8-tap blocker search and 16 compare taps over the penumbra (source radius
  times receiver minus blocker over blocker, at most 24 texels); smaller
  lights keep a 4-tap filter. Cookies and angular profiles: see
  `interior-light.md`.
- Cost: the light records, shadow faces and cells live in the scene tables
  buffer (shade stays at 8 storage buffers); a uniform in bind group 2 holds
  the grid. Off, shade compiles none of it. Grass and opaque surfaces get
  the lights; transparency, volumetric light and the forward renderer do
  not yet.
- Probe: `cargo run --release -p render --example clustered_lights_probe --
  --out DIR` (400 lamps over a plaza of pillars, 24 shadowed).
- Spellstorm turns it on: every lamp, staff, shrine and spell light, lamps
  shadowed. On an RTX 5070 Ti at 2560x1440 the chapel frame went from 5.94
  to 5.54 ms (shade 5.05 to 4.52, shadow tiles 0.13 ms).

### Everything else

- Physical units: sun and sky in lux, exposure in EV100, emissive
  materials in nits. `Lighting::default()` is a sunny day.
- Sun shadows, plus shadowed spot lights for headlights; up to 32
  unshadowed point or spot lights per frame (`set_local_lights`), with a
  `falloff` exponent. Opt-in: up to 1024 clustered lights, some shadowed
  (below).
- A procedural atmosphere bake: Rayleigh and aerosol Mie single scattering
  with ozone absorption on a spherical planet, supplying both the
  background and SH9 sky lighting. No clouds, moon or visible sun disc.
  See [atmosphere](atmosphere.md).
- Indoor light: `irradiance_volume` probes (an ambient cube per grid
  point) reserved with `Scene::reserve_irradiance_volume` and filled with
  `Renderer::set_irradiance_volume`. Inside the grid they replace sky light
  and the outdoor bounce for diffuse ambient and rough reflections, fading
  out over the outer half cell. `irradiance_volume::BoxRoom::bake` bakes them
  on the CPU from boxes (walls, furniture, emissive tubes) and the local
  lights, with shadow rays and any number of bounces; a game can also
  supply its own probes. Use a black sky for a closed room.
- Height fog with a finite-distance aerosol layer.
- Opt-in volumetric light: in-scattering from chosen local lights and
  scatter-only beams, at half resolution after reconstruction
  ([volumetric-light.md](volumetric-light.md)). Off by default.
- `Lighting::gi` marches foliage and terrain volumes for indirect light;
  without foliage or a heightfield the marches are skipped exactly.
  `scene.mark_solid()` adds near-field contact shading for a handful of
  objects.

## Content systems

- **Primitives and models:** `render::primitives` (cube, sphere, capsule,
  hewn blocks, gradient sky), `ClusterMesh::build` for your own meshes,
  glTF through the `assets` crate.
- **Dynamic objects:** `render::slots::Slots` reserves instances up front;
  add, move, re-mesh and remove at runtime without a renderer rebuild.
  New meshes or materials still need a new renderer.
- **Live in-world screens:** `Scene::add_live_screen_texture(name, source_view,
  aspect)` reserves a shared layer for an `EMISSIVE | LIVE_SCREEN` material.
  After all views finish, the GPU copies the selected view before HUD into
  mip 2 (512 square) and generates smaller mips, at most 30 times per second.
  Displays sample the layer on the next frame.
  Split-view cropping, aspect fitting, gamma conversion and output resize are
  handled by `screen_feed`. The shared GPU pass is reported as `live screens`.
  The texture array still allocates a full layer; only update/sample cost is
  restricted to the smaller mips. First frame and missing source views are black.
  For headless captures render twice to prime the feed. Growing Guns' example
  is `games/growing-guns/scripts/stadium-tv.json`. A game can supply a
  `BroadcastShot` for a live layer. Its camera renders a selected range of the match's
  live spectators and simple depth-tested backdrop triangles into a small texture.
  The sample number controls when the renderer refreshes it. All player views and
  displays share the result. A scene shot uses the same output and draws the
  current clustered geometry with a coarse LOD and a single forward lighting
  pass. The GPU profile reports the whole alternate-camera pass as `TV audience`
  or `TV battle`, separately from the final shared `live screens` blit.
- **Characters:** weighted GPU skinning, evaluated once per frame before
  all views and shadow maps (`skinning.rs`), fed by `assets::skinned`.
- **Effects:** CPU particles on slots (`particles`), explosions with fire,
  smoke, embers, heat shimmer and lights drawn after TAA (`fx`),
  GPU-simulated debris against an occupancy grid (`debris`), exhaust gas
  and backfire, wheel dust and gravel, persistent lit tyre marks.
- **Terrain:** procedural terrain as GPU patches of one height function,
  with water in ditches, streams and rivers at every distance, regions,
  zones and flat plots ([procedural terrain](terrain.md)). Sideways still
  uses its baked terrain mesh, with normals that follow the rendered
  surface and a heightfield for tracing.
- **Vegetation:** in `terrain_fly`, trees and terrain grass share one GPU
  route: assemblies of shared parts, GPU selection of a complete cut per
  frame, bounded source and voxel paging, shared poses and wind
  ([vegetation checklist](vegetation-architecture.md)). Sideways uses its
  procedural meadow and clustered source trees with alpha-tested crowns,
  plus a distant forest. Distant trees freeze to their rest pose between
  60 and 90 m.
- **Buildings:** static opaque structures (`GeometryCategory::Structures`)
  occlude like terrain; `terrain_fly --town` puts them on zoned plots.

## Where it stands

The target is four players on one 3840 × 2160 output at 120 FPS (8.33 ms
per frame). **It is not reached.** The latest controlled four-player,
4K-output route measured about 38 to 44.5 ms with DLSS Quality; that is
not native 4K and not a typical-gameplay guarantee. A smaller scene such
as Growing Guns' arena is much cheaper (see its README for measured,
dated numbers).

The cost in Sideways' forest is dominated by grass source geometry, tree
raster, dynamic sun shadows and shading. In `terrain_fly` much of the forest
cost is GPU selection and shadow work that does not fall when the view is
blocked. The [vegetation checklist](vegetation-architecture.md) has the
current gates and a per-category budget for the 8.33 ms target.

This is not a complete Nanite or virtual shadow map implementation.

## Known gaps

- New meshes or materials need a new renderer (pipeline compiles take
  seconds); reserve slots up front.
- Fixed per-frame work remains for features a scene does not use.
- `frame.rs` still refers to Sideways features in many places.
- The default look is pale without tuning (AgX, physical units).
- Water is a surface in the terrain only (no simulated water); seeing the
  bed through it is opt-in ([water optics](water.md)). Buildings are plain opaque geometry. Indoor light is a baked probe
  grid: no dynamic GI, light leaks through walls thinner than a cell.

## Further reading

- [Status](status.md) and [renderer switches](renderer-switches.md).
- [Procedural terrain](terrain.md) and the
  [vegetation checklist](vegetation-architecture.md).
- [Live render debug panel](debug-ui.md) and
  [native profiling](native-profiling.md).
- [Development log](log/README.md): every iteration with its measurements.
