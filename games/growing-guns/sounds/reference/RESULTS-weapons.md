# Weapon sounds: Luau patches against the Godot recipes

The Luau patches in `games/growing-guns/sounds/` against 1:1 numpy
transliterations of the GDScript synths in `sfx.gd` (`reference/weapons_ref.py`).
The reference runs Godot's per-sample loop at 44.1 kHz with Godot's jitter draws
and clamps, then clamps and quantises to 16 bits as `_samples_to_wav` does. Only
the noise realisation differs from Godot. The patch is rendered with
`warp-audio render` at 48 kHz.

Run it (about 8 minutes on 2 CPUs):

```sh
CARGO_TARGET_DIR=... cargo build -p audio --features luau --bin warp-audio
WARP_AUDIO=$CARGO_TARGET_DIR/debug/warp-audio \
  python3 games/growing-guns/sounds/reference/compare_weapons.py --seeds 48 --markdown out.md
```

## How to read the table

Each voice draws its own jitter (Godot's seeded `U(a, b)` became `rand(a, b)`),
so every row averages many seeds: reference seeds 1001.., patch seeds 1...
Short sounds get 4x the seeds, because their seed-averaged envelope is dominated
by the slowest-decaying draws.

- **duration**: last sample above -60 dBFS, mean over seeds. **peak**: mean
  peak in dBFS. Both at the native rate.
- **RMS env**: seed-averaged power in 10 ms windows, in dB, over windows within
  40 dB of the loudest: mean |patch - reference|, the mean signed difference
  (bias) and the largest window difference.
- **centroid**: whole-sound spectral centroid (0-20 kHz), and the mean
  difference of the per-10 ms centroid (40 ms Hann frames, power-weighted, over
  windows within 30 dB of the loudest).
- Envelope and centroids are measured after resampling the patch to 44.1 kHz,
  so both sides are compared in the same 0-22 kHz band.
- **[floor]**: the same measure for the reference against itself with a second
  set of seeds. That is how far two honest renders of the same recipe differ;
  a patch gap at or near the floor is not a real difference.

## Results (48 base seeds)

| case | seeds | duration ref / patch (ms) [floor] | peak ref / patch (dBFS) [floor] | RMS env mean abs / bias / max diff (dB) [floor mean] | centroid ref / patch (Hz) | centroid over time diff [floor] |
|---|---|---|---|---|---|---|
| shot base | 192 | 77.8 / 77.8 [0.8] | -0.0 / -0.0 [0.0] | 0.10 / +0.01 / 0.3 [0.06] | 8581 / 8502 | 0.8% [0.6%] |
| shot heavy (dmg 3, acc 1) | 192 | 192.9 / 192.9 [1.3] | 0.0 / -0.0 [0.0] | 0.08 / +0.05 / 0.3 [0.05] | 3050 / 3011 | 1.0% [1.3%] |
| shot fast spray (acc .3, vel 3) | 192 | 44.9 / 44.9 [0.4] | -0.0 / -0.0 [0.0] | 0.12 / -0.00 / 0.3 [0.08] | 8071 / 7969 | 1.1% [0.9%] |
| shot bazooka (dmg 6, vel .5) | 192 | 312.7 / 313.5 [1.0] | 0.0 / -0.0 [0.0] | 0.09 / +0.05 / 0.3 [0.10] | 1994 / 1964 | 1.3% [1.4%] |
| grenade_launch | 96 | 176.3 / 176.3 [0.0] | -3.1 / -3.1 [0.0] | 0.03 / -0.01 / 0.1 [0.03] | 611 / 607 | 0.7% [0.6%] |
| next_round | 192 | 51.8 / 51.6 [0.0] | -6.4 / -6.0 [0.0] | 0.08 / -0.08 / 0.2 [0.03] | 6134 / 6102 | 0.5% [0.2%] |
| empty_chamber | 192 | 22.1 / 22.0 [0.1] | -6.7 / -6.3 [0.0] | 0.05 / -0.05 / 0.1 [0.02] | 5605 / 5605 | 0.3% [0.3%] |
| reload 1.2 s | 48 | 1176.5 / 1177.4 [0.8] | -16.7 / -16.3 [0.2] | 0.76 / +0.03 / 2.6 [0.63] | 6582 / 6526 | 1.4% [1.3%] |
| reload 0.4 s | 48 | 400.0 / 400.0 [0.0] | -16.4 / -16.2 [0.0] | 0.26 / -0.05 / 0.8 [0.23] | 6578 / 6539 | 0.9% [0.8%] |
| reload 3.0 s | 48 | 2915.8 / 2918.1 [1.8] | -16.7 / -16.2 [0.2] | 1.54 / +0.06 / 6.4 [1.39] | 6582 / 6529 | 2.0% [2.1%] |
| melee pr .71 | 192 | 199.3 / 199.3 [0.0] | -9.8 / -9.6 [0.0] | 0.04 / -0.03 / 0.1 [0.04] | 4338 / 4331 | 0.5% [0.7%] |
| melee pr 1.2 | 192 | 199.5 / 199.5 [0.0] | -8.3 / -8.0 [0.0] | 0.08 / -0.08 / 0.2 [0.03] | 6444 / 6396 | 0.7% [0.5%] |
| melee pr .5 | 192 | 199.1 / 199.1 [0.0] | -10.9 / -10.9 [0.0] | 0.04 / -0.01 / 0.1 [0.06] | 3328 / 3328 | 0.6% [0.8%] |
| casing_drop | 192 | 180.0 / 180.0 [0.0] | -2.2 / -2.0 [0.0] | 0.20 / -0.20 / 0.4 [0.06] | 9002 / 8996 | 0.0% [0.1%] |
| bullet_zip sf 1 | 192 | 121.2 / 120.3 [0.6] | -6.9 / -6.7 [0.0] | 0.43 / -0.43 / 0.7 [0.22] | 11621 / 11469 | 1.3% [0.3%] |
| bullet_zip sf .2 sc 2 | 192 | 363.6 / 360.8 [1.8] | -2.7 / -2.5 [0.0] | 0.31 / -0.31 / 0.7 [0.28] | 9548 / 9474 | 0.9% [0.7%] |
| bullet_zip sf 4 | 192 | 75.7 / 75.2 [0.4] | -3.1 / -2.9 [0.0] | 0.52 / -0.52 / 0.7 [0.24] | 12009 / 11859 | 1.2% [0.1%] |
| bullet_zip sf 5 sc 4 | 192 | 72.7 / 72.2 [0.4] | -0.0 / -0.0 [0.0] | 0.58 / -0.58 / 0.9 [0.29] | 11758 / 11619 | 1.2% [0.2%] |
| impact dmg 2 | 192 | 43.0 / 42.8 [0.1] | -1.4 / -1.2 [0.0] | 0.14 / +0.04 / 0.3 [0.09] | 7380 / 7249 | 1.5% [0.7%] |
| impact dmg 1 | 192 | 27.6 / 27.6 [0.2] | -1.6 / -1.4 [0.0] | 0.46 / -0.46 / 0.7 [0.04] | 10971 / 10900 | 0.7% [0.8%] |
| impact dmg 3 | 192 | 55.7 / 55.4 [0.2] | -1.0 / -1.0 [0.0] | 0.13 / +0.07 / 0.3 [0.04] | 3584 / 3497 | 2.0% [0.5%] |
| impact dmg 6 | 192 | 81.0 / 80.6 [0.2] | -0.5 / -0.5 [0.0] | 0.17 / +0.16 / 0.4 [0.03] | 1069 / 1049 | 1.4% [0.4%] |
| explosion_bang r 2 | 48 | 555.8 / 552.1 [7.4] | -0.2 / -0.2 [0.0] | 0.38 / -0.01 / 1.7 [0.32] | 1937 / 1911 | 2.7% [4.9%] |
| explosion_bang r 6 | 48 | 817.8 / 817.1 [5.9] | -0.2 / -0.2 [0.0] | 0.46 / +0.02 / 1.7 [0.34] | 1803 / 1779 | 2.5% [4.8%] |
| explosion_bang r 24 | 48 | 1394.3 / 1391.8 [4.6] | -0.1 / -0.1 [0.0] | 0.47 / +0.05 / 1.8 [0.47] | 559 / 555 | 2.1% [2.9%] |
| explosion_rumble r 2 | 48 | 304.6 / 303.9 [0.6] | -4.7 / -4.7 [0.2] | 0.80 / +0.41 / 2.2 [0.83] | 28 / 25 | 11.8% [9.0%] |
| explosion_rumble r 6 | 48 | 538.2 / 537.9 [3.0] | -4.2 / -3.9 [0.2] | 0.81 / +0.42 / 3.1 [0.84] | 26 / 25 | 10.4% [8.6%] |
| explosion_rumble r 12 | 48 | 1316.7 / 1315.4 [6.3] | -5.6 / -5.4 [0.2] | 0.79 / +0.10 / 2.9 [0.80] | 23 / 22 | 10.3% [8.9%] |
| explosion_rumble r 24 | 48 | 4384.1 / 4399.7 [35.2] | -7.7 / -7.6 [0.2] | 0.87 / +0.01 / 4.1 [0.87] | 16 / 16 | 10.5% [10.2%] |
| ion_burst r 24 | 48 | 733.5 / 739.1 [4.1] | -3.2 / -3.2 [0.0] | 0.61 / +0.15 / 1.7 [0.52] | 140 / 141 | 1.7% [2.7%] |
| ion_burst r 38 | 48 | 843.5 / 849.7 [4.6] | -3.2 / -3.2 [0.0] | 0.63 / +0.15 / 1.8 [0.56] | 134 / 135 | 2.0% [2.8%] |
| ion_burst r 48 | 48 | 918.8 / 926.3 [5.3] | -3.2 / -3.2 [0.0] | 0.64 / +0.17 / 1.9 [0.57] | 129 / 130 | 2.1% [2.9%] |

The gunshot rumble (`dmg / 22 > 1.4`) is `explosion_rumble.luau` at radius
`round(clamp(1.5 dmg / 22, 2, 12))`, the same synth Godot uses, so the rumble
rows at r 2, 6 and 12 cover it.

## What the patches do beyond a literal port

- **One-poles** (`y = lerp(y, x, k)`, fixed or swept k, including coefficients
  the GDScript computes from Hz with the small-k formula and clamps) use the
  prelude's `godot_lp(x, k)` / `godot_hp(x, k)`, which take Godot's k directly.
  A 44.1 kHz one-pole low-pass levels off near its Nyquist (`|H| -> k / (2 - k)`);
  a 48 kHz one at the exact cutoff `-ln(1 - k) * 44100 / 2π` keeps falling there,
  which put the filtered-noise centroids 1-3% low. `godot_lp` lowers that cutoff
  slightly and adds a small direct term, fitting the 44.1 kHz power response to
  0.03 dB up to 20 kHz (and is the plain exact cutoff at 44.1 kHz). Against the
  plain exact-cutoff form (same engine, same seeds) this closed the centroid gap
  on empty chamber (2.4% -> 0.3%), next round (2.5 -> 0.5%), melee (2.4-2.5 ->
  0.5-0.7%), reload (1.9-2.6 -> 0.9-2.0%) and the heavy shots (3.1 -> 1.0-1.3%),
  and halved their envelope bias. `godot_hp` also matches Godot's high-pass within
  0.25 dB for k <= 0.6 (computed). The cost: reload goes from 234 to 250 nodes,
  and its render takes 5-9% longer (`warp-audio render --json`, 15 interleaved
  seeds on a loaded 2-CPU machine: 1.2 s min 71.6 -> 76.9 ms, median 83.7 -> 91.0;
  3.0 s min 155 -> 162 ms, median 206 -> 221).
- **Noise density.** Godot draws one uniform sample per 44.1 kHz tick. At 48 kHz
  the same per-sample variance spreads over a wider band, so every filtered
  noise lost 0.37 dB (10 log10(48000 / 44100)); this was the systematic -0.3 to
  -0.7 dB in the first comparison. Patches scale white noise by `sqrt(sr / 44100)`.
  The gunshot thwack is the exception: its tanh flattens the noise to full scale,
  so it keeps Godot's per-sample level going in and scales the result. That
  removed a 0.4 dB deficit and the 6% centroid gap of the heavy shots.
- **Phase.** Godot advances a phase before reading it; the formula oscillators
  use `t + 1 / sr`.
- **Buffer ends and clamps.** Godot's buffer stops at `int(dur * SR)` and its WAV
  writer hard-clamps to ±1. Patches gate at `dur`; the gunshot also clamps (its
  output high-pass overshoots 1 by up to 0.7 dB).
- **Seeded variants** (`hash([..., variant, tag])`) and global `randf` jitter both
  become per-voice `rand(a, b)`; integer variant picks (grenade body pitch,
  reload tap counts) use `floor(rand(...))`.

## Remaining gaps

- **Bullet zip centroid 0.9-1.3% low, envelope -0.3 to -0.6 dB, 0.5-3 ms short.**
  `godot_lp` did not move it beyond seed noise: its first filter sits at the
  k 0.98 clamp (nearly a pass-through on both rates), so the band is mostly
  `godot_hp` of the noise. The gap is upstream of the filters; a likely cause is
  the tanh drive (up to 3.5 x 4) on noise scaled to Godot's density rather than
  its per-sample level, the issue the gunshot thwack already handles. Untested.
- **Impact centroid over time 0.7-2.0%** against floors of 0.4-0.8%. With the
  runner's seeds it read worse than the exact-cutoff form (0.5-0.9%), but with
  patch seeds 5001.. the new form was as good or better on every case (0.2-1.4%
  against 0.4-1.9%). The extra nodes renumber the voice's draws, so the click
  cutoff, decays and thump draws come out differently. The shift is draw noise,
  not the filter.
- **Peaks 0.2-0.4 dB high on the filtered-noise sounds**: the noise-density
  scaling, with the 22-24 kHz band Godot does not have in the native-rate peak.
- **Reload peak +0.2 to +0.5 dB.** Godot gives each tap a fresh noise and
  filter pair. The patch runs one band-pass per cluster that jumps to each tap's
  pitch at its onset, under the power sum of the tap envelopes. Each tap starts on
  a settled filter instead of an empty one, so onsets are a little sharper. RMS,
  duration and spectrum match to the floor. A per-tap version (20 noise and filter
  slots, 410 nodes) matched no closer (peak +0.2 to +0.5 dB, envelope 0.7 dB,
  centroid 1.8%) and cost about 1.5x.
- **Rumble centroid over time, 10-12% against a floor of 9-10%.** The rumble's
  energy is at 15-30 Hz, below the 25 Hz resolution of a 40 ms frame, so the
  per-window centroid is mostly leakage noise. The whole-sound centroids agree
  to 1 Hz and the envelopes sit at the floor.
- The bang durations scatter within their floors.
- **Mono.** The ion burst's right channel is 0.99 x left in Godot; the patch is
  mono and the 0.04 dB difference is left out.
