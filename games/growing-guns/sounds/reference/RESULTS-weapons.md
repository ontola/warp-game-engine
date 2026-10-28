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
| shot base | 192 | 77.8 / 77.0 [0.8] | -0.0 / -0.0 [0.0] | 0.23 / -0.23 / 0.5 [0.06] | 8581 / 8595 | 0.3% [0.6%] |
| shot heavy (dmg 3, acc 1) | 192 | 192.9 / 191.2 [1.3] | 0.0 / -0.0 [0.0] | 0.11 / -0.11 / 0.3 [0.05] | 3050 / 3045 | 0.8% [1.3%] |
| shot fast spray (acc .3, vel 3) | 192 | 44.9 / 44.5 [0.4] | -0.0 / -0.0 [0.0] | 0.17 / -0.17 / 0.3 [0.08] | 8071 / 8123 | 0.6% [0.9%] |
| shot bazooka (dmg 6, vel .5) | 192 | 312.7 / 312.8 [1.0] | 0.0 / -0.0 [0.0] | 0.09 / +0.05 / 0.2 [0.10] | 1994 / 1986 | 0.9% [1.4%] |
| grenade_launch | 96 | 176.3 / 176.3 [0.0] | -3.1 / -3.1 [0.0] | 0.03 / -0.02 / 0.2 [0.03] | 611 / 599 | 1.1% [0.6%] |
| next_round | 192 | 51.8 / 51.7 [0.0] | -6.4 / -6.1 [0.0] | 0.13 / -0.13 / 0.2 [0.03] | 6134 / 5977 | 2.5% [0.2%] |
| empty_chamber | 192 | 22.1 / 22.0 [0.1] | -6.7 / -6.4 [0.0] | 0.07 / -0.07 / 0.1 [0.02] | 5605 / 5467 | 2.8% [0.3%] |
| reload 1.2 s | 48 | 1176.5 / 1177.3 [0.8] | -16.7 / -15.9 [0.2] | 0.73 / +0.23 / 2.0 [0.63] | 6582 / 6434 | 2.7% [1.3%] |
| reload 0.4 s | 48 | 400.0 / 400.0 [0.0] | -16.4 / -15.7 [0.0] | 0.19 / +0.03 / 0.7 [0.23] | 6578 / 6436 | 2.3% [0.8%] |
| reload 3.0 s | 48 | 2915.8 / 2918.0 [1.8] | -16.7 / -16.2 [0.2] | 1.68 / +0.01 / 5.9 [1.39] | 6582 / 6493 | 3.0% [2.1%] |
| melee pr .71 | 192 | 199.3 / 199.3 [0.0] | -9.8 / -9.6 [0.0] | 0.07 / -0.05 / 0.2 [0.04] | 4338 / 4221 | 2.7% [0.7%] |
| melee pr 1.2 | 192 | 199.5 / 199.4 [0.0] | -8.3 / -8.0 [0.0] | 0.13 / -0.13 / 0.2 [0.03] | 6444 / 6276 | 2.6% [0.5%] |
| melee pr .5 | 192 | 199.1 / 199.1 [0.0] | -10.9 / -10.8 [0.0] | 0.07 / -0.02 / 0.1 [0.06] | 3328 / 3242 | 2.6% [0.8%] |
| casing_drop | 192 | 180.0 / 180.0 [0.0] | -2.2 / -2.0 [0.0] | 0.05 / +0.05 / 0.1 [0.06] | 9002 / 9039 | 0.4% [0.1%] |
| bullet_zip sf 1 | 192 | 121.2 / 121.9 [0.6] | -6.9 / -6.6 [0.0] | 0.19 / -0.04 / 0.3 [0.22] | 11621 / 11448 | 1.4% [0.3%] |
| bullet_zip sf .2 sc 2 | 192 | 363.6 / 365.8 [1.8] | -2.7 / -2.6 [0.0] | 0.24 / -0.01 / 0.4 [0.28] | 9548 / 9375 | 1.7% [0.7%] |
| bullet_zip sf 4 | 192 | 75.7 / 76.2 [0.4] | -3.1 / -2.8 [0.0] | 0.17 / -0.04 / 0.5 [0.24] | 12009 / 11851 | 1.3% [0.1%] |
| bullet_zip sf 5 sc 4 | 192 | 72.7 / 73.2 [0.4] | -0.0 / -0.0 [0.0] | 0.39 / -0.03 / 1.5 [0.29] | 11758 / 11609 | 1.2% [0.2%] |
| impact dmg 2 | 192 | 43.0 / 43.0 [0.1] | -1.4 / -1.3 [0.0] | 0.18 / -0.06 / 0.3 [0.09] | 7380 / 7366 | 0.4% [0.7%] |
| impact dmg 1 | 192 | 27.6 / 27.7 [0.2] | -1.6 / -1.4 [0.0] | 0.21 / -0.21 / 0.2 [0.04] | 10971 / 10871 | 0.8% [0.8%] |
| impact dmg 3 | 192 | 55.7 / 55.7 [0.2] | -1.0 / -0.9 [0.0] | 0.15 / -0.10 / 0.3 [0.04] | 3584 / 3629 | 1.0% [0.5%] |
| impact dmg 6 | 192 | 81.0 / 81.0 [0.2] | -0.5 / -0.4 [0.0] | 0.20 / -0.06 / 0.6 [0.03] | 1069 / 1104 | 2.0% [0.4%] |
| explosion_bang r 2 | 48 | 555.8 / 550.3 [7.4] | -0.2 / -0.2 [0.0] | 0.42 / -0.12 / 1.3 [0.32] | 1937 / 1926 | 2.9% [4.9%] |
| explosion_bang r 6 | 48 | 817.8 / 808.5 [5.9] | -0.2 / -0.2 [0.0] | 0.47 / -0.02 / 1.7 [0.34] | 1803 / 1780 | 3.4% [4.8%] |
| explosion_bang r 24 | 48 | 1394.3 / 1390.1 [4.6] | -0.1 / -0.1 [0.0] | 0.50 / +0.22 / 1.8 [0.47] | 559 / 541 | 3.6% [2.9%] |
| explosion_rumble r 2 | 48 | 304.6 / 304.3 [0.6] | -4.7 / -4.8 [0.2] | 0.84 / +0.31 / 2.3 [0.83] | 28 / 28 | 11.9% [9.0%] |
| explosion_rumble r 6 | 48 | 538.2 / 537.6 [3.0] | -4.2 / -4.2 [0.2] | 0.76 / +0.42 / 2.4 [0.84] | 26 / 26 | 11.2% [8.6%] |
| explosion_rumble r 12 | 48 | 1316.7 / 1320.3 [6.3] | -5.6 / -5.6 [0.2] | 0.86 / -0.03 / 3.2 [0.80] | 23 / 23 | 11.0% [8.9%] |
| explosion_rumble r 24 | 48 | 4384.1 / 4409.4 [35.2] | -7.7 / -7.7 [0.2] | 0.96 / +0.21 / 3.8 [0.87] | 16 / 16 | 10.8% [10.2%] |
| ion_burst r 24 | 48 | 733.5 / 737.2 [4.1] | -3.2 / -3.2 [0.0] | 0.36 / +0.14 / 2.3 [0.52] | 140 / 141 | 1.8% [2.7%] |
| ion_burst r 38 | 48 | 843.5 / 847.3 [4.6] | -3.2 / -3.2 [0.0] | 0.35 / +0.13 / 1.9 [0.56] | 134 / 135 | 1.9% [2.8%] |
| ion_burst r 48 | 48 | 918.8 / 921.3 [5.3] | -3.2 / -3.2 [0.0] | 0.33 / +0.08 / 1.0 [0.57] | 129 / 130 | 2.1% [2.9%] |

