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
