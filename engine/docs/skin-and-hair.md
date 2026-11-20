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
- Where the sun map's texels at the point are finer than 4 mm (fading out
  to 10 mm), the sun's visibility on skin is how much skin lies in its way
  instead of a lit-or-dark test: sixteen taps on a golden-angle spiral
  1.8 texels wide, each comparing the map with the receiver's own plane
  and attenuating by Jensen's diffusion lengths (3.67, 1.37, 0.68 mm). A
  sliver just past the terminator lets the wrapped light through; a cast
  shadow's edge fades over the spiral instead of following the map's grid
  or the mesh's triangles. Where the plane would leave what the triangle
  spans along the sun, the taps close up along the slope. A triangle's
  own flatness (its longest edge times how far its facing of the sun
  departs from its vertices') is not counted as skin in the way. Shadows
  from beyond the fine map (the shared map 3 m up the ray, world terrain,
  clouds) still apply.
- A scene with skin gets fine sun levels round the camera: half extents of
  3, 12 and 24 m, then doubling to 768 m (`KERN_SUN_CLOSE=0` keeps the ladder from
  24 m). The two finest are centred ahead of the camera unless the game
  set a lighting focus.
- Sunlight passes through thin parts such as ears, nostrils and fingers. The
  thickness comes from the finest sun level covering the point, else
  the shared sun map (receiver depth minus the occluder's depth over the
  same sixteen taps), plus one and a half shadow texels because that depth
  is only accurate to about a texel. Transmission falls off with the same
  diffusion lengths. Where the shadow map's texels are centimetres wide,
  as in a wide stage scene, ears hardly glow.
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

A `SKIN` surface's normal map carries in its alpha how much of the sheen
shows there (255 all of it, the default of any normal map; 0 none): it
scales the skin's and hair's sun highlights and the reflection of the
surroundings, not the diffuse light. Warp Humans write 0 on the scalp under
grown hair, which stands in the hair's shade.

## Growing Guns

Warp Human players use the fitted skin and face materials. Use `growing-guns human-review --out build/human-review` to inspect generated people. See [Warp Humans](warp-humans.md) for the player and spectator rendering paths.

## Open

- Hair shading for beards and longer hairstyles.
- Screen-space diffusion for skin (light spreading sideways across pores);
  only the wrap and transmission approximations exist.
