# Native Quest / OpenXR

Growing Guns' native Android build uses the existing Rust `Gg` application,
Warp wgpu/Vulkan renderer, shared character proxies, procedural gun rigs,
arena, Luau rules and all 49 cards. The shared minimum graphics profile omits stadium decoration. `engine/crates/xr` owns OpenXR and
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
are cubes with readable 3% joints, characters use the existing primitive proxies, GPU rubble is off,
and effect particles are capped at 32. It skips general material textures, GI, shadows, bloom,
volumetrics, TAA and expensive procedural surface detail. Lava is the exception:
a seamless 512×512 colour field baked once on the CPU (about 1.33 MiB including mipmaps)
provides dark crust and slowly flowing orange/yellow cracks with one filtered
texture read in the existing opaque pass. Solid meshes draw first so hidden
lava can fail the depth test. Distant cracks fade to an average colour to reduce
shimmer. This uses the generic `LAVA` material flag on PC and Quest. Gameplay billows, explosion embers, ion columns and streaks render in place
with exposure conversion; heat refraction is omitted. Original sprite effects, names,
HUD, bullets, growing guns, hit flashes and up to four unshadowed flashlights
remain. VR muzzle flashes spawn from the current controller pose, including
the first shot after an arena rebuild, instead of a cached previous-frame
barrel position. The simpler flashlights can leak through walls. Transparent mesh
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
screen-centred crosshair is hidden; aim follows the gun. The pause menu has
HOST LAN GAME and JOIN LAN GAME: the host announces itself on the local network
(`net::lan`) and JOIN takes the newest host heard, so nobody types a ticket.
Head and gun poses travel in the network input. Splitscreen stays omitted. Locomotion is smooth, not
teleportation; jumps, falling and physical knockback remain gameplay movement.
Headset views have no camera shake, recoil rotation, desktop zoom or TAA jitter.

## Rendering and lifecycle

The runtime supplies both eye poses (including roll/IPD) and asymmetric FOVs
at predicted display time. Warp applies reversed-Z projection overrides to
color rendering, vegetation animation scheduling and shadow receiver demand.
Both eyes hide the local player's body and show the same tracked gun.

The OpenXR `XR_KHR_vulkan_enable` path queries runtime-required extensions,
selects its physical device and imports runtime-owned eye/HUD images as
non-owning wgpu colour attachments. The minimum renderer draws into each eye
directly, with sRGB output conversion. There is no intermediate stereo copy or
CPU pixel readback. GPU completion is awaited before releasing images in
`COLOR_ATTACHMENT_OPTIMAL`; wrapper lifetimes end before the OpenXR swapchains.
Full-field tints (round start, ion/explosion flash, death/ghost/phoenix) use a
one-pixel alpha projection layers over both complete eye frusta. Ghost/death
tints remain behind the HUD so cards stay readable; round/phoenix/explosion
flashes cover the HUD too. Only active tint layers are acquired and drawn.
Menus and text remain on the comfortable head-relative quad.

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
Desktop replay recording remains available on PC. The native adapter skips
recording because the replay format does not include tracked head/hand poses.

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

On 30 October 2026 the physical Quest 2 was authorized over USB, installed and
played. The user confirmed the arena and weapon were visible. Profiling located
25–44 ms in the old raw Vulkan copy stage. Removing that stage produced many
119–120 fps windows at the runtime's full 1440×1584 resolution per eye, with
short dips; the user confirmed markedly smoother play. This measurement
preceded restored gameplay FX and is not an effects-heavy worst-case guarantee.
The effects-on APK is installed. Final in-headset effects/flash visibility and
sustained FPS await an active playtest; the headset was asleep at the end of
this iteration. Synthetic GPU fixtures separately check direct sRGB eye parity,
per-eye hiding, explosion/ion visibility and expiry in ordinary and sRGB targets.
Workspace/all-target checking, 83 Growing Guns app tests (two ignored), three
XR tests, two minimum GPU tests, three sprite tests and the HDR ion-volume GPU
fixture passed. The volume fixture verifies interior views, opaque occlusion,
axial rays, edge continuity and separate view projections.

The cage mesh's generic coarsest LOD had collapsed 756 source triangles to 22,
erasing its thin bars because a 90 m chain shared the bounding box. Cages now
retain their small authored geometry on PC and VR; source bounds and triangle
retention have a regression test and a GPU capture. Minimum-profile cube joints
are widened from 0.8% to 3% without added geometry; distant unfiltered seams
can still alias. Floor surface height and simulation collision stay unchanged.
Audio initialization waits for headset focus, avoiding an audio-start timeout
aborting VR when launched with the headset asleep.

