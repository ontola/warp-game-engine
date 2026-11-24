# Store media

`preview.mp4` (5 s, 150 frames, 960x540, 30 fps, silent H.264) and `preview.jpg` are real headless `warp script` captures of the Warp version (warp 69ba2c6, without the round heads beside the well), software render in the cloud, so not GPU quality. `gameplay.png` is the TV screenshot (1280x720). The store keeps the cover and icon uploaded for the earlier LÖVE version. Guide: https://github.com/ontola/gamepad-party/blob/main/docs/gameplay-previews.md

Capture (2026-11-24): a scratch copy of the game with `bot = true` for both halves (staging: two bots play, nothing else changed), `warp script COPY take.json --out takeSEED --players 1 --bots 1 --seed SEED` for seeds 11 and 23, where take.json is 60 warm-up ticks and then 900 frames, one capture every 2 ticks (30 fps). Edit, 30 frames per cut: take23 f100 (opening, row clear), take11 f40 (row clear), take23 f165 (second clear), take11 f400 (both halves near the top), take23 f478 (the stack tops out: the coral cross). Poster: take23 f120 (a clear). Encoded with libx264 CRF 24, preset slow, yuv420p, +faststart, no audio, bit-exact. 150 distinct frames.
- `08159a4ce46b5a0dc5973b08bbc4763485930791e908333073ae8aaeee7453e1`  preview.mp4
- `bc02180c900a35e530263c31cdc364215bd205694c685e7e56a62ad895dc4ff6`  preview.jpg
- `4c7a196726db6fefe6d28a191faad0313b962b204f6366ce221ed4067e326e1a`  gameplay.png
