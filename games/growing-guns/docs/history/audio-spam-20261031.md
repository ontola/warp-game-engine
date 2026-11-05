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

## Playtest correction: explosions must dominate shots

The first protected mix was too subdued in the actual playtest. Explosions
now use a +10 dB report and 0 dB bass, without the old close-range boost.
Own/world shot bases are -12/-14 dB, with only +1 dB per damage doubling
(capped at +3). Explosive weapons reserve their extra bass for impacts,
rather than triggering the same bass layer under every muzzle report.
The blast-report budget is 24; the shared bass budget stays six.

A new isolated close-impact regression loads the same uzi x2 + bazooka x2
weapon and compares the loudest 50 ms RMS after the entire game master chain:
shot 0.211, explosion 0.385, explosion advantage 5.23 dB. It requires at
least 4 dB, so a protected but inaudible explosion mix cannot pass again.
The sustained tests still have zero stolen voices, now maximum 77 active;
peaks are 0.747 stereo/48 kHz and 0.824 surround/44.1 kHz. All three tests
pass. These are offline measurements; the user evaluates the physical mix.


## Restoring large-blast bass

Large impacts now add an audible 38-72 Hz body, remove subsonic rumble energy,
and retain a slower tail up to 2.6 seconds. Bass has 12 reserved voices and
size-dependent gain up to +3 dB; overlap attenuation and the master ceiling
remain active. Stereo/surround spam peaks: 0.7754 / 0.8695; 83 maximum voices,
zero stolen. Loudest 50 ms explosion advantage: 5.00 dB. These are offline
mix measurements; the packaged playtest is the listening check.


## Final listening correction: sustained noisy aftershock

The user preferred the Godot-style brown-noise aftermath over the added sine
body. Rumble now lasts up to 3.2 seconds, with a slower decay, a quieter tone
and a roughly 6 Hz high-pass that preserves audible low-frequency noise.
The 1.5-2.5 second window of the isolated large explosion rises from -25.6 to
-16.4 dBFS RMS. A regression requires at least 0.08 RMS in that window.
Stereo/surround spam peaks are 0.7951 / 0.8857, below the -1 dBFS ceiling;
83 maximum voices, zero stolen voices. Explosion/report advantage is 4.79 dB.
All three audio regressions pass. The user approved the packaged listening
playtest; no separate physical receiver or Quest validation is claimed.
