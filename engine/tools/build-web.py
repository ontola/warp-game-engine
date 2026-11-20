#!/usr/bin/env python3
"""Build terrain_fly (or another example: --example) for the browser
(wasm32 + WebGPU) into build/web.

Needs the wasm32-unknown-unknown target (rustup target add
wasm32-unknown-unknown), clang (for meshoptimizer's C++) and wasm-bindgen-cli
at the wasm-bindgen version in Cargo.lock (the script says which). --serve
then serves build/web on http://localhost:8790 (WebGPU needs localhost or
HTTPS). See engine/docs/browser.md.

    build-web.py --package architecture --example house_view --glb fzk-haus.glb --glb house.glb
builds the house viewer with those GLBs in assets/ (open ?house=fzk-haus).

    build-web.py --warp [--games neon-siege,stack-together] --serve
builds Splitscreen Club for the browser: the Game Room lobby
(lobbies/game-room/examples/browser.rs) with those Luau game folders on
its shelf (?game=neon-siege starts in that game). --bare leaves the room
out (engine/crates/warp/examples/warp_web.rs: the games and a picker). Luau is C++ that needs a C and
C++ standard library: wasi-libc and libc++ for wasm32 (Debian/Ubuntu:
apt install wasi-libc libc++-dev-wasm32 libclang-rt-dev-wasm32, or
WASI_SYSROOT=a wasi-sdk share/wasi-sysroot).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'build/web'
# The ground textures terrain_fly reads (WEB_TEXTURES in the example).
# Browsers keep an AudioContext suspended until the player clicks or
# presses a key on the page (a gamepad button does not count): remember
# every context the engine makes and resume them on the first such event.
AUDIO_UNLOCK = '''<script>
(() => {
  const made = [], Real = window.AudioContext || window.webkitAudioContext;
  if (!Real) return;
  window.AudioContext = class extends Real { constructor(...a) { super(...a); made.push(this); } };
  const resume = () => made.forEach(c => c.state !== 'running' && c.resume());
  for (const e of ['pointerdown', 'keydown', 'touchend']) addEventListener(e, resume, { capture: true });
  window.warpAudio = made;
})();
</script>
'''

# The --warp page's buttons: each game switches in the running engine
# (switch_game), without a reload; with the room, "room" goes back to it
# (back_to_room, as Back on a pad).
WARP_PICKER = '''<div id="warp-games" style="position:fixed;top:8px;right:8px;display:flex;gap:6px;flex-wrap:wrap;justify-content:flex-end;max-width:70vw"></div>
<script type="module">
const bar = document.getElementById('warp-games');
const games = await (await fetch('games/index.json')).json();
const room = ROOM;
const current = () => new URLSearchParams(location.search).get('game') || (room ? 'room' : games[0]);
const paint = () => { for (const b of bar.children) b.style.background = b.dataset.game === current() ? '#8fd19e' : '#222'; };
for (const g of (room ? ['room'] : []).concat(games)) {
  const b = document.createElement('button');
  b.textContent = g; b.dataset.game = g;
  b.style.cssText = 'font:12px monospace;color:#ddd;border:1px solid #555;border-radius:4px;padding:4px 8px;cursor:pointer';
  b.onclick = () => {
    if (!window.warp) return;
    const q = new URLSearchParams(location.search);
    if (g === 'room') q.delete('game'); else q.set('game', g);
    history.replaceState(null, '', '?' + q);
    paint(); b.blur();
    if (g === 'room') window.warp.back_to_room(); else window.warp.switch_game(g);
  };
  bar.appendChild(b);
}
paint();
</script>
'''

# Luau games the --warp build serves by default, and folders it leaves out.
WARP_GAMES = ['neon-siege', 'stack-together', 'neon-trails', 'hello-warp', 'relay-keepers', 'dead-miles']
SKIP = {'scripts', 'tests', 'docs', 'build'}
TEXTURES = ['grass_ground', 'forest_leaves_02', 'forest_ground_04', 'rock_face_03', 'gravel_ground_01', 'mossy_rock']

parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument('--serve', action='store_true', help='serve build/web after building')
parser.add_argument('--no-build', action='store_true', help='only serve what is there')
parser.add_argument('--port', type=int, default=8790)
parser.add_argument('--package', default='app', help='crate of the example (default app)')
parser.add_argument('--example', default='terrain_fly', help='example to build (default terrain_fly)')
parser.add_argument('--glb', action='append', default=[], help='a GLB to copy into assets/, with its NAME.light.bin when present (house_view)')
parser.add_argument('--warp', action='store_true', help='build the Game Room with Luau games (lobbies/game-room/examples/browser.rs)')
parser.add_argument('--bare', action='store_true', help='with --warp: the games without the room (examples/warp_web.rs)')
parser.add_argument('--games', default=','.join(WARP_GAMES), help='games/ folders to serve with --warp (comma separated)')
args = parser.parse_args()
if args.warp:
    args.package, args.example = ('warp', 'warp_web') if args.bare else ('game-room', 'browser')


def wasi_sysroot():
    """Include and library folders of wasi-libc and libc++ for wasm32."""
    roots = [Path(os.environ['WASI_SYSROOT'])] if os.environ.get('WASI_SYSROOT') else [Path('/usr')]
    for root in roots:
        for triple in ('wasm32-wasi', 'wasm32-wasip1'):
            inc, lib = root / 'include' / triple, root / 'lib' / triple
            if (inc / 'string.h').exists() and (lib / 'libc++.a').exists():
                return triple, inc, lib
    sys.exit('Luau needs wasi-libc and libc++ for wasm32: apt install wasi-libc libc++-dev-wasm32 libclang-rt-dev-wasm32 (or WASI_SYSROOT=wasi-sdk/share/wasi-sysroot)')


def luau_env(env):
    """Compile Luau against wasi-libc and libc++, without exception support
    (script/src/web.rs stops the game on a thrown error), and hand the linker
    one libstdc++.a (what mlua-sys asks for) holding libc++, libc++abi and libc."""
    triple, inc, lib = wasi_sysroot()
    cxx = ROOT / 'target/web-cxx'
    cxx.mkdir(parents=True, exist_ok=True)
    ar = shutil.which('llvm-ar') or next((shutil.which(f'llvm-ar-{v}') for v in range(30, 14, -1) if shutil.which(f'llvm-ar-{v}')), None)
    if not ar: sys.exit('llvm-ar is needed to combine libc++, libc++abi and libc')
    script = 'CREATE libstdc++.a\n' + ''.join(f'ADDLIB {lib / name}\n' for name in ('libc++.a', 'libc++abi.a', 'libc.a')) + 'SAVE\nEND\n'
    (cxx / 'libstdc++.a').unlink(missing_ok=True)
    subprocess.run([ar, '-M'], input=script, text=True, cwd=cxx, check=True)
    env['LUAU_CXXFLAGS'] = f'--target={triple} -isystem {inc / "c++/v1"} -isystem {inc} -fexceptions -fignore-exceptions'
    env['RUSTFLAGS'] += f' -L {cxx}'


def copy_games(names):
    """games/NAME for each Luau game, with files.json (its files),
    games/index.json (the names) and games/library.json (the room's shelf:
    id, title and player counts from gamenight.json): the page fetches these."""
    out = OUT / 'games'
    shutil.rmtree(out, ignore_errors=True)
    library = []
    for name in names:
        src = ROOT / 'games' / name
        if not (src / 'main.luau').exists():
            sys.exit(f'games/{name}: not a Luau game (no main.luau)')
        files = sorted(str(p.relative_to(src)).replace(os.sep, '/') for p in src.rglob('*')
                       if p.is_file() and p.relative_to(src).parts[0] not in SKIP and p.suffix not in ('.md',))
        for rel in files:
            (out / name / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(src / rel, out / name / rel)
        (out / name / 'files.json').write_text(json.dumps(files))
        kb = sum((src / f).stat().st_size for f in files) / 1024
        print(f'build/web/games/{name}: {len(files)} files, {kb:.0f} KB')
        meta = json.loads((src / 'gamenight.json').read_text()) if (src / 'gamenight.json').exists() else {}
        players = meta.get('players', {})
        lo, hi = players.get('min', 1), players.get('max', 4)
        library.append({'id': name, 'title': meta.get('title', name), 'tagline': meta.get('tagline', ''),
                         'players': f'{lo}-{hi}' if lo != hi else str(lo), 'min_players': lo, 'max_players': hi})
    (out / 'index.json').write_text(json.dumps(names))
    (out / 'library.json').write_text(json.dumps(library))

if not args.no_build:
    lock = (ROOT / 'Cargo.lock').read_text()
    version = re.search(r'name = "wasm-bindgen"\nversion = "([^"]+)"', lock).group(1)
    bindgen = shutil.which('wasm-bindgen')
    found = subprocess.run([bindgen, '--version'], capture_output=True, text=True).stdout.split()[-1] if bindgen else None
    if found != version:
        sys.exit(f'wasm-bindgen {version} is needed (found {found}): cargo install wasm-bindgen-cli --version {version} --locked')
    env = dict(os.environ, RUSTFLAGS='--cfg getrandom_backend="wasm_js"')
    if args.warp:
        luau_env(env)
    subprocess.run(['cargo', 'build', '--release', '--target', 'wasm32-unknown-unknown', '-p', args.package, '--example', args.example], cwd=ROOT, env=env, check=True)
    OUT.mkdir(parents=True, exist_ok=True)
    wasm = ROOT / f'target/wasm32-unknown-unknown/release/examples/{args.example}.wasm'
    subprocess.run([bindgen, '--target', 'web', '--no-typescript', '--out-dir', str(OUT / 'pkg'), str(wasm)], check=True)
    page = (ROOT / 'engine/crates/app/web/index.html').read_text().replace('terrain_fly', args.example)
    if args.warp:
        page = page.replace('<title>Terrain fly</title>', '<title>Warp</title>')
        page = page.replace(f"import init from './pkg/{args.example}.js';", f"import init, * as warp from './pkg/{args.example}.js';\nwindow.warp = warp;")
        if not args.bare:
            page = page.replace('<title>Warp</title>', '<title>Splitscreen Club</title>')
        page = page.replace('</body>', WARP_PICKER.replace('ROOM', 'false' if args.bare else 'true') + '</body>').replace('</head>', AUDIO_UNLOCK + '</head>')
    (OUT / 'index.html').write_text(page)
    (OUT / 'assets').mkdir(exist_ok=True)
    if args.example == 'terrain_fly':
        for name in TEXTURES:
            for kind in ('albedo', 'normal'):
                shutil.copy(ROOT / f'games/sideways/assets/textures/{name}_{kind}.jpg', OUT / 'assets')
    if args.warp:
        copy_games([g for g in args.games.split(',') if g])
    if args.warp and not args.bare:
        # The room's light, baked here on every core: the page would take
        # most of a minute on its one thread.
        subprocess.run(['cargo', 'run', '--release', '-p', 'game-room', '--', 'bake-light', '--out', str(OUT / 'room-light.bin')], cwd=ROOT, check=True)
        print(f"build/web/room-light.bin: {(OUT / 'room-light.bin').stat().st_size / 1e6:.1f} MB")
    for glb in args.glb:
        shutil.copy(glb, OUT / 'assets')
        # Its light baked ahead (house_view --bake-out NAME.light.bin), so
        # the tab doesn't bake on one thread. Hosts that serve no binary
        # files take base64 NAME.light.txt instead (base64 -w0).
        light = Path(glb).with_suffix('.light.bin')
        if light.exists():
            shutil.copy(light, OUT / 'assets')
    size = (OUT / f'pkg/{args.example}_bg.wasm').stat().st_size / 1e6
    print(f'build/web: {args.example}_bg.wasm {size:.1f} MB')

if args.serve:
    class Handler(SimpleHTTPRequestHandler):
        def end_headers(self):
            self.send_header('Cache-Control', 'no-cache')
            super().end_headers()
    server = ThreadingHTTPServer(('127.0.0.1', args.port), partial(Handler, directory=str(OUT)))
    print(f'Open http://localhost:{args.port} in Chrome (or ?style=alpine, ?trees=0)', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
