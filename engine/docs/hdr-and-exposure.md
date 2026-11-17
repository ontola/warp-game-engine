# Exposure and HDR output

Warp renders in linear HDR (`Rgba16Float`) with physical exposure (EV100)
and tonemaps with AgX. This guide covers the two things on top of that:
**automatic exposure**, which meters each view and adapts like an eye, and
**HDR output**, which shows highlights brighter than SDR white on an HDR
screen. Both are off by default; a game's SDR look does not change unless it
asks for them.

## Automatic exposure

### When to use it

- **Use it** when the light changes a lot during play: a day/night cycle,
  driving into a tunnel or a forest, caves, interiors next to sunlit
  exteriors, or split screen where players are in very different light.
- **Keep a fixed exposure** for a scene with one designed look (an arena, a
  neon game, a menu backdrop): auto exposure would brighten a deliberately
  dark scene towards mid grey. A fixed `exposure` per sky preset is also more
  predictable for screenshots and golden images.

### Luau

```lua
-- Meter and adapt with the defaults.
light.set({ exposure = "auto" })

-- Tuned: follow scenes between EV 4 (dusk) and EV 15 (sun), show
-- everything half a stop brighter, adapt fairly quickly.
light.set({ exposure = "auto", exposure_min = 4, exposure_max = 15,
            exposure_bias = 0.5, adapt_speed = 4 })

-- Back to a fixed exposure.
light.set({ exposure = 12 })
```

| Option | Meaning | Default |
| --- | --- | --- |
| `exposure` | A number fixes the exposure (EV100). `"auto"` meters it. | the sky's |
| `exposure_min` | The darkest scene (EV100) the camera follows. Below it the scene is shown as dark as it is: a night stays a night. | 2 |
| `exposure_max` | The brightest scene it follows. Above it the scene is shown brighter: snow and glare stay bright. | 16 |
| `exposure_bias` | Compensation in stops: +1 shows every scene twice as bright. | 0 |
| `adapt_speed` | Adaptation to brighter scenes, per second (3: most of the way in about a second). Adapting to the dark takes three times as long, as eyes do. 0 adapts at once. | 3 |

Setting any of the tuning options turns automatic exposure on. `light.reset()`
returns to the sky's fixed exposure. The sky's own exposure (`sky.set`) is still
used as the starting point of each view.

### Rust

```rust
let mut light = sky.lighting();
light.auto_exposure = Some(render::AutoExposure {
    min_ev100: 4.0, max_ev100: 15.0, compensation: 0.5,
    ..Default::default()
});
```

[`render::AutoExposure`](../crates/render/src/auto_exposure.rs) also has the
meter's percentiles (`low`, `high`: by default the darkest half and the
brightest 5 % of samples are ignored), `center` (how much more the middle of
the view counts) and separate `speed_light`/`speed_dark`. `Lighting::set`
takes the same fields by name (`auto_exposure=1`, `auto_min_ev100=4`, ...),
so they can be tuned live.

Useful renderer calls:

- `Renderer::exposure_ev100(view)` and `Renderer::metered_ev100(view)`: the
  exposure a view is at and what its meter reads, a few frames old. Use them
  for a light-meter HUD, or to drive game logic that should follow the eye.
- `Renderer::reset_exposure()`: snap to the next measurement after a
  teleport or a level change instead of adapting. `reset_history()` (a camera
  cut) does this too.

### Tuning it

1. Turn it on with **F10, Rendering, Auto exposure** (it overrides the game's
   choice for the session). Its help line shows the current EV and what the
   scene meters.
2. Walk or drive through the darkest and brightest places of the level and
   note the metered EV. Set `exposure_min` a little above the darkest place
   that should still *look* dark, and `exposure_max` a little below the
   brightest place that should still look bright.
3. Use `exposure_bias` for the overall look, not `exposure_min`/`max`. The F10
   *Exposure offset* still works on top (positive darkens).
4. Press F12: the screenshot's JSON records `auto_exposure`,
   `exposure_ev100` and `metered_ev100` in `debug`.

### How it works and what it costs

