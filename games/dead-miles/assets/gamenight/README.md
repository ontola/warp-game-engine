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

The cover, icon and gameplay shot are real gameplay captured with `warp script` and `debug.hud(false)`. No `preview.mp4` yet.
Read the [gameplay preview guide](https://github.com/ontola/gamepad-party/blob/main/docs/gameplay-previews.md) first.