On 31 October 2026, the minimum lava fixture passed on both Metal and the
physical Quest 2 Adreno 650: dark crust/hot cracks, animation, and ordinary
UNORM/direct sRGB parity within two channel values. At 1440×1584 per eye,
120 serial stereo frames after 20 warmups measured the following CPU wall times
including GPU completion (not GPU timestamps, no OpenXR compositor):

| Fixture | Median | p95 |
| --- | --- | --- |
| Flat emissive lava | 4.349 ms | 4.799 ms |
| Animated lava | 4.666 ms | 5.431 ms |
| Flat lava hidden behind an opaque floor | 3.221 ms | 3.303 ms |
| Animated lava hidden behind an opaque floor | 3.554 ms | 3.778 ms |

The exposed-lava scene fills almost the entire view. A preceding run measured
3.544/5.168 ms for flat/animated lava; sequential standalone runs are affected
by device clocks and are not a precise incremental gameplay cost. These checks
do not establish 120 fps with active VR gameplay. Reproduce with
`cargo run -p render --example minimum_lava -- OUT --bench`; add `--flat` for
the baseline or `--covered` for opaque occlusion. Android builds use cargo-ndk
and link the NDK's `libc++_shared.so`. The current-pose muzzle regression and
97 active Growing Guns app tests pass (two ignored); the native stereo firing
capture shows the flash at the tracked barrel. Physical headset visibility still needs the player's confirmation.

A subsequent active headset run of `6717994` on 31 October (09:53:00–09:54:04
Europe/Amsterdam, after startup) recorded 64 one-second VrApi samples at
1440×1584 per eye and 120 Hz. Mean reported application FPS was 107.1, median
118; 37 samples were at least 115 and 13 were below 90. The lowest sample was
46 FPS. The highest reported sample was 121, an interval-counting artifact
on a 120 Hz display. Controller tracking and focus were active; the filtered
WarpXR/AndroidRuntime capture contained no errors. This confirms rendering
during play, but not stable 120 FPS or the visibility of each effect. There
was no matched pre-change gameplay run, so these dips cannot be attributed
to lava from this capture. Local evidence is in
`build/quest-native/lava-live-20261031.{log,json}`.

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

For the subsequent per-pass investigation, controlled lava comparison and HUD
cache diagnosis, see [Quest cost investigation, 31 October](quest-profile-2026-10-31.md).

### November 2026 optimization work

Low/minimum opt into `HudDrawList.cached_text`: bounded label bitmaps at fixed
raster sizes are scaled as quads, avoiding alphabet warmup for every animated
font size. Unsupported coloured glyphs and custom strokes keep the glyph path.
The label cache holds at most 256 labels / 16 MiB of source pixels.

Explosion admission is independent of damage: nearby events within 3 metres
are merged before uploading instances; a per-tick and half-second budget selects
full or compact bursts. Minimum keeps at most 12 decorative puffs, one full
burst per tick and four per half-second. Ion warnings are never evicted by
that decorative budget. Minimum billows use bounded six-vertex analytic blobs;
a projected coverage budget prioritizes fire over smoke. Ion columns now draw
a conservative bounding quad rather than a full-screen triangle.

Controlled Quest probes before integration: animated label preparation median
0.023 ms CPU (previous 4.793 ms); almost-full-view lava paired incremental GPU
cost 0.232 ms (previous 2.671 ms). These isolate individual costs and do not
prove sustained 120 fps in live gameplay.

Bullet tracers now use a short, subdued textured wake plus one round glow.
The generic `sprites::Facing::Screen` aligns the glow to each eye's actual
view plane; minimum pixel width scales both axes, retaining a circular core
at distance and when looking up or down. This adds one six-vertex quad per
tracer, reuses the muzzle-flash image, and needs no blur/bloom pass. The
stereo GPU regression covers direct sRGB eye attachments and distant end-on
views at pitches -0.7, 0 and +0.7 radians.

After integrating main through `2256242`, 736 renderer tests and 103 game
app tests passed (17/2 intentionally ignored). The signed Android APK builds.
The final integrated APK was installed and launched on Quest. The updated
512-request fixture costs 2.273 ms effects GPU time outside the blasts and
6.660 ms inside (both eyes); an active playtest is separate from these
synthetic results. [Optimization results and remaining limits](quest-optimization-2026-11-01.md).
