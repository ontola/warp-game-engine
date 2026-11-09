# Shared wardrobe, body proportions and planted motion

On 9 November 2026 we compared a Windows release build against `c4acf451` on Mancave (GeForce RTX 5070 Ti). The baseline had the original pullover, jeans and shoes.

The new close mesh fits the CC0 hm08 short-sleeved T-shirt and jacket alongside the pullover. A garment visibility mask selects clothing and the exposed skin. Both new tops share their jeans. The existing outfit atlas tile stores the three cloth maps in RGB and print glyphs in alpha. A single shader combines two digits or selects a wordmark at the chest. The same print sheet feeds the player's skinned shirt. Full-sized players share the animation matrices and socket tracks of their donor rig.

| Close geometry | Before | After |
| --- | ---: | ---: |
| Shared vertices | 26,711 | 34,852 |
| Bytes per GPU vertex | 68 | 44 |
| Bytes per index | 4 | 2 |
| GPU vertex and index data | 2,342,236 B | 1,876,184 B |
| Cooked model on disk | 1,540,914 B | 1,667,080 B |

The close mesh uses 19.9% less GPU geometry memory. The 126,166 additional bytes on disk hold the new forms. The crowd still uses a 64-byte identity per spectator and a shared 2048 × 1024 RGBA8 atlas. The figures above cover the close mesh; game-wide textures, animation buffers and render targets are separate.

The visual comparison used seed 52, a 960 × 540 travelling crowd view with a fixed second camera, and a full-graphics Growing Guns lineup at 1600 × 900. Shoulder, waist and hip widths vary under the clothing. The stance has a fore-aft foot stagger and a lateral pelvis shift fed back into the ankle targets. While stepping along a row the shoulders counter-rotate with the step.

With 6,638 spectators at seed 52, total resident crowd buffers in the two-camera fixture went from 4,802,112 to 4,307,728 bytes. Appearance storage was 8,388,608 bytes in both versions. A second old/new/new/old series of 61 frames gave median GPU times of 0.129 ms before and 0.131 ms after for the wide camera. The travelling camera during resident close detail measured 0.801–0.802 ms before and 0.806–0.808 ms after.

A separate offscreen 1920 × 1080 full match with 10,700 spectators, one view and 120 frames returned 23.0 and 24.5 serialized FPS before and 18.1 and 20.9 after. The Warp Humans pass itself measured 0.71–0.95 ms before and 0.52–0.54 ms after. Whole-frame median GPU times overlapped (27.5–30.2 ms before, 27.1–31.5 ms after). Compiler activity and varying CPU submission times limit the interpretation of whole-game throughput on this machine during these runs.

The final match captures exercised three bots, a ragdoll and a separate two-shot and explosion scenario. The latter completed 229 ticks and all spectator-count assertions.

After the six focused tests passed and compilation finished, a reversed new/old/old/new full-match series gave 14.7, 91.3, 22.7 and 25.1 serialized FPS. Even the two consecutive baseline runs differed fourfold. These full-match runs cannot isolate a wardrobe FPS delta on this machine. The allocation counts and the separate two-camera measurements above are repeatable.
