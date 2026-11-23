# Procedural atmosphere, aerial perspective and clouds

The procedural sky is an explicit developer option. The regular playtest
still uses the HDRI until the new sky has been visually reviewed.
`terrain_fly` uses it, with aerial perspective and clouds on.

`render::atmosphere` integrates Rayleigh and aerosol Mie scattering,
ozone absorption, atmospheric transmittance and the planet's shadow. Units
are kilometres and linear RGB. A solar elevation/azimuth controls the sky,
direct sunlight and the SH9 diffuse environment together. The texture has
no solar disc, so a bright sun is not counted again as diffuse sky lighting.
The background renderer adds a visible disc and GPU lens flare, described in
[Live sky controls](live-sky-controls.md#visible-sun-and-lens-flare).
Specular environment lookup uses that same sun-free texture. Lower-hemisphere
radiance is an approximate Lambertian ground bounce, not a terrain simulation.

Multiple scattering follows Hillaire (2020): a 32×32 LUT, per height and
sun angle, of all scattering orders past the first (isotropic, with one
ground bounce, summed as a geometric series), added to the single-scattering
source wherever the sky is integrated. It is on by default
(`Atmosphere::multiple_scattering`); it never changes the direct sun. At the
default atmosphere it raises the noon skylight by about 40 % (green
5.7 to 8.0 klux at 60° sun) and makes shadowed sky and twilight bluer.
`false` gives the earlier single-scattering bake.

## Aerial perspective and clouds (`Lighting::sky_volume`)

`render::sky_volume` is an opt-in pass after temporal reconstruction (like
[volumetric light](volumetric-light.md)), into a separate output that never
enters TAA or DLSS history. `Lighting::sky_volume = Some(SkyVolume)` turns it
on; `None` (the default) adds no pass and changes nothing.

- **Aerial perspective.** Per view, a 32×32×32 camera volume (froxels, 80 km
  deep, quadratic slices) of the light the air scatters towards the camera
  and its transmittance, integrated on the GPU with the same medium,
  transmittance LUT and multiple-scattering LUT as the CPU sky bake
  (`BakedSky::tables`, uploaded once per bake). Every surface pixel is
  `colour * T + L` at its depth. Below the horizon, the background (the
  bake's ground seen from ground level) fades with the air up to where the
  ray meets the planet. While it is on, the shade pass's distance fog is off
  (its extinction is set to zero) so the scene is not fogged twice.
- **Clouds.** One cumulus layer (`Clouds`: coverage, base, thickness,
  extinction, wind, shadow strength, steps), raymarched at half the depth
  resolution up to the scene depth with 64 samples, sun light through the
  cloud in 6 doubling steps, a two-lobe phase, four octaves of approximate
  multiple scattering (Wrenninge), sky light from above and ground bounce
  from below. Its shape is Perlin-Worley and Worley noise in two tiling 3D
  textures generated on the GPU once; a 2D weather field
  (`shaders/cloud_weather.wgsl`) decides where clouds stand. The result
  accumulates over frames in its own half-resolution history (reprojected
  at the cloud's depth, clamped to the current neighbourhood), and is
  composited behind its own aerial perspective. Clouds drift with
  `wind * Lighting::time`.
- **Explosion effects.** Each view composites its cloud layer before drawing
  smoke, fire and heat distortion. These effects read cloud transmission at
  their own depth. The cloud march records its first, mean and last
  contributing distance in the existing depth texture. Inside a cloud this
  approximates how much opacity is in front of the effect. Effects closer
  than the layer skip cloud texture reads. The fixed-camera GPU regression
  can be run with `cargo run -p render --example cloud_fx_order`.
- **Cloud shadows.** The shade pass reads the same weather field along the
  sun ray (`cloud_sun_transmittance`), so terrain, meadow blades, rigid geometry and voxel
  trees fall dark where clouds stand over them; the air in their shadow
  scatters less sun too.

`Lighting::set` keys, also through `--light` in `terrain_fly`:
`sky_volume=0|1`, `sky_aerial=0|1`, `sky_aerial_far=<m>`, `clouds=0|1`,
`cloud_coverage` (0..1, default 0.45), `cloud_base` (m, 1800),
`cloud_thickness` (m, 1800), `cloud_density` (per m, 0.03), `cloud_wind_x`,
`cloud_wind_z` (m/s, 4 and 1.5; 0 for still clouds), `cloud_shadow` (0..1, 0.85), `cloud_steps`
(16..128), `cloud_light_steps` (2..8), `cloud_far` (m, 60000). The debug
menu's Light → Clouds removes the clouds; Render → Distance fog off removes
the aerial perspective as well. Without a procedural sky the passes bake the
default atmosphere once for their tables.

Portability: no storage buffers, at most two storage textures per entry
point, all formats in core WebGPU (rgba16float, rgba8unorm 3D).

Cost: on lavapipe (software Vulkan, 1280×720, terrain_fly default view) the
pass took 102 ms against 128 ms for `shade` and 134 ms for `shade terrain`,
so expect it to cost a bit less than one shade pass. Not yet measured on a
real GPU.

Limits: the background sky is still the bake seen from the ground (no sky
seen from altitude, no clouds in reflections or in the SH9 skylight, so an
overcast sky does not dim the scene); terrain does not shadow the air;
legacy grass and foliage cards in their separate passes get no cloud shadows;
one layer of one cloud type.

## The CPU bake

The default bake uses a 128×64 transmittance LUT and a 256×128 sky texture.
It is bounded CPU work at world construction, shared by all split-screen
views. Rebuilding the stage with the same parameters reuses the cached bake.
A changed solar position reuses atmospheric transmittance but rebakes the
sky. The cache retains only the latest sky and one transmittance LUT; this
is not per-frame time-of-day animation. Updating a live sky currently needs
a new renderer. Defaults and exposure are not automatically changed.

To opt into a development playtest or shot, set these process environment
variables before using the normal launch command:

```powershell
$env:SIDEWAYS_SKY = 'procedural'
$env:SIDEWAYS_SUN_ELEVATION = '60'
$env:SIDEWAYS_SUN_AZIMUTH = '125'
```

Elevation is in degrees from −90 to 90. Azimuth is in degrees, with zero
towards −Z and 90 towards +X. Setting `SIDEWAYS_SKY=hdri` or removing it
restores the original sky. Invalid opt-in values fail explicitly. These
settings and actual solar/sky illuminance are recorded in shot, F12 and
playtest metadata. The developer launch manifest also records the three
environment variables. An explicit shot `--set` lighting override applies
after the bake and is separately recorded; it can intentionally break the
shared physical lighting state for diagnosis.

Use clear weather when evaluating the foundation. Existing overcast, mist
and wet presets still alter exposure, shadow strength and distance fog;
they do not yet create corresponding clouds in this procedural atmosphere.

## Validation

CPU tests check normalized scattering phase functions, transmittance bounds
and high-step numerical references, solar reddening near the horizon, night
with no residual direct sun, SH from the same radiance, vacuum with no
double-counted sunlight, cache reuse and invalid/budgeted inputs. CLI tests
check opt-in parsing, actual applied-light metadata and reuse on another
world. WGSL validation covers the shared sky/fog lookup. The sky texture
uses bilinear sampling with longitude wrap and latitude clamping.
`multiple_scattering_brightens_the_sky_within_physical_bounds` checks that higher orders only add light, stay below 2.5 times the first
order in daylight and never change the sun. `sky_volume` tests validate
the WGSL with naga (no storage buffers), the settings, the uniform layout
and that cloud coverage sets the share of sky with cloud (a CPU copy of
`cloud_weather`). Images: `terrain_fly --shot` (clouds on by default) at
`--at 0,0,30,2.2,0.5` (looking up), `--hour 19.3` (sunset) and
`--at 0,0,2600,0.6,-0.12` (from above); see
[the 8 November log](log/2026-11-08-clouds-and-aerial-perspective.md).

`cargo build --locked --release -p render --example atmosphere_gpu` builds a
real-renderer fixture without running it. Once the GPU is free, run:

```powershell
cargo run --locked --release -p render --example atmosphere_gpu -- --out FRESH_DIRECTORY
```

It captures morning, noon, sunset and night at a fixed EV100 of 14, with
neutral/coloured objects in the same scene. The fixed exposure deliberately
leaves night dark. The JSON report records bake cost and lighting inputs;
it does not claim gameplay FPS or four-player performance. GPU images still
need review before changing a default.

## Remaining work

- Sky seen from altitude and clouds in the sky texture, reflections and
  skylight (overcast should dim the scene), weather presets driving clouds.
- Terrain shadows in the air, more cloud types and layers.
- A moon and additional night-specific light sources.
- Scheduled GPU LUT updates, live solar movement and exposure policy.

Without `sky_volume` the distance fog converges to the visible sky in the
same direction, which removes mismatched fully fogged silhouettes. It is an
image-based blend; `sky_volume` is the physically integrated alternative.

The model draws on Hillaire's [2020 atmosphere paper](https://sebh.github.io/publications/egsr2020.pdf)
and [author's reference implementation](https://github.com/sebh/UnrealEngineSkyAtmosphere).
The clouds follow the published ideas of Schneider's Nubis (Perlin-Worley
shape noise, weather-driven coverage) and Hillaire's 2016 Frostbite notes
(energy-conserving integration, multiple-scattering octaves); they are an
independent implementation, not either production system.
