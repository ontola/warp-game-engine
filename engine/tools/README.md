# Engine acceptance fixtures

Run from `engine/`. Python fixtures require Python 3.10+ and `websockets`
(validated with 17.3). Install with `python -m pip install websockets==17.3`.
Use a fresh `--out` directory for every run; keep failed evidence alongside
later successes. Run rendering fixtures serially.

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
