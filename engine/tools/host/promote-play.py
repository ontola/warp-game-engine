#!/usr/bin/env python3
"""Install a checked /play/ release candidate on the host: the only way
splitscreen.club/play/ changes.

    python3 promote-play.py status
    python3 promote-play.py COMMIT [--dry-run]     # 40 hex, or latest
    python3 promote-play.py rollback [--dry-run]

A candidate is the GitHub prerelease play-<12 hex> of ontola/warp-game-engine
(public), made by its Play release candidate workflow
(.github/workflows/play-release.yml) from a commit on main after the
release checks in engine/tools/play-release.py passed, including the
two-seat Apache in Warp Theft Auto. This script checks the release, its
workflow run and job, that the commit is still on main, every check in
release.json, and the archive's SHA-256. It unpacks into
/srv/gamenight/play-releases/<12 hex>-<UTC stamp>, carries over the live
godot/ folder (/play/godot/ stays outside releases), switches the
/srv/gamenight/play symlink atomically, checks that the public
/play/version.json serves the new build and switches back if not. It keeps
the newest 3 releases it installed (never the live or previous one) and
never touches other folders in play-releases/ (previews may link there).
Every switch is appended to play-releases/HISTORY.

Run only after Joep's written yes in the prod monitor thread. There is no
timer: unlike /app/, /play/ is promoted by hand.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tarfile
import tempfile
import time
import urllib.request

REPO = 'ontola/warp-game-engine'
WORKFLOW = '.github/workflows/play-release.yml'
ROOT = Path(os.environ.get('GAMENIGHT_SRV', '/srv/gamenight'))
LIVE = ROOT / 'play'
RELEASES = ROOT / 'play-releases'
HISTORY = RELEASES / 'HISTORY'
LOCK = ROOT / '.play-promote.lock'
PUBLIC = os.environ.get('PLAY_PUBLIC_URL', 'https://splitscreen.club/play/')
KEEP = 3
OURS = re.compile(r'[0-9a-f]{12}-\d{14}')
REQUIRED = ['version', 'engine', 'shelf', 'wta-apache',
            'native:theft_auto_flight::two_local_pads_board_the_apache_and_its_light_reaches_ahead_at_altitude',
            'native:theft_auto_apache_combat::both_apache_stations_can_fire_and_switch_pilots_independently']


def get(url, raw=False):
    request = urllib.request.Request(url, headers={'User-Agent': 'splitscreen-club-deployment', 'Accept': 'application/vnd.github+json'})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read() if raw else json.load(response)


def live_version():
    try:
        return json.loads((LIVE / 'version.json').read_text()).get('version')
    except (OSError, ValueError):
        return None


def public_version():
    request = urllib.request.Request(f'{PUBLIC}version.json?check={int(time.time())}', headers={'Cache-Control': 'no-cache'})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response).get('version')


def log(line):
    RELEASES.mkdir(exist_ok=True)
    with HISTORY.open('a') as f:
        f.write(time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()) + ' ' + line + '\n')


def history_targets():
    try:
        return [l.split()[2] for l in HISTORY.read_text().splitlines() if len(l.split()) >= 3 and l.split()[1] in ('baseline', 'promote', 'rollback')]
    except OSError:
        return []


def switch(target):
    """Point /srv/gamenight/play at target in one rename; returns the old target."""
    if LIVE.exists() and not LIVE.is_symlink():
        raise SystemExit(f'{LIVE} is not a symlink: fix that by hand first')
    previous = LIVE.resolve() if LIVE.is_symlink() else None
    link = ROOT / '.play-next'
    link.unlink(missing_ok=True)
    link.symlink_to(target, target_is_directory=True)
    os.replace(link, LIVE)
    return previous


def verify_public(version, previous):
    for attempt in range(6):
        try:
            if public_version() == version:
                return
        except Exception as e:
            err = e
        time.sleep(2)
    if previous:
        switch(previous)
        log(f'restore {previous} public /play/ did not serve {version}')
    raise SystemExit(f'Public {PUBLIC}version.json does not serve {version}; restored {previous}')


def validate(commit, release, manifest, run, jobs, compare):
    if release['draft'] or release['tag_name'] != f'play-{commit[:12]}':
        raise SystemExit('Expected the published candidate release')
    if manifest.get('commit') != commit or manifest.get('repository') != REPO:
        raise SystemExit('release.json names another commit')
    if manifest.get('passed') is not True or not all(c['ok'] for c in manifest['checks']):
        raise SystemExit('Not every release check passed: ' + ', '.join(c['name'] for c in manifest['checks'] if not c['ok']))
    names = {c['name'] for c in manifest['checks']}
    missing = [n for n in REQUIRED if n not in names]
    if missing:
        raise SystemExit(f'release.json lacks required checks: {missing}')
    if (run['path'] != WORKFLOW or run['head_sha'] != commit or run['id'] != manifest.get('run_id')
            or run['event'] not in ('push', 'workflow_dispatch') or run['repository']['full_name'] != REPO):
        raise SystemExit('The candidate did not come from the release workflow on this commit')
    if not any(j['name'] == 'release' and j['conclusion'] == 'success' for j in jobs):
        raise SystemExit('The release job did not finish successfully')
    if compare['status'] not in ('ahead', 'identical'):
        raise SystemExit(f'{commit} is not on main ({compare["status"]})')
    if not re.fullmatch('[a-f0-9]{64}', manifest['archive']['sha256']):
        raise SystemExit('Invalid archive checksum')


def extract(archive, stage):
    with tarfile.open(archive, 'r:gz') as package:
        for member in package.getmembers():
            name = Path(member.name)
            if (name.is_absolute() or '..' in name.parts or name.parts[0] != 'warp-web'
                    or not (member.isfile() or member.isdir())):
                raise SystemExit(f'Unsafe archive entry: {member.name}')
        try:
            package.extractall(stage, filter='data')
        except TypeError:  # Python without extraction filters; members are checked above.
            package.extractall(stage)
    return stage / 'warp-web'


def carry_godot(previous, destination):
    """/play/godot/ (Ballkickers, Frog Fighter) lives outside the releases."""
    if not previous:
        return
    old = previous / 'godot'
    if old.is_symlink():
        (destination / 'godot').symlink_to(os.readlink(old), target_is_directory=True)
    elif old.is_dir() and not (destination / 'godot').exists():
        shutil.copytree(old, destination / 'godot', copy_function=os.link)


def prune(keep):
    ours = sorted((p for p in RELEASES.iterdir() if p.is_dir() and OURS.fullmatch(p.name)), key=lambda p: p.name[13:])
    protect = {LIVE.resolve()} | {Path(t) for t in history_targets()[-2:]} | set(keep)
    for path in ours[:-KEEP]:
        if path.resolve() not in protect:
            shutil.rmtree(path, ignore_errors=True)


def promote(commit, dry):
    if commit == 'latest':
        tags = [r['tag_name'] for r in get(f'https://api.github.com/repos/{REPO}/releases?per_page=50')
                if r['tag_name'].startswith('play-') and not r['draft']]
        if not tags:
            raise SystemExit('No candidate release')
        commit = json.loads(get(f'https://github.com/{REPO}/releases/download/{tags[0]}/release.json', raw=True))['commit']
    if not re.fullmatch('[a-f0-9]{40}', commit):
        raise SystemExit('Give the full 40-hex commit')
    tag = f'play-{commit[:12]}'
    release = get(f'https://api.github.com/repos/{REPO}/releases/tags/{tag}')
    assets = {a['name']: a['browser_download_url'] for a in release['assets']}
    manifest = json.loads(get(assets['release.json'], raw=True))
    run = get(f'https://api.github.com/repos/{REPO}/actions/runs/{manifest["run_id"]}')
    jobs = get(f'https://api.github.com/repos/{REPO}/actions/runs/{manifest["run_id"]}/jobs?per_page=100')['jobs']
    compare = get(f'https://api.github.com/repos/{REPO}/compare/{commit}...main')
    validate(commit, release, manifest, run, jobs, compare)
    print(f'Candidate {tag}: {manifest["subject"]}')
    print(f'  version {manifest["version"]}, {len(manifest["checks"])} checks passed, run {run["html_url"]}')
    print(f'  live now: {live_version()}')
    if live_version() == manifest['version']:
        print('/play/ already serves this build')
        return
    if dry:
        print('Dry run: nothing changed')
        return
    RELEASES.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(dir=RELEASES, prefix='.incoming-') as temporary:
        stage = Path(temporary)
        digest = hashlib.sha256()
        with urllib.request.urlopen(assets['warp-web.tar.gz'], timeout=300) as response, (stage / 'web.tar.gz').open('wb') as out:
            for chunk in iter(lambda: response.read(1 << 20), b''):
                digest.update(chunk)
                out.write(chunk)
        if digest.hexdigest() != manifest['archive']['sha256']:
            raise SystemExit('Archive checksum mismatch')
        web = extract(stage / 'web.tar.gz', stage)
        if json.loads((web / 'version.json').read_text()).get('version') != manifest['version']:
            raise SystemExit('The archive holds another build')
        (web / 'release.json').write_text(json.dumps(manifest, indent=1))
        for path in [web, *web.rglob('*')]:
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() else 0o644)
        destination = RELEASES / f'{commit[:12]}-{time.strftime("%Y%m%d%H%M%S", time.gmtime())}'
        os.rename(web, destination)
    previous = LIVE.resolve() if LIVE.is_symlink() else None
    if previous and not history_targets():
        log(f'baseline {previous} - {live_version()} (live before the first promotion)')
    carry_godot(previous, destination)
    switch(destination)
    log(f'promote {destination} {commit} {manifest["version"]} (was {previous})')
    verify_public(manifest['version'], previous)
    prune([destination])
    print(f'/play/ now serves {manifest["version"]} ({destination})')


def rollback(dry):
    current = LIVE.resolve()
    earlier = [t for t in history_targets() if Path(t) != current and Path(t).is_dir()]
    if not earlier:
        raise SystemExit('No earlier release in HISTORY to go back to')
    target = Path(earlier[-1])
    version = json.loads((target / 'version.json').read_text())['version']
    print(f'Roll back /play/ from {current} to {target} ({version})')
    if dry:
        return
    previous = switch(target)
    log(f'rollback {target} - {version} (was {previous})')
    verify_public(version, previous)


def status():
    print(f'{LIVE} -> {os.readlink(LIVE) if LIVE.is_symlink() else "(not a symlink)"}')
    print(f'live version.json: {live_version()}')
    try:
        print(f'public version.json: {public_version()}')
    except Exception as e:
        print(f'public version.json: {e}')
    try:
        print('HISTORY:\n' + '\n'.join(HISTORY.read_text().splitlines()[-5:]))
    except OSError:
        print('HISTORY: none yet')


if __name__ == '__main__':
    args = [a for a in sys.argv[1:] if a != '--dry-run']
    dry = '--dry-run' in sys.argv
    if len(args) != 1:
        raise SystemExit(__doc__)
    if args[0] == 'status':
        status()
        sys.exit(0)
    ROOT.mkdir(parents=True, exist_ok=True)
    with LOCK.open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        rollback(dry) if args[0] == 'rollback' else promote(args[0], dry)
