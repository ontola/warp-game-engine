# Volumetric light from local lights

Opt-in single in-scattering from local lights, so a floodlight shows its
beam in the night air and warm lamps (Growing Guns' lava) a haze. Code:
`render::volumetric`, `shaders/volumetric.wgsl`.

![Growing Guns at night: off (left), on (right)](images/volumetric-night-arena.jpg)

## API

- `Lighting::volumetric: Option<Volumetric>`, default `None`.
  `Volumetric { density, color, anisotropy, intensity, extinction, ambient, length }`:
  scattering per metre (also the extinction inside the integral), albedo
  tint, Henyey-Greenstein g, artistic multiplier; then, for indoor fog like
  Godot's (the Game Room lobby):
  - `extinction` (0..1, default 0): the scene behind is multiplied by
    `exp(-extinction * density * distance)`. 0 only adds light.
  - `ambient` (default 0): radiance the medium converges to from light
    arriving from everywhere (ambient or bounce light times albedo, plus
    emission); a ray adds `ambient * (1 - exp(-density * t))`.
  - `length` (default 0, unbounded): metres from the camera the medium
    reaches (Godot's `volumetric_fog_length`).
- `Lighting::set` keys: `volumetric=0|1`, `vol_density`, `vol_anisotropy`,
  `vol_intensity`, `vol_r`, `vol_g`, `vol_b`, `vol_extinction`, `vol_length`.
- `Renderer::set_local_lights_scattering(lights, weights)`: the local
  lights, plus a scatter weight each (missing weights are 0). Surface light
  ignores the weights. `set_local_lights` is the same with no weights.
- `Renderer::set_volumetric_beams(lights)`: lights that only scatter and
  light no surface, e.g. narrow beams inside a wide flood whose surface light
  is one local light.
- At most 32 lights scatter: weighted local lights first, then beams.

## Method

Per pixel and light, the view ray is clipped to the light's range sphere and,
for spots narrower than about 87 degrees, to the outer cone (a quadratic). The
irradiance model is `local_light_sample`'s: cone smoothstep, range fade
`(1 - (d/R)^4)^2`, and `(d^2 + s^2)^(falloff/2)` spread. Inside the source
radius `s` this differs from the surface's `max(d^2, s^2)`, which keeps the
integrand smooth.

The integral is taken in the angle of the closed form, with
`h'^2 = h^2 + s^2` (h: the ray's miss distance):

- for falloff 1.75 and up, `t = tc + h' tan(a)`. For inverse square this is
  the exact atan form;
- for lower falloffs, `t = tc + h' sinh(a)`. This is exact for 1/d, the
  falloff of Growing Guns' floodlights.

What remains (cone, range fade, HG phase and transmittance
`exp(-density (t + d))`) is smooth in `a`. It is integrated with 6-point
Gauss-Legendre. Against 50,000-step quadrature this is within 1% for
omnis, spots, rays through the source and rays starting inside it.

There is no occlusion term: local lights are unshadowed for surfaces too, so
a shadow test in the air would have no map to read. A beam therefore passes
through walls; the depth buffer only ends the ray at the first surface.

## Passes

The passes run after TAA or DLSS and before effects and sprites. An analytic
integral has no noise to hide, and the passes never write temporal history.

1. `scatter`: half of the internal resolution. Each texel takes the nearest
   or the farthest depth of its 2x2 footprint, in a checkerboard. It stores
   the RGB in-scatter and the linear depth.
2. `composite`: output resolution. It upsamples bilinearly, with each tap
   weighted by `exp(-gap^2)`, where `gap` is the depth difference relative to
   5% of the nearer depth. When every tap is far off, it falls back to the
   nearest-depth tap. It writes `input + scatter` to a separate texture.

With `volumetric: None`, or no light with a weight or beam, the frame does
nothing: no pass, texture or uniform. Growing Guns' goldens and every
day, sunrise, sunset and `godot` capture in `scripts/sky.json` are
byte-identical to the build before this change, and so are the night
captures with `light volumetric=0`. Sideways never sets it. The scene's
extinction stays with the existing distance fog; this pass only adds light.

## Cost

Measured with `cargo test -p render --lib volumetric::tests::gpu_cost --
--ignored --nocapture` on llvmpipe (`KERN_ADAPTER=software`, 2 CPUs, 28
October 2026). The figures are the wall time of both passes per view, at sky
depth (full-length rays, the worst case), with a Growing Guns-like set of
lights: 4 floods (38 degrees, range 80 m), 12 beams and 14 omnis.

| internal size | 1 light | 8 lights | 30 lights |
|---|---|---|---|
| 1280x720  | 53 ms | 60 ms | 154 ms |
| 1920x1080 | 96 ms | 139 ms | 405 ms |

These are CPU-rasteriser numbers, not GPU numbers. On a real GPU I estimate,
without having measured it:

- each fully evaluated light costs about 400 ALU operations per half-res
  texel (6 nodes of tan/sinh, sqrt, pow and exp each);
- for 1080p internal (518k half-res texels) with about 15 lights hit, that is
  about 3 GFLOP, so 0.2-0.4 ms on a 20 TFLOP desktop GPU;
- the composite at 1080p output is about 0.05-0.1 ms;
- a native 4K view is about 4x, so 1-1.5 ms.

Lights whose sphere or cone the ray misses cost a few operations. For
Sideways' four-view budget it should stay off.

## Growing Guns

`sky::volumetric` turns it on once the sun is below the horizon, fading in
down to -6 degrees. Day, sunrise and sunset are unchanged. Density is
0.0003/m in clear air, 2.5x in haze and 5x in mist. The tint is slightly
warm, with g = 0.45.

Scatter weights (`view::Scatter`):

| light | weight | count |
|---|---|---|
| wide floods | 0.2 | 4 |
| scatter-only beams (2.5-6.5 degrees, fanned over the field) | 1 | 3 per bank, 12 |
| lava glows | 1 | up to 14 |
| roof beacons | 0 | |
| pickup ground lights (`pickup_scatter`) | 4 | 1 per pickup |
| pickup drop beams: scatter-only searchlights up from the landing spot (0.6-2.2 degrees) | 4 | 1 per pickup, after the stadium beams |

The beacons have weight 0 because otherwise the 32-light budget runs out.

Tune live with the `command` tool, for example
`light vol_density=0.0005 vol_anisotropy=0.6 flood_scatter=0.3 lava_scatter=0.5 pickup_scatter=2`.
`light` shows the current values and `light reset` drops them. Overrides
survive sky changes.

## Limits

- No occlusion of the air: there are no shadows in the beams, and a lamp
  behind a wall still glows in front of it if the ray reaches its sphere.
- Single scattering in a homogeneous medium, with no height falloff.
  Transmittance is approximate. The scene behind is dimmed only with
  `extinction`, by the opaque depth, so a blended surface in front of a wall
  is dimmed as if at the wall.
- Half resolution: thin silhouettes can show a slightly soft haze edge.
  There is no temporal filtering, and none is needed.
- The beams are scatter-only by design. Narrow surface spots would light the
  field unevenly.
- Dark haze gradients can show 1-LSB banding after tonemapping, because
  there is no dither.
