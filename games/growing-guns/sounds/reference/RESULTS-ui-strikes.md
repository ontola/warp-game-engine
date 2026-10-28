# Strike, movement and UI sounds: Luau patches against the Godot recipes

The Luau patches in `games/growing-guns/sounds/` for spec sections 6-8 (air strike
inbound, ion cannon charge, rocket descent, mine plant, dash, lava sizzle,
hit_received, kill_confirm, hitmarker, pick countdown beep, pling, card_flip,
pickup_drop, enemy_incoming), against 1:1 numpy transliterations of the GDScript
synths in `sfx.gd` (`reference/ui_ref.py`). The reference runs Godot's per-sample
loop at 44.1 kHz with Godot's draws and clamps (and the callers' cache buckets),
then clamps and quantises to 16 bits as `_samples_to_wav` does. Only the noise
realisation differs from Godot. The patch is rendered with `warp-audio render`
at 48 kHz.

Run it (about 7 minutes on 2 CPUs):

```sh
CARGO_TARGET_DIR=... cargo build -p audio --features luau --bin warp-audio
WARP_AUDIO=$CARGO_TARGET_DIR/debug/warp-audio \
  python3 games/growing-guns/sounds/reference/compare_ui.py --seeds 48 --markdown out.md
```

The metrics are those of `RESULTS-weapons.md` (the runner imports
`compare_weapons.py`'s analysis): duration (last sample above -60 dBFS), peak,
seed-averaged RMS envelope in 10 ms windows (mean |diff|, bias, max), whole-sound
centroid and per-10 ms centroid, each with its **[floor]**: the reference against
itself with a second set of seeds. Envelope and centroids are measured after
resampling the patch to 44.1 kHz. Two extra measures:

- **wave err**, for the recipes Godot renders without any random draw (mine plant,
  kill confirm, pick beep, pling, pickup drop): RMS of (patch - reference) over the
  reference RMS. Below about -40 dB they are the same waveform.
- **lava bands**: the lava sizzle's low band (< 400 Hz, the bubble bed) and high
  band (> 2 kHz, the hiss) envelopes separately, and the depth of the ~30 Hz bubble
  AM: the 60-250 Hz power envelope divided by the known body envelope, its 25-35 Hz
  line over its mean (a steady, unjittered AM would read 0.9).

## Patches

| file | params (game passes) | loop / fixed | playback (game side) |
|---|---|---|---|
| air_strike_inbound.luau | `travel_seconds` (air_strike.gd `_travel_seconds`, 2.3; Godot buckets to 0.25 s, clamps 1.75-6.5), `distant` 0/1 (always 0 in game) | fixed clip, plays once attached to the rocket; the approach is t / travel_seconds | raytraced 3D attached, unit 58 (88), -4 dB (-18), big-tail send -28 (-44) at spawn, intensity >= .42 |
| ion_cannon_charge.luau | `charge_seconds` (CHARGE_SECONDS 3.8; bucket 0.1 s, clamp 2.5-6.5) | fixed clip, plays once attached at the target; the build-up is t / charge_seconds | raytraced 3D attached, unit 140, -14 dB, big-tail -28, intensity >= .52; plus a companion copy at -10 dB, SPL 118 (HDR-gated, delayed, big-tail -34) |
| rocket_descent.luau | none | fixed 0.85 s | 2D, -4 dB, SPL 130 |
| mine_plant.luau | none | fixed 0.35 s | raytraced 3D, unit 12, -10 dB, SPL 90 |
| dash.luau | none | fixed 0.16 s | raytraced 3D, unit 12, -24 dB, SPL 75 |
| lava_sizzle.luau | `fall_death` 0/1 | fixed (0.42-0.58 s / 1.55-1.95 s per voice) | raytraced 3D, unit 18, no big tail, pitch U(.96, 1.04); contact -23 +-1 dB SPL 95, fall -26 +-1 dB SPL 72 |
| hit_received.luau | none (intensity is gain only) | fixed 0.18 s | 2D, -12 + 20 log10(clamp(i, .05, 2)) dB, SPL 130 |
| kill_confirm.luau | none | fixed 0.28 s | 2D, -6 dB, SPL 130 |
| hitmarker.luau (`hitmarker`, `hitmarker_head`, `hitmarker_kill`) | `dmg_ratio` = clamp(dmg / 22, .5, 5), 1 if dmg 0 | fixed 50-140 ms | 2D, -5 + clamp(2 log2 r, 0, 4) dB, SPL 130 |
| pick_countdown_beep.luau | `seconds_left` (3, 2, 1) | fixed 0.16 s | 2D, -8 dB, SPL 140 |
| pling.luau | `pitch_ratio` (1.15) | fixed 0.25 s | 2D, -10 dB, SPL 70 |
| card_flip.luau | none | fixed 0.16-0.22 s per voice | 2D, -10 dB, SPL 60 |
| pickup_drop.luau | none | fixed 0.62 s | raytraced 3D, unit 12, -11 dB, SPL 75 |
| enemy_incoming.luau | none | fixed 0.42 s | raytraced 3D, unit 12, -8.5 dB, SPL 95 |

No patch loops. The two attached cues (air strike, ion charge) are fixed clips in
Godot: one render as long as the flight or charge, played once, whose envelope and
filter sweep follow t / duration rather than any game state. A state-driven version
would replace `t / d` with a per-frame `progress` param and `loop()`; that is a
change of design, not a port, so it is not done.

## Results (48 base seeds)

Short noisy sounds get 4x the seeds, the rocket and lava 2x; deterministic recipes use 2.

| case | seeds | duration ref / patch (ms) [floor] | peak ref / patch (dBFS) [floor] | RMS env mean abs / bias / max diff (dB) [floor mean] | centroid ref / patch (Hz) | centroid over time diff [floor] | wave err (dB) |
|---|---|---|---|---|---|---|---|
| air_strike_inbound 2.25 s | 48 | 2250.0 / 2250.0 [0.0] | -4.2 / -4.2 [0.0] | 0.26 / -0.03 / 1.0 [0.26] | 29 / 29 | 12.5% [13.1%] | - |
| air_strike_inbound 1.75 s | 48 | 1750.0 / 1750.0 [0.0] | -4.2 / -4.2 [0.0] | 0.27 / -0.05 / 1.0 [0.27] | 29 / 29 | 12.8% [14.7%] | - |
| air_strike_inbound 6.5 s | 48 | 6500.0 / 6500.0 [0.0] | -4.2 / -4.2 [0.0] | 0.28 / -0.01 / 1.2 [0.27] | 29 / 29 | 13.2% [13.9%] | - |
| air_strike_inbound distant | 48 | 2250.0 / 2250.0 [0.0] | -4.2 / -4.2 [0.0] | 0.26 / -0.03 / 1.0 [0.26] | 28 / 28 | 12.4% [13.1%] | - |
| ion_cannon_charge 3.8 s | 48 | 3800.0 / 3800.1 [0.0] | -15.6 / -15.5 [0.0] | 0.19 / -0.02 / 0.6 [0.20] | 306 / 306 | 0.4% [0.4%] | - |
| ion_cannon_charge 2.5 s | 48 | 2500.0 / 2500.0 [0.0] | -15.6 / -15.6 [0.3] | 0.19 / -0.01 / 0.6 [0.21] | 306 / 306 | 0.4% [0.4%] | - |
| ion_cannon_charge 6.5 s | 48 | 6500.0 / 6500.0 [0.0] | -15.7 / -15.6 [0.0] | 0.19 / -0.02 / 0.7 [0.20] | 306 / 306 | 0.4% [0.4%] | - |
| rocket_descent | 96 | 850.0 / 850.0 [0.0] | -8.5 / -8.5 [0.1] | 0.30 / -0.05 / 0.8 [0.31] | 138 / 140 | 3.8% [3.5%] | - |
| mine_plant | 2 | 350.0 / 350.0 [0.0] | -10.1 / -10.1 [0.0] | 0.01 / -0.01 / 0.0 [0.00] | 387 / 388 | 0.0% [0.0%] | -60.1 |
| dash | 192 | 156.3 / 156.2 [0.0] | -20.7 / -20.7 [0.0] | 0.08 / -0.05 / 0.2 [0.06] | 2449 / 2447 | 0.9% [1.1%] | - |
| lava_sizzle contact | 96 | 502.6 / 503.0 [2.7] | -12.6 / -12.2 [0.0] | 0.16 / -0.10 / 0.7 [0.24] | 9788 / 9734 | 0.7% [0.5%] | - |
| lava_sizzle fall | 96 | 1721.6 / 1721.9 [6.7] | -13.5 / -13.3 [0.1] | 0.16 / -0.10 / 0.8 [0.27] | 8772 / 8742 | 1.5% [1.2%] | - |
| hit_received | 192 | 170.1 / 170.1 [0.0] | -1.7 / -1.5 [0.0] | 0.01 / -0.01 / 0.0 [0.00] | 316 / 315 | 0.1% [0.0%] | - |
| kill_confirm | 2 | 280.0 / 280.0 [0.0] | -9.0 / -9.0 [0.0] | 0.00 / -0.00 / 0.0 [0.00] | 450 / 450 | 0.0% [0.0%] | -67.3 |
| hitmarker body r 1 | 192 | 50.0 / 50.0 [0.0] | -2.4 / -2.3 [0.0] | 0.01 / -0.00 / 0.0 [0.01] | 2639 / 2633 | 0.2% [0.1%] | - |
| hitmarker body r 2 | 192 | 58.0 / 58.0 [0.0] | -2.3 / -2.2 [0.0] | 0.01 / -0.00 / 0.0 [0.00] | 2060 / 2052 | 0.3% [0.0%] | - |
| hitmarker body r 5 | 192 | 111.1 / 111.1 [0.0] | -2.1 / -2.0 [0.0] | 0.01 / -0.01 / 0.0 [0.00] | 1529 / 1523 | 0.2% [0.1%] | - |
| hitmarker head r 1 | 192 | 50.0 / 50.0 [0.0] | -2.5 / -2.4 [0.1] | 0.01 / -0.01 / 0.0 [0.00] | 3406 / 3399 | 0.2% [0.1%] | - |
| hitmarker head r 2 | 192 | 58.0 / 58.0 [0.0] | -2.4 / -2.3 [0.0] | 0.01 / -0.00 / 0.0 [0.01] | 2618 / 2614 | 0.1% [0.1%] | - |
| hitmarker head r .5 | 192 | 50.0 / 50.0 [0.0] | -2.5 / -2.4 [0.0] | 0.01 / -0.01 / 0.0 [0.00] | 4477 / 4474 | 0.1% [0.1%] | - |
| hitmarker kill r 1 | 192 | 50.0 / 50.0 [0.0] | -2.4 / -2.2 [0.0] | 0.01 / -0.00 / 0.0 [0.00] | 2024 / 2016 | 0.3% [0.1%] | - |
| pick beep 3 s left | 2 | 160.0 / 160.0 [0.0] | -10.2 / -10.2 [0.0] | 0.00 / -0.00 / 0.0 [0.00] | 440 / 440 | 0.0% [0.0%] | -72.0 |
| pick beep 2 s left | 2 | 160.0 / 160.0 [0.0] | -10.2 / -10.2 [0.0] | 0.00 / -0.00 / 0.0 [0.00] | 466 / 466 | 0.0% [0.0%] | -71.8 |
| pick beep 1 s left | 2 | 160.0 / 160.0 [0.0] | -10.2 / -10.2 [0.0] | 0.00 / -0.00 / 0.0 [0.00] | 494 / 494 | 0.0% [0.0%] | -71.7 |
| pling 1.15 | 2 | 249.9 / 249.9 [0.0] | -6.1 / -6.1 [0.0] | 0.01 / -0.00 / 0.0 [0.00] | 1010 / 1010 | 0.0% [0.0%] | -64.0 |
| pling 1 | 2 | 249.9 / 249.9 [0.0] | -6.1 / -6.1 [0.0] | 0.01 / -0.01 / 0.0 [0.00] | 878 / 878 | 0.0% [0.0%] | -65.7 |
| card_flip | 192 | 184.3 / 183.9 [1.6] | -9.2 / -8.7 [0.0] | 0.63 / -0.40 / 4.0 [0.49] | 6411 / 6375 | 1.0% [2.0%] | - |
| pickup_drop | 2 | 620.0 / 620.0 [0.0] | -10.8 / -10.8 [0.0] | 0.01 / -0.01 / 0.1 [0.00] | 1034 / 1034 | 0.0% [0.0%] | -42.0 |
| enemy_incoming | 192 | 420.0 / 420.0 [0.0] | -6.8 / -6.7 [0.0] | 0.06 / -0.05 / 0.2 [0.02] | 3935 / 3898 | 0.7% [0.4%] | - |

| lava case | low band (<400 Hz) env mean abs / bias (dB) [floor] | low energy patch - ref (dB) | high band (>2 kHz) env mean abs / bias (dB) [floor] | high energy patch - ref (dB) | 30 Hz AM depth ref / patch [ref 2nd seeds] |
|---|---|---|---|---|---|
| lava_sizzle contact | 0.65 / -0.04 [0.51] | -0.00 | 0.16 / -0.12 [0.22] | -0.15 | 0.461 / 0.422 [0.478] |
| lava_sizzle fall | 0.66 / +0.04 [0.56] | -0.05 | 0.14 / -0.13 [0.23] | -0.16 | 0.607 / 0.612 [0.637] |

The air strike's per-10 ms centroid gap (12-13%) sits at its floor: its energy is
at 15-40 Hz, under the 25 Hz resolution of a 40 ms frame, so the per-window
centroid is mostly leakage noise (as for the explosion rumble). Whole-sound
centroids agree to 1 Hz and the envelope is at the floor.

## The lava sizzle

Checked line by line against `_synth_lava_sizzle` (1830-1861): draw order (length,
then bubble phase), the attack `smoothstep(0, 0.075 | 0.42, t)` in seconds, the
contact body `exp(-4.2 u)` and the fall body `smoothstep(0, .18, u) * cos^1.65` over
the last 62%, the hiss `(x - lerp(h, x, .08)) * attack * body * (.26 | .11)` (a
585.2 Hz one-pole high-pass), the bubble bed `tanh(4 lerp(lo, y, .018))` (127.5 Hz)
scaled by the body *without* the attack, the per-sample U(22, 38) Hz phase step, and
`tanh(sum * (1.35 | 1.05))`.

- Duration, peak, envelope and centroid are at or near the floor in both modes.
- Bands: the bubble bed (< 400 Hz) matches in energy (0.00 / -0.05 dB) and envelope
  (floor-level scatter); the hiss (> 2 kHz) reads -0.15 dB, which is the comparison's
  own resampler: one 48 -> 44.1 kHz pass of resample_poly removes about 0.14 dB of
  white noise near 20-22 kHz (measured: a 44.1 -> 48 -> 44.1 round trip loses 0.27 dB).
- Bubble AM: Godot's U(22, 38) Hz redrawn every sample is a phase random walk, not a
  wobbling pitch. The patch drives a sine with `30 + 8 w sqrt(sr / 44100)`, which keeps
  Godot's walk per second at 48 kHz. Demodulated depth: fall 0.607 / 0.612 (floor
  0.637); contact 0.461 / 0.422 at 96 seeds, but 0.460 / 0.454 [0.476] at 320 seeds,
  and 0.152 / 0.152 on a fixed-length bubble-only render over 64 seeds, so the 96-seed
  contact gap is sampling noise.
