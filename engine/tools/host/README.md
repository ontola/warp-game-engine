# Host scripts for /play/ and /playtests/

Installed by the prod monitor in `/opt/gamenight-cloud/` on the Splitscreen
Club host (root, 0755). They live in this public repository so the host can
fetch them by commit without a token:

```sh
curl -fsSLo promote-play.py https://raw.githubusercontent.com/ontola/warp-game-engine/COMMIT/engine/tools/host/promote-play.py
```

- `promote-play.py`: the only way `/play/` changes (a checked candidate
  `play-<12 hex>`, after Joep's yes). See [../../docs/play-release.md](../../docs/play-release.md).
- `playtest-receive.py`: forced command of the preview key; writes only
  below `/srv/gamenight/playtests/`.
