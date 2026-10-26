# Camera terrain occlusion of solar haze

The reported orange glow over an opaque cliff came from the solar term in
`fog_scattering_source`. It used the view/sun angle without testing the terrain
between the camera and the sun. Surface shadow maps do not mask this term.

The renderer now evaluates the terrain horizon once per camera, using the
existing CPU heightfield and its actual triangle diagonal. An allocation-free
DDA checks every crossed cell and diagonal; large march steps cannot skip a
narrow ridge. The visible fraction of a finite solar disc fades at the horizon.
It depends on camera position and sunlight direction, never camera rotation,
jitter or the focus used for near shadow maps. Each split view is independent.

Only direct solar fog scattering is multiplied by that visibility. Diffuse
skylight, extinction and ordinary material lighting remain unchanged. The same
fog function is used by opaque, transparent, grass and foliage-volume shading.
The scalar occupies spare camera uniform space; no extra GPU pass or texture
is needed. `KERN_CAMERA_SUN_OCCLUSION=0` restores the old behavior for comparison;
the fix is on by default. F12 profiles record `camera_sun_visibility` per view.

This is a bounded camera-conditioned approximation requested for the glare,
not fully shadowed volumetric integration. It does not resolve foliage or cars
blocking the camera, nor sunlit air beyond a camera in shadow. Unknown terrain
outside the loaded heightfield is treated as unobstructed. The solar-disc
fraction assumes a locally straight horizon across the disc's azimuth width.

## Validation, 24 October

- 191 renderer CPU tests passed, one existing ignored test. These include the
  seven horizon cases and validation of every composed fog consumer shader.
- Four long horizon queries on a 1025-square, 2 m grid measured 121 microseconds
  total per batch in the optimized test profile. This is CPU-only evidence,
  not a gameplay or four-player FPS result.
- `sun_fog_visibility` renders matching terrain mesh/heightfield, blocked,
  rotated, elevated and side cameras, and independent split views. Enabled
  visibility is exactly `[0, 0, 1, 1]`; disabled is `[1, 1, 1, 1]`.
  Both final GPU runs pass, and exposed-camera PNGs are byte-identical on/off.
  A one-byte blocked RGB tolerance accommodates real grazing surface Fresnel
  at three samples; the visibility and single/split image checks remain exact.
- Replayed both user F12 captures at the recorded camera, light and weather.
  Orange screenshot `sideways-1792853911138-99004-1` changes from visibility 1
  to 0, removing the glow over the wall. Afternoon screenshot
  `sideways-1792854017421-99004-2` remains visibility 1 and pixel-identical:
  the 28-degree sun clears the sampled terrain horizon at that camera.
  Its remaining white haze is not evidence of a terrain blocker in this model.

Evidence: `C:/dev/perf/warp-sideways-20261024/r24-sun-fog/`.
`orange-before/lit.png`, `orange-after/lit.png`, `white-before/lit.png`,
`white-after/lit.png` are native 1920x1080 diagnostic reconstructions without
cars/HUD, not original DLSS gameplay captures. Both comparison pairs share
the updated rolling background terrain; only the sun-occlusion toggle changes.
`fixture-enabled-final` and `fixture-disabled` contain the accepted GPU checks;
the earlier `fixture-enabled` retains the initial overly strict one-byte failure.
