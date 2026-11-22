# Store media

`preview.mp4` is the store's five-second gameplay clip (960x540, 30 FPS,
150 frames, silent H.264) and `preview.jpg` its poster: a real render of the
Warp port on the Mancave GPU. `preview/script.json` is the `warp script`
capture and `preview/gen.py` cuts and encodes it. Follow the
[gameplay preview guide](https://github.com/ontola/gamepad-party/blob/main/docs/gameplay-previews.md).

The store's cover, icon and screenshots are still the ones uploaded from the
Godot game. Upload these with `gamenight-publish.py store --game growing-guns
--manifest none --media none --video preview.mp4 --poster preview.jpg`.
