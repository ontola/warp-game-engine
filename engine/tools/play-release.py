#!/usr/bin/env python3
"""The one way to make a splitscreen.club/play/ release: build the browser
host from a clean checkout of a commit on main, run the release checks and
package the result.

    python3 engine/tools/play-release.py build [--native-tests] [--browser] [--hlsl] [--no-build]
    python3 engine/tools/play-release.py check build/web

`build` refuses a dirty tree or a commit that is not on origin/main, runs
`build-web.py --warp` (the default shelf: plain --warp, the Godot games live
under /play/godot/ outside the release), checks build/web and writes
build/play-release/: warp-web.tar.gz (top folder warp-web/) and release.json
(the commit, the build version, every check and the archive's SHA-256).
It exits non-zero when any check fails, and then packages nothing.

The GitHub workflow .github/workflows/play-release.yml runs this on every
push to main and publishes the candidate as the GitHub release
play-<12 hex>. Production installs a candidate only through
engine/tools/host/promote-play.py, after Joep's yes in the
prod monitor thread. Nobody copies files into /play/ by hand: previews go
to /playtests/ (see engine/docs/play-release.md).

The checks, each in release.json:
- version: version.json and index.html name this commit.
- engine: the wasm, its JavaScript and the baked room light are there.
- shelf: games/library.json has every default game (WARP_GAMES and
  RUST_GAMES in build-web.py).
- files:<game>: each Luau game ships exactly the files of its folder at
  this commit (nothing missing, nothing extra, byte for byte); built-in
  Rust games ship every file their files.json names.
- wta-apache: Warp Theft Auto has the two-seat Apache: the eight files of
  assets/apache (body, rotor, tail rotor, livery, credits), the gunner
  optic (apache_hud), the Apache airframe in vehicles and the pilot and
  gunner seats in instruments, with each glTF's buffers present.
- browser:<name> (--browser): engine/tools/play-smoke.cjs boots the bundle
  in Chromium with WebGPU (SwiftShader, under xvfb-run when there is no
  display): the Game Room, then every game on the shelf and back, failing
  on a panic, GPU error, console error, missing first frame or a blank
  picture.
- hlsl (--hlsl, needs --browser): every WGSL module the browser compiled
  goes through naga's HLSL output and DXC (engine/tools/hlsl-check.py, the
  Firefox-on-Windows path); NAGA and DXC name the tools.
- native:<test> (--native-tests): the Warp Theft Auto crew tests:
  two local pads board one Apache as pilot and gunner, both stations
  fire, and the car-then-helicopter crew handover.
"""
import argparse
import ast
import hashlib
import json
import os
import subprocess
import sys
import tarfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WEB = ROOT / 'build/web'
OUT = ROOT / 'build/play-release'

# Must match build-web.py's copy_games filter for Luau games.
SKIP = {'scripts', 'tests', 'docs', 'build'}
KEEP_MD = {'CREDITS', 'LICENSE', 'NOTICE'}

APACHE_FILES = ['assets/apache/' + f for f in
                ('body.gltf', 'body.bin', 'rotor.gltf', 'rotor.bin', 'tail-rotor.gltf', 'tail-rotor.bin', 'green.jpg', 'CREDITS.md')]
# (bundled file, text it must contain, what it shows)
APACHE_CODE = [
    ('apache_hud.module.luau', '', 'the gunner optic'),
    ('vehicles.module.luau', 'assets/apache/', 'the Apache airframe from assets/apache'),
    ('vehicles.module.luau', 'tag="seat"', 'the Apache seat mounts'),
    ('instruments.module.luau', 'Apache gunner seat', 'boarding the gunner seat'),
    ('instruments.module.luau', 'Apache pilot seat', 'boarding the pilot seat'),
    ('instruments.module.luau', 'require("apache_hud")', 'the optic in the HUD'),
]
NATIVE_TESTS = [
    ('theft_auto_flight', 'two_local_pads_board_the_apache_and_its_light_reaches_ahead_at_altitude'),
    ('theft_auto_flight', 'gunner_holds_world_direction_when_pilot_turns_and_zooms_until_a_stop_binds'),
    ('theft_auto_apache_combat', 'both_apache_stations_can_fire_and_switch_pilots_independently'),
    ('theft_auto', 'shared_car_then_helicopter_with_an_abrupt_pilot_departure'),
]


