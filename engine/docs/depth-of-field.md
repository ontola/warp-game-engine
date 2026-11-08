# Depth of field

`Renderer::set_depth_of_field(gpu, &[depth_of_field::Settings])` opts a game
into a thin-lens blur, one lens per view in view order. Nothing is allocated
until a lens with `f_stop > 0` arrives; `f_stop <= 0` (the default) or an
empty slice skips the passes again. Growing Guns uses it in replay review
(F, right-click to focus, Q/E aperture; see its README).

```rust
renderer.set_depth_of_field(gpu, &[render::depth_of_field::Settings {
    f_stop: 2.8,             // f/1.4 very shallow .. f/16 nearly all sharp
    focus_m: 0.0,            // > 0: fixed distance in metres
    focus_point: [0.5, 0.4], // otherwise autofocus here (0..1, top left)
    ..Default::default()     // 24 mm film, max radius 12 px at 720p
}]);
```

The blur follows the camera: the focal length comes from the view's vertical
field of view and the film height (`sensor_mm`, 24 = full frame), and the blur
radius is the thin-lens circle of confusion f²/(N(S−f))·|d−S|/d, in pixels of
the view. A narrow field of view (long lens) therefore blurs much more than a
wide one at the same f-number, as with a real camera. `max_radius` caps the
radius (pixels at 720 vertical pixels, scaled per view, at most 32).

A typical wide game camera on 24 mm film keeps almost everything sharp even at
f/1.4. For a photo-mode look, raise `sensor_mm` (Growing Guns' replay lens
uses 70, medium format): at the same field of view the blur grows with it.

Autofocus reads the depth under `focus_point` on the GPU in the same frame
(the nearest of five texels, so a thin subject edge wins over the background).
There is no readback and no focus smoothing: a moving subject refocuses
instantly. For smooth pulls, animate `focus_m` from game code.

## How it renders

Two compute passes after motion blur, before auto exposure, bloom and tone
mapping, so out-of-focus highlights bloom as soft discs and the exposure meter
sees the final image:

1. `tile_max`: the largest blur radius of each 8×8 tile (R32Float).
2. `gather`: each pixel takes 64 golden-angle taps on a disk as large as the
   largest radius within up to four tiles, weighting each tap by how far its
   own blur disk reaches (scatter as gather). A tap behind the pixel never
   blurs over it more than the pixel's own blur, so sharp subjects keep their
   edges against a blurred background, while a blurred foreground spreads
   over a sharp background as a translucent halo. Fully sharp tiles copy.

Textures and one uniform only: no storage buffers, and its own pipelines and
bind groups, so it adds nothing to the shared scene bindings (WebGPU minimum
limits). Per-view output and tile targets, so split-screen views never bleed.
It reads the reversed infinite-Z depth as `near / depth`; custom projections
must keep that relation. GPU time shows as `depth of field` under
post-processing in F3/F10 timings.

## Limits

- Particles, exhaust, sprites and other effects that do not write depth take
  the blur of the surface behind them.
- No separate near/far layers: a blurred foreground over a sharp background is
  a translucent halo, not a fully opaque disc, and does not reveal background
  hidden behind the foreground.
- Large radii use 64 taps with a per-pixel rotation, so very shallow focus
  shows fine grain. Bokeh is round; no aperture blades or cat's eye.
- Opt-in until checked on a real GPU (Mancave). Verified so far on lavapipe
  (software Vulkan) only.
