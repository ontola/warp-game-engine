# Audio

The `audio` crate makes sound rather than only playing it back. Everything
is deterministic from a seed and runs offline as well as live, so a test
or an agent can render a WAV and check it. The sample rate constant is
`audio::SAMPLE_RATE` (48 kHz); the newer synth takes the rate as an
argument.

It has three parts:

1. engine and tyre sound for vehicles (used by Sideways);
2. sample playback for game effects, `audio::sfx` (used by Growing Guns);
3. a procedural synth, `audio::synth`: units, patches as data, and a
   surround mixer, used by Growing Guns for its procedural effects and
   recorded footsteps, hurts and deaths.

`app::sound::start(voices)` opens the default output device with `cpal`
and mixes `sfx` in its callback; without a device the game stays silent.
Growing Guns uses `app::sound::open` to combine the synth, the crowd's
sample mixer and its dedicated stem music player at the device's rate.

## Engine sound

On Windows, `cargo run --profile quest -p warp-engine-generator` opens a
standalone listening bench. Start the engine, set RPM and load, and switch
between configurations A and B to compare engine layouts and mufflers.
**Copy Luau** copies the selected setup as a game sound patch. To build the
single-file Windows MSVC demo with its C runtime included, use
`cargo rustc --locked --profile quest -p warp-engine-generator --bin warp-engine-generator -- -C target-feature=+crt-static`.

