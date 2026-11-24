# Store media

The store's gameplay clip is generated, not kept: `preview/gen.py` is the
source. It writes `preview/script.json`, a `warp script` capture of one staged
match (v2, 2026-11-24: minigun, bazooka blasts, a big masonry-carving blast,
shotgun with an extra barrel, explosive minigun past the crowd; 250 frames,
30 FPS, 1920x1080). Render it on a real GPU (the Mancave), encode the frames to
a silent H.264 `preview.mp4` plus a `preview.jpg` poster, and upload both
through the publish API, which stores them in S3. The video files do not go
into this repo; regenerate them after graphics changes by running the same
script again (Mancave v2: `growing-guns script preview/script.json --players 1 --seed 7`, ffmpeg crf 26). Follow the
[gameplay preview guide](https://github.com/ontola/gamepad-party/blob/main/docs/gameplay-previews.md).

The store's cover, icon and screenshots are still the ones uploaded from the
Godot game. Upload the clip with `gamenight-publish.py store --game growing-guns
--manifest none --media none --video PATH/preview.mp4 --poster PATH/preview.jpg`.
