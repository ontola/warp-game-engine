# Rendering overview

The `render` crate is the largest part of Warp (about 50,000 lines, more
than half of it vegetation). This page is a map of what exists. The
detail, the measurements and the dead ends are in the
[engineering notes](README.md#engineering-notes), the
[handoff](../HANDOFF.md) and the forest roadmap.

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
  on the GPU, with a conservative current-frame depth pyramid and tree
  occlusion from real alpha-tested depth.
- **Visibility buffer, compute shading.** Clusters raster into a visibility
  buffer; the main shade pass lights each pixel once, in compute, in HDR
  (`shaders/shade.wgsl`).
- **Shader specialisation.** The shade pipeline compiles only the paths the
  scene's materials and resources can reach (`shade_features.rs`); images
  are pixel-identical to compiling everything. A pipeline cache shortens
  later starts.
- **Reconstruction and output.** Temporal anti-aliasing, optional DLSS
  (the `dlss` cargo feature; quality, balanced and performance modes),
  subtle bloom, then AgX tone mapping. A requested DLSS mode never falls
  back silently to another path.
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

## Light

- Physical units: sun and sky in lux, exposure in EV100, emissive
  materials in nits. `Lighting::default()` is a sunny day.
- Sun shadows, plus shadowed spot lights for headlights; up to 16
  unshadowed point or spot lights per frame (`set_local_lights`), with a
  `falloff` exponent.
- A procedural atmosphere bake: Rayleigh and aerosol Mie single scattering
  with ozone absorption on a spherical planet, supplying both the
  background and SH9 sky lighting. No clouds, moon or visible sun disc.
  See [ATMOSPHERE.md](../ATMOSPHERE.md).
- Height fog with a finite-distance aerosol layer.
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
- **Characters:** weighted GPU skinning, evaluated once per frame before
  all views and shadow maps (`skinning.rs`), fed by `assets::skinned`.
- **Effects:** CPU particles on slots (`particles`), explosions with fire,
  smoke, embers, heat shimmer and lights drawn after TAA (`fx`),
  GPU-simulated debris against an occupancy grid (`debris`), exhaust gas
  and backfire, wheel dust and gravel, persistent lit tyre marks.
- **Terrain and vegetation:** clustered terrain with normals that follow
  the rendered surface and a heightfield for tracing, a procedural meadow generated, culled, animated and drawn on the
  GPU, trees as clustered meshes with alpha-tested crowns, distant forest
  and mountains, and whole-plant wind evaluated once per instance on the
  GPU. Several volume and voxel representations exist as experiments and
  are off by default.

## Where it stands

The target is four players on one 3840 × 2160 output at 120 FPS (8.33 ms
per frame). **It is not reached.** The latest controlled four-player,
4K-output route measured about 38 to 44.5 ms with DLSS Quality; that is
not native 4K and not a typical-gameplay guarantee. A smaller scene such
as Growing Guns' arena is much cheaper (see its README for measured,
dated numbers).

The cost in Sideways' forest is dominated by grass source geometry, tree
raster, dynamic sun shadows and shading. The forest roadmap orders the
next work:

1. Make work fall with screen size: coarse shared representations of
   grass from the same source plants, selected by projected error.
2. Skip whole invisible subtrees. An experimental GPU hierarchy of spatial
   instance groups exists (off by default); top-down cluster traversal is
   missing.
3. Reduce raster and vertex work: opaque reusable parts with coarse
   aggregation, before considering a dedicated micro-triangle rasteriser.
4. Bound animation and shadow caching together: animation LOD on visible
   error, poses once for the union of views, shadow pages driven by what
   receivers need.
5. Share residency across views instead of multiplying per-view budgets.

This is not a complete Nanite or virtual shadow map implementation, and
the notes say so wherever a result is partial.

## Known gaps

- New meshes or materials need a new renderer (pipeline compiles take
  seconds); reserve slots up front.
- Fixed per-frame work remains for features a scene does not use.
- `frame.rs` still refers to Sideways features in many places.
- The default look is pale without tuning (AgX, physical units).
- Animated pixels reject temporal history until deformation motion vectors
  exist.
- No water, buildings or general indoor rendering.

## Further reading

- [Engineering notes](README.md#engineering-notes): every iteration with
  its measurements.
- [Handoff](../HANDOFF.md): the running status, newest first.
- [Forest roadmap](forest-rendering-roadmap.md) (in Dutch): the
  architecture audit and the priorities above.
- [Second game findings](second-game-findings.md): what Growing Guns
  changed in the renderer.
- [Unified vegetation](unified-vegetation.md) and the
  [live render debug panel](debug-ui.md).
