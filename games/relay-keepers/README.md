# Relay Keepers

A shared-screen Warp platformer for two people. Move with the stick or D-pad and jump with the south face button. A wing carrier can press jump again in the air. The two lights on an exit belong to the two players. Falls return the player to the room entrance. Back/Select is reserved for GameNight.

Run with `warp run .` or start the game from a two-player GameNight party. After the final room, press jump to play again.

## Editing the campaign

`rooms.module.luau` contains all 14 hand-built rooms in play order. Coordinates are measured in tiles, with the origin at the lower left. Each platform has a left edge, a right edge, a top height and an optional material. The two start positions are the players' feet. Each room enables its own objects by placing them. The return value is ordinary Luau data.

`main.luau` handles jumping, stacking, movable cargo, switches, time shifts and room progression. `view.module.luau` draws the scene with Warp primitives. Edit with `warp run . --live`; restart after changing a room layout.

Run `python scripts/campaign.py path/to/warp.exe` after an edit. Each room has a two-controller solution with expected landings in the test. The test loads a temporary copy in Warp, walks and jumps through the entire campaign and checks that both players finish with zero falls and no script errors. For a render check run `warp script . scripts/capture.json --out captures`.
