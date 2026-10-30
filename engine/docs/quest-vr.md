# Standalone Quest VR demo

Warp has an experimental **WebXR shell** in `engine/webxr` and a standalone
Growing Guns adapter in `games/growing-guns/quest`. The existing Rust simulation
compiles to WebAssembly and runs inside Quest Browser. The headset renders both
eyes using a lightweight Three.js/WebGL2 renderer. A computer or HTTPS host only
serves the initial static files: it does not simulate the game or stream video.

This is a mobile presentation backend, not an Android APK or an OpenXR/Vulkan
port of the desktop wgpu renderer. Other Warp games need an explicit adapter;
existing desktop binaries do not automatically gain VR support.

## Play on Quest 2

1. Open the demo's **HTTPS** URL in Quest Browser. HTTP on a LAN IP cannot
   request an immersive session. Use the temporary HTTPS serving command below,
   or upload the contents of `build/quest-demo` to your own HTTPS static host.
2. Wait for the game to load, then select **Start VR op je headset**.
3. The in-world menu starts paused. Use the right trigger to begin.
4. Once the start page reports **Offline bestanden opgeslagen**, the static
   files are cached. The game can continue and reload on that same URL offline.
   The browser may evict its storage; this is not a guaranteed permanent install.

Controls (Touch controllers, standard layout):

| Control | Action |
| --- | --- |
| Left stick | Head-relative movement, about 3.5 m/s before card/effect changes |
| Right stick | 30-degree snap turn; return the stick to centre to turn again |
| Right controller + trigger | Independent weapon aim and fire |
| Right grip or X | Reload |
| A | Jump / double jump |
| Left trigger | Grenade |
| B | Pause / resume |
| Y | Put the visible menu back in front of you |
| Left stick in a menu | Select card or menu row |
| Right trigger in a menu | Confirm |

The pause menu also restarts a match and exits VR. The Meta system button still
opens the headset's own menu. Loss of focus or viewer tracking clears held inputs
and stops simulation catch-up. Missing controller tracking releases that hand's
buttons. There is no camera shake, artificial camera recoil or smooth turning.
Jumping, falling, explosions and weapon knockback still move the simulated body.
The demo uses smooth stick locomotion, not teleport locomotion.

On a computer, **Bekijk de demo** uses WASD/mouse, left click, R, Space and right
click. Escape pauses. The phone page is a responsive launcher/instructions view;
a phone is not used as the VR controller and has no touch gameplay mode.

## Build and serve

Build scripts support macOS and Linux. Requires Rust/rustup, Python 3, Node/npm. The build pins wasm-bindgen CLI to the
workspace's binding version and installs it under `build/wasm-tools`, leaving a
user's global CLI alone. npm dependencies are locked, and no CDN is required at
runtime. The emulator is a dev dependency and is excluded from the demo bundle.

```sh
python3 engine/tools/build-quest-demo.py
python3 engine/tools/serve-quest-demo.py
# For a headset-accessible temporary HTTPS URL (requires cloudflared):
python3 engine/tools/serve-quest-demo.py --public
```

Output: `build/quest-demo/` and `build/growing-guns-quest-webxr.zip`.
The serving tool binds to loopback and serves only generated demo files.
`--public` creates a public temporary URL through Cloudflare; anyone with that
URL can load the demo. Stop the serving process to close the tunnel. A cached
copy continues to run without it. It does not publish or modify GameNight.

For USB development, `adb reverse tcp:8787 tcp:8787` makes
`http://localhost:8787` available to Quest Browser as a secure localhost context.
This requires Developer Mode, USB debugging and the headset's authorization.
The public HTTPS route does not require Developer Mode or USB.

## What is included

One local player and three bots, the real procedural destructible arena,
collision, projectile combat, grenades, rounds, death/spectating, rematch, simple
sound/haptics and eight data-only upgrade cards. Matches play to three wins.
Damage is reduced to 40% for the demo and random round modifiers and pickups are
disabled. Characters are simple coloured robots. The arena geometry follows the
live collision chunks and the stadium's solid pieces, with instanced drawing.

The eight cards are Heavy Hitter, Healthy, Rapid Fire, Big Mag, Quick Reload,
Ricochet, Extra Barrel and Lifesteal. Their effects use the simulation's existing
weapon fields. The desktop Luau host and its complete 52-card deck, full materials,
animated character assets, crowd, TV screens, ragdolls, music, multiplayer,
GameNight integration and the other games are not part of this demo.

## Engine boundaries

- `engine/webxr/host.mjs`: immersive-session lifecycle, runtime-provided stereo
  projections/poses, local-floor reference space, input by handedness and tracking
  availability. Three.js handles the XR render target. Quest is requested at 72 Hz
  when supported, framebuffer scale 0.8, maximum available fixed foveation.
- `engine/webxr/input.mjs`: fixed 60 Hz stepping, bounded catch-up, persistent press
  edges and snap-turn hysteresis. Games supply their own scene and sim callback.
- `games/growing-guns/quest/src/lib.rs`: fixed-tick adapter and small presentation
  snapshots. Physical horizontal movement uses the existing collision capsule.
  A tracking-origin reset rebases the camera instead of teleporting the body.
- `World::step_tracked`: optional per-tick muzzle offsets/directions separate from
  the existing Input and Snapshot wire formats. The rifle and grenade use them;
  ordinary `World::step` clears them and preserves the desktop path. Muzzles are
  clipped against cover and nonfinite/out-of-reach poses are ignored. This is a
  local-demo API, not a network protocol for untrusted client weapon positions.

Transforms are in metres with -Z forward. The XR view follows physical head
rotation independently of weapon aim. Presentation interpolates simulation
positions; headset poses remain runtime-predicted and are not interpolated at
60 Hz. Vertical head movement affects the view; the collision capsule and hit
volumes remain the game's standing body. This is not crouching/dodging parity.

## Validation

```sh
cargo test --locked -p growing-guns -p growing-guns-quest
npm test --prefix engine/webxr
# Build and serve first, then (Chrome installed):
npm run test:browser --prefix engine/webxr
npm run test:cards --prefix engine/webxr
python3 website/build.py
```

Browser validation uses Meta's IWER Quest 2 emulator with two eyes. It exercises
startup, real simulation, shooting, movement, snap-turn latching, pause, controller loss, headset-menu visibility, VR exit/re-entry, offline
reload and the responsive launch page. The longer card check lets bots eliminate the
human and selects an actual upgrade through the in-world UI; evidence is in `build/quest-evidence`.
Rust tests cover controller-directed firing, returning to desktop aiming,
unchanged untracked stepping, geometry, cards and rematch.

**Physical Quest 2 testing remains required.** Emulated stereo screenshots and a
successful Wasm build do not establish headset framerate, motion comfort, real
controller mapping, haptics or on-headset offline launch. No physical headset was
connected during implementation. The 72 Hz setting is a request, not a measured
performance claim.

References: [Meta WebXR overview](https://developers.meta.com/vr/documentation/web/webxr-overview/),
[Meta WebXR workflow and emulator](https://developers.meta.com/vr/documentation/web/webxr-workflow/),
[wasm-bindgen web target](https://rustwasm.github.io/docs/wasm-bindgen/print.html).
