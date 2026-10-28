# Engine efficiency changes, 28 October 2026

Baseline: **`fe656b4`**, before these changes. The work removes repeated
allocation, copying and resource creation in existing engine paths. It keeps
the game's simulation rules, animation arithmetic and sound parameters.
Growing Guns runs at 60 Hz; Sideways' separate physics timing is unchanged.

The timings below are **CPU-only microbenchmarks**. They do not establish a
frame-time, FPS or GPU-time improvement. The integrated release also passed a
real-GPU image parity fixture; further cache invalidation checks are listed
separately below.

## Seven changes

| Finding | Change and preserved behavior |
| --- | --- |
| Repeated audio graph cloning and formula compilation | `Patch::compile()` produces an immutable `CompiledPatch` shared by voices. Input routing, defaults and constant-fold eligibility are prepared once. Growing Guns' bank, recordings and live tools, plus repeated `warp-audio` playback, use prepared patches. Voices retain independent DSP state, seeds, parameter values, pitch rates and lifetimes. Hot reload affects future voices; active voices keep their snapshot. |
| Recreated render bindings and texture views | Bounded caches retain views and bind groups for frame shading, TAA, tone mapping, volume, depth reduction, meadow and presentation paths. Keys compare actual resource/layout identities and buffer ranges, so same-size replacement does not reuse stale bindings. Two-entry history caches accommodate alternating targets. Cache counters expose hits, misses and retained entries. |
| Quadratic ray duplicate checks | `BoxGridScratch` uses generation-stamped visited IDs instead of searching a growing tested vector. DDA traversal, early exits and equal-distance index tie-breaking remain unchanged. Scratch can be supplied explicitly or leased from a thread-local pool. |
| Collision query and movement allocations | Queries deduplicate candidates before sorting unique indices. `Boxes::near` reuses scratch, and `Mover::slide` processes candidates directly without copying boxes to another vector. Each contact pass keeps its original query bounds and increasing index order. The compatibility `query()` method still returns an owned vector. Co-op spawn checks use the reusable callback path. |
| Animation palettes allocated and uploaded per actor | `Character::sample_into` and `blend_palette_into` write retained palettes using the same interpolation arithmetic. Each actor retains idle/jump and run buffers; the renderer retains packed matrices. Growing Guns uploads the complete actor palette set in one queue write after validating the batch. |
| Music callback holds the game's mutex | `Player` owns rendering state and drains ordered commands. Game-side controls retain synchronous query state; the callback never locks that controls object. Commands carry the original game delta and preserve round, bar, fade and duck ordering. Stem buses are allocated on the producer, and replaced banks are returned for producer-side reclamation. |
| Snapshots copied into an unbounded network backlog | A host mailbox replaces consecutive snapshots at enqueue time while retaining reliable-message barriers. It holds at most 64 queued messages; overflow disconnects the slow peer and cancels a blocked writer. Growing Guns shares one immutable snapshot across peers, keeps per-peer acknowledgements, and skips snapshot construction with no peers. Writers retain serialization buffers. The wire schema remains unchanged. |

Implementation entry points: [audio](audio.md),
[`resource_cache.rs`](../crates/render/src/resource_cache.rs),
[`collide.rs`](../crates/kern/src/collide.rs),
[`skinning.rs`](../crates/render/src/skinning.rs),
[`music.rs`](../../games/growing-guns/app/src/music.rs), and
[`pending.rs`](../crates/net/src/pending.rs).

## Measured audio evidence

The old release audio library from `fe656b4` and the changed implementation
were linked into standalone CPU probes with the same release dependencies.
The probes opened no sound device or GPU. All **174 sample hashes and voice
stopping lengths matched exactly**: 28 named game patches plus a synthetic
clip/envelope/modal fixture, each at 24/48/96 kHz and two seeds. Cases include
parameter overrides, a later parameter glide and release, for up to 900
64-sample blocks. The different rates also exercise the voice-rate mechanism
used for pitch changes.

The gunshot launch probe uses the game's 46-node patch, 4,096 launches at
48 kHz, and a counting allocator. It measures voice construction and disposal;
it excludes loading, device output and mixer work.

| Per voice | Baseline `Patch::voice` | Retained `CompiledPatch::voice` |
| --- | ---: | ---: |
| Allocation/reallocation calls | 543 | 25 |
| Allocated bytes | 66,620 | 30,112 |
| Observed CPU time across local runs | 33.2–37.5 µs | 1.78–1.89 µs |

