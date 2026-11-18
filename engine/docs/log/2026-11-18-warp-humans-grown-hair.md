# Warp Humans: hair grown from the seed (18 November 2026)

Scene: `games/human-lab`, `scripts/portraits.json` and `moving.json`, 1280x720, lavapipe in the cloud container (not a real GPU). Base: main 6da6ef25.

Joep on the fitted groom shells: "super raar. Alsof er onder het haar een soort kapje zit". The shell was the hair: three cooked meshes with a hair texture, and strands only on top of it.

## What changed

- `assets::skinned::warp_hair`: 16,384 strands of ten points per fitted person, rooted on their own scalp, with a cut from the seed (cropped, short, collar length, long; parting, sweep, fringe, lift, wave).
- Fitted people no longer draw groom triangles. The scalp texels of the seed skin are tinted toward the hair colour.
- `PlayerStrands` takes `Scene::hair_strands` (binding 9) and draws style 3 from it.

## Tried and dropped

- More strands and an undercoat over the old shell: still a felt helmet.
- Pushing strands sideways off the forehead: a headband. Strands are cut where they enter the face instead.

## Cost

2.6 MB of strand points per person (16,384 x 10 x 16 bytes). Growing them takes well under the 1.2 s the skin takes. Frame cost on a real GPU not measured.

## Open

- Strands cast no shadow, and with the shell gone nothing shades the forehead or scalp under the hair.
- The ear shows through a slot in long hair seen from the side; long hair passes through the shoulders (no collision below the skull).
- The fringe ends in a straight line with square corners at the temples.
- Not yet looked at on a real GPU, nor in Growing Guns or Dead Miles.

## Later the same day (main f7f1dcaf)

Joep on the first version: odd round the ears of long hair, and the skull shows through a short cut at the crown.

- Hair rested on an ellipsoid; the head's skin lies outside it in places, so flat hair sank into the scalp, and ears were not in it at all. It now rests on the measured skin (64 by 32 directions from the head's middle).
- Lifted top hair ran straight on past the head and left a gap above the sides. Hair now bends along the head it lies on.
- The parting was a bare strip (hair left it to both sides at once); hair beside it now runs back along it first.
- Scalp tint under the hair 0.42 to 0.7 of the hair colour: under pale hair the roots read as a dark patch.

Still open: a faint whorl on pale hair where the fringe leaves the crown, and the items above.

## Third pass (main 528e64a6)

Joep on the second pass: the cuts had lost their volume ("minder hippe kapsels, is dat om het probleem te verbergen?"), the crown was still wrong, and the ear's shape showed through, lighter. He was right about the volume: bending every strand along the skull closed the gap by flattening everything.

- Bending along the skull removed. Top to sides now change over a wide band of the head (length and lift), which closes the gap without flattening.
- The star on the crown and the strands fanning over the forehead had one cause each, both gravity: on top of the head a strand's weight dragged it downhill, so hair parted in a star round the highest point; and at the front hairline "down the head" is over the face. Hair on top now stays where the comb left it, and a front that is combed back is not laid down.
- The head's measured reach is widened and smoothed (furthest skin of the 3 by 3 directions round each, averaged, twice), so hair lies over an ear as over a soft rise.
- human-lab has a crown and a back view, and `scripts/hair.json` captures four views of all eight. The first two passes were judged from the front only, which is why the crown got through.
- Eyebrows and lashes from the seed in the face texture; the brow card is not drawn for fitted people.

Open: no cut in the fixture has a fringe now (only short cuts with a forward sweep get one, and none of the eight rolls it); the edge where long top hair ends over short sides is a hard line.
