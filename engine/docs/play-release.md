# Releasing splitscreen.club/play/

`/play/` (the browser host: the Game Room and its games) changes in one way
only: a checked release candidate of a commit on main, installed on the
host after Joep's written yes. Nobody copies files into `/play/` or
`/srv/gamenight/play-releases/` by hand, from any machine. Previews go to
`/playtests/`.

```
push to main ──► Play release candidate workflow (GitHub, public repo)
                 build-web.py --warp from that commit
                 play-release.py checks (incl. two-seat Apache, crew tests)
                 ──► prerelease play-<12 hex>: warp-web.tar.gz + release.json
Joep: "ja, zet <commit> live" in the prod monitor thread
                 ──► promote-play.py <commit> on the host
                     checks release, run, job, main, every check, SHA-256
                     /srv/gamenight/play ──► play-releases/<12 hex>-<stamp>
                     public /play/version.json checked, else switched back
```

## 1. Candidates

`.github/workflows/play-release.yml` runs on every push to main that
touches `engine/`, `games/`, `lobbies/` or the Cargo files (a newer push
cancels a candidate still building), and by hand (Actions, *Play release
candidate*, *Run workflow*, optionally with a commit on main). It runs
`python3 engine/tools/play-release.py build --native-tests` on a clean
checkout. When every check passes it publishes the prerelease
`play-<first 12 hex of the commit>` with:

- `warp-web.tar.gz`: build/web under one top folder `warp-web/`
- `warp-web.tar.gz.sha256`
- `release.json`: the commit, the build version (`<short sha>-<UTC stamp>`,
  as in `/play/version.json`), the run id, every check and the archive's
  SHA-256

A failed check publishes nothing; the run's summary lists the checks. The
newest 15 candidates are kept. The repository is public, so the host
downloads them without a token.

Release checks (`play-release.py`, each a line in `release.json`):

| check | what it proves |
| --- | --- |
| `version` | version.json and index.html name this commit |
| `engine` | the wasm, its JavaScript, the baked room light, web-catalog.js and game-worker.js are there |
| `shelf` | games/library.json has every default game (WARP_GAMES and RUST_GAMES in build-web.py) |
| `files:<game>` | each Luau game ships exactly its tracked folder at this commit, byte for byte (an overlaid or left-out file fails) |
| `wta-apache` | Warp Theft Auto has the two-seat Apache: the eight files of `assets/apache`, each glTF's buffers, the gunner optic (`apache_hud`), the airframe and seat mounts in `vehicles`, pilot and gunner seats in `instruments` |
| `native:…` | the Warp Theft Auto crew tests: two pads board one Apache as pilot and gunner, the gunner optic holds its bearing, both stations fire, the car-then-helicopter handover |

The 26 November regression (release `56346853…`: the Apache built from
primitives, the meshes and optic missing) fails `wta-apache`. A game that
loses the Apache on purpose must change this check in the same commit.

Run the checks yourself before pushing:

```sh
python3 engine/tools/build-web.py --warp
python3 engine/tools/play-release.py check build/web
```

## 2. Going live

Only the prod monitor thread (Remote Control on the host) installs, and only
after Joep names the commit (or "de nieuwste kandidaat") in that thread:

```sh
python3 /opt/gamenight-cloud/promote-play.py <commit> --dry-run   # what would change
python3 /opt/gamenight-cloud/promote-play.py <commit>
python3 /opt/gamenight-cloud/promote-play.py status
python3 /opt/gamenight-cloud/promote-play.py rollback
```

`latest` instead of a commit takes the newest candidate. The script lives in
gamenight-internal `deploy/cloud/promote-play.py`; see its docstring for
every check. It keeps the godot/ folder (`/play/godot/`) across releases,
keeps its last 3 installs, and logs every switch in
`/srv/gamenight/play-releases/HISTORY`.

## 3. Previews

Build, test and push to main as usual. To let someone try a build before it
is a candidate, put it under `/playtests/GAME/NAME/`. The preview key
(`gamenight_deploy`) is restricted on the host to
`playtest-receive.py` (gamenight-internal `deploy/cloud`), which writes
only below `/srv/gamenight/playtests/`:

```sh
# a whole build folder
tar -czf - -C build/web . | ssh -i KEY root@91.99.156.221 put warp-theft-auto/apache-3
# a few changed files on top of the live /play/ (hard links, the live release is untouched)
tar -czf - -C changes . | ssh -i KEY root@91.99.156.221 overlay warp-theft-auto/apache-3
# rsync into playtests/ (paths are relative to it)
rsync -rtz --chmod=D755,F644 -e "ssh -i KEY" build/web/ root@91.99.156.221:warp-theft-auto/apache-3/
ssh -i KEY root@91.99.156.221 list warp-theft-auto
```

The preview opens at `https://splitscreen.club/playtests/warp-theft-auto/apache-3/`.
A preview that should go live goes to main, becomes a candidate and is
promoted as above.
