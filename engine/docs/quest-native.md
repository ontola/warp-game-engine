# Native Quest / OpenXR

Growing Guns' native Android build uses the existing Rust `Gg` application,
Warp wgpu/Vulkan renderer, original character assets, procedural gun rigs,
arena, Luau rules and all 52 cards. The shared low graphics profile omits stadium decoration. `engine/crates/xr` owns OpenXR and
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

Desktop accepts `--graphics low` or `GG_GRAPHICS=low`; `full` remains its default.
The native Quest adapter selects the same `low` profile before creating the scene.
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
serializes submissions and waits on fences; reducing that cost is future work.

Android uses 512px material arrays, a 2048px main shadow map, no near shadow
pass, and at most 1024x1024 rendered pixels per eye. Models and game code are
shared with desktop. These are initial memory/performance settings, not a
measured guarantee of 72 Hz on Quest 2.

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
  --diagnose --frames 60 --size 512x512 --out stereo.png
```

The fixture runs one player and three bots with synthetic tracked poses. Its
frame-throughput interval excludes initial compilation/warmup and final image
readback. It includes no OpenXR compositor, physical input or display pacing;
headset acceptance and sustained headset FPS must still be measured separately.
Do not run this GPU workload alongside the VR app.
