# Finite-distance aerosol layer

The sunset haze/mist captures `sideways-1792830927904-269560-2` and
`sideways-1792830972228-269560-3` show a horizontal orange/dark band through
opaque forest. The previous distance fog used the complete sky texture as
its scattering colour. That texture includes a dark planetary lower
hemisphere, which cannot be treated as light scattered between a camera and
a nearby tree.

Procedural skies now set `Lighting::fog_height_m > 0`. Surface fog integrates
Beer-Lambert extinction to the actual hit. The density profile is
`fog_density * exp(-max(world_y, 0) / fog_height_m)`. Its column integral is
analytic, including the horizontal limit and crossings of the reference
height. A normalised Cornette-Shanks phase uses the same aerosol asymmetry
as the sky bake. Scattering albedo is the model's scattering/extinction
ratio. The incident source combines local solar illuminance and the mean
sky radiance from SH0. It does not sample the sky horizon.

Background rays pass through the same additional layer. Only background
rays use the baked sky as a boundary radiance. Opaque, transparent, near
grass, far grass and voxel-tree shaders call the shared finite-distance
helper. `fog_height_m = 0` retains legacy HDR image fog.

The physical path ignores the old linear `fog_start`/`fog_end`. It disables
the associated radial culling because exponential extinction never gives
an exactly opaque finite radius, and the density changes with height.
This may increase geometry cost in mist; GPU cost must be measured before
making performance claims.

The layer is an inexpensive local approximation, not a complete aerial
perspective volume: illumination is constant along the segment, diffuse
light uses SH0, and no terrain/canopy shadow is integrated through the air.
There is no multiple scattering. The density datum is world y=0, not the
local terrain surface. These limits can still make forest mist too bright.

CPU evidence is under `C:/dev/perf/warp-sideways-20261024/r16/finite-fog.json`:
9 fog tests, 5 atmosphere tests, and the LightUniform layout test pass.
The fog tests execute the actual parsed WGSL expressions against independent
4096-step quadrature, check horizon continuity and phase normalisation, and
validate the five composed shader consumers. GPU appearance and cost remain
unverified until a controlled replay; the live playtest was not interrupted.
