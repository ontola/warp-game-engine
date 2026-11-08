# GPU particles

`render::gpu_particles` is the engine's general particle system for effects
that want many small pieces: sparks, embers, grit, dust motes. The game sends
emitters; everything per particle (birth, motion, death, culling and drawing)
happens on the GPU. Nothing is read back, so particles are cosmetic and never
affect the simulation.

It sits beside the other effect paths:

| Path | What it is for |
| --- | --- |
| `particles` | A few hundred CPU particles that are scene meshes (blood drops, casings): lit, shadowed, collide through a game callback. |
| `debris` | GPU-simulated mesh shards that collide with an occupancy grid. |
| `fx` | Explosions: billowing fire and smoke volumes, heat, lights. |
| `wheel_particles` | Sideways' wheel ejecta. |
| `gpu_particles` | Tens of thousands of billboards: sparks, embers, grit. |

**Opt-in:** a pool made with `GpuParticles::new` ignores emitters unless
`KERN_GPU_PARTICLES=1` is set when the game starts. It becomes the default
once it is timed on a real GPU. `GpuParticles::forced(capacity, true)` turns a
pool on regardless (tests, tools).

## Using it

```rust
use render::gpu_particles::{Emitter, GpuParticles};

let mut sparks = GpuParticles::new(65_536);     // ring capacity
sparks.set_floor(0.0);                           // land on y = 0
// On the event, on the same clock as flush:
sparks.emit(time, &Emitter { position, count: 2000, ..Emitter::sparks() });
// Once per frame:
sparks.flush(&gpu, &renderer, time)?;
```

An `Emitter` is a burst from a ball (`position`, `radius`) into a cone
(`direction`, `spread`), with speed, life, size and colour ranges, a `glow`
in nits (0: lit by sun and sky instead), `additive` (1 adds light, 0 covers),
gravity, drag towards the wind, a swirl (`turbulence`), floor `bounce`
(negative: dies where it lands) and `stretch` (a velocity streak). Births can
be spread over `duration` seconds without further CPU work. The presets
`Emitter::sparks()`, `embers()` and `dust()` are starting points; override
fields with struct update syntax.

Particles land on a horizontal floor (`set_floor`) and, optionally, on a
heightfield (`set_ground`): the top of each column of a grid, typically
`Ground::from_occupancy` of the debris grid, refreshed with `Ground::refresh`
over the area where pieces broke. A particle that runs into a column below
its top bounces off its side. `set_wind` sets what drag pulls towards.

The pool is a ring: when it is full the oldest particles are overwritten.
`clear` forgets everything (a new round). If the clock passed to `flush`
goes back, the pool starts over.

## How it works

- **Spawn** (compute): the frame's emitters are uploaded (at most 1024 per
  frame); one thread per birth finds its emitter by binary search over the
  birth prefix sums and writes a 64-byte particle into its ring slot. Colours
  are packed to 8 bits per channel, sizes and physics to half floats.
- **Simulate** (compute): one thread per slot runs the frame's fixed steps of
  1/120 s (at most 12; a longer gap ages particles without moving them), then
  appends live particles to a list with an atomic counter that is also the
  instance count of an indirect draw.
- **Draw** (per view): after temporal reconstruction, fx and sprites, into
  the sprites' disposable output (or a copy), never into TAA or DLSS history.
  Each live particle is one camera-facing quad (streaks follow the velocity
  across the screen), drawn between the last step and the frame's time, at
  least about a pixel wide (a widened particle dims to keep its light),
  softened against the opaque depth. Premultiplied blending, unsorted.

Storage buffers: five in the compute stage (the ground in its own bind group)
and two in the vertex stage, within the WebGPU minimum of eight
(`shaders_validate_and_fit_webgpu_minimum` checks the counts).

## Limits

- Unsorted: covering (non-additive) particles overlap in arbitrary order, so
  keep them small and faint. Glowing particles add and need no order.
- No shadows cast or received, no local lights on lit particles; lit particles
  use the sun and sky only.
- Not drawn by the minimal forward renderer (Quest) or the portable/browser
  renderer.
- The draw covers every live particle in every view; there is no per-view
  culling beyond the vertex shader.

## Where it is used

- Growing Guns: every blast (`EffectsView::blast_particles`) throws sparks that
  bounce over the arena's blocks (ground from the debris occupancy grid), embers
  and lit grit. 64k particles on full, 16k on low, none on minimum.
- Spellstorm: eruptions (magma novas) and bombs (`Fx::gpu_burst`) on the
  level floor.

Headless captures: `games/growing-guns/scripts/gpu-particles.json` (run with
and without the switch), and `spellstorm shot` with a `--lua` nova (see the
[log entry](log/2026-11-08-gpu-particles.md)).
