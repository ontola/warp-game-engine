# Warp Human face texture memory

Compared main at `7cc30032` with the per-face upload in this change on Mancave
(Windows, RTX 5070 Ti). Both release builds used the same 2048 × 2048 face
sources and a 1280 × 720 headless output. Each run used a fresh process with
`KERN_TEXTURE_CACHE=0`. The table reports peaks for the whole Windows process.

| Scene | Faces | Peak working set before → after | Peak process commit before → after |
| --- | ---: | ---: | ---: |
| Dead Miles startup, tick 2 | 25 | 3310 → 948 MiB | 4930 → 2591 MiB |
| Human Lab portraits | 8 | 1281 → 543 MiB | 1980 → 1253 MiB |

The Dead Miles opening title card and all 15 images from
`games/human-lab/scripts/portraits.json` had identical SHA-256 hashes before
and after. Dead Miles had initialized all five character pools and reached the
play phase. Both scripts reported zero game errors. The working set at the
Dead Miles capture fell from 3052 to 800 MiB.

Each 2048-square RGBA image uses 16 MiB. The fitted character previously
retained an albedo and two copies of the normal map, and the scene retained a
separate albedo and normal. That accounts for 80 MiB per distinct face.
A 64-square preview uses 16 KiB per image. The detailed bake now needs at
most 66 rows of raster and relief data, about 4.64 MiB at 2048, alongside
the two 16 MiB output maps. Mipmap generation, encoding, upload staging and
the rest of the game also contribute to the process peaks above.

Headless captures wait for every face to finish. With the texture cache off,
Human Lab took 15.6 s before and 31.9 s after to write its first capture at
tick 34. Interactive native play displays the previews while a worker fills
the full-resolution layers. A separate headless GPU test exercised that path,
rendered the preview, drained the stream and matched the colour and normal
views of a synchronous upload byte for byte. The browser uploads one face
at a time during scene setup. These measurements cover native headless runs.
