# Close faces, gaze and eyelids

On 10 November 2026 we compared release builds based on `f53223c9` on Mancave (Windows, Vulkan, RTX 5070 Ti). The Growing Guns fixture holds the second player in arena 13 with paused bots, then frames them from the front and three quarters. We captured two views in a 2560 × 1440 image with full graphics, noon lighting and seeds 11, 22, 23, 45, 52 and 873. The night fixture uses the same camera and player as seed 52. Separate fixtures start among the spectators at noon and night.

The hm08 nose profile, chin, temple and cheek targets extend the four existing sparse identity fields. A separate 230-entry lid shape drives seeded blinks. The crowd shader unpacks those deltas from spare vertex components. Players reuse the head pose for two eye pivots and six signed lid translations. The blink sequence samples every two simulation ticks around tick 130. A light-dependent corneal highlight and shade under the upper lid give the eye a surface. The face atlas now carries the T-zone and lip sheen in its alpha channel.

The first match captures showed coarse highlights on the bob and bands across the coiled style. We lowered the bob's sheen and replaced the intersecting curl sheets with a fitted crown, tapered outline and repeatable curl texture. All three styles keep their seeded tint. The far and medium geometry use the same instance data as before.

The previous shared mesh and atlas occupied 3,714,555 bytes on disk. The new mesh occupies 1,234,007 bytes and the atlas 2,290,288 bytes, a combined 3,524,295 bytes. The close mesh contains 35,750 vertices and 59,575 triangles including all outfits and hair options. The spectator instance is 64 bytes, the packed mesh vertex is 44 bytes and the resident atlas still occupies 8 MiB. Each fitted player adds eight matrices to its animation palette.

The six 17-frame stadium reviews streamed the anatomical mesh by frame 6. On the delivered build the six fixtures contained 6,530 to 9,139 spectators. The same player fixture recorded 58 consecutive frames at 30 fps while the player looked up, returned to neutral and blinked. All 23 focused character, ragdoll, asset and crowd tests passed. A second cook using the pinned sources reproduced both shipped assets byte for byte.

The serialized Vulkan match benchmark used seed 52, full graphics, 1920 × 1080 split between two views and 10,700 spectators. Each run measured 150 frames after 30 warmup frames. We ran old, new, new, old after compilation and capture had finished. Times in milliseconds.

| Run | Median wall | P95 wall | Median covered GPU | P95 covered GPU | Mean Warp Humans pass |
|---|---:|---:|---:|---:|---:|
| Old 1 | 6.09 | 7.46 | 3.04 | 3.28 | 0.263 |
| New 1 | 6.06 | 7.47 | 3.08 | 3.34 | 0.284 |
| New 2 | 6.20 | 7.19 | 3.06 | 3.33 | 0.284 |
| Old 2 | 6.08 | 7.46 | 3.03 | 3.29 | 0.263 |
