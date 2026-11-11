# Growing individual hairs on the GPU

On 11 November 2026 on Mancave (RTX 5070 Ti, Vulkan), the reference build was `145d8a58`. We captured 36 consecutive match seeds in arena 13 with full graphics at noon, two views at a combined 2560 by 1440, and a fixed camera, player slot and tick. The three cuts occurred 14, 10 and 12 times. Seeds 12 and 27 had especially similar fitted face surfaces.

The first experiment placed textured strips on the fitted hairstyle. Large facets showed through at the temples and the fringe. A GPU strand experiment then showed hard rows of pixels at the brow. The player pass now expands a ribbon in the view plane around each procedural curve. Its alpha preserves coverage when a hair projects to less than a pixel. Independent tip lengths, tapered brow clearance and a filtered edge interrupt the repetitive hairline.

The cooker adds one 73,728-byte guide table. The GPU interpolates the guide surface and face morphs and builds nine segments per hair using the vertex ID. No per-strand mesh or simulation buffer is stored. Players use their animated head palette and game-profile hair colour. Close spectators use their existing GPU pose, stadium lighting and fog. The base hair carries the volume and its shadows. Free tips move with a deterministic oscillation, with no strand collision solver.

The [Rockstar trailer](https://www.rockstargames.com/VI/downloads/videos) is a visual reference. The [AMD TressFX developer guide](https://gpuopen.com/download/TressFX5.0_DeveloperGuide.pdf) documents a separate guide-and-follow hair implementation.
