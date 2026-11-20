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

Playtester on the result: the real issue is the hair density lower down.
Measured from behind (strand length times width over a 10 cm square in
0.2 mm cells): mean 2.0 strands deep for the afro, yet 40 % of cells
open, since tufts leave gaps and coils seen end-on cover one turn. With
strands 1.8 times as wide for coiled hair and coils and drawn stretch up to
1.6 times larger for long hair: 5.6 deep, 19 % open (the square reaches
past the head); short coiled hair 3.8 deep. The head no longer shows
through from the back, the side or above.

Checked on lavapipe, human-lab at 30, all eight from four sides. Open:
strands still cast no shadow.
