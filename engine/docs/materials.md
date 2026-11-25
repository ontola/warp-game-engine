# Materials and textures

A material (`render::scene::Material`, 32 bytes on the GPU) is a tint, an
albedo texture layer, roughness, metalness, a repeat scale and flag bits. On
top of that a material can have a material map, weathering layers and, for a
game's own looks, a WGSL surface function. Textures are block compressed
where the device allows it and can stream in after the renderer starts.

None of this adds a GPU binding: material maps are extra layers of the
normal map array, and the per-layer and per-material words live in the scene
tables buffer (`material_maps.rs`). The engine rule of at most 8 storage
buffers per stage ([browser](browser.md)) still holds.

## Thin foliage in glTF

The glTF material boolean `extras.warpThinFoliage` enables reflected and transmitted light for leaves and needles. It defaults to `false`. A masked woody twig can share the same atlas and keep an opaque lighting response. The alpha mask controls the silhouette independently. A cutout spray can represent many needles or leaves with one shading normal. Warp weights its direct solar highlight by the projected area and uses roughness to limit the grazing reflection of the group. The diffuse light transmitted through the leaves keeps its source colour. The same filtering applies to the leaf fraction of a foliage voxel.

## Procedural bark

A material using `render::scene::BARK_LAYER` sets the plate size in metres with `scale`. Warp samples grain and relief on the undeformed mesh. The planting transform supplies its scale, using the geometric mean for unequal axes. The branch and wind transforms carry relief normals with the wood.

## Material maps

Each texture layer can carry a material map (`TextureSource::orm`) in glTF's
packing: R ambient occlusion, G roughness, B metalness. Each channel
multiplies the material's own value, so a material with `roughness: 1.0`
takes the map's roughness as it is, and a missing map (or a white texel)
keeps the scalar. Occlusion darkens only sky, bounce and reflections, never
the sun.

Where they come from:

- `Scene::add_texture(x_albedo.jpg)` finds the siblings beside the albedo:
  `x_orm` or `x_arm` (packed), else `x_ao`, `x_rough` and `x_metal`, packed
  into one (`material_map_beside`). Poly Haven names work too:
  `x_diff_1k.jpg` with `x_arm_1k.jpg`, `x_rough_1k.jpg` and
  `x_nor_gl_1k.jpg` (`sibling_map`).
- glTF assets (`assets::gltf`) pack the metallic-roughness and occlusion
  textures of each material, and their factors become the material's
  roughness and metalness.
- In code, set `scene.textures[layer].orm` to any RGBA image, or build one
  from separate channels with `render::scene::pack_material_map`.

uv-mapped and TRIPLANAR materials sample the map (`orm_w`,
`triplanar_orm` in shade.wgsl). The terrain, road, rock and grass paths have
their own surface models and do not read it.

## Weathering layers

`Lighting::wetness` (rain) and `Lighting::snow` (snow cover) are scene
weather, both 0..1. Each material takes them by its `MaterialLayers`
(`Scene::set_material_layers`, or `Renderer::set_material_layers` at
runtime, no new renderer needed):

| Field | Default | Effect |
| --- | --- | --- |
| `wet` | 1 | How wet it gets: darker by `porosity`, a glossy film everywhere, mirror pools on flat ground. |
| `snow` | 1 | Snow on upward faces; more faces as `Lighting::snow` grows. |
| `moss` | 0 | Moss on upward faces broken up by noise, thinner where the material map is occluded. Independent of the weather. |
| `porosity` | 0.5 | 1 darkens like soil or wood when wet, 0 only gets a film like stone or metal. |

Car paint, roads, lava and emissive surfaces never take these layers. The
road-side terrain (TERRAIN without SURFACE) keeps its own wet look, which
also follows `Lighting::wetness`. Sideways' `wet` weather sets wetness 0.85.
Grass blades, the meadow and voxel trees do not take snow or wetness yet.

## A game's own surfaces

A game adds materials without changing the engine through the SURFACE flag:
`render::set_game_surface(source)` supplies `game_surface` (see
`shaders/game_surface.wgsl`), which turns the shading point into albedo,
normal, roughness, metalness and emission; `layer = -2 - kind` tells the
game's surfaces apart. SpaceRacer and Spellstorm use it, and the `warp`
runtime hot reloads it from a game's `surface.wgsl`. Its result then takes
the weathering layers like any other material.

## Shading on simplified glTF meshes

Set `"extras": {"warpPreserveShading": true}` on a glTF material when broad
glossy panels show distorted reflections as the camera pulls back. For
primitives using that material, Warp considers authored vertex normals
alongside position when building lower-detail geometry. Textured materials
also preserve UV seams. The mesh still
simplifies with distance. Leave the field unset to use the default geometric
error metric.

