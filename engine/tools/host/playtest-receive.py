#!/usr/bin/env python3
"""Forced SSH command for the preview key (gamenight_deploy, used by the
Mancave sessions): it can write previews under /srv/gamenight/playtests/
and nothing else. /play/ only changes through promote-play.py.

authorized_keys (one line, root):
    restrict,command="/usr/bin/python3 -I /opt/gamenight-cloud/playtest-receive.py" ssh-ed25519 AAAA... gamenight_deploy

Commands (SSH_ORIGINAL_COMMAND):
    list [GAME]                 the previews (or one game's)
    put GAME/NAME[/...]         a .tar.gz on stdin becomes that preview, in one rename
    overlay GAME/NAME[/...]     the live /play/ release (hard links) plus the
                                .tar.gz on stdin on top: a preview of a few changed files
    rsync --server ...          rsync into playtests/ (needs rrsync), e.g.
                                rsync -rtz --chmod=D755,F644 build/ root@host:warp-theft-auto/v7/
Previews open at https://splitscreen.club/playtests/GAME/NAME/.
An existing preview of that name is replaced (one rename, the old one removed).
The archive holds the files at its top level or under one top folder;
only plain files and folders, at most 2 GB.
"""
import os
from pathlib import Path
import re
import shlex
import shutil
import sys
import tarfile
import tempfile
import time

ROOT = Path(os.environ.get('GAMENIGHT_SRV', '/srv/gamenight'))
PLAYTESTS = ROOT / 'playtests'
LIVE = ROOT / 'play'
NAME = re.compile(r'[a-z0-9][a-z0-9._-]{0,63}(/[A-Za-z0-9][A-Za-z0-9._-]{0,63}){1,3}')
LIMIT = 2 << 30


def fail(message, code=64):
    print(message, file=sys.stderr)
    sys.exit(code)


def target(name):
    if not NAME.fullmatch(name) or '..' in name.split('/'):
        fail('Preview path: GAME/NAME (lowercase game id, up to 4 parts, letters, digits, . _ -)')
    path = PLAYTESTS / name
    if not path.parent.resolve().is_relative_to(PLAYTESTS.resolve()) and path.parent.exists():
        fail('Preview path leaves playtests/')
    return path


def receive(stage):
    archive = stage / 'in.tar.gz'
    size = 0
    with archive.open('wb') as out:
        for chunk in iter(lambda: sys.stdin.buffer.read(1 << 20), b''):
            size += len(chunk)
            if size > LIMIT:
                fail('Archive over 2 GB')
            out.write(chunk)
    files = stage / 'files'
    files.mkdir()
    with tarfile.open(archive, 'r:*') as package:
        members = package.getmembers()
        for m in members:
            p = Path(m.name)
            if p.is_absolute() or '..' in p.parts or not (m.isfile() or m.isdir()):
                fail(f'Unsafe archive entry: {m.name}')
        try:
            package.extractall(files, filter='data')
        except TypeError:
            package.extractall(files)
    archive.unlink()
    top = [p for p in files.iterdir()]
    root = top[0] if len(top) == 1 and top[0].is_dir() and not (files / 'index.html').exists() else files
    for p in [root, *root.rglob('*')]:
        p.chmod(0o755 if p.is_dir() else 0o644)
    return root


def place(new, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    final = path.parent / f'.{path.name}.new-{os.getpid()}'
    os.rename(new, final)
    old = None
    if path.is_symlink():
        path.unlink()
    elif path.exists():
        old = path.parent / f'.{path.name}.old-{int(time.time())}'
        os.rename(path, old)
    os.rename(final, path)
    if old:
        shutil.rmtree(old, ignore_errors=True)
    print(f'https://splitscreen.club/playtests/{path.relative_to(PLAYTESTS).as_posix()}/')


def overlay(changes, base, into):
    """into = hard links of base; each changed file replaces its link (never
    written through it, so the live release stays untouched)."""
    shutil.copytree(base, into, copy_function=os.link, symlinks=True)
    for src in changes.rglob('*'):
        rel = src.relative_to(changes)
        dst = into / rel
        if src.is_dir():
            dst.mkdir(parents=True, exist_ok=True)
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        if dst.exists() or dst.is_symlink():
            dst.unlink()
        os.rename(src, dst)
    (into / 'release.json').unlink(missing_ok=True)


def main():
    words = shlex.split(os.environ.get('SSH_ORIGINAL_COMMAND', ''))
    if not words:
        fail(__doc__)
    PLAYTESTS.mkdir(parents=True, exist_ok=True)
    if words[0] == 'rsync':
        if words[1:2] != ['--server']:
            fail('Only rsync --server')
        rrsync = '/usr/bin/rrsync' if os.access('/usr/bin/rrsync', os.X_OK) else (shutil.which('rrsync') or '')
        if not os.access(rrsync, os.X_OK):
            fail('rrsync is not installed on the host; use put')
        os.execv(rrsync, [rrsync, '-no-del', str(PLAYTESTS)])
    if words[0] == 'list' and len(words) <= 2:
        base = target(words[1] + '/x').parent if len(words) == 2 else PLAYTESTS
        for p in sorted(base.rglob('index.html')):
            print(p.parent.relative_to(PLAYTESTS).as_posix())
        return
    if words[0] in ('put', 'overlay') and len(words) == 2:
        path = target(words[1])
        with tempfile.TemporaryDirectory(dir=PLAYTESTS, prefix='.incoming-') as temporary:
            stage = Path(temporary)
            changes = receive(stage)
            if words[0] == 'put':
                if not (changes / 'index.html').is_file():
                    print('note: the preview has no index.html at its top', file=sys.stderr)
                place(changes, path)
            else:
                overlay(changes, LIVE.resolve(), stage / 'preview')
                place(stage / 'preview', path)
        return
    fail(__doc__)


if __name__ == '__main__':
    main()