def git(*args):
    return subprocess.run(['git', *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def default_shelf():
    """WARP_GAMES and RUST_GAMES' names from build-web.py, without running it."""
    tree = ast.parse((ROOT / 'engine/tools/build-web.py').read_text())
    found = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name) and node.targets[0].id in ('WARP_GAMES', 'RUST_GAMES'):
            value = node.value
            found[node.targets[0].id] = [ast.literal_eval(k) for k in value.keys] if isinstance(value, ast.Dict) else ast.literal_eval(value)
    return found['WARP_GAMES'], found['RUST_GAMES']


def luau_files(name):
    """The files build-web.py ships for games/NAME, from git (tracked only)."""
    listed = git('ls-files', '-z', '--', f'games/{name}').split('\0')
    files = []
    for path in filter(None, listed):
        rel = Path(path).relative_to(f'games/{name}')
        if not (ROOT / path).is_file() or rel.parts[0] in SKIP:
            continue
        if rel.suffix == '.md' and rel.stem.upper() not in KEEP_MD:
            continue
        if rel.parts[:2] == ('assets', 'gamenight') or rel.name.endswith('.d.luau'):
            continue
        files.append(rel.as_posix())
    return sorted(files)


def check(web, sha=None):
    results = []

    def note(name, ok, detail):
        results.append({'name': name, 'ok': bool(ok), 'detail': detail})
        print(f"{'ok  ' if ok else 'FAIL'} {name}: {detail}", flush=True)

    sha = sha or git('rev-parse', 'HEAD')
    short = git('rev-parse', '--short', sha)
    # version
    try:
        version = json.loads((web / 'version.json').read_text())['version']
        page = (web / 'index.html').read_text()
        ok = version.startswith(short + '-') and sha.startswith(short) and f'data-version="{version}"' in page
        note('version', ok, f'{version} (commit {short})')
    except (OSError, ValueError, KeyError) as e:
        version = None
        note('version', False, f'no version.json or index.html: {e}')
    # engine
    wasm = web / 'pkg/browser_bg.wasm'
    missing = [p for p in ('pkg/browser_bg.wasm', 'pkg/browser.js', 'room-light.bin', 'web-catalog.js', 'game-worker.js') if not (web / p).is_file()]
    size = wasm.stat().st_size / 1e6 if wasm.is_file() else 0
    note('engine', not missing and size > 5, f'wasm {size:.1f} MB' + (f', missing {missing}' if missing else ''))
    # shelf
    warp_games, rust_games = default_shelf()
    try:
        library = json.loads((web / 'games/library.json').read_text())
    except (OSError, ValueError) as e:
        library = []
        note('shelf', False, f'no games/library.json: {e}')
    else:
        folders = {g['folder'] for g in library}
        absent = [g for g in warp_games + rust_games if g not in folders]
        note('shelf', not absent, f'{len(library)} games' + (f', missing {absent}' if absent else ''))
    # files per game
    for name in warp_games + rust_games:
        try:
            shipped = json.loads((web / 'games' / name / 'files.json').read_text())
        except (OSError, ValueError):
            note(f'files:{name}', False, 'no files.json')
            continue
        absent = [f for f in shipped if not (web / 'games' / name / f).is_file()]
        if name in rust_games:
            note(f'files:{name}', shipped and not absent, f'{len(shipped)} files' + (f', missing {absent[:5]}' if absent else ''))
            continue
        source = luau_files(name)
        left_out = sorted(set(source) - set(shipped))
        extra = sorted(set(shipped) - set(source))
        differ = [f for f in shipped if f in source and (web / 'games' / name / f).is_file()
                  and (web / 'games' / name / f).read_bytes() != (ROOT / 'games' / name / f).read_bytes()]
        ok = not (absent or left_out or extra or differ)
        detail = f'{len(shipped)} files'
        for label, items in (('not in the bundle', absent), ('left out', left_out), ('not in the commit', extra), ('differ from the commit', differ)):
            if items:
                detail += f'; {label}: {items[:5]}' + (f' (+{len(items) - 5})' if len(items) > 5 else '')
        note(f'files:{name}', ok, detail)
    # The two-seat Apache in Warp Theft Auto.
    game = web / 'games/warp-theft-auto'
    problems = []
    try:
        shipped = set(json.loads((game / 'files.json').read_text()))
    except (OSError, ValueError):
        shipped = set()
    for f in APACHE_FILES:
        if f not in shipped or not (game / f).is_file():
            problems.append(f'missing {f}')
    for f in [f for f in APACHE_FILES if f.endswith('.gltf') and (game / f).is_file()]:
        try:
            for buf in json.loads((game / f).read_text()).get('buffers', []):
                uri = buf.get('uri', '')
                if uri and not uri.startswith('data:') and not (game / 'assets/apache' / uri).is_file():
                    problems.append(f'{f} needs {uri}')
        except ValueError:
            problems.append(f'{f} is not glTF JSON')
    for f, text, what in APACHE_CODE:
        path = game / f
        if f not in shipped or not path.is_file() or (text and text not in path.read_text()):
            problems.append(f'{what} ({f})')
    note('wta-apache', not problems, 'two-seat Apache present' if not problems else '; '.join(problems))
    return results, version


def browser_smoke():
    smoke = OUT / 'smoke.json'
    OUT.mkdir(parents=True, exist_ok=True)
    smoke.unlink(missing_ok=True)
    command = ['node', str(ROOT / 'engine/tools/play-smoke.cjs'), str(WEB), '--report', str(smoke),
               '--shots', str(OUT / 'shots'), '--shaders', str(OUT / 'wgsl')]
    if not os.environ.get('DISPLAY'):
        command = ['xvfb-run', '-a', '-s', '-screen 0 1280x800x24'] + command
    run = subprocess.run(command, cwd=ROOT)
    if not smoke.is_file():
        return [{'name': 'browser:room', 'ok': False, 'detail': f'play-smoke.cjs stopped (exit {run.returncode}) without a report'}]
    return [{'name': 'browser:' + r['name'], 'ok': r['ok'], 'detail': r['detail'] or ''} for r in json.loads(smoke.read_text())['results']]


def hlsl_check():
    # The tools must still reject the shader that broke Zen on 26 November.
    tools = ['--naga', os.environ.get('NAGA', 'naga'), '--dxc', os.environ.get('DXC', 'dxc')]
    known_bad = subprocess.run([sys.executable, str(ROOT / 'engine/tools/hlsl-check.py'), str(ROOT / 'engine/tools/hlsl-fixtures')] + tools,
                               cwd=ROOT, capture_output=True, text=True)
    if known_bad.returncode != 1:
        return [{'name': 'hlsl', 'ok': False, 'detail': 'naga + DXC did not reject engine/tools/hlsl-fixtures (the 26 Nov Zen shader):\n'
                 + (known_bad.stdout + known_bad.stderr)[-1500:]}]
    report = OUT / 'hlsl.json'
    run = subprocess.run([sys.executable, str(ROOT / 'engine/tools/hlsl-check.py'), str(OUT / 'wgsl'), '--report', str(report),
                          '--naga', os.environ.get('NAGA', 'naga'), '--dxc', os.environ.get('DXC', 'dxc')], cwd=ROOT, capture_output=True, text=True)
    print(run.stdout + run.stderr, flush=True)
    lines = (run.stdout + run.stderr).strip().splitlines()
    return [{'name': 'hlsl', 'ok': run.returncode == 0, 'detail': lines[-1] if run.returncode == 0 and lines else '\n'.join(lines[-40:])}]


def native_tests():
    results = []
    for test, name in NATIVE_TESTS:
        run = subprocess.run(['cargo', 'test', '--locked', '--release', '-p', 'warp', '--test', test, '--', '--exact', name],
                             cwd=ROOT, capture_output=True, text=True)
        passed = run.returncode == 0 and ' 1 passed' in run.stdout
        tail = (run.stdout + run.stderr).strip().splitlines()[-12:]
        results.append({'name': f'native:{test}::{name}', 'ok': passed, 'detail': 'passed' if passed else '\n'.join(tail)})
        print(f"{'ok  ' if passed else 'FAIL'} native:{test}::{name}", flush=True)
        if not passed:
            print('\n'.join(tail), flush=True)
    return results


def package(web, release):
    OUT.mkdir(parents=True, exist_ok=True)
    archive = OUT / 'warp-web.tar.gz'
    with tarfile.open(archive, 'w:gz', compresslevel=6) as tar:
        for path in sorted(web.rglob('*')):
            if path.is_symlink():
                sys.exit(f'{path}: a release holds no symlinks')
            tar.add(path, arcname='warp-web/' + path.relative_to(web).as_posix(), recursive=False)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    release['archive'] = {'name': archive.name, 'sha256': digest, 'bytes': archive.stat().st_size}
    (OUT / 'release.json').write_text(json.dumps(release, indent=1) + '\n')
    (OUT / 'warp-web.tar.gz.sha256').write_text(f'{digest}  {archive.name}\n')
    print(f'{archive}: {archive.stat().st_size / 1e6:.1f} MB, sha256 {digest}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    b = sub.add_parser('build', help='build, check and package a release of HEAD')
    b.add_argument('--native-tests', action='store_true', help='also run the Warp Theft Auto crew tests')
    b.add_argument('--browser', action='store_true', help='also boot the bundle and every game in Chromium (play-smoke.cjs)')
    b.add_argument('--hlsl', action='store_true', help='with --browser: compile its WGSL through naga HLSL and DXC')
    b.add_argument('--no-build', action='store_true', help='check and package the existing build/web')
    b.add_argument('--branch', default='origin/main', help='the commit must be on this branch (default origin/main)')
    c = sub.add_parser('check', help='only check a built directory')
    c.add_argument('dir', type=Path)
    args = parser.parse_args()

    if args.command == 'check':
        results, _ = check(args.dir.resolve())
        sys.exit(0 if all(r['ok'] for r in results) else 1)

    sha = git('rev-parse', 'HEAD')
    dirty = git('status', '--porcelain', '--untracked-files=no')
    if dirty:
        sys.exit(f'The tree has changes; a release is built from a clean commit:\n{dirty}')
    if subprocess.run(['git', 'merge-base', '--is-ancestor', sha, args.branch], cwd=ROOT).returncode != 0:
        sys.exit(f'{sha} is not on {args.branch}: push it to main first')
    if not args.no_build:
        subprocess.run([sys.executable, str(ROOT / 'engine/tools/build-web.py'), '--warp'], cwd=ROOT, check=True)
    results, version = check(WEB, sha)
    if args.native_tests:
        results += native_tests()
    if args.browser and all(r['ok'] for r in results):
        results += browser_smoke()
        if args.hlsl:
            results += hlsl_check()
    release = {'repository': 'ontola/warp-game-engine', 'commit': sha, 'version': version,
               'subject': git('log', '-1', '--format=%s', sha),
               'built_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
               'run_id': int(os.environ['GITHUB_RUN_ID']) if os.environ.get('GITHUB_RUN_ID') else None,
               'checks': results, 'passed': all(r['ok'] for r in results)}
    failed = [r['name'] for r in results if not r['ok']]
    if failed:
        OUT.mkdir(parents=True, exist_ok=True)
        (OUT / 'release.json').write_text(json.dumps(release, indent=1) + '\n')
        sys.exit(f'Release checks failed: {", ".join(failed)}')
    package(WEB, release)


if __name__ == '__main__':
    main()