- Peak +0.4 dB (contact) / +0.2 dB (fall): the hiss is broadband, and its sample peak
  follows the per-sample amplitude, which the noise-density scaling raises by 0.37 dB.
  Without the scaling the peak would match and the hiss band would read -0.37 dB.
- Godot plays one of 3 seeded variants per mode (`hash([variant, "lava_sizzle",
  fall_death])`); the patch draws its length and phase per voice from the same
  distributions. Godot's three exact renders (its `hash` seeds) are not reproduced.

## What the patches do beyond a literal port

The weapons' conventions (`RESULTS-weapons.md`): exact 44.1 kHz cutoffs for fixed
one-pole coefficients, noise scaled by `sqrt(sr / 44100)`, advance-then-read phases
(`p0 + hz / sr` as the oscillator's phase offset), gates at Godot's buffer length,
per-voice `rand` for seeded and global draws. Also:

- **RC one-poles** (air strike, rocket, enemy incoming: `alpha = dt / (rc + dt)`
  with a swept cutoff) become their exact 44.1 kHz cutoff per sample,
  `-ln(1 - alpha) * 44100 / 2π`.
- **Brown noise** (air strike: `0.998 b + 0.04 w`) is 20 x a 14.05 Hz one-pole
  low-pass of white noise, as in the rumble; the engine's `noise("brown")` has a
  different leak and clamp.
- **Large one-pole k** (dash 0.08-0.22, card flip 0.2-0.55): a 44.1 kHz one-pole
  levels off at `k / (2 - k)` near its Nyquist; a 48 kHz one-pole of the same cutoff
  keeps falling, which left the dash centroid 2.7% low (floor 1.1%). `lp44(x, k)`
  adds a direct term `c = 0.025 k + 0.06 k²` and lowers the cutoff by `0.025 k`,
  which fits Godot's power response over 0-20 kHz to 0.03 dB for k up to 0.55 (the
  high-pass is `x - lp44`). Dash centroid now 2447 / 2449 Hz. The term fades to
  zero at 44.1 kHz and is fitted at 48 kHz. The same fix would remove the 1-3%
  centroid gap the weapon patches list.
