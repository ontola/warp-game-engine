# Sustained combat audio, 31 October 2026

Warp-only change. Reproduce with:

```powershell
$env:GG_AUDIO_CAPTURE = 'C:/temp/gg-spam.wav'
cargo test --locked -p growing-guns-app --lib audio_stress_tests -- --nocapture
```

The fixture loads the real base cards and Luau patches, equips four players
with uzi, uzi, bazooka, bazooka, then deliberately drives the sound event
path above its gameplay fire rate: 12 close shots and radius-24 blasts per
second for four seconds, four simultaneous shooters for four seconds, then
six seconds of decay. Deferred casing and bolt cues also run. This tests
sound dispatch/mixing, not projectile simulation or renderer frame time.

Stereo/48 kHz before: peak 0.966 (-0.3 dBFS), 160 active voices,
602 hard voice steals, 865 priority-culls. Same fixture after: peak 0.581
(-4.7 dBFS), maximum 102 active voices, zero steals. Priority culls (447)
are still allowed for incidental sounds; weapon reports bypass this gate.
Final one-second peak is below 0.000001 and all voices finish. 5.1/44.1 kHz
also passes: peak 0.562, maximum 102 voices, zero steals. Both tests assert
finite samples, spare pool capacity, headroom and complete decay.

Separate serial runs took 11.30 s before and 3.92 s after to dispatch and
render 14 s of audio in the optimized dev profile on this Windows machine.
This is an offline cost measurement, not an audio-device underrun guarantee.

Changes: authored report/bass gain; no close-range combat boost; one report
voice feeding the room rather than another synth copy; separate lifetime
budgets for own reports, world reports, blast reports and shared bass.
Overload drops only new layers, never cuts an existing waveform. Bass tails
last at most two seconds. Replay resets clear the budgets. The game master
retains a linked peak limiter with a -1 dBFS ceiling.

No physical receiver, speakers or Quest listening validation is claimed.
The runtime synth still has its deliberately saturated weapon timbre;
this change addresses mix overload and voice churn, not a new sound design.
