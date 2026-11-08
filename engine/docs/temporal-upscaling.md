# Temporal upscaling and motion vectors

Warp renders a frame at a lower resolution and rebuilds the output
resolution from several jittered frames. Two reconstructions share the
same inputs:

| Mode | Render scale per axis | Runs on |
| --- | --- | --- |
| `native` | 1 (TAA at full resolution when the light enables it) | every GPU |
| `temporal-quality` | 2/3 | every GPU, also WebGPU |
| `temporal-balanced` | 0.58 | every GPU, also WebGPU |
| `temporal-performance` | 1/2 | every GPU, also WebGPU |
| `dlss-quality`, `-balanced`, `-performance` | chosen by NGX | NVIDIA GPUs with the `dlss` feature |

Pick one with `Renderer::set_upscale`. A mode that cannot start returns an
error and never falls back silently. `terrain_fly --upscale MODE` takes any
mode, and F5 cycles native and the temporal modes while flying. Sideways
offers all of them in its settings (`upscale`).

## The portable upscaler

`render/src/temporal_upscale.rs` with `shaders/temporal_upscale.wgsl`: one
compute pass per view, after the motion pass, where DLSS would run. It
binds only textures (five sampled, one `rgba16float` storage texture) and
no storage buffers, so it fits WebGPU's minimum limits.

Per output pixel it:

1. gathers the 3x3 render pixels around it, weighting each jittered sample
   by how close it landed (a Gaussian of about one render pixel for a frame
   without history, about one output pixel for the samples that are added
   to history one by one);
2. reprojects its own output-resolution history with the motion vector of
   the nearest surface around (Catmull-Rom filtered);
3. clips that history to the current samples' colour range (YCoCg of
   range-compressed colour, as TAA does) and drops its weight when it had to
   be pulled far;
4. blends: history weight plus the new samples' weights, capped at
   `1 / taa_blend`.

It uses TAA's settings (`Lighting::taa_blend`, `taa_gamma`), the same
Halton jitter as DLSS (32 phases) and the auto exposure meter's previous
value. Texture mip bias follows the render scale, as for DLSS.

On a still half-resolution image (the `still_image_converges_past_bilinear`
GPU test) the converged result has half the error of a bilinear upscale and
0.57 of a single frame's.

## Motion vectors

`render/src/motion.rs` writes, per render pixel, where the surface was last
frame, a depth, and a bias (1 where history must not be used). Covered:

- cameras, rigid instances, wheels, branch poses (part wind) and affine
  wind shear;
- skinned meshes: the pass keeps a copy of last frame's skinned vertices
  (copied after all views, in the same buffer as the previous wind) and
  takes each pixel's barycentric point on the same triangle of the old
  vertices;
- rasterized foliage voxels: the visibility id (bit 31 marks a voxel)
  holds the splat; the view's splat records name its tree, whose fifth
  record is the owner instance, and the pass moves the point with that
  instance's previous pose, so branch voxels that sway keep their history;
- grass, meadow and the voxel composite through their own reprojection
  layers.

Foliage selection changes (a source/voxel swap, a new mip) no longer reset
a whole view's history; that threw away all history every frame while
pages streamed. Swaps are handled per pixel by colour clipping, as native
TAA always did. A per-pixel owner/representation id test was tried and
dropped: leaves, branches and voxels interleave per pixel and flicker
between owners under jitter, so it left tree crowns unresolved.

The pass uses 8 storage buffers (the engine's limit per stage) and 11
sampled textures.

## Open

- No sharpening pass after the upscaler.
- Measured only on lavapipe (software Vulkan) so far; GPU timing on real
  hardware is still to do.
