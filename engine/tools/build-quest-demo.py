#!/usr/bin/env python3
"""Build the self-contained Growing Guns WebXR demo. No cloud runtime required."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'build/quest-demo'
VERSION = '0.2.129'

def run(*args, **kwargs):
    subprocess.run(args, cwd=ROOT, check=True, **kwargs)

def main():
    tools = ROOT / 'build/wasm-tools'
    bindgen = tools / 'bin/wasm-bindgen'
    if not bindgen.exists() or subprocess.check_output([str(bindgen), '--version'], text=True).strip() != f'wasm-bindgen {VERSION}':
        run('cargo', 'install', 'wasm-bindgen-cli', '--version', VERSION, '--locked', '--root', str(tools))
    run('rustup', 'target', 'add', 'wasm32-unknown-unknown')
    run('npm', 'ci', '--prefix', 'engine/webxr', '--ignore-scripts')
    run('cargo', 'build', '--locked', '--release', '-p', 'growing-guns-quest', '--target', 'wasm32-unknown-unknown')
    metadata = json.loads(subprocess.check_output(['cargo','metadata','--no-deps','--format-version','1'], cwd=ROOT))
    wasm = Path(metadata['target_directory']) / 'wasm32-unknown-unknown/release/growing_guns_quest.wasm'
    # Only clear the known generated directory, never any game sources.
    if OUT.exists(): shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    run(str(bindgen), str(wasm), '--target', 'web', '--out-dir', str(OUT/'pkg'), '--no-typescript')
    source = ROOT / 'games/growing-guns/quest/web'
    for name in ('index.html','style.css','manifest.webmanifest','icon.svg'):
        shutil.copy2(source/name, OUT/name)
    shutil.copy2(ROOT/'engine/assets/fonts/BarlowCondensed-Medium.ttf', OUT/'barlow.ttf')
    shutil.copy2(ROOT/'engine/assets/fonts/OFL-BarlowCondensed.txt', OUT/'OFL-Barlow.txt')
    shutil.copy2(ROOT/'engine/webxr/node_modules/three/LICENSE', OUT/'THREE-LICENSE.txt')
    run(str(ROOT/'engine/webxr/node_modules/.bin/esbuild'), str(source/'demo.mjs'), '--bundle', '--format=esm', '--minify', '--external:./pkg/*', f'--outfile={OUT}/demo.js')
    files = sorted(p.relative_to(OUT).as_posix() for p in OUT.rglob('*') if p.is_file())
    digest = hashlib.sha256(b''.join((OUT/f).read_bytes() for f in files)).hexdigest()[:16]
    # Installation is all-or-nothing, with a versioned app-only cache. No CDN
    # dependencies, no tracking, no runtime server endpoints, no stale WASM/JS pair.
    (OUT/'sw.js').write_text('''const CACHE = "warp-quest-'''+digest+'''";
const ASSETS = '''+json.dumps(['./'+f for f in files])+''';
self.addEventListener('install',event=>event.waitUntil(caches.open(CACHE).then(c=>c.addAll(ASSETS)).then(()=>self.skipWaiting())));
self.addEventListener('activate',event=>event.waitUntil(caches.keys().then(keys=>Promise.all(keys.filter(k=>k.startsWith('warp-quest-')&&k!==CACHE).map(k=>caches.delete(k)))).then(()=>self.clients.claim())));
self.addEventListener('fetch',event=>{
  if(event.request.method!=='GET'||new URL(event.request.url).origin!==self.location.origin)return;
  event.respondWith(caches.open(CACHE).then(async cache=>
    (event.request.mode==='navigate'?await cache.match('./index.html'):await cache.match(event.request,{ignoreSearch:true})) || fetch(event.request)));
});
''')
    (OUT/'build.json').write_text(json.dumps({'content':digest,'runtime':'standalone WebXR + Rust/WASM','wasm_bindgen':VERSION,'headset_verified':False},indent=2)+'\n')
    archive = ROOT/'build/growing-guns-quest-webxr.zip'
    with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED) as z:
        for path in sorted(OUT.rglob('*')):
            if path.is_file(): z.write(path,path.relative_to(OUT))
    print(f'Built {OUT}\nArchive: {archive} ({archive.stat().st_size/1e6:.2f} MB)\nStart: python3 engine/tools/serve-quest-demo.py')

if __name__ == '__main__': main()
