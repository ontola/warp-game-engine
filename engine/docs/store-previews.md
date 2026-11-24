# Store preview clips

Every game in the Splitscreen Club store leads its page with a five-second
gameplay clip (960x540, 30 FPS, 150 frames, silent H.264) and a poster frame.
What makes a good one, and the recipe format, are in Splitscreen Club's
[store clip guide](https://splitscreen.club/docs/store-clip#keep-the-recipe-not-the-video).

The repository keeps the clip's recipe, never the clip: the video lives on
the store's storage, uploaded through the API with a publishing key. When a
game's graphics change, run the recipe again and publish. The tool is
`gamenight-publish.py clip`, the same script every developer uses to publish
(`curl -O https://splitscreen.club/developers/gamenight-publish.py`, or
`scripts/gamenight-publish.py` in ontola/splitscreen-club):

```sh
cd games/spellstorm/store
python3 gamenight-publish.py clip                        # render, cut, encode, check
python3 gamenight-publish.py clip --render-size 480x270  # quick draft for timing
python3 gamenight-publish.py clip --only boss            # redo one cut
GAMENIGHT_API_TOKEN=gn_pub_... python3 gamenight-publish.py clip --reuse --publish
```

The recipe is `preview.json` next to the game's `gamenight.json`. Each cut is
a command that writes numbered PNG frames into `{dir}` from the game's real
simulation and renderer (a headless `spellstorm shot --out-dir`, a
`warp script` capture), with how many frames to skip and keep; the kept frames
add up to 150. Keep seeds, levels and tick ranges in it with a `note` on what
each cut shows, so the next run makes the same clip with the new graphics.

Without `--publish` it only makes the clip, in `build/preview/` next to the
recipe (ignored by git): `preview.mp4`, `preview.jpg`, the frames and
`provenance.json` (commit, recipe hash, cuts, the files' SHA-256). Watch it
before you publish. Render the real clip on a GPU; a software renderer is for
drafts only. `--publish` uploads both files through the API and sets them as
the store page's video and poster; title, text, cover and screenshots stay as
they are.

[Spellstorm's recipe](../../games/spellstorm/store/preview.json) is the
example: three headless `spellstorm shot --out-dir` sequences with two bots,
cut 70 + 40 + 40 frames.
