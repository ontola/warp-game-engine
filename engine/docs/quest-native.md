# Native Quest / OpenXR

Growing Guns' native Android build uses the existing Rust `Gg` application,
Warp wgpu/Vulkan renderer, shared character proxies, procedural gun rigs,
arena, Luau rules and all 52 cards. The shared minimum graphics profile omits stadium decoration. `engine/crates/xr` owns OpenXR and
presentation; `games/growing-guns/app/src/xr.rs` adapts the game's controls and
world origin. There is no browser or streaming dependency at runtime.

This is an experimental local demo, initially one player and three bots.
It is separate from the earlier [simplified WebXR demo](quest-vr.md).
Other games can implement `warp_xr::Game`; existing desktop binaries do not
acquire VR support automatically.

## Build and install

Requires Rust's `aarch64-linux-android` target, `cargo-ndk`, Python 3, Java 17+,
Android SDK platform 35, build-tools 36.0.0 and NDK 28.2.13676358.
`ANDROID_HOME` and `ANDROID_NDK_HOME` override SDK/NDK locations. macOS and Linux
hosts are supported. The build downloads a checksum-pinned Khronos OpenXR
loader (1.1.65) from Maven Central and packages its license.

```sh
python3 engine/tools/build-quest-native.py
# With an authorized Quest attached through USB debugging:
python3 engine/tools/build-quest-native.py --install
# Optional original sound recordings, alongside the built-in synth patches:
python3 engine/tools/build-quest-native.py --audio /path/to/growing-guns/assets/audio --install
adb shell am start -a android.intent.action.MAIN -c com.oculus.intent.category.VR -n com.ontola.warp.growingguns/android.app.NativeActivity
```

Output: `build/quest-native/growing-guns-quest2.apk` plus `build.json` with
source revision, dirty state, loader version and APK SHA-256. The local debug
signing key is in `~/.android/warp-quest-debug.keystore`, never in the repository.
The APK is a development build for sideloading, not a store release.

## Shared graphics profiles

Desktop accepts `--graphics full|low|minimum` or `GG_GRAPHICS`; `full` remains its default.
The native Quest adapter selects `minimum`; slower PCs can select the same profile.
Profiles are fixed at startup; restart to change them.

| Modifier | Full | Low |
| --- | --- | --- |
| Stadium, crowd and TVs | Enabled | Not allocated |
| Sky occlusion volume | Enabled | Disabled |
| Material lighting | Full PBR/procedural | Textured diffuse sun, sky and local lights |
| Sun shadows and procedural relief | Enabled | Disabled |
| Mesh culling | General scene path | Plain mesh fast path (when supported) |
| Block surface | Detailed hewn blocks | Coarse hewn blocks |
| GPU rubble capacity | 4096 | 128 |
| Rubble per impact | Original count | One sixteenth, rounded up |
| Rubble size | Original | 1.6 times larger |
| Effect particle capacity | 768 | 128 |
| Geometry LOD error | 1 pixel | 4 pixels |

Gameplay blocks, collisions, destruction, characters, weapons and cards keep
using the same simulation. These modifiers are shared with slower PCs; they do
not substitute a separate VR scene. GPU rubble is reduced, not disabled.
Emission, lava glow, hit flashes, fog and flashlight illumination remain; low
omits normal maps, procedural surface detail, specular reflections and GI.
The engine exposes these lighting/culling choices through `Scene::graphics`.
Scenes using vegetation, GPU crowd/rigid animation or selection experiments
retain the general culling path; the fast path is for plain meshes and skinning.

The `minimum` profile additionally uses the engine's indexed forward renderer:
coarsest authored mesh LOD, flat material colours and vertex lighting. Blocks
are cubes, characters use the existing primitive proxies, GPU rubble is off,
and effect particles are capped at 32. It skips textures, GI, shadows, bloom,
volumetrics, TAA and procedural surface detail. Original sprite effects, names,
HUD, bullets, growing guns, hit flashes and up to four unshadowed flashlights
remain. The simpler flashlights can leak through walls. Transparent mesh
materials are approximated as solid colour; this is not the full material path.
CPU-owned rigid mesh scenes can opt in with `Scene::graphics.minimal_forward`;
unsupported GPU-deformed/generated scenes retain the general renderer.

## Touch controls

| Control | Action |
| --- | --- |
| Left stick | Head-relative movement; navigate existing menus/cards |
| Right stick left/right | 30-degree snap turn; centre to rearm |
| Right controller aim pose + trigger | Aim and fire the existing growing gun |
| Left trigger | Current special (initially grenade) |
| Right grip | Dash |
| A | Jump / double jump; confirm menu/card |
| X | Reload |
| Y | Confirm menu/card |
| B | Back in menus; current special in play |
| Left menu button | Pause / resume |

The game starts directly in the arena. Pause and card menus use the original
HUD, submitted on a head-relative OpenXR quad two metres away. The fixed
screen-centred crosshair is hidden; aim follows the gun. Online and splitscreen
entries are omitted from the native demo's pause menu because tracked poses
are not sent by the existing network protocol. Locomotion is smooth, not
teleportation; jumps, falling and physical knockback remain gameplay movement.
Headset views have no camera shake, recoil rotation, desktop zoom or TAA jitter.

## Rendering and lifecycle

The runtime supplies both eye poses (including roll/IPD) and asymmetric FOVs
at predicted display time. Warp applies reversed-Z projection overrides to
color rendering, vegetation animation scheduling and shadow receiver demand.
Both eyes hide the local player's body and show the same tracked gun.

