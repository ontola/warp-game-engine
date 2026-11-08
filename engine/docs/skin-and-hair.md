# Skin and hair

Characters can light their skin and hair differently from other surfaces.
Both are material flags (`render::scene::SKIN`, `HAIR`) handled inside the
deferred shade pass: no extra textures, storage buffers, bind groups or passes,
so they work wherever the shade pass works (including WebGPU's limit of eight
storage buffers per stage). Scenes without such a material do not compile the
code (`FEATURE_SKIN_HAIR`, [shade_features.rs](../crates/render/src/shade_features.rs)).

## What they do

**Skin** (`SKIN`) approximates light scattering under the skin:

- Diffuse light wraps past the terminator further in red than in green and
  blue (a cheap stand-in for pre-integrated skin), so the light-to-shadow
  edge turns warm instead of grey.
- Sun shadow edges let red through first (the penumbra is widened per
  channel).
- Sunlight passes through thin parts such as ears, nostrils and fingers. The
  thickness comes from the sun shadow map (receiver depth minus the first
  occluder's depth), plus one and a half shadow texels because that depth is
  only accurate to about a texel. Transmission falls off with Jensen's
  measured mean free paths for skin (3.67, 1.37, 0.68 mm), lengthened four
  times for sideways spread. Where the shadow map's texels are centimetres
  wide, as in a wide stage scene, ears hardly glow.
- Specular is two GGX lobes (roughness 0.42 and 0.91 around a base of 0.7,
  30 % on the sharp one) with a skin F0 of 0.028.

**Hair** (`HAIR`) uses Kajiya-Kay strand highlights in the style of
Marschner's model: a white R highlight shifted towards the tips and a
dimmer, hair-tinted TRT highlight shifted towards the roots, with per-texel
jitter so strands do not shine as one band; diffuse light wraps generously.
Strands run along the instance's up axis over the surface, which suits
scalps and short hair. There is no strand or card geometry yet.

## Marking a character

`assets::skinned::Character::set_skin_hair(surface, kind)`, before adding
actors:

- `SkinHair::Skin` or `SkinHair::Hair`: the whole surface.
- `SkinHair::Mask(gray)`: per texel of the surface's albedo texture, 0 hair,
  128 skin, 255 neither (the surface keeps its own lighting), linear between.
  The mask becomes the albedo alpha (resized to the texture), and the
  material gets `SKIN | HAIR`. Not for alpha-tested surfaces.

## Growing Guns

Opt-in with `GG_SKIN_HAIR=1` until checked on a real GPU. The zombie's bare
body is skin; the paladin's face, scalp and ears are masked by
`paladin_skin.png`, written by `games/growing-guns/tools/skin-masks.py` (they
are hidden under his helmet in play). The knight is all armour.

`growing-guns character-review --out build/character-review` renders each
character close up from the front, three-quarter and backlit, and in full,
with and without skin and hair. `--hair` lights the zombie as hair instead,
to check the hair model.

## Open

- Strand or card hair geometry (beards, longer hair) and a hair shadow
  term; no current character has visible hair.
- Screen-space diffusion for skin (light spreading sideways across pores);
  only the wrap and transmission approximations exist.
- A visual check on a real GPU before Growing Guns turns it on by default.
