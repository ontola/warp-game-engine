#!/usr/bin/env python3
"""Build terrain_fly for the browser (wasm32 + WebGPU) into build/web.

Needs the wasm32-unknown-unknown target (rustup target add
wasm32-unknown-unknown), clang (for meshoptimizer's C++) and wasm-bindgen-cli
at the wasm-bindgen version in Cargo.lock (the script says which). --serve
then serves build/web on http://localhost:8790 (WebGPU needs localhost or
HTTPS). See engine/docs/browser.md.
"""
import argparse
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
TEXTURES = ['grass_ground', 'forest_leaves_02', 'forest_ground_04', 'rock_face_03', 'gravel_ground_01', 'mossy_rock']

parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument('--serve', action='store_true', help='serve build/web after building')
parser.add_argument('--no-build', action='store_true', help='only serve what is there')
parser.add_argument('--port', type=int, default=8790)
args = parser.parse_args()

if not args.no_build:
    lock = (ROOT / 'Cargo.lock').read_text()
    version = re.search(r'name = "wasm-bindgen"\nversion = "([^"]+)"', lock).group(1)
    bindgen = shutil.which('wasm-bindgen')
    found = subprocess.run([bindgen, '--version'], capture_output=True, text=True).stdout.split()[-1] if bindgen else None
    if found != version:
        sys.exit(f'wasm-bindgen {version} is needed (found {found}): cargo install wasm-bindgen-cli --version {version} --locked')
    env = dict(os.environ, RUSTFLAGS='--cfg getrandom_backend="wasm_js"')
    subprocess.run(['cargo', 'build', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'app', '--example', 'terrain_fly'], cwd=ROOT, env=env, check=True)
    OUT.mkdir(parents=True, exist_ok=True)
    wasm = ROOT / 'target/wasm32-unknown-unknown/release/examples/terrain_fly.wasm'
    subprocess.run([bindgen, '--target', 'web', '--no-typescript', '--out-dir', str(OUT / 'pkg'), str(wasm)], check=True)
    shutil.copy(ROOT / 'engine/crates/app/web/index.html', OUT / 'index.html')
    (OUT / 'assets').mkdir(exist_ok=True)
    for name in TEXTURES:
        for kind in ('albedo', 'normal'):
            shutil.copy(ROOT / f'games/sideways/assets/textures/{name}_{kind}.jpg', OUT / 'assets')
    size = (OUT / 'pkg/terrain_fly_bg.wasm').stat().st_size / 1e6
    print(f'build/web: terrain_fly_bg.wasm {size:.1f} MB')

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
