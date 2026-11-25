# Graphics tiers

Warp renders the same scene on three tiers. A game does not choose a
renderer; it describes its scene, and the host picks the tier from the
device and the game's frame target (see gamenight-internal
`docs/strategy/hardware-fit.md` for the choice, and the plan "Mobiel
renderpad voor Warp" for the reasoning).

| Tier | Name in the catalog | Renderer | Made for |
| --- | --- | --- | --- |
| high | Ultra | Visibility buffer, full shade pass, dynamic GI, shadows, TAA, HDR bloom | Desktop GPUs (Mancave, strong Macs) |
| medium | Standard | The same, with the phone preset (compact and split shade pass) | Mid-range laptops, tablets as fallback |
| low | Mobile | Forward renderer with per-pixel light (`forward.rs`, `forward_lit.wgsl`) | Tile GPUs: tablets, phones, Quest, weak laptops |

Today the low tier is chosen with `KERN_FORWARD=1` (on Android:
`env=KERN_FORWARD:1` in the host's query). The Android host defaults to
`KERN_FORWARD=auto`: the low tier wherever the scene has no terrain, grass,
foliage, crowds or debris, and the game's surface compiles
there. `env=KERN_FORWARD:0` turns it off. The renderer logs its choice, for
example `graphics tier: medium; low tier wanted but skinned meshes`. The
automatic choice per device comes with the hardware-fit work.

Measured on a Samsung Tab S9+ (Adreno 740, 120 Hz), Game Room, 25 November
2026: the medium tier 722 lines, GPU 14 ms, 60 fps; the low tier 1172 lines,
GPU 7.7 ms, about 104 fps.

## Why the low tier is cheap

A phone or tablet GPU draws the frame in small tiles in fast on-chip
memory. As long as everything happens in one render pass, depth and colour
never leave the chip. The high tier's compute passes per pixel each write
and read main memory, and one large shading kernel runs fewer threads at
once on Adreno (measured on the Tab S9+, 25 November 2026: texture, GI and
lights were each cheap, together in one kernel slow).

The low tier draws every visible mesh once, in one pass, with a small
fragment shader:

1. the material's tint times its albedo texture layer, or the game's own
   `game_surface` (`SURFACE`), or a car part's colour, metal and clear coat,
2. the sky's irradiance (its SH) plus the room fill, or where there is one
   the room's baked light: the irradiance probes from
   `Renderer::set_irradiance_volume` as one 3D texture, three filtered taps,
3. every spot and local light, without shadows, and of the clustered lights
   the 32 nearest the cameras,
4. the sky in what is glossy: its SH at the mirror direction (a blurred
   reflection) by Fresnel,
5. emission,
6. light beams of `Lighting::volumetric` (the analytic in-scatter of
   `volumetric.wgsl`, three samples, nearest 16 lights),
7. exposure, AgX, contrast and saturation, straight to the display format.

Where no mesh is, the sky's SH as a soft gradient. Glow (`forward_bloom.wgsl`) then works on the finished image at an eighth of
its size. Sprites, text tags and `fx` draw as on the other tiers.

## Material primitives

On the low tier each material falls into one of a few primitives. The flags
a game already sets decide which; there is nothing extra to declare. Pick
the cheapest one that gives the look.

| Primitive | How to get it | Cost on the low tier | Notes |
| --- | --- | --- | --- |
| Glow | `LAVA` flag: colour times `scale`, no light at all | Cheapest | Neon, screens that light up, signs |
| Lit colour | Default: tint, `layer: -1` | One probe lookup plus the light loop | Walls, furniture, simple props |
| Lit texture | `layer` set to an albedo layer | Plus one texture tap | Posters, wood, painted screens (`Screen`) |
| Lit plus glow | `EMISSIVE` with `scale` | As lit, plus an add | A lamp shade that is lit and glows |
| Cutout | `ALPHA_TEST` with a texture | As lit texture, plus discard | Leaves, fences. Costs more on tile GPUs; use sparingly |
| Glass, cards | `BLEND` (`set_material_blend`, `set_material_additive`) | As lit, drawn after the opaque meshes without depth write, premultiplied; additive adds | Glows and halos as additive emissive cards |
| Two-sided | `TWO_SIDED` | Same | The normal turns towards the camera |
| Game surface | `SURFACE` with `game_surface` | Whatever the surface costs | Gets `SURFACE_LOW_DETAIL = true`: branch on it for the cheap version. Reads only `view.cam`, `light.sun_dir/sun_color/anim`, `aux_word`, `instances`, `albedo`, `samp` and `sky_irradiance` |

What the low tier leaves out: shadows of any kind, highlights of lights
(roughness only fades the sky's reflection), normal and material maps,
`TERRAIN` materials, foliage, grass, terrain, crowds and debris (weighted
characters are skinned by the same compute pass as on the other tiers), and a game surface that reads more than listed above (Warp Theft
Auto's reads the sky's radiance and GI). Such a scene stays on the medium
tier until the low tier supports it.

## Checking a game on the low tier

- Desktop: `KERN_FORWARD=1` before starting the game, or for the Game Room
  `KERN_FORWARD=1 game-room shot --out room.png`.
- Android: `adb shell am start -n club.splitscreen.host/android.app.NativeActivity --es query "cloud=off&env=KERN_FORWARD:1"`
  (and `fps=120` for 120 Hz).

Compare with the same shot without the switch. What carries gameplay (an
attack's telegraph, a pickup's glow) must stay visible on every tier.
