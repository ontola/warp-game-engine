# Store media

`gamenight-publish.py store` uploads these files to the store page:

| File | What |
| --- | --- |
| `preview.mp4` | five seconds of real gameplay, 960x540, 30 FPS, silent; required to go live |
| `preview.jpg` | poster frame for the clip |
| `cover.png` | portrait cover, 360x480 |
| `icon.png` | square icon, 128x128 |
| `gameplay.png` | screenshot for the TV, 16:9 |
| `screenshots/*.png` | extra gallery images, in name order |

The cover, icon and gameplay shot are real gameplay captured with `warp script` and `debug.hud(false)`.
Read the [gameplay preview guide](https://github.com/ontola/gamepad-party/blob/main/docs/gameplay-previews.md) first.

## preview.mp4

Five cuts of real play, captured headless (software renderer, `warp script`) at warp main 73d0a21c, `--players 2 --seed 11`, 960x540, one frame every 2 ticks (30 fps). Staging (scripts in `preview/`, made by `preview/gen.py`, run by `preview/run.sh`): HUD off, `autopilot` 1 (bots play both people), camera `cam_height` 15 (closer than the default 24) and `night_ev` 4.4 (a little brighter than 5.1); debug commands set up each moment.

| Cut | Source frames | Length | What it shows |
| --- | --- | --- | --- |
| 1 | E 4-35 | 32 | a sleeping horde wakes round the group; torch beams in the fight (`debug.event("sleeping_horde")`) |
| 2 | D 8-37 | 30 | a burning car lights the street (`debug.event("burning_car")`) |
| 3 | F 16-43 | 28 | inside a dark building, only the torch (`debug.room(2)`, people standing) |
| 4 | B 2-29 | 28 | the dead grab and pull people down (`debug.peak()`) |
| 5 | A 12-43 | 32 | heavy blows, bodies and blood on the street (`debug.peak()`) |

Encoded with libx264 crf 24, preset slow, yuv420p, limited range, +faststart, no audio; 150 frames, 150 distinct. Poster: A frame 20.
preview.mp4 sha256 4211bc1e03b8dc9d5661376403c7cfb27d88e120fc99369d2369e5fea24495c8
preview.jpg sha256 4f79a535563449d5db17bdb8311b22636cc1c36273b18750c313f6780d04e1eb