## Artist lighting and image effects

A game folder can contain either or both of these WGSL files. Warp reads
them at game load. Relaunch the game after editing them. Rust scenes supply
the same sources through `Scene::style`.

- `lighting.wgsl` defines `fn game_lighting(s: LightingSample) -> StyledLight`.
  The sample includes world position, shading normal, albedo, the engine's
  lit HDR radiance, direction toward the sun, sun illuminance in lux,
  shadow visibility (0–1), time in seconds, material flags and surface kind.
  Return linear HDR radiance and an integer image-effect tag (0–2047).
  Zero is the untagged group. Fog, hit flashes and exposure follow lighting.
  Ordinary materials and engine terrain report kind -1. A Luau object's
  `surface` supplies its kind. An untextured opaque glTF material can
  declare the same kind with integer `extras.warpSurface` (0–255).
- `finish.wgsl` defines `fn game_finish(pixel: vec2<i32>, center: FinishSample) -> vec3<f32>`.
  Read neighbours using `scene_sample(pixel + offset)`. Each sample
  contains HDR colour, the visible part's tag and reversed device depth
  (nearer is larger, sky is zero). Reads clamp at the edges of the current
  view. Return HDR colour. A finish uses one additional RGBA16F target at
  render resolution per view. Games without a finish use the regular HDR target.

Lighting runs on opaque meshes, terrain, visibility-buffer voxel crowns, grass cards, and
procedural meadow blades, including the meadow's aggregate lighting path.
The simple opaque lighting profile supplies shadow visibility 1. The finish
runs after opaque shading and before separate vegetation, translucency,
temporal reconstruction and tone mapping. Consequently a grass blade from
a later pass covers finished ink naturally. Tags belong to the opaque
image. Later volumetric vegetation and transparent layers keep their own
compositing paths.

[Painted Meadow](../../games/painted-meadow/README.md) supplies a working pair.
The palette, shade bands, brush filter and rules for drawing ink all live
in that game.

## Compression

When the device has `TEXTURE_COMPRESSION_BC` (desktop GPUs; desktop
browsers offer it as WebGPU's `texture-compression-bc`, not tried in the
browser build yet) the albedo, normal and
material map arrays are BC7: a quarter of the memory and bandwidth of
RGBA8. A 2048 square albedo layer with mips takes 5.6 MB instead of 22 MB.
The processed mips are encoded on the CPU (`block_compression`, a port of
Intel's ISPC kernels, with its fastest presets: about 0.1 s per megatexel
on one core for opaque layers and 0.4 s with alpha, 41 to 42 dB PSNR on
Sideways' rock and pine textures) and kept in the [processed texture cache](processed-texture-cache.md)
under a key derived from the mips' key, so a second start reads blocks
instead of encoding.

RGBA8 stays when:

- `KERN_TEXTURE_COMPRESSION=0`;
- the device has no BC support (phones, Quest: there is no ASTC path yet);
- for the albedo array only, the scene has screen layers
  (`Scene::add_screen_texture`), since those are drawn into on the GPU.

## Streaming

With `Scene::texture_streaming` (or `KERN_TEXTURE_STREAMING=1`; `=0` turns
it off whatever the scene says), the renderer uploads only each layer's
mips from 64 texels down when it is created, and up to four worker threads
build and encode the full chains (`texture_stream.rs`). At the start of
each frame, finished layers are uploaded within a 24 MB budget. Until a
layer is complete, every albedo, normal and material map sample raises its
gradients so it reads no finer mip than is there (`albedo_lod_floor`,
`normal_lod_floor`), so the stage shows slightly soft textures for a moment
instead of black ones. `Renderer::textures_streaming()` counts layers still
coming; `Renderer::finish_texture_streaming(gpu)` waits for all of them.

Native interactive frames stream layers with a procedural generator, including
Warp Human faces. Their CPU previews supply the small mips while a single
worker builds each full albedo and normal pair. Completed jobs wait in a
bounded queue. Headless frames load procedural layers in sequence at startup
so the first capture has the full detail. Sideways also streams ordinary
textures for played windows and the next stage it loads. The browser and
portable terrain renderer upload during scene setup. Screen layers are
always available in full.

All layers eventually have their full mip chains on the GPU. Camera-driven
residency and eviction of unused mips are open.

## Checking it

`cargo run --release -p render --example material_probe -- --out DIR`
renders a small scene of Sideways' textures: `maps.png` with material maps
off and on, `grid.png` dry, wet, snow and moss, and `report.json` with the
albedo format and renderer creation time. `--stream` also saves the first
streamed frame (`stream-first.png`).
