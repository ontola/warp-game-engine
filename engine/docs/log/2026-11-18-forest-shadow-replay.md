# Swiss forest shadow replay

18 November 2026. Mancave, RTX 5070 Ti, Vulkan, 2560 × 1440. The original
capture used branch `claude/project-thread-s7vbe4` at 82e597d7, seed 1,
Swiss spruce forest, 300 frames of warmup and a 4 m/s walk from
`-21,-116,1.7,0.4,0`. The pop meter used a 16-pixel search radius.

## The meadow

The original clip has a 576 × 208 px one-frame change on frame 39. Two
older builds (a62ef085 and 2c86ddf1) also reproduce the change. Neither
sun scale, shadow cut nor voxel residency changed at the event. With all
sun maps redrawn every frame, the jump moves to frame 23. Tracking terrain
retessellation as damage also exposes it on frame 23.

For a fixed receiver at (−23.52, 653.95, −125.20), the fully redrawn
inner depth map at frame 22 contains a caster 593.2 m towards the sun at
(346.83, 932.44, 245.15). On frame 23 it contains the receiver itself.
The terrain field along the same ray rises above it at the ridge. The
camera-driven terrain cut had coarsened an off-screen caster that still
projected onto the nearby meadow. Reuse delayed the visible change until
the affected tiles were redrawn.

Terrain selection now accounts for receiver-side projected shadow error.
Over the first 60 transitions the largest full-frame pop score falls from
24,081 to 246. Frame 39 alone falls from 24,081 to 43. The mean limited-range
8-bit luma in the meadow rectangle moves from 30.51 to 55.51 across frames
38–39 before the change and stays at 29.06 afterwards. A separate cache
fix retains deferred damage through the consuming draw.

## The near branch and the distant trees

Two 295-frame walks used identical inputs. One held the forest snapshot
from frame 281 through the rest of the walk. The normal run refines tree
22966 on frames 283 and 286. For the 272 × 224 px region at (1552, 0),
the mean absolute frame-283-to-284 change was 55.47 grey levels in both
runs. Comparing the two frame-284 images directly gives less than 0.01
grey levels of mean absolute difference. Camera disocclusion accounts for
this measured event. The highest-scoring event in the lowland tree clip
also follows a trunk across the lens.

In the normal walk all 2,998 on-screen appearances and 3,033
disappearances coincided with the corresponding current-depth root visibility
bit changing. The closest root was 576 m away. This measures selection
churn. Around 271 such events during frames 282–294 we compared 593,828
pixel samples between the normal and frozen runs. Seven samples differed
by more than eight grey levels. The largest region-mean absolute difference
was 0.078 on an 8-bit scale.

[Comparison clips and pop graph](https://github.com/ontola/warp-game-engine/tree/look-reports/reports/2026-11-18-forest-pop)
