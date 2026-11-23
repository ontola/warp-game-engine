#!/usr/bin/env python3
"""Give a dedicated Warp Game Room shelf stable asset URLs and negotiated-by-loader gzip.
Run on a fresh build, with --previous pointing at the exact snapshot players received.
"""
import argparse
import gzip
import hashlib
from io import BytesIO
import json
import mimetypes
from pathlib import Path
import re

CONFIG_TAG = '<script id="warp-shelf-assets" type="application/json">'
BEGIN = '<!-- warp-shelf-cache -->'
END = '<!-- /warp-shelf-cache -->'
DIGEST = re.compile(r'[a-f0-9]{64}')
VERSION = re.compile(r'[a-zA-Z0-9._-]+')
WASM = 'pkg/browser_bg.wasm'
GLUE = 'pkg/browser.js'
EDITOR = 'stories/editor.js'
BOOT = 'async function wasm() {'
WASM_HOOK = '  if (window.warpShelfWasm) return window.warpShelfWasm(show);'
ENGINE_FETCHES = {'games/library.json', 'room-light.bin', GLUE, WASM}


def file_map(folder):
    return {p.relative_to(folder).as_posix(): p for p in sorted(folder.rglob('*'))
            if p.is_file() and not p.relative_to(folder).as_posix().startswith('pkg/compressed/')
            and p.relative_to(folder).as_posix() not in {'index.html', 'version.json'}}


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def legacy_key(name, version):
    if name in ENGINE_FETCHES or (name.startswith('games/') and name != 'games/index.json'):
        return version
    return ''


def previous_keys(folder, version):
    html = (folder/'index.html').read_text(encoding='utf-8')
    if CONFIG_TAG in html:
        return json.loads(html.split(CONFIG_TAG, 1)[1].split('</script>', 1)[0])['keys']
    return {name: legacy_key(name, version) for name in file_map(folder)}


def patch_editor(path):
    text = path.read_text(encoding='utf-8')
    replacements = [
        ("new Worker(new URL('./worker.js',import.meta.url),", "new Worker(window.warpShelfAsset(new URL('./worker.js',import.meta.url)),"),
        ("version:document.documentElement.dataset.version", "version:window.warpShelfAsset('pkg/browser.js').searchParams.get('v')"),
    ]
    for old, new in replacements:
        if new not in text:
            if text.count(old) != 1: raise ValueError('Unrecognized story editor dependency: ' + old)
            text = text.replace(old, new)
    path.write_text(text, encoding='utf-8', newline='\n')


