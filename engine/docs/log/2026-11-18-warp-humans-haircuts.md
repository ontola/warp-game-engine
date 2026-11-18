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
