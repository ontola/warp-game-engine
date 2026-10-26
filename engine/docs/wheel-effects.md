# Wheel particles and tyre marks

The playable adapter is `crates/cli/src/wheel_effects.rs`. `play` and `shot`
observe it after each real 120 Hz physics tick and upload it once before all
views render. It reads the race; it never changes forces, grip, race RNG or
wheel state. `SIDEWAYS_WHEEL_EFFECTS=0`, read at construction, disables both
systems for matched comparisons. The default is enabled.

## Contact and lifecycle

Up to four cars contribute all four wheels, with stable IDs `car * 4 + wheel`.
Both the particle and track emitters receive the same contacts: position,
surface, normal, load, tread speed and longitudinal/lateral slip. Particle
velocity includes the physical point velocity, `linear + angular ×
(contact - centre_of_mass)`. Dirt emission follows contact and slip, so a
braking or sliding wheel can emit even when it is not driven.

Water is queried at each contact, independently of the car-centre query used
by handling. Spray is placed at that local water surface; water contacts do
not leave ground marks. Wetness comes from the authoritative race weather
(clear 0, overcast .25, mist .45, wet 1), not from an independent visual sky
setting. Existing per-surface rules produce gravel stones/dust, soil clods,
grass fragments, mud clods, water drops or tyre smoke on slipping asphalt.

Repeated observations at the same tick do nothing. Pause freezes particle
age. Stage replacement or a tick discontinuity resets history. Recovery,
large pose jumps and stale contact points break each affected wheel's trail.
This is necessary because a vehicle teleport can temporarily retain its old
wheel contact. Existing airborne particles can finish their lifetime after
a local car recovery; no new bridge is drawn across the teleport.

## Bounded rendering

The renderer receives births with their original simulation timestamps.
Motion, drag and gravity are evaluated analytically on the GPU, shared by
all views. The shared pool holds 4096 particles; each view also has a 2048
visible-particle limit and a conservative screen-coverage budget. These are
visual particles, not separately simulated rigid bodies.

Marks are persistent material data in a world-space sparse atlas, with
height gating to distinguish crossing surfaces. They affect the existing
lit surface rather than floating above it as black geometry. The atlas is
bounded to 512 tiles of 128 × 128 texels at 8 cm per texel. Older tiles can
be evicted. Tyre width currently defaults to 0.24 m because the car spec
does not expose an authored width.

The adapter queues at most 4096 births and 32768 marks between renders.
Expired births are discarded before upload. Marks upload in batches of at
most 8192; acknowledged chunks are consumed only after success. Overflow,
expiration, atlas eviction, emission totals by kind/surface and the latest
16 contact samples are recorded in shot/F12/state metadata. Normal play
drains queues each rendered frame; offline shots retain marks while they
advance the simulation without rendering.

## Validation and limits

The combined CLI CPU suite passed 115 tests with one ignored, including
five adapter tests. They cover 600 actual simulation ticks with four cars
and unchanged race digests, recovery, duplicate observations, surface water,
point velocity, original birth ages, queue bounds and stage resets.
Runtime acceptance on 2026-10-24 is in
`C:/dev/perf/warp-sideways-20261024/r22`. `track-gpu/report.json` passes
continuous marks, raised-surface rejection, four-view agreement, zero light
and stage clearing. `particle-gpu/report.json` passes all seven kinds,
opaque blocking, sun shadows, four views, reset, and unchanged DLSS history.
The full render suite passed 172 tests and the simulation suite 30, with
one ignored in each suite.

Actual gameplay captures `trail-front-on.png` / `trail-front-off.png` use
the same seed, physics tick, camera and DLSS Quality. Dust trails and subtle
compressed wheel paths are visible behind the cars; no tuning exaggerated
this low-slip rolling case into a burnout. Its 6248 marks occupy 26 tiles,
221 surviving births upload, and neither queue drops records. Ordinary
rolling marks remain subtle against dense tree shadows; moving playtest
feedback is still useful for artistic strength.

The actual native 3840x2160/four-player pair `four-native-on.png` and
`four-native-off.png` also rendered successfully: 11,499 marks, 31 tiles,
465 surviving births, including grass fragments. This bounded held-pose
comparison is not gameplay FPS or the 120 FPS target. Logged shade time is
10.7ms off / 10.9ms on (rounded); raster remains about 20.7ms, so overall
performance is still far from that target. Particle fixtures are tiny and
their timings must not be extrapolated to full gameplay.

Marks are visual compaction/rubber only: they do not deform the road mesh,
alter grip or flatten the source grass geometry. Debris uses a local
contact-plane approximation, not rigid-body collisions with the whole
world. Dust is a bounded particle effect, not a volumetric fluid simulation.
No native 4K/four-player/120 FPS performance claim follows from the CPU tests.