def prepare(folder, previous, report=None):
    folder, previous = folder.resolve(), previous.resolve()
    if folder == previous or previous.is_relative_to(folder) or folder.is_relative_to(previous):
        raise ValueError('Use separate previous and output shelves')
    version = json.loads((folder/'version.json').read_text())['version']
    prior_version = json.loads((previous/'version.json').read_text())['version']
    if not VERSION.fullmatch(version) or not VERSION.fullmatch(prior_version) or version == prior_version:
        raise ValueError('Expected two distinct build versions')
    previous_files = file_map(previous)
    prior = previous_keys(previous, prior_version)
    if prior.keys() != previous_files.keys():
        raise ValueError('Previous shelf differs from its asset URL map')
    game_id, = json.loads((folder/'games/index.json').read_text())
    manifest = json.loads((folder/'games'/game_id/'files.json').read_text())
    game_files = {f'games/{game_id}/{name}' for name in manifest}
    patch_editor(folder/EDITOR)
    files = file_map(folder)
    keys, compressed, changed = {}, {}, {}
    sidecars = folder/'pkg/compressed'
    sidecars.mkdir(exist_ok=True)
    sizes = {name: path.stat().st_size for name, path in files.items()}
    for name, file in files.items():
        identity = digest(file)
        if name in previous_files and digest(previous_files[name]) == identity:
            keys[name] = prior[name]
        else:
            keys[name] = identity
            changed[name] = sizes[name]
        if (name not in game_files and name not in {WASM, 'room-light.bin'}) or sizes[name] < 8192:
            continue
        # Files already compressed by this static host need no second representation.
        if file.suffix.lower() in {'.png','.jpg','.jpeg','.webp','.ogg','.json','.gz'}: continue
        raw = file.read_bytes()
        buffer = BytesIO()
        with gzip.GzipFile(filename='', mode='wb', fileobj=buffer, compresslevel=6, mtime=0) as gz:
            gz.write(raw)
        packed = buffer.getvalue()
        if len(raw) - len(packed) < max(2048, len(raw)//10): continue
        (sidecars/(identity+'.gz')).write_bytes(packed)
        mime = 'application/wasm' if name == WASM else mimetypes.guess_type(name)[0] or 'application/octet-stream'
        compressed[name] = [identity, len(raw), len(packed), mime]
    expected_sidecars = {item[0]+'.gz' for item in compressed.values()}
    for path in sidecars.iterdir():
        if path.name not in expected_sidecars:
            if not path.is_file() or not DIGEST.fullmatch(path.stem) or path.suffix != '.gz':
                raise ValueError('Unrecognized staged sidecar: '+str(path))
            path.unlink()

    config = {'keys':keys, 'compressed':compressed, 'wasm':WASM}
    html_path = folder/'index.html'
    html = html_path.read_text(encoding='utf-8')
    if BEGIN in html:
        start = html.index(BEGIN)
        html = html[:start] + html[html.index(END,start)+len(END):].lstrip('\n')
    if html.count(BOOT) != 1 or ('data-version="'+version+'"') not in html: raise ValueError('Unrecognized shelf boot page')
    if WASM_HOOK not in html:
        html = html.replace(BOOT, BOOT+'\n'+WASM_HOOK)
    # Direct imports and markup bypass window.fetch. Keep their effective original
    # URLs on migration and give edited scripts/styles/fonts their own keys later.
    def apply_url(m):
        prefix, name, suffix = m.group('prefix','name','suffix')
        key = keys[name]
        return prefix + name + ('?v='+key if key else '') + suffix
    direct = [GLUE, WASM, EDITOR, 'stories/editor.css','splitscreen-club-icon.png',
              *(name for name in files if name.startswith('fonts/') and name.endswith('.ttf'))]
    names = '|'.join(re.escape(n) for n in direct)
    html = re.sub(r'(?P<prefix>["\'(](?:\./)?)(?P<name>'+names+r')(?:\?v=[\w.-]+)?(?P<suffix>["\')])', apply_url, html)
    loader = Path(__file__).with_name('web-shelf-cache.js').read_text(encoding='utf-8')
    payload = json.dumps(config,separators=(',',':')).replace('<','\\u003c')
    block = BEGIN+'\n'+CONFIG_TAG+payload+'</script>\n<script>\n'+loader+'</script>\n'+END+'\n'
    charset = '<meta charset="utf-8">'
    if charset not in html: raise ValueError('Missing UTF-8 declaration')
    html = html.replace(charset, charset+'\n'+block,1)
    html_path.write_text(html, encoding='utf-8', newline='\n')
    transmitted = {n: compressed[n][2] if n in compressed else size for n,size in sizes.items()}
    summary = {'previous':prior_version,'version':version,'game_files':len(game_files),
               'game_raw_bytes':sum(sizes[n] for n in game_files),'game_cold_payload_bytes_before_http_encoding':sum(transmitted[n] for n in game_files),
               'runtime_raw_bytes':sizes[WASM],'runtime_cold_wire_bytes':transmitted[WASM],
               'added_game_files':sorted(game_files-previous_files.keys()),
               'added_game_raw_bytes':sum(sizes[n] for n in game_files-previous_files.keys()),
               'added_game_payload_bytes_before_http_encoding':sum(transmitted[n] for n in game_files-previous_files.keys()),
               'changed_assets':changed,'changed_game_files':sorted(game_files & changed.keys()),
               'changed_game_payload_bytes_before_http_encoding':sum(transmitted[n] for n in game_files & changed.keys()),
               'removed_assets':sorted(previous_files.keys()-files.keys()),'gzip_representations':len(compressed)}
    if report: report.write_text(json.dumps(summary,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(summary,indent=2))
    return summary


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('shelf',type=Path)
    parser.add_argument('--previous',required=True,type=Path)
    parser.add_argument('--report',type=Path,help='Write a byte budget outside the public shelf')
    args = parser.parse_args()
    prepare(args.shelf,args.previous,args.report)