The gunshot rumble (`dmg / 22 > 1.4`) is `explosion_rumble.luau` at radius
`round(clamp(1.5 dmg / 22, 2, 12))`, the same synth Godot uses, so the rumble
rows at r 2, 6 and 12 cover it.

## What the patches do beyond a literal port

- **Fixed one-pole coefficients** (`y = lerp(y, x, k)`) become their exact
  44.1 kHz cutoffs, `-ln(1 - k) * 44100 / 2π`, including coefficients the GDScript
  computes from Hz with the small-k formula and clamps. The cutoffs then hold in
  Hz at 48 kHz.
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

- **Centroids 1-3% low on filtered-noise sounds** (bolt, empty chamber, melee,
  zip, reload). A 44.1 kHz one-pole low-pass levels off near its Nyquist
  (`|H| -> k / (2 - k)` at 22 kHz); the 48 kHz one with the same cutoff keeps
  falling there. Computed for the gunshot tail's filter, the 48 kHz centroid is
  2% lower over 0-20 kHz, as measured. A one-pole cannot match both shapes; the
  difference is above 15 kHz and 1-2 dB.
- **Peaks 0.2-0.3 dB high on the same sounds**: the noise-density scaling, with
  the 22-24 kHz band Godot does not have in the native-rate peak.
- **Reload peak +0.5 to +0.8 dB.** Godot gives each tap a fresh noise and
  filter pair. The patch runs one band-pass per cluster that jumps to each tap's
  pitch at its onset, under the power sum of the tap envelopes. Each tap starts on
  a settled filter instead of an empty one, so onsets are a little sharper. RMS,
  duration and spectrum match to the floor. A per-tap version (20 noise and filter
  slots, 410 nodes) matched no closer (peak +0.2 to +0.5 dB, envelope 0.7 dB,
  centroid 1.8%) and cost about 1.5x.
- **Rumble centroid over time, 11-12% against a floor of 9-10%.** The rumble's
  energy is at 15-30 Hz, below the 25 Hz resolution of a 40 ms frame, so the
  per-window centroid is mostly leakage noise. The whole-sound centroids agree
  to 1 Hz and the envelopes sit at the floor.
- **Bullet zip 0.5-2 ms long** (0.6-1.5%, about twice the floor), and the bang
  durations scatter within their floors. Some of this is the engine's `rand`
  draws: the first draws of different nodes in one voice correlate (r = -0.15
  over 1000 seeds: `Patch::voice` derives node seeds linearly and xorshift32 is
  linear, so first outputs of related seeds are related); each marginal is uniform.
- **Mono.** The ion burst's right channel is 0.99 x left in Godot; the patch is
  mono and the 0.04 dB difference is left out.