Luau sound patches can use the shared pulse exhaust model with `engine("inline_four", rpm, throttle, load, cut, gear, speed)` and add `tyres(speed, slip, contact, loose, soft, wet)`. Built-in engine presets include `inline_four`, `inline_five` and `boxer_four`. A caller can register additional presets or override a built-in ID. Use `sound.move_to(handle, worldPosition)` to keep a running spatial voice attached to its vehicle as it passes the listeners. [Warp Theft Auto's motor patch](../../games/warp-theft-auto/sounds/motor.luau)

Each cylinder generates a blowdown pulse at its exhaust valve timing. Firing
order, RPM, load and ignition cuts shape the source. The shared engine can
mix separately synthesized intake, valvetrain, turbo and gear sounds.

To model a muffler, pass the base engine ID and its exhaust dimensions to
the pulse engine. Lengths and internal diameters are in metres. `packing`
ranges from 0 (bare chamber) to 1 (fully lined).

```luau
local sedan = {
    model = "inline_four",
    muffler = {
        inlet_length = 1.85, inlet_diameter = 0.050,
        chamber_length = 0.46, chamber_diameter = 0.165,
        outlet_length = 0.66, outlet_diameter = 0.045,
        packing = 0.78,
    },
}
local exhaust = engine(sedan, rpm, throttle, load, cut, gear, speed)
```

Cylinder pulses meet at the collector and excite the inlet. Two travelling
pressure waves run through each of the three sections. The bore changes
split incoming waves into reflected and transmitted pressure. Chamber length
sets the travel time between those reflections. Lining dissipates energy,
and the outlet radius determines the open-end response. The resulting
tailpipe sound uses the same geometry on native and WebAudio. Each voice
takes its geometry when it starts. The model assumes linear plane waves at
343 m/s and approximates the absorption of the chamber lining. The vehicle
solver supplies engine speed and torque independently of exhaust backpressure.
[Digital waveguide theory](https://www.dsprelated.com/freebooks/pasp/Digital_Waveguide_Theory.html)

- `EngineVoice` has two exhaust models: `Model::Pulse` (delay-line
  headers with optional waveguide muffler) and `Model::Fluid` (`fluid`:
  0D cylinders over a slider-crank, compressible valve flow, and the 1D
  Euler equations in each header and the pipe; costly).
- `es` is a port of engine-sim's synthesizer and filters (MIT), including
  convolution with a recorded exhaust impulse response.
- `tyres` makes rolling roar, gravel crunch and stones from grains, not
  steady noise.
- `Sounds` holds caller-supplied engine presets (TOML, with a `base`
  table merged under each model). Games own their presets; Sideways keeps
  them in its `data/`.
- `stream` runs each car on its own worker thread a few milliseconds ahead;
  the device callback only places and mixes finished blocks, so a slow
  voice drops out briefly instead of stalling the callback.

Try it offline: `cargo run --release -p cli -- sound out.wav --car ID`.
`audio::analysis` writes WAVs and spectra for tests.

## Effects: `audio::sfx`

- Clips: `load_clip` decodes WAV (PCM 8/16/24/32-bit or float), Ogg
  Vorbis and FLAC to mono at `SAMPLE_RATE`.
- `sfx(max_voices, sr)` returns a sender for the game and a mixer for the
  audio callback. The sender plays clips with gain, pitch, equal-power pan
  and a distance low-pass, and runs looping beds whose gain the game moves
  every frame (ambience, crowds).
- `spatial(at, listeners, unit)` places a sound for one or more
  listeners; in split screen each sound is heard from the nearest player.
- `SfxMixer::render_stereo(seconds)` mixes offline. A script `audio` step
  uses this to record what played.

## Recorded samples in a Warp game: `samples.lock`

A Luau game can play real recordings without putting audio files in git.
`samples.lock` (TOML, beside main.luau) lists where each file comes from and
its sha256; `[[sample]]` entries cut sounds out of the files:

```toml
[source.freesound]   # a Hugging Face datasets-server rows endpoint
rows = "https://datasets-server.huggingface.co/rows?dataset=MoamenElSayed/freesound-commercial-50k&config=default&split=train"
id_field = "freesound_id"

[file.sick_zombie]
source = "freesound"
row = 10345          # or url = "https://..." for a direct download
id = 341518          # checked against the row's id_field
sha256 = "..."
title = "Sick Zombie"
author = "Massivecarcrash"
license = "CC0-1.0"

[[sample]]
name = "groan.1"     # groan.1, groan.2 ... are variants of `groan`
file = "sick_zombie"
from = 6.11          # seconds into the file
to = 6.73
db = 0.3             # level to one loudness
# loop = true        # a bed: plays until stopped, end crossfaded into start
```

- `warp run` fetches missing files in the background (with the system's
  `curl`) into the user cache (`WARP_SAMPLES_CACHE` overrides it) and checks
  each hash; `warp samples DIR` does it in the foreground, `--bundle` copies
  them into `DIR/samples/` for a package (gitignored), `--credits` prints
  who made them.
- A group replaces the synth patch of the same name once its file is here:
  `sound.play("groan")` plays a different recording each time (never the
  same twice in a row), else `sounds/groan.luau`. A recording takes only
  `rate` and `gain` (linear, for beds: `sound.set_param(h, "gain", g)`);
  other parameters the game passes are dropped. `sound.recorded(name)` says
  which one plays now, `sound.has(name)` whether anything does.
- Headless runs (`script`, `check`, tests) never fetch; they use what the
  cache already holds, so a test behaves the same either way (a sound never
  touches the game's random numbers).
- Pick sources whose licence allows it: Dead Miles uses only CC0 rows of a
  public Freesound mirror (games/dead-miles/samples.lock).

## The procedural synth: `audio::synth`

Sound as a small graph of units, described as data, prepared once for
repeated voices and placed in any speaker layout. It renders in blocks of
`synth::BLOCK` (64) samples and does not allocate while rendering.

### Units

Every unit reads its inputs and writes one block of mono samples. Inputs
are constants, named patch parameters, or the output of an earlier node.
Missing inputs take the unit's default.

| Unit | Inputs | What it is |
| --- | --- | --- |
| `osc` | freq, pm, width | Band-limited oscillator: sine, triangle, saw, square, random (stepped). |
| `noise` | | White, pink or brown noise. |
| `dust` | density, gain | Random clicks, `density` per second (crackle, rain, gravel). |
| `clip` | rate | A recorded sound (48 kHz mono) at `rate`, optionally looping. |
| `env` | | Envelope through `{time, level, curve}` points from 0, starting with the voice. |
| `adsr` | attack, decay, sustain, release | Envelope on the voice's gate; releases when stopped. |
| `lfo` | rate, depth | Low-frequency modulator. |
| `wander` | rate | Smooth random wander in -1..1. |
| `mul`, `add` | a, b | Arithmetic. |
| `clamp` | x, lo, hi | Clamp. |
| `map` | x, in_lo, in_hi, out_lo, out_hi | Map a range onto another, optionally exponential (frequencies). |
| `filter` | x, cutoff, q | State-variable filter: low, high, band, notch, peak. |
| `lowpass1`, `highpass1` | x, cutoff | One-pole filters. |
| `comb` | x, freq, feedback | Feedback comb (metal, tubes). |
| `delay` | x, time, feedback, mix | Delay line with feedback. |
| `drive` | x, drive | tanh saturation. |
| `fold` | x, amount | Wave folder. |
| `crush` | x, bits, hold | Bit and sample-rate reduction. |
| `modal` | x, freq, damping | Bank of damped resonators (struck metal, wood, stone). |
| `pluck` | x, freq, decay, brightness | Karplus-Strong string. |
| `hold` | x, trig | Sample and hold on a rising trigger. |
| `expr` | up to six named | Per-sample formula of `t`, `sr` and its inputs. |
| `rand` | lo, hi | One random value per voice, from its seed (per-play variation). |
| `engine` | rpm, throttle, load, cut, gear, speed | A car engine (the engine-sim model, Sideways' cars) by car id. |
| `tyres` | speed, slip, contact, loose, soft, wet | Tyres on a stage: roll, slide, gravel, grass, splash. |

`expr` formulas such as `f1 + (f0 - f1) * exp(-t * k)` compile when a patch
is prepared into a shared program that runs a block at a time, with constant parts computed
once (fast exp, ln, tanh and sin keep them cheap). They support `+ - * / ^`, unary minus, `<` and `>`
(1 or 0), and exp, log, sin, cos, tan, tanh, sqrt, abs, sign, floor,
fract, min, max, pow, clamp, lerp, smoothstep and step, so formulas from
procedural sound code in GDScript or C port as they are.

The table above is also in the code as `synth::units::UNITS`, which is
what the docs and the Luau front end name units by.

### Patches as data

A `Patch` is a name, a list of nodes, the output node, parameters with
defaults, an optional `max_length` and a `looping` flag. It derives
`Serialize` and `Deserialize`. `validate()` checks the graph: inputs only
from earlier nodes (no cycles), known parameters and input counts within
the unit's.

Call `compile()` when loading or editing a patch and retain the resulting
`CompiledPatch`. It validates the source, resolves parameter inputs and
defaults, and compiles expression programs. The immutable graph, input
metadata, envelope points and modal definitions are shared across voices.
`CompiledPatch::voice(params, seed, sr)` allocates only the voice's mutable
DSP state, parameter values and reusable block buffers, off the audio
thread. Rendering does not scan the unit registry or recompile formulas.

`Patch::voice` remains available for one-off playback and builds a prepared
snapshot for that call. Use a retained `CompiledPatch` for repeated sounds.
Both paths preserve seeds, parameter overrides and glides, sample data,
voice duration and the rate used for pitch changes.

A struck bell, from the crate's tests:

```rust
use audio::synth::*;

let bell = Patch {
    name: "bell".into(),
    params: vec![("freq".into(), 600.0)],
    nodes: vec![
        Node { unit: Unit::Noise { color: NoiseColor::White }, inputs: vec![] },
        Node { unit: Unit::Env { points: vec![
            Point { time: 0.001, level: 1.0, curve: 0.0 },
            Point { time: 0.01, level: 0.0, curve: 4.0 },
        ] }, inputs: vec![] },
        Node { unit: Unit::Mul, inputs: vec![In::N(0), In::N(1)] },
        Node { unit: Unit::Modal { modes: vec![
            Mode { ratio: 1.0, decay: 0.6, gain: 1.0 },
            Mode { ratio: 2.76, decay: 0.3, gain: 0.5 },
        ] }, inputs: vec![In::N(2), In::P("freq".into())] },
    ],
    out: 3,
    ..Default::default()
};
let bell = bell.compile()?;
let samples = bell.voice(&[("freq", 440.0)], 1, 48_000.0).render_all(2.0);
```

A voice ends when its envelopes are done and it has been quiet for about
0.1 s, at `max_length` (or the value of the `length_param` parameter), or
(when `looping`) after release. Parameters set while playing glide over
the next block. Nodes whose inputs are all constant this block (random
values, parameters, values derived from them) are computed once.

`engine` units find their car in the tables given to
`synth::register_engines(&audio::Sounds)` (Sideways' `data/sounds.luau`,
or a TOML file with the same tables), so a patch can wrap a car's engine with its own layers and filters.

### The mixer

`synth::mixer(layout, max_voices, sr)` returns a `SoundSender` for the
game thread (cheap to clone) and a `Mixer` for the audio callback. The game
builds voices and sends them; the audio thread only mixes, and hands
finished voices back so they are dropped elsewhere.
`SoundSender::play` accepts `Arc<CompiledPatch>` and the original
`Arc<Patch>` API; the latter prepares a snapshot on each call.

```rust
let (tx, mut mix) = synth::mixer(Layout::surround71(), 256, 48_000.0);
tx.ears(vec![Ear { pos: [0.0; 3], forward: [0.0, 0.0, -1.0], right: [1.0, 0.0, 0.0] }]);
tx.room(1.5, 0.4, 0.5);                       // reverb: decay s, damping, level
let bell = std::sync::Arc::new(bell);          // the CompiledPatch above
let h = tx.play(&bell, &[("freq", 300.0)], Play {
    place: Place::At([0.0, 0.0, -68.6]),
    sound_delay: true,                        // arrives after d / 343 m/s
    reverb: 0.2,
    ..Default::default()
});
tx.set(&h, "freq", 320.0);                    // glides over one block
let out = mix.render_seconds(1.0);            // offline, interleaved
```

What it does with each voice:

- **Layouts:** mono, stereo, quad, 5.1 (L R C LFE Ls Rs) and 7.1 (L R C
  LFE Lb Rb Ls Rs), or any ring of up to 16 speakers given by azimuth.
  `Layout::for_channels(n)` picks the usual one for a device.
- **Panning:** pairwise constant-power amplitude panning between the two
  speakers around the source on the horizontal ring (VBAP), blended toward
  all speakers by `spread`. Stereo folds sounds behind onto the front. The
  LFE channel gets a send, not panning.
- **Several listeners:** a sound is placed for the nearest ear; with
  several ears (split screen on one set of speakers) the image widens.
- **Distance:** full gain within `Falloff::reference` (default 4 m), then
  reference / distance, silent beyond `max` (default 400 m).
  `Falloff::godot(unit)` is Godot 4's model: unit / distance, capped at
  +18 dB close in, an 8 kHz high shelf down to -12 dB far away, and
  optionally a linear fade (`fade`).
- **Air absorption:** highs fall with distance (`air`, the fraction of
  20 kHz lost per 100 m), applied as a low-pass.
- **Occlusion:** 0 (clear) to 1 (behind walls) moves a low-pass from 20 kHz
  to 250 Hz in log frequency (gliding, as Godot's raytraced players do) and
  lowers gain by `occlusion_cut`.
- **Pitch:** `Play::pitch` is the playback rate (resampling: duration and
  pitch together).
- **Not positioned:** `Place::Head` (centred), `Place::Pan(p)` (a stereo
  panner on the front pair, as Godot's 2D players).
- **Loudness priority:** with `cull(Some(Cull::default()))`, a sound whose
  `spl` is more than 35 dB below the loudest recent one is not played (the
  window releases at 110 dB/s). `loudest()` reads it, for ducking.
- **Speed of sound:** with `sound_delay`, a voice starts after distance /
  343 m/s. A test checks that a sound 68.6 m away arrives at 0.2 s.
- **Reverb:** a feedback delay network of eight lines (30 to 75 ms) with
  one decorrelated output per speaker, so the tail surrounds the listener
  in any layout. `room(decay, damping, level)` sets it; `Play::reverb` is
  each voice's send.
- **Hall:** a second, long and dark reverb fed before distance
  (`Play::tail`), whose level follows how loud things were recently
  (`Hall`, `lift`): the big tail after explosions.
- **Master:** `master(Master { compressors, ceiling })`; the default is a
  peak limiter at -1 dBFS, `Master::godot()` adds two compressors as
  Growing Guns' Godot bus.

`Mixer::render(out, channels)` fills an interleaved device buffer: a
device with more channels than the layout gets silence on the rest, one
with fewer folds the extra channels into the first ones. A test renders
100 struck-metal voices in 7.1 with reverb and asserts it runs faster than
real time.

### Patches in Luau

With the `luau` feature, `synth::lua::load(name, src)` builds patches from
Luau, where nodes are values and arithmetic builds the graph:

```lua
-- A gunshot kick: a sine whose pitch drops 220 Hz to 55 Hz, per-shot jitter.
local damage = param("damage", 1)
length(0.6)
local kick = sine(sweep(220 * rand(0.88, 1.12), 55, rand(70, 95)))
  * expr("exp(-t * 35 / sqrt(d))", { d = damage })
return lowpass1(kick, 2500) + highpass(noise(), 2000) * decay(0.08, 1.6) * 0.5
```

Every unit is a function (`sine`, `saw`, `noise`, `lowpass`, `modal`,
`engine("asti", rpm, ...)` and so on), plus helpers: `decay(dur, power)`,
`sweep(f0, f1, k)`, `hump(dur)`, `after(delay, x)`, `expdecay(tau)`,
`expr(src, {name = signal})`, `mix(...)`, `db(v)`, `param(name, default)`,
`length(seconds or param)`, `loop()` and `sound(name, fn)` for several
sounds in one file. Each load runs in a fresh sandboxed VM under a step
budget, so a broken edit fails to load instead of hanging the game.
Growing Guns' sounds are such files (`games/growing-guns/sounds`), ported
from Godot's sfx.gd and checked against 1:1 references; the game reloads
them when saved.

The game's `sounds::Bank` compiles each patch during loading and hot reload,
then supplies `Arc<CompiledPatch>` to gameplay and live sound tools. A
failed load keeps the last good version. A successful reload replaces the
snapshot used by future plays; active voices retain their original graph.
Live `set_param` uses `CompiledPatch::with_default` to replace the default
snapshot while sharing its compiled graph. Trigger overrides still win,
and a later file reload restores the file's defaults. Recorded effects are
also prepared once when loaded.

### Growing Guns music

`games/growing-guns/app/src/music.rs` owns the game's recorded soundtrack:
two sets of three synchronized stems, bar-aligned energy changes, muffle
and preview fades, ducking, and per-set filtering and compression. `Music`
can render directly offline. It is a game-owned player because those
stem and round rules are specific to Growing Guns.

For live playback, `shared(music)` creates game-side `Controls`, and
`Player::new(&shared)` transfers the actual `Music` to the audio source
once during setup. Control producers use a short mutex to serialize
ordered commands. The callback owns its rendering state and drains the
channel before rendering; it never acquires that controls mutex. The game
can continue issuing commands while a device buffer is being rendered.

A lightweight control shadow answers energy, round-level and sample-rate
queries immediately, including the return value of `start_round`. `update`
commands carry the original game delta, keeping the existing track clock,
duck decay and fade timing. Commands retain their order through round
changes, rebakes, volume changes and stem loads. Stem decoding and bus
allocation happen on the producer; replaced stem banks return to a
producer for reclamation after the callback relinquishes them.

### warp-audio

`cargo run -p audio --features luau --bin warp-audio --` (also
`warp audio ...` in the [warp executable](warp-cli.md); the command line
is `audio::cli`):

- `render FILE.luau [--sound NAME] [--set k=v] [--seed N] [--seconds S]
  [--layout 5.1] [--at x,y,z] [--reverb R] [--repeat N --every S]
  [--out x.wav] [--json]` renders through the mixer and prints the length,
  peak, loudness, spectral centroid, per-speaker levels and a text
  spectrogram, so a sound can be judged without listening (by a person or
  a model);
- `check FILES` validates patches; `units` lists units and helpers;
- `--engines sounds.luau` (or a .toml) registers cars for `engine` units (by default
  Sideways' table when run from the repository).

`warp-audio render` prepares its selected patch once, including when
`--repeat` starts many voices. For a CPU-only launch/allocation probe:

```powershell
cargo run --release -p audio --features luau --example patch_launch
```

See the [28 October efficiency report](log/engine-efficiency-20261028.md) for
baseline parity and allocation measurements; these are not frame-rate results.