Each view is metered on its own, after TAA and before bloom and the tone
pass: a 256-sample-wide grid of the final HDR picture goes into a 128-bin log
luminance histogram (a quarter stop per bin), the bins between the two
percentiles are averaged, and the exposure moves towards the result with
`1 - exp(-dt * speed)`. All of it runs on the GPU in the same frame; tone,
bloom and TAA copy the adapted exposure into their uniforms on the GPU, so
nothing waits for a readback. The cost is two tiny compute dispatches per
view (the `auto exposure` line in F10's GPU timings). A copy reaches the CPU
asynchronously a few frames later for emissive compensation, lens flare and
the calls above.

Limits:

- Adaptation follows wall-clock time (at most 0.1 s per frame, so a hitch
  does not jump the exposure). Headless stills snap to the measurement on
  their first frame; use a fixed exposure where images must match bit for bit.
- The minimum graphics profile (`forward` renderer) keeps the fixed exposure.
- One view's sun or a white wall right in front of the camera does darken
  that view: raise `high` or lower `center` if that is too eager.
- Exposure only follows what the lighting gives it: a room is only darker
  than the field outside when it has its own light data. See
  [interior-light.md](interior-light.md).

## HDR output

### What players and developers see

On an HDR screen the picture keeps its SDR look (the same AgX tone, contrast,
saturation, vignette, grain and bloom) and only the highlights go brighter
than SDR white: the sun and its glare, specular glints, lamps, emissive
materials, explosions. Everything up to about the brightness of a white
surface in the light is unchanged. Captures, video and live screens stay
SDR. An F4 feedback capture on an HDR screen keeps the frame on screen as
it was (HDR, the HUD at SDR white) while the point is picked, and saves
`<name>-hdr.png` beside the SDR PNG: the HDR output without the HUD as a
16-bit BT.2100 PQ PNG (BT.2020 primaries, `cICP` chunk), SDR white at the
level the screen showed it (JSON: `capture.hdr_png`). Chrome and Windows
Photos show it in HDR; the SDR PNG is what an SDR screen shows.

### Turning it on

HDR output is off unless something asks for it, in this order:

1. **F10, Rendering, HDR output** (developer override for the session).
2. The game: `fn hdr(&self) -> Option<bool>` on `app::Game`, for a settings
   menu toggle. `None` defers to the next step. A Luau game sets `hdr = true` in
   game.toml.
3. `WARP_HDR=1` in the environment (`WARP_HDR=0` refuses it).

It switches live: the window's surface is reconfigured and the presenter and
HUD are rebuilt on the next frame. It needs a screen and driver that offer an
fp16 surface with linear extended sRGB: Windows with HDR turned on in the
display settings (DX12 or Vulkan), macOS (EDR, also on the built-in XDR
screens), and Vulkan drivers that expose `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT`.
Elsewhere it logs that it stays SDR and the F10 toggle is greyed out.

### How bright

The highlight headroom (how many times brighter than SDR white the brightest
highlight gets) comes from the display: `max_nits / sdr_white_nits` on
Windows, the current EDR headroom on macOS. F10 shows what the display
reports. Override it with **F10, HDR peak** or `WARP_HDR_HEADROOM=4`; on
platforms that report nothing (Vulkan on Linux) the headroom is 1, so the
picture looks like SDR until you set one.

On Windows, SDR white follows the OS's *SDR content brightness* slider, so
the HUD and the SDR part of the picture match other windows.

### Authoring for it

- Nothing has to change: a scene that looks right in SDR looks right in HDR.
- Glows and lamps that clip to white in SDR get their brightness back in HDR,
  in proportion to how bright they are. Keep emissive values physical-ish
  (`emissive` 0 to about 1 in daylight) rather than very high to force white:
  a value 100 times too bright is a blinding patch at full peak in HDR.
- Check both: toggle F10's HDR output while looking at the brightest places
  (the sun, lamps at night, fire).
- The HUD is drawn at SDR white on top of the scene. It does not get brighter.

### How it works

The tone pass writes the usual 8-bit SDR picture and, while a headroom above
1 is set (`Renderer::set_hdr_headroom`), the same picture linear into an
`Rgba16Float` texture (`Renderer::hdr_output`) with 1.0 as SDR white. Above
a display-linear knee of 0.6 (AgX shows an exposed 1.0 at 0.59) the
brightest channel is stretched up to the headroom along
`m + (headroom - 1) * ((m - 0.6) / 0.4)^2`, which is continuous with the SDR
curve and keeps AgX's hue. The presenter
([`present.rs`](../crates/render/src/present.rs)) draws the HUD into its own
SDR layer and composes scene and HUD on the fp16 scRGB surface, scaled to the
OS's SDR white ([`hdr.rs`](../crates/render/src/hdr.rs)).

Cost with HDR on: one extra `Rgba16Float` write per pixel in the tone pass,
an 8-bit HUD layer at window size, and the composition pass reading both:
about 200 MB of extra memory traffic per frame at 4K, and 100 MB more memory.

Limits:

- No HDR10/PQ output and no HDR metadata yet; fp16 scRGB covers Windows and
  macOS without them.
- Only the `app` shell (every Luau game and the engine examples) presents HDR.
  Sideways has its own presenter and stays SDR for now.
- The minimum graphics profile tones SDR only; an HDR surface then shows the
  SDR picture at SDR white.