That is 95.4% fewer allocation calls and 54.8% fewer allocated bytes for this
fixture. Rendering the prepared voice incurred **zero allocations across
4,096 blocks**. Mutable DSP buffers and expression scratch are still allocated
when a voice starts. The compatibility `Patch::voice` prepares a fresh snapshot
per call; repeated runtime playback must retain the compiled result.

Reproduce the launch/allocation probe with:

```powershell
cargo run --release -p audio --features luau --example patch_launch
```

Local evidence is under `C:/dev/warp-bench/engine-efficiency-20261028/`:
`audio-results.txt`, baseline/prepared hash files, probe sources and executables.
These artifacts preserve the actual baseline comparison independently of later
builds. The numbers must not be added to unrelated CPU or GPU savings.

## Validation status

Completed focused checks:

- Audio library: 38 tests passed, one existing manual benchmark ignored.
  Coverage includes prepared defaults, active-voice independence, samples,
  seeds, parameter glides, rates, release and lifetime.
- Current `music.rs` in a standalone harness: 10 tests passed, one WAV export
  test ignored. Queued and direct playback match bitwise, including stem and
  rate changes; rendering completes while the game controls mutex is held.
- Core test log: 9 collision tests passed, one manual benchmark ignored;
  43 Growing Guns simulation tests and 6 networking tests passed. Collision
  checks compare the old algorithms after removal/restoration, at boundaries
  and ties, through nested callbacks and scratch reuse. Network checks cover
  ordered coalescing, bounded overflow, cancellation and loopback transport.

Additional integrated checks on the optimized `fe656b4` worktree:

- `cargo check --locked --workspace`: passed.
- `cargo test --locked -p kern -p audio -p net -p assets -p growing-guns --lib --features audio/luau`:
  101 passed, two manual benchmarks ignored. The five asset tests include
  retained character palettes with exact interpolation parity.
- `cargo test --locked -p render --lib`: 568 passed, two ignored. This includes
  palette/cache tests, shader validation and existing GPU HUD/sprite checks.
- `cargo build --locked --release -p growing-guns-app`: passed.
- `after.exe sim --seconds 120 --seed 3`: the baseline's exact digest
  `fa7054c72526d7f5`, six rounds, 509 shots, 96 hits, scores `[2,1,0,3]`.
- `after.exe nettest --seconds 4`: real QUIC loopback passed; all 74 destroyed
  chunks, arena replacement, card change and player input reached the client.
- `ion-transparency.json` ran before and after on the RTX 5070 Ti, same seed,
  one view at 960x540. All eight decoded RGBA captures are **byte-identical**,
  including charge, impact and aftermath; all script expectations passed.
  Images and `image-parity.json` are in the evidence directory.
- The optimized release started a Windows playtest with two music sets,
  29 recordings and 28 prepared patches. Window response and startup logs
  were verified; subjective playtest feedback is separate evidence.

Collision release microbenchmark: three alternating old/new rounds, synthetic
8,001-box scene. Medians: 10,000 queries 45.0453 -> 13.571 ms; 4,000 rays
150.6561 -> 53.5876 ms; 4,000 moves 18.2269 -> 6.9105 ms. Reproduce using
`cargo test --release -p kern collision_scratch_benchmark -- --ignored --nocapture`.
These numbers do not represent an entire game tick.

- Rebased onto `c03ccfe`, preserving incoming HUD, menu, controller rumble
  and indoor irradiance-volume changes. Post-rebase workspace check passed.
- The final Growing Guns app suite passed **27 tests**, one manual WAV test
  ignored. This includes shared snapshot wire-format and music callback tests.
  A pre-existing generated-doc test needed CRLF normalization on Windows;
  the comparison still checks every generated character apart from line endings.
- `cargo check --locked -p render --example resource_cache_probe`: passed.

The dedicated `resource_cache_probe` exercises steady frames, same-sized replacements,
presentation, resize, TAA histories and split views. Its GPU run is deferred
while the user is playtesting; do not run it concurrently with the game.

Any later frame-time comparison must identify its executable,
scene, player count, resolution, reconstruction settings and sample count, and
exclude startup, warmup and readback. A successful build alone does not verify
presentation.
