# Blood stains under stadium lights

Mancave, Windows, RTX 5070 Ti. The player marked pixel (1903, 1091) in
Growing Guns capture `1794248531904`. The blood on a vertical stone face
looked pale grey in night haze. A floodlight was close to the mirror angle
for that face and camera. In a software check of the renderer's GGX function,
the specular term for that lamp fell from 0.1242 to 0.0045 when the splat
material's roughness rose from 0.39 to 0.92. The decal also opts out of the
rain gloss layer so wet weather cannot turn it back into a polished surface.

Compared release builds before and after the material change, starting
at `ab63655a`. Both rendered frame 1073 of the saved replay at alpha
0.82365346, full graphics, 3840 × 2160. In the 430 × 275 crop starting at
(1670, 965), 1956 pixels differed by more than 18 sRGB code values in at
least one channel. Their median RGB went from (144, 119, 114) to (112, 40, 39).
The detached drops and central stain retained their original silhouettes.
