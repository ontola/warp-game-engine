# Store preview clips

Every game in the Splitscreen Club store leads its page with a five-second
gameplay clip (960x540, 30 FPS, 150 frames, silent H.264) and a poster frame.
What makes a good one is in Gamepad Party's
[gameplay preview guide](https://github.com/ontola/gamepad-party/blob/main/docs/gameplay-previews.md).

The repository keeps the clip's recipe, never the clip: the video lives on
the store's storage, uploaded through the API with a publishing key. When a
game's graphics change, run the recipe again and publish.

```sh
engine/tools/store-preview.sh games/spellstorm            # render, cut, encode, check
GAMENIGHT_API_TOKEN=gn_pub_... engine/tools/store-preview.sh games/spellstorm --publish
```

Without `--publish` it only makes the clip, in `build/store-preview/GAME/`:
`preview.mp4`, `preview.jpg`, the 150 edit frames and `provenance.txt` (the
commit, the recipe, when, and the files' SHA-256). Watch it before you
publish. With `--publish` it uploads both files to
`/v1/developer/games/GAME/uploads` and sets them as the store page's video
and poster; title, text, cover and screenshots stay as they are. The key is
the game's publishing key from the developer portal (`gn_pub_...`), the same
one `gamenight-publish.py` takes; `--server URL` or `GAMENIGHT_SERVER`
points it at another Splitscreen Club server.

It needs bash (Git Bash on Windows), curl, ffmpeg and ffprobe, and the GPU
the game renders on.

## A recipe

A game's recipe is `store/preview/recipe.sh` (or `preview/recipe.sh`) in its
folder, next to the `gamenight.json` (in the folder or its `store/`) whose
`id` names the game. It defines one bash function, `capture`, that:

1. renders each shot as numbered frames (`00000.png` and on) into a folder
   under `$OUT`, from the game's real simulation and renderer: a headless
   shot, a `warp script` capture, bots or scripted input;
2. cuts the edit with `cut DIR FIRST LAST` (0-based, inclusive frames,
   appended in order) until it has exactly 150 frames;
3. sets `POSTER` to the edit frame used as the poster (`00020.png`).

`$ROOT` is the repository, `$GAME_DIR` the game's folder, `$OUT` the output
folder and `$EDIT` the edit. The tool then checks the count, that at least
100 frames differ, and the encoded clip's size, rate and length. Keep the
seeds, levels and tick ranges in the recipe with a line on what each shot
shows, so the next run makes the same clip with the new graphics.

[Spellstorm's recipe](../../games/spellstorm/store/preview/recipe.sh) is the
example: three headless `spellstorm shot --out-dir` sequences with two bots,
cut 70 + 40 + 40 frames.
