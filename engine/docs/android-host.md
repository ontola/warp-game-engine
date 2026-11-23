# Native Android host

The Splitscreen Club host on an Android phone, without a browser: the Game
Room and its games in a NativeActivity, drawing with Vulkan through wgpu.
It replaces the browser host in the app's WebView (where Android said
"Splitscreen Club isn't responding" while the page was busy, and the phone
ran slowly) and in Chrome. Cast or mirror the phone to a TV; controllers
paired with the phone play, phones join the cloud room with the QR code.

It is the browser host itself ([browser.md](browser.md),
`lobbies/game-room/examples/browser`), compiled for Android:

- `lobbies/game-room/android` is a `cdylib` crate (`libsplitscreen_host.so`)
  that includes `examples/browser/main.rs` and calls its `page::main` from
  `android_main`.
- On Android, `app::web` is `app::android` (`engine/crates/app/src/android.rs`):
  the same API as the browser shell (`run`, `switch`, `switch_keeping`,
  `resume`, `on_exit`, `on_devices`, `fetch`, `request`, `spawn`, `log`,
  `progress`, `stored`), so the host's code is shared line for line. The
  page's HTML hints (join with A, Back to the room, the empty room) are a
  line over the picture there (`app::android::banner`).
- `warp::web` (Luau games from fetched folders), the vfs (files handed over
  first, then the filesystem) and the built-in Rust games' `web::load`
  (Spellstorm, Space Racer) compile for Android too.

## Threads and lifecycle

The activity's thread only pumps Android's events: it hands window changes,
touches and the phone's Back to the render thread and writes pads into
`app::android_pads` at once, so Android never waits on a frame. The render
thread owns the GPU, the game on screen, the host's state and its tasks: a
busy-polled executor (`app::android::spawn`) runs the host's futures between
frames; HTTPS (`request`, ureq with rustls) runs on helper threads.

The surface follows the window: it goes when the app goes to the
background (the GPU, the room and a paused game stay), and comes back with
the window. Before the first frame a loading screen shows the step.

## Input

Pads come from the activity's input events: buttons as key events
(BUTTON_A..., BACK as Select), sticks Z/RZ, triggers LTRIGGER/RTRIGGER or
BRAKE/GAS, d-pad as HAT_X/HAT_Y or d-pad keys. Android uses this order for
every pad it knows (the 8BitDo Ultimate too). A pad is `Device::Pad(id)`
with Android's device id; one Android no longer lists
(`InputDevice.getDeviceIds`, every 2 s) is gone. Pad buttons are consumed,
so B never becomes Android's Back. The phone's own Back: in a game, back
to the room; in the room, twice within 3 s stops hosting. Touches reach the
room as the pointer (its mouse menu).

## Files

What the browser fetches next to its page comes from the APK's
`assets/web/` (`app::android::fetch`): `games/library.json`, each game's
`files.json` and files, covers and `room-light.bin`.
`build-web.py --warp --assets-only DIR` writes that bundle. `localStorage`
is `files/store/` in the app's data. The page's query string is the
launching intent's `query` extra (`cloud=off`, `players=2`, `quality=full`,
`game=neon-siege`, `render=720`); the cloud room is
`https://splitscreen.club` unless `cloud=` says otherwise.

## Build

```sh
# Rust aarch64-linux-android, cargo-ndk, SDK platform 35 + build-tools 36.0.0, NDK, Java 17+
python3 engine/tools/build-android-host.py            # build/android-host/splitscreen-host.apk
python3 engine/tools/build-android-host.py --install  # and adb install
adb shell am start -n club.splitscreen.host/android.app.NativeActivity --es query "cloud=off&players=1"
adb logcat -s SplitscreenHost
```

The APK is `club.splitscreen.host` ("Splitscreen Club Host"), signed with a
local debug key unless `--keystore` is given. The Splitscreen Club app's
"Host on this phone" starts it when it is installed (gamepad-party
`mobile/android/.../HostLauncher.kt`), and otherwise falls back to the
browser host in Chrome. `--lib-only` stops after the library and the
bundle, for packing them into the app itself later.

## Renderer on phones

Measured on a Galaxy Tab S9+ (Adreno 740) and a Xiaomi 13 class phone
(Adreno 830), 2026-11-23:

- The full `shade` pass cost 67 ms a frame on the 740 at 378 lines; the
  simple one (`KERN_SIMPLE_LIGHTING`) 9.5 ms at 1080 lines. The host turns
  it on, and the 3D view starts at 1080 lines (adaptive quality lowers it).
- The 830's driver drops indirect draws: only emissive and transparent
  things showed. The host draws with bounded direct draws
  (`KERN_SIMPLE_CULL`, `KERN_BOUNDED_DRAWS`).
- Texture arrays use the desktop sizes (render feature `phone`); the 512 px
  headset layers cut off and stretched the room's painted screens.
- `env=NAME:VALUE,NAME:VALUE` in the query sets renderer switches for a test
  run. Frame and GPU pass times are in logcat every 300 frames or 5 s.

## Open

- Rumble, the refresh rate request (120 Hz panels run at 60 unless asked),
  pausing the sound in the background, keyboard input.
- Game saves stay in memory (as in the browser).
