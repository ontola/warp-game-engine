# Stack Together

Two minds, one stack. One twelve-column well: each player owns six columns
and drops their own pieces there, and rows only clear when both halves are
full. Clear 12 rows together to win; the round is lost when a stack reaches
the top. Y swaps your falling piece with the one slot both players share, so
you can pass a piece to your teammate (once per piece).

A Luau game on the `warp` runtime, ported from the LÖVE party pack
(`ontola/gamenight-games`, `love-party/games/coop/stack.lua` and
`stack_view.lua`): the same well, pieces, bag, rotation kicks, fall speed,
shared hold slot and bot, in a 3D well with one shared camera. Same id
(`stack-together`), so it replaces the LÖVE entry once registered.

Controls: stick or d-pad moves, A (Space) rotates, B (Shift) hard-drops,
Y (F) holds, down falls faster. After a round, A starts the next one.

```sh
warp run . --live                   # you and a bot; edit main.luau while playing
warp run . --players 2 --bots 0     # two people
warp check . --ticks 3000           # bots play without a GPU
warp script . scripts/smoke.json --out /tmp/stack
```

Tuning lives in the `-- @knobs` block at the top of main.luau (target rows,
speed, fall times, repeat delays, bot patience, glow): `knob speed 130` in
the console or `set_knob` over MCP changes it live.

Not ported yet: the party settings (target rows and speed are knobs until
Luau games can declare Splitscreen Club settings) and the players' drawn faces
(here a round head in the player's colour with eyes that follow its piece).
