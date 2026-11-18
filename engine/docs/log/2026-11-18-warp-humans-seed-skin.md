# Warp Humans: skin and eyes from the seed (18 November 2026)

Scene: `games/human-lab`, `scripts/portraits.json`, 1280x720, lavapipe in the cloud container (not a real GPU). The same captures of commit a07327b8 on Mancave (RTX 5070 Ti) showed the same faults: square patches on shadowed cheeks, a stepped nose shadow, flat eyes without a highlight, cut-out brows.

Joep's direction: big steps, small seeds with much variation, no pile of meshes per head; one body (hm08), everything around it new.

## What changed

- `assets::skinned::warp_skin`: a face albedo and normal map per fitted person from their seed, 2048 texels, 1.2 s on four cores. Eyes are drawn into a free corner of the same texture, so a person costs one texture layer.
- `shade.wgsl` `sun_through_skin`: transmission blends four shadow-map texels.

## Findings

- The square patches were the skin transmission term, not shadow acne: it read one nearest shadow-map texel for the thickness. With the term off they vanish; with four blended texels they vanish too and thin parts still glow.
- The stepped nose shadow is the sun map's resolution. Level 0 of the clipmap has a half extent of 24 m at 2048 texels: 2.3 cm per texel, a nose is 3 cm. With `NEAR_SHADOW_EXTENT` set to 3 m as an experiment (2.9 mm texels) the edge is smooth. Not kept: it shortens the clipmap's reach from 3 km to 384 m. A fine level near the camera is the next step.
- Pores and furrows are below a pixel in the portrait framing (0.25 mm texels, 0.6 mm pixels). They show as a slight breakup of the skin, no more. Up close the gain has to come from shadows, eyes, brows and hair first.

## Open

Fine sun shadow level near the camera; eye highlight and shadow from the lids on the eyeball; lashes; soft brows; the bob's parting; the shared body skin (untextured).
