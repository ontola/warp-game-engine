# Skin tone and seeded hair colour

On Mancave (Windows, RTX 5070 Ti), the Growing Guns change based on `d7b87bba` ties the six natural hair swatches to the final sRGB skin tone. The same seed keeps its percentile when a GameNight profile supplies a different skin colour. A separate draw can replace the hair swatch with dye while the brows retain their natural shade. These odds are art direction for the game.

Across seeds 0 through 8191, the dark test skin `[0.28, 0.15, 0.09]` produced 105 naturally honey- or pale-blond heads (1.28%). For the same seeds the light test skin `[0.95, 0.80, 0.70]` produced 1547 (18.88%). Both runs gave the same 199 dyed heads (2.43%). The spectator instance still occupies 64 bytes. Its wardrobe word carries the hair swatch and natural brow grade in previously unused bits. All grooms reuse the existing atlas.
