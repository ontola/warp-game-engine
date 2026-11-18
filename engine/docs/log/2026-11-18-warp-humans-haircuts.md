# Warp Humans: haircuts instead of lengths (2026-11-18)

Cloud container, lavapipe, `games/human-lab/scripts/hair.json` (face, crown,
back, profile of eight people), on top of 8561135d.

Playtester on the eight cuts: hair stands out at the temple, and "het zijn
ook wel eigenaardige kapsels zo".

What was wrong, found by looking at each cut from four sides:

- A cut was two lengths (top, sides) and every strand had its full length.
  Top hair combed aside hung over the short sides and ended in one line; the
  back had the side length from the crown to the nape, a mullet.
- Hair over the face was cut at fringe height wherever it was in front of
  z = -3 cm, also beside the face. That made the wings at the temples.
- The hairline ran 3 cm above the ear and dropped to the nape only 3 cm
  behind it: a bare rectangle round the ear in every cut.
- Hair beside the ear was pushed out to the ear's reach at its first step
  (the reach map is widened round the ear) and stood out.
- Long strands had ten evenly spaced points, 4 cm apart: polygons over the
  crown. Hair kept its direction on top and left the head as it curved away.
- Ribbons narrowed from the root (1 - 0.85 t^2): the lower half of long hair
  was see-through.

Now: nine named cuts; a taper by height between hairline and crown and a
blend rule (hair past a point is no longer than 1.1 times what grows there);
a hem for long cuts; the face rule only before the face; hairline round the
ear with a sideburn; a slope for rising over the ear; strands laid in fine
steps and resampled by length plus bend; ribbons narrow at the tip only
(1 - 0.85 t^4).

Tried and dropped: a taper band in metres above the hairline (the nape
hairline is 10 cm lower than the one over the ear, so the back stayed long);
smoothstep for the taper (60 % of the top length halfway up); blend 1.6
(sides still hung over the ear).

Open: the back of long hair shows dark blocks (strands take the body's sun
shadow and cast none themselves); a bob hangs before the cheek in profile;
no gender or age in the choice of cut; not seen on a real GPU.

## Later the same day: age, coiled hair, rarer long hair

Playtester: still odd cuts; men with very long hair are rare; black people
mostly have coiled hair, not straight; wants young/old as a parameter.

- The eight from three-quarter showed what the four-view sheet hid: bangs
  lay as a flat lid from 8 cm back and were cut 2 to 4 cm above the eyes.
  Bangs now start 3 to 6 cm back, end at the brows and are rare.
- Polygon-shaped light and dark patches on short hair came from the strand's
  sun lookup 8 mm off the scalp, inside the head's own flat triangles. The
  lookup is 3 cm out now; the head's own shading comes from the wrap term.
- A person has no sex (one body, four blended heads), so long hair cannot
  follow it. Long cuts are rare for everyone until there is a female body.
- Coiled hair: a helix of 2 to 4 mm radius and 10 to 18 mm pitch round the
  strand's line, standing off the head, no sag. Ten points per strand give
  about four per turn at 4 cm; at 10 cm it is a zigzag, which reads as
  kinky in the mass.
- Grey: first try `smooth(32, 68) * (0.5 + roll)` made 51 to 58 year olds
  white. Now `smooth(35, 80)^1.5 * (0.3 + 1.2 roll)`.
- Age series (same eight at 20, 50, 80) rendered from copies of
  games/human-lab with `age = [n, n]`.

Open: an old face is a young face with lines and white hair (no sagging,
no thinner lips, no stoop); no children; no women.

## Evening: old faces sag, old heads have less hair

Playtester: "oud moet minder haar en saggy skin en zo".

- `warp_skin::settle` moves the head's vertices by a sum of seven lumps
  (jowl, cheek, eye bag, brow, temple, nose tip, under the chin) given on a
  reference head and scaled by the person's face width and eyes-to-chin
  length (measured: chin at -110 to -120 mm, side of the face 64 to 83 mm
  over three seeds). Normals turn by the change of the welded smooth normal.
- Folds are grooves along polylines in the same reference coordinates.
  First try had the marionette line at 0.7 and 22 % darkening: a ruled dark
  stripe from the mouth corner, like a puppet. Now 0.4, 12 %, and the lines
  wander by 2 mm of noise.
- The strand budget was shared over the hair-bearing area, so a receding
  hairline made the rest denser. It is shared over a full head now, times
  `Cut::full`.

Open: thin white hair reads as separate stiff strands (ribbons are opaque
and as wide as young hair); the neck below the chin and the hands do not
age; no stoop.

## Night: grey blends, coiled hair is coils

Playtester, on the three-quarter sheet: "Dit ziet er belachelijk uit, die
halve grijze haren zo" and "En moet dit kroeshaar voorstellen?".

- Grey was on or off per strand: white wires among dark hair. It is now a
  share per strand in eight steps, mixed into the colour in the shader, with
  streaks from 16 mm noise.
- Coiled hair was a spiral per strand with its own phase: fuzz. Now tufts
  (7 mm cells, warped so they are not in rows) share radius, pitch, hand
  and phase.
- Afro, tried in order: (1) all ten points on the outer 18 mm after a
  straight stretch from the root: the stretches showed as sticks; (2) only
  the outer shell drawn: a halo round a bare scalp; (3) one 20 mm stretch
  per strand at a random depth: fills, but radial rows like a bottle brush;
  (4) a kink per strand (radius 12 to 32 mm) and warped tufts: reads as an
  afro.
- Straight combs of hair behind the ears and down the neck: path points
  were rested on the head's reach without the rise limit the walk itself
  has, so strands rooted under an ear or at the nape jumped 3 cm out in one
  segment. Both now use the same limit (straight hair too). Found by
  printing strands with a first segment over 15 mm.

Checked on lavapipe only, at 30, 50, 65 and 80, three-quarter and four
sides. Open: white coiled hair on a thin head reads coarse; a strand of an
afro is not one continuous hair.
