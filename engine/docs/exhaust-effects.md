# Exhaust gas, backfire and heat

The CLI observes actual car state once per 120 Hz physics tick. This is a
visual-only emitter with its own hash sequence; it never consumes race RNG,
changes controls or adds forces. Input is engine RPM, throttle, cut and
downshift state. A high-RPM shift, throttle-lift or cut edge can produce a
two-tick backfire (16.7 ms), with a 400 ms cooldown. Sustaining an input does
not repeatedly fire. Paused play, multiple views and repeated screenshots do
not advance emission. Stage/session replacement clears the pool.

## Mounts

These are measured body-space centres before the regular car ride offset.
Emission starts 15 mm behind the pipe opening; +Z is rear. Coordinates are
metres. Unknown car models have no exhaust until a hardpoint is authored.

| Model | Original component | Centre(s) x/y/z |
| --- | --- | --- |
| Asti | Two 58-triangle tubes in `UCB_BOTTOM.001` | −0.602283/−0.016082/1.859950; +0.602331/−0.023824/1.859949 |
| MV6 | Two 70-triangle tubes in `UCB_BOTTOM_LT.001` | −0.064506/−0.004010/1.977772; +0.057851/−0.004010/1.977771 |
| Toyama | 128-triangle tube in `Body_Chrome.001` | −0.380331/−0.025942/2.177629 |

Asti's rear lip vertices give its centre directly. MV6 and Toyama have
angled lips, so their full tube cross-section determines the vertical
centre; averaging only the rearmost vertices would put the outlet on the
upper rim. These are measured visual attachment approximations, not an
authored mechanical exhaust model. Mounts follow body rotation and ride;
puffs inherit linear velocity plus angular velocity at the mount.
Source measurements: `C:/dev/perf/warp-sideways-20261024/r20/exhaust-mounts-source.json`.

## Renderer and cost model

Four cars share at most 96 puffs, eight heat plumes and eight short flames
(112 sprites; engine buffer cap 128). Per-view CPU culling/sorting inspects
this bounded pool. One shared sprite upload serves all views. Unknown models
and distant/offscreen pools add no full-screen copy.

Visible effects copy resolved HDR into a separate cached output, draw local
soft quads there, then tone-map it. TAA/DLSS input and history remain intact.
Heat samples a displaced position in the original HDR scene, rather than
adding a glow. Both plume depth and displaced-sample depth reject opaque
foreground objects; internal jittered depth and resolved output use separate
projection matrices. Each active view costs one HDR copy plus local covered
pixels. This is a cost model, **not a measured GPU performance result**.

Gas is a subtle procedural soft puff with ambient sky illumination and
approximate drag/buoyancy. Flames emit HDR radiance but do not project light
onto the road. This is not fluid simulation or shadowed volumetric smoke.
Transparent glass does not write opaque depth, so its foreground occlusion
is not included. Lineup comparison shots disable exhaust because those cars
are synthetic copies rather than matching simulated emitter poses.

## Validation status

On 2026-10-24, `cargo test -p render --lib exhaust::tests --release` passed
both tests (WGSL, shared layout and input validation).
`cargo test -p cli --bin sideways exhaust_fx::tests --release` passed all
five tests: mount transforms/unknown models, actual GLB outlet cross-sections,
render cadence and pause independence, four-car bounds/round reset, and
engine edges/two-tick flash/cooldown. No physics code changed.

The actual native GPU fixture `render/examples/exhaust_probe.rs` passed on
2026-10-24. Its 512×320 checker-background captures show real heat distortion,
a soft grey smoke puff and a simple tapered emissive flame. These are basic
procedural forms, not a claim of final smoke/fire art quality.

| Native fixture check | Result |
| --- | --- |
| Heat / smoke / flame changed pixels | 2,625 / 2,817 / 3,199 |
| Heat behind a fully covering opaque blocker | 0 changed pixels |
| Effects switched off after three TAA frames versus untouched reference history | 0 changed pixels |
| Four identical 512×320 views in a 1024×640 output | Every tile exactly matches the single-view heat image |

Evidence: `C:/dev/perf/warp-sideways-20261024/r20/exhaust-gpu/report.json`

Two real stage captures also rendered successfully using DLSS Quality at
1920x1080: `exhaust-car-dlss.png` (tick799, 17 active sprites) and
`exhaust-car-rear-dlss.png` (tick1199, 10 active sprites) in the same evidence
directory. The second wider rear view includes the actual tailpipes. These
held-frame captures validate integration; the subtle smoke/refraction and
intermittent backfire still require moving visual judgement. They do not
establish 4K/four-player cost or demonstrate a backfire at the captured instant.
and adjacent `heat.png`, `smoke.png`, `flame.png`, `heat-blocked.png` and
`heat-four.png`. The three fixture exhaust timestamps were respectively
0.009440, 0.007520 and 0.007616 ms. They are individual small native test
measurements, **not gameplay FPS, a 4K cost estimate or a moving DLSS result**.

The fully covering blocker proves plume occlusion. A partially covering
foreground edge, moving reconstruction and real-car plume appearance still
need separate visual acceptance. The shader's displaced-sample depth check
exists, but the complete-blocker fixture alone does not prove that edge case.
