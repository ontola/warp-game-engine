# SpaceRacer on Warp

Playable **native development port**, using Rust/Warp and hot-reloaded Luau.
This is not yet a content-equivalent replacement for the original game or a
valid equal-content performance comparison. Source provenance remains pinned
in `port-status.json`; original assets and their attribution are retained.
There is no Godot runtime or Godot build/export step.

## Play and capture

From the workspace root:

```sh
cargo run --locked -p spaceracer-app --bin spaceracer -- --players 2 --fullscreen
cargo run --locked -p spaceracer-app --bin spaceracer -- --demo --seed 31 --players 2 --ticks 750 --shot build/spaceracer.png
cargo run --locked -p spaceracer-app --bin spaceracer -- --script games/spaceracer/scenarios/native-race.json --out build/spaceracer-flow
```

Windows packaging (builds only, does not launch or stop another game):

```powershell
./games/spaceracer/tools/build-native.ps1
./build/spaceracer-native/spaceracer.exe --players 2 --fullscreen
```

The packaged executable resolves its adjacent `spaceracer/rules` and
`spaceracer/assets`. Development runs fall back to the source directory;
`--game-dir PATH` overrides both. Seeds are 1–99999. Worlds: `city`, `forest`,
`cell`, `desert`, `ocean`; difficulty: `easy`, `normal`, `hard`.
The existing GameNight catalog/release is deliberately not switched to this
incomplete port.

Controller: RT throttle, LT brake/slide, left X steer/yaw, right X strafe/roll,
either stick Y grip/nose pitch, B boost, X fire, Y reset, LB/RB side bump.
Start pauses. Menu: up/down select, left/right adjust, Start plays/resumes.
A selects an action. Changing settings waits until Play to regenerate the world.
Controllers take the local seats; keyboard takes seat one when no pad is present.
Keyboard: W/S throttle/brake, A/D steer, arrows up/down trim, Space boost,
R fire, F reset, Q/E bump, Enter start/pause. F10/F11/F12 are engine debug,
fullscreen and feedback capture. Touch input is not ported.

## Luau / native boundary

Edit [`rules/game.luau`](rules/game.luau). No rebuild for:

- `track`: six recipe families, road width, base radius/stretch, elevation and
  corner ranges, loop radii/positions, tunnels, jumps, sections, boost strips,
  exposed edges. Spans are normalized positions around the lap (0–1); physical
  dimensions are metres. `road_width` is base **half-width**, wave amplitude,
  and additional easy-mode half-width.
- `ai` and `plan`: steering/braking/boost decisions, racing line and pace changes;
  deterministic random samples and geometric observations come from Rust.
- `pickup` and `activate`: rank-weighted items and weapon activation policies,
  including airborne-only glide bombs and no warp pickup for the leader.
- `balance`: cooldowns, regeneration, weapon damage/duration, pickup density.
- `menu`: navigation and settings changes.
- `environment`: biome density, dimensions, ground relief, lighting/exposure.

Rust keeps 120 Hz handling/flight, track tessellation and projection, swept
collision queries, targeting geometry, projectile integration, rendering and
controller/host transport. Race bookkeeping still lives in `sim/src/race.rs`;
it is not claimed to be an all-Luau game. Geometry-specific component formulas
remain native. Luau has a 16 MiB memory limit and execution budget; design
parameters are validated before allocation/generation.

Rules poll twice a second. AI, item and menu edits apply live; validated balance
values refresh on reload. Existing timers retain their elapsed state. Track,
scenery and pickup placement edits apply to the **next race**. A syntax error
keeps the previous valid module and logs the failure. A runtime/schema error
pauses simulation with a diagnostic until a valid edit is loaded. A script
edit intentionally changes replay behavior: save the exact rules alongside
any replay/seed used for comparisons.

## Implemented and remaining

The native build connects seeded roads, grounded handling, smooth flight and
landing, physical checkpoints, boost/recharge, side bumps, six racers, automated
recovery, ranks/laps, AI finishing, basic weapon gameplay, mounted items, target
indicators, menus, 1–4 viewports and controller input. Ship control surfaces
animate for humans and AI. City facades/posters and original forest meshes are
retained; exhaust lights illuminate road and ships, with shared Warp explosion
and additive effects. Scenery collision uses swept bounds and ground heights.

This is a **first visual integration**, not visual parity. City traffic,
full biome set dressing, detailed water/vegetation shading, the visor/distortion
stack, wear/debris/respawn presentation, victory camera, full weapon models and
original audio still need work. Five authored ship variants are retained but
this build displays the first design. Forest collision uses trunk proxies;
cell/rock proxies are approximate. Vehicle contact is simplified rather than
the original complete rotated hull solver. Weapon tuning and AI behavior need
controller playtesting. No Android or Steam Deck build/hardware claim is made.

GameNight uses the shared native client: prepare/ready/start/pause/resume/dispose,
seat tokens, names, host overlay, difficulty/world settings, new-race action and
finished notification. Ready is sent after scene upload/draw, on the next tick.
Roster name updates apply live. **Face decals/profile artwork, complete settings
parity and an installed-daemon end-to-end playtest remain outstanding.**

## Verification

```sh
cargo test --locked -p spaceracer --lib
cargo test --locked -p spaceracer-app --bin spaceracer
cargo test --locked -p app --lib
cargo check --locked --workspace --all-targets
```

Retained fixtures check 4,800 ground ticks, 1,800 flight ticks, 700 drafting
and 1,680 bump ticks, 18 full ribbons, 90 centreline cases, 1,800 camera ticks
and five landing cases. These run directly against committed data; the retired
Godot extraction tools are not part of the workflow. All original asset bytes
remain unchanged (`assets/manifest.json`).

New tests cover deterministic full-race replay, three AI race completions,
EMP/recovery, swept obstacle contact, script reload/validation, changing road
width without rebuilding, analog bindings and a real loopback GameNight
WebSocket with sparse seat IDs and pause/resume/dispose. The committed headless
scenario checks two independent controllers, partial throttle, right stick,
paused time, EMP owner exemption and screenshots. This does not prove real
controller feel or production-host compatibility. Captures were reviewed at
1280×720 (one/two views) and 1920×1080 (four views), native rendering. All five
biomes render from the portable package. No steady-frame FPS/4K performance
result is claimed.
