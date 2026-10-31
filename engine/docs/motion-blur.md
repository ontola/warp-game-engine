# Per-view motion blur

`Renderer::set_motion_blur(gpu, &[motion_blur::Settings])` opts a native game
into bounded camera and rigid-instance motion blur. The default renderer does
not allocate this effect. Supply one setting per view, in view order.

`delta_seconds` is elapsed **presentation simulation time**, not CPU wall time.
A fixed `shutter_seconds` divides this interval so changing the presentation
rate does not change exposure length. Set strength to zero during pauses,
respawns and camera transitions. Zero/nonfinite/over-100-ms intervals and
invalid motion history skip blur. `max_pixels` is a 720-pixel-height reference
and scales with each viewport. SpaceRacer policy is in `rules/effects.luau`.

The standard visibility renderer generates its existing motion metadata even
without DLSS, retaining previous camera/instance/wind state per view. A 12-tap
HDR compute pass follows reconstruction, particles and exhaust, before bloom,
tonemapping and HUD. Depth/velocity rejection protects stationary foregrounds.
Targets and uniforms are view-local, so split-screen seams cannot bleed.
Blurred output never feeds TAA/DLSS history. The `motion blur` GPU timestamp is
classified as post-processing; motion generation has its own timestamp.

Limits: no tile-max velocity dilation or full disocclusion reconstruction;
not every custom vertex deformation supplies motion (SpaceRacer octopus skin
motion is not independently reconstructed). Particle motion uses scene motion
under the particle. The forward-only path does not run this effect. Once opted
in, disabling strength skips blur but retains motion resources/generation;
restart without opting in to measure the entire overhead. Mobile/VR quality
and performance have not been validated.

Tests validate shader/layout and shutter/pause/cut limits. SpaceRacer's
`scenarios/crash-effects.json` captures advancing boost and crash frames;
static warmed screenshots are insufficient for reviewing motion blur.
