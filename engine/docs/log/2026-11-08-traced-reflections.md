# Traced reflections, stage 1 (8 November 2026)

First stage of dynamic GI ([dynamic-gi.md](../dynamic-gi.md)): glossy
surfaces trace their reflection through the depth buffer and read the hit's
radiance from last frame's TAA history.

Checked in the cloud container on lavapipe (software Vulkan, not a GPU):

- terrain_fly `--style polder --at 0,0,1.2,0.6,-0.12 --speed 1 --dump 4`,
  960x540, frame 4: the ditches now mirror the trees, crowns and banks
  around them; before, they showed only sky and the flat ground bounce.
- Sideways `shot --frames 400 --taa 4 --cam yaw=150,dist=5,height=1.4`,
  960x540: car windows and paint pick up the road and trees (subtle: the
  coat is the only glossy layer, and most of it faces the sky).
  lavapipe GPU time, run alone, on vs off: shade terrain 29.9 vs 30.7 ms,
  shade 114.6 vs 97.4 ms (+18 % in a close-up full of car paint). Not a
  real GPU measurement; time it on Mancave or the M5 Pro.

Render lib tests: 844 passed. The meadow visibility test's texture budget
for hidden shade goes from 20 to 21 (the reflection history), and so does
`portable::FULL_SAMPLED_TEXTURES`.
