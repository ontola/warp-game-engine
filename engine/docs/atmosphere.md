# Procedural atmosphere foundation

The procedural sky is an explicit developer option. The regular playtest
still uses the HDRI until the new sky has been visually reviewed.

`render::atmosphere` integrates Rayleigh and aerosol Mie single scattering,
ozone absorption, atmospheric transmittance and the planet's shadow. Units
are kilometres and linear RGB. A solar elevation/azimuth controls the sky,
direct sunlight and the SH9 diffuse environment together. The texture has
no solar disc, so a bright sun is not counted again as diffuse sky lighting.
The background renderer adds a visible disc and GPU lens flare, described in
[Live sky controls](live-sky-controls.md#visible-sun-and-lens-flare).
Specular environment lookup uses that same sun-free texture. Lower-hemisphere
radiance is an approximate Lambertian ground bounce, not a terrain simulation.

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

- Atmospheric multiple scattering and a true aerial-perspective volume.
- Volumetric clouds, cloud transmittance/shadows and shared wind-driven weather.
- A moon and additional night-specific light sources.
- Scheduled GPU LUT updates, live solar movement and exposure policy.

The current distance fog converges to the visible sky in the same direction,
which removes mismatched fully fogged silhouettes. It is still an image-based
blend, not physically integrated aerial perspective.

The model draws on Hillaire's [2020 atmosphere paper](https://sebh.github.io/publications/egsr2020.pdf)
and [author's reference implementation](https://github.com/sebh/UnrealEngineSkyAtmosphere).
This independent single-scattering foundation does not implement that full
production method or Nubis's volumetric cloud system.
