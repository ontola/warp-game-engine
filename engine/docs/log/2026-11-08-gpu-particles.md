# GPU particles first pass, 8 November 2026

Base commit 310872a. Cloud container, lavapipe (software Vulkan), no real
GPU: images only, no timings.

## What was built

`render::gpu_particles` (guide: [GPU particles](../gpu-particles.md)): compute
spawn into a ring, fixed-step compute simulation with a heightfield ground,
an atomic live list feeding an indirect billboard draw after temporal
reconstruction. Opt-in with `KERN_GPU_PARTICLES=1`. Growing Guns blasts and
Spellstorm eruptions and bombs emit sparks, embers and (Growing Guns, bombs)
lit grit.

## Captures

Growing Guns, a 6.5 m blast seen from the usual smoke review position:

```sh
KERN_GPU_PARTICLES=1 growing-guns script games/growing-guns/scripts/gpu-particles.json \
  --out build/gpu-particles-on --players 1 --seed 52 --sky noon --no-music
growing-guns script games/growing-guns/scripts/gpu-particles.json \
  --out build/gpu-particles-off --players 1 --seed 52 --sky noon --no-music
```

Spellstorm, a magma nova 4 m ahead of the wizard in the chapel, 0.2, 0.5 and
1.2 s after it:

```sh
KERN_GPU_PARTICLES=1 spellstorm shot --out ss.png --seed 7 --bots 0 --humans 1 \
  --ticks 430 --size 960x540 --lua-at 400 \
  --lua 'local w = g:wizards()[1]; local p = g:pos(w); g:nova(vector.create(p.x, 0, p.z - 4), 3, "magma")'
```

`spellstorm shot` now hands the events of a `--lua` line to the effects
before the next tick clears them; without that a nova from the console never
showed.

Seen: at 0.3 s the Growing Guns blast has a dense orange spray of sparks and
dark grit around the fireball that the old look lacks; at 0.6 s embers and
grit hang in and above the smoke; by 1.2 s the embers have risen out of that
camera's view. In Spellstorm the eruption gains a spray of orange sparks over
the old soft dots, and at 1.2 s a column of embers swirls over the smoke.
First tuning whitened the sparks: colours this bright saturate in the tone
curve, so the emitters now use saturated colours.

## Lavapipe miscompile

The first simulation loop (`continue` for unborn particles, `break` on death,
both inside an `if` in a `for`) killed every particle on its first floor
contact on lavapipe, although the bounce was positive. The same logic with a
`dead` flag in the loop condition and no `break`/`continue` works. Not
checked on other drivers; the flag form is kept.

## Open

- Timing on Mancave (RTX 5070 Ti) and the Mac before making it the default:
  `gpu particles sim` and `gpu particles` passes in the F3 overlay with a
  Growing Guns airstrike or a bench with many blasts.
- Sorting for covering particles; shadows and local lights on particles.
