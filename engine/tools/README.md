# Engine acceptance fixtures

Run from `engine/`. Python fixtures require Python 3.10+ and `websockets`
(validated with 17.3). Install with `python -m pip install websockets==17.3`.
Use a fresh `--out` directory for every run; keep failed evidence alongside
later successes. Run rendering fixtures serially.

## Short Windows development loop

Run explicitly from `engine/`:

```powershell
tools\play-dev-windows.ps1 -Autoplay
tools\play-dev-windows.ps1 -Restart -Autoplay
tools\play-dev-windows.ps1 -BuildOnly
tools\play-dev-windows.ps1 -Restart -Views 4 -Windowed -Size 1920x1080
```

The helper builds the current checkout, stages a uniquely named executable,
and launches only when explicitly invoked without `-BuildOnly`. It keeps the
previous tracked playtest running throughout compilation. `-Restart` replaces
only the helper's own recorded process after the build succeeds, verifying
its exact executable path, PID and creation time again after compilation.
Games started by other tools are never stopped by this helper; stop those
separately before adopting it, to avoid concurrent GPU workloads.

Release is the default. `-Profile dev` uses the workspace's faster incremental
development profile; do not compare its performance with release results.
`-TargetDirectory C:\dev\warp-target-20261023` or the existing
`CARGO_TARGET_DIR` can share Cargo artifacts. No source snapshot is taken;
avoid editing source during a build intended for a reproducible comparison.

Each `build/dev-runs/<run>/` preserves build/play logs, executable SHA256,
arguments, source revision/dirty status and an `iteration.json` manifest.
Normal game shutdown also writes `final-state.json`; continuous state tracing
is not enabled by default.
Success requires the game's first **presented** frame log. A startup timeout
reports uncertainty and leaves that exact process recorded for inspection or
the next `-Restart`; it does not claim a working playtest or kill it blindly.
This is a launch check, not GPU benchmarking or a physical controller test.
F12 saves the current final image and metadata under `build/screenshots/`
and copies the image to the Windows clipboard. Assets still resolve from the
build-time checkout, which must remain in place. GameNight launches continue
through the host; this standalone helper rejects inherited host credentials.

`tools\test-play-dev.ps1` checks process identity, build artifact handling,
presentation evidence, quoting and script syntax without Cargo, a game process
or GPU work.

## Windows lifecycle and controller contract

```powershell
python tools/play_lifecycle_acceptance.py --exe C:/path/to/sideways.exe --out C:/temp/sideways-mock-001 --extended
python tools/real_daemon_acceptance.py --exe C:/path/to/sideways.exe --out C:/temp/sideways-real-001
```

Both launch the candidate with an invisible window and preserve JSON reports,
wire logs, executable hashes, state traces and exit evidence. `--cwd` defaults
to this engine directory. `--size` defaults to 640x360. These are functional
checks, not performance benchmarks or proof of visible presentation.

The mock fixture exercises explicit SDK6 lifecycle, settings, controller
reordering, stale input, button edges and deferred roster changes. It does not
certify the installed daemon or physical hardware.

The real fixture discovers the current user's GameNight Preview daemon under
`LOCALAPPDATA`. For another installation, pass
`--daemon C:/path/to/gamenight-daemon.exe`. It uses a fresh loopback port,
private temporary library/data directory and an authenticated, invisible
`headless_lobby_fixture.py` process. The installed runtime samples a connected
physical controller; no fixture publishes controller frames. No button presses
are required. It verifies frame arrival and canonical seat binding, launch
authentication, session/settings/profile behavior, and natural candidate exit.
Neutral frames do not prove live driving responsiveness. Only the fixture's
own process tree is cleaned up; the normal GameNight catalog is not changed.

SDK5 hosts do not support SDK6 menu actions. The adapter records this in
`host.capabilities`; the real report lists the resulting feature gap. A passing
fixture does not imply full Godot feature parity or a 120 FPS result.

## Matched Godot screenshot

`godot_matched_shot.gd` runs against a separate imported Godot reference project;
it never edits that reference. First generate a native one-view `shot` and keep
its `.png.json` sidecar, which records actual camera and car transforms. Use
matching seed, distance, length, weather, frame count and rival count:

```sh
# Native executable; shot uses one rival by default.
sideways shot /tmp/kern.png --views 1 --seed 1 --km 3 --at 300 --frames 30 --weather clear --size 1920x1080

# Linux/WSL: real Vulkan renderer on a private X server, not --headless.
env -u GAMENIGHT -u GAMENIGHT_ADDR -u GAMENIGHT_TOKEN \
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json \
  xvfb-run -a -s '-screen 0 1920x1080x24' \
  godot --path /absolute/imported-godot-reference --fixed-fps 120 \
  --resolution 1920x1080 --disable-vsync \
  --script /absolute/engine/tools/godot_matched_shot.gd -- \
  --kern-metadata /tmp/kern.png.json --shot /tmp/godot.png \
  --frames 30 --render 1 --views 1 --rivals 1 --seed 1 --km 3 --at 300 \
  --weather clear --render-scale 100% --anti-aliasing spatial
```

Use the Vulkan ICD path installed on that Linux system. Godot 4.5.2 is the
reference version. Import the reference project first. The fixture rejects
existing outputs, wrong dimensions, multiple views and mismatched scene inputs.
It captures production Godot materials/postprocessing with forced native
camera/body/wheel/paint poses and sun direction; the JSON records original and
forced values. Lighting energies, tone mapping, far plane and terrain/physics
may still differ. This is a frozen visual comparison, not a matched simulation
or GPU-speed comparison.
