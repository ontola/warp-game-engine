# Warp Humans: shine through an afro, 20 November 2026

Playtester, pin on a pale patch in the afro seen in profile: "die shine is
issue".

The patch is the scalp seen between the strands, at grazing sun. Strands
cast no shadow, so the scalp under 4 cm of hair is in full sun and showed
the skin's oily sheen (white on dark skin), from above also the sky.

- Shading the scalp under hair as hair (albedo alpha to 0): the profile
  patch went, but from above a white ring stayed, the hair material's own
  highlight lobes and the reflected sky.
- A material map for occlusion would cost a 2048 layer per person and only
  dims ambient light. The normal map's alpha was unused: it now scales
  sheen and reflections of SKIN surfaces, 0 under hair longer than a buzz.
  Diffuse light is left alone: under fair hair the scalp is bright in
  reality too. Under dark hair the fill colour is up to 60 % darker.

Checked on lavapipe, human-lab at 30, all eight from four sides. Open: seen
from above the front of an afro is thin and the dark scalp behind the
hairline shows as a V; strands still cast no shadow.