The OpenXR `XR_KHR_vulkan_enable` path queries runtime-required extensions,
selects its physical device, creates a compatible wgpu device, and copies Warp's
offscreen result directly into runtime-owned Vulkan swapchain array layers.
Images are transitioned back to `COLOR_ATTACHMENT_OPTIMAL` and a fence completes
before release. There is no CPU pixel readback. The initial implementation
serializes submissions and waits on fences. Stereo colour and HUD now share
one native copy submission and one completion fence.

The minimum path renders each eye at the full resolution recommended by the
OpenXR runtime, without downscaling or reconstruction. Growing Guns requests
120 Hz through `XR_FB_display_refresh_rate` when the runtime exposes it, and
logs supported, requested and actual changes. A display-rate request does not
prove 120 rendered frames per second: the per-frame budget is 8.33 ms, including
presentation. Device throughput and actual sustained headset FPS are separate
acceptance checks.

Tracking loss suppresses rendering until head/eye poses are valid. Loss of
focus clears controls and simulation catch-up; missing right-controller poses
release firing/special input and hide the gun. READY/STOPPING and exit/loss
states are handled through OpenXR. LOCAL reference space also supports Quest stationary boundaries. Recenter resets
the game tracking origin. Android lifecycle polling stays on its main native
thread while OpenXR and pipeline compilation run on an attached render thread.
Android input events are drained even though gameplay input comes from OpenXR,
so unhandled native key events cannot stall Android input dispatch.
Tracked weapon origins retain the simulation's cover clipping and 2.5m bound.
Room-scale movement offsets the viewpoint; it does not move the simulated
player capsule, and leaning through scenery is still a limitation.

## Validation

```sh
cargo check --locked --workspace --all-targets
cargo ndk -t arm64-v8a -P 29 check --locked -p growing-guns-app --lib
cargo test --locked -p warp-xr --lib
cargo test --locked -p growing-guns-app --lib
cargo test --locked -p app --lib
cargo run -p growing-guns-app -- xr-shot --out build/quest-native/stereo.png
```

`xr-shot` uses the native game adapter and real Warp renderer on the host GPU
with synthetic stereo poses. It validates shared drawing, not Android drivers,
OpenXR lifecycle, physical tracking, headset readability or sustained framerate.
The APK tool verifies signature and alignment. Actual Quest launch/render and
controller checks must be recorded separately; neither compilation nor an
installed APK proves those behaviors.

For diagnostics, use `adb logcat --pid=$(adb shell pidof com.ontola.warp.growingguns)`.
Startup errors are tagged `WarpXR`. Preserve logs before restarting the app.

An opt-in device diagnostic runs before the OpenXR session is created:

```sh
adb shell run-as com.ontola.warp.growingguns touch files/growing-guns/diagnose
# Launch the app; captures and pass timings are written from the Quest GPU.
adb exec-out run-as com.ontola.warp.growingguns cat files/growing-guns/diagnostic-lit.png > quest-lit.png
adb shell run-as com.ontola.warp.growingguns rm files/growing-guns/diagnose
```

Diagnostics include lit/albedo/normal stereo frames and independent single-cube
probes on both the OpenXR device and a standard wgpu device. Readbacks and
startup compilation are excluded from gameplay framerate measurements.

## Quest driver workarounds and isolated GPU checks

On the tested Quest 2 Adreno 650 driver, indirect draws produce no geometry.
For plain mesh scenes with simple culling, Android Qualcomm Vulkan automatically
uses conservative direct draw counts; the vertex shader rejects excess slots
against the real GPU-selected count. CPU bounds include all resident LODs and
track instance changes and debris spawns, without a per-frame GPU readback.
`KERN_BOUNDED_DRAWS=0|1` is a constructor-time diagnostic override. Advanced
GPU-generated scenes are outside this workaround's supported scope.

A separate driver issue reports the wrong runtime length for storage arrays
of structs: a 320-byte buffer of four 80-byte records reported two records,
although direct reads of all four and its scalar length were correct.
Culling and skinning now use their known uploaded counts; wind dispatch uses
the matching output array. `cargo run -p render --example storage_bounds`
reproduces the driver observation. This does not certify every optional engine
shader on Android. The full lighting shader also remains unsupported on this
Quest driver; the shared low lighting path is the tested alternative.

`cargo run -p render --example simple_meshes` compares selected cluster IDs
and pixels between general/indirect and simple/bounded paths on a working
desktop driver, including activation, hiding, empty scenes and both eyes.

For actual Quest GPU checks without an OpenXR session, cross-compile the
`growing-guns` CLI with cargo-ndk, place it with `libc++_shared.so` and the APK's
extracted game assets in `/data/local/tmp/warp-probe`, then run there:

```sh
LD_LIBRARY_PATH=. GG_CHARACTERS=assets/characters GG_MODS=mods GG_SFX=sounds \
KERN_PIPELINE_CACHE_DIR=pipeline-cache ./growing-guns xr-shot \
  --frames 300 --fire --size 2048x2048 --out stereo.png
```

The fixture runs one player and three bots with synthetic tracked poses, and
`--fire` holds the trigger. It draws the same 1280x720 HUD as the headset.
It waits for GPU completion every frame by default, matching native submission
serialization; `--queued` is an explicit throughput-only alternative. The timed
interval excludes initial compilation/warmup and final image readback. It
includes no OpenXR copies/compositor, physical input or display pacing;
headset acceptance and sustained headset FPS must still be measured separately.
Do not run this GPU workload alongside the VR app.
