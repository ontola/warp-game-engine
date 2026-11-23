# Port Meridian shopfront tint

In `b0ff614d` the distant glass transmits 80% of the light left after reflection. Its normal-incidence coating parameter is 0.125. The close 4 mm panes use sRGB #8e9b95 with 0.064 scattering opacity and 0.055 roughness. The doorway gives a direct view of the same lobby.

The matched native replay compared the QUAY office in city seed 1948 to `c02a2bd3` at 960 x 540 on an RTX 5070 Ti. At 14 m in daylight, the sampled pane's mean pixel luminance fell from 39.1 to 34.5 front-on and from 36.5 to 33.8 at 62 degrees. The opposite building and road stripe remained visible. At 2.8 m with the real room fully loaded, the daylight pane went from 37.35 to 32.16 on a 0-255 scale, while the open door went from 21.08 to 21.12. At night the close pane went from 86.59 to 85.21 and the distant front pane went from 85.12 to 79.82 after eye adaptation. The desk remained visible in the lit lobby.