- **Non-integrated chirps** (hit_received, pickup_drop, enemy_incoming) are kept as
  Godot wrote them, `sin(2π f(t) t)`, so their artefacts (pickup_drop's whoosh sweeps
  through 0 Hz near the end) are reproduced.
- **Card flip windows**: Godot runs the wash filter only while `t < noise_dur` and the
  tick filter from `tick_start`, each from an empty state; the patch gates each
  filter's input the same way and keeps the voice alive across the gap between them.
- **Hitmarker kinds** are sounds in one file: `hitmarker` (body), `hitmarker_head`,
  `hitmarker_kill`, matching Godot's debug labels.

## Remaining gaps

- **Card flip peak +0.5 dB**, envelope 0.63 dB against a 0.49 floor. The peak is the
  tick's first sample, `(1 - k) n`: the 48 kHz coefficient for the same cutoff is
  smaller, so the onset is up to 0.4 dB higher, plus the noise scaling. The envelope
  excess is in the tick windows, whose start time is a per-voice draw; the wash
  windows sit at -0.1 dB (the resampler loss above).
- **Peaks +0.1 to +0.4 dB** on sounds with broadband noise (lava, hit_received,
  hitmarker): noise scaling, as for the weapons.
- **pickup_drop waveform -42 dB** (the others -60 to -72 dB): `expr` evaluates
  `sin(2π f t)` in f32, and at 0.6 s that phase is 3600 rad, so the f32 rounding
  of `t` shows. Envelope and centroid are exact; it is inaudible.
- **Enemy incoming centroid -0.9%** (per window 0.7% against 0.4%): its bursts are
  raw white noise into tanh; the 22-24 kHz band of the 48 kHz noise is filtered out by
  the comparison, the rest of the spectrum is matched by density.
- **Mono**: the ion charge's right channel is 0.98 x left in Godot (-0.18 dB); the
  patch is mono.
