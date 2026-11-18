# Human lab

Eight seeded Warp Humans in a row, seen by fixed portrait cameras in the full
renderer: the place to judge faces, skin, hair, clothes and posture up close.
Run it with `warp run games/human-lab`.

- **Left / right** pick a person, **up / down** a view (face, three-quarter,
  profile, bust, body, row), **A / Space** turns the person a quarter.
- From the console or a script: `state.walk = 1.4` walks everyone ahead at
  that speed (m/s), `state.pose = "wave"` sets a pose.

The people come from `[humans.person]` in game.toml: change `seed` for
another eight, `sky` for other light.

Repeatable captures, headless:

```sh
warp script games/human-lab games/human-lab/scripts/portraits.json --out build/human-lab/portraits
warp script games/human-lab games/human-lab/scripts/moving.json --out build/human-lab/moving
```

`portraits.json` saves the row, six views of the first person and every
face; `moving.json` a walk, a jog and a few poses. Run both before and after
a change to people and compare the images.
