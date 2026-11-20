# Relay Keepers

A shared-screen Warp platformer for two people. Move with the stick or D-pad and jump with the south face button. A wing carrier can press jump again in the air. The two lights on an exit belong to the two players. Falls return the player to the room entrance. Back/Select is reserved for GameNight.

Run with `warp run .` or start the game from a two-player GameNight party. Press the north face button (Y on Xbox) to open the wordless room map. Move with the stick or D-pad, confirm with jump, or press north again to return. All 56 rooms are available immediately. The first expansion room is the first tile of the third row. After the final room, press jump to play again.

## Editing the campaign

`rooms.module.luau` contains the opening 14 rooms. It appends six seven-room districts in order: Drydock, Switchyard, Aviary, Almanac, Foundry and Summit. Each district has its own hand-authored Luau module. Coordinates are measured in tiles, with the origin at the lower left. Each platform has a left edge, a right edge, a top height and an optional material. A numbered material becomes solid after the corresponding pair of relay contacts has been occupied. Each entry in `links` specifies two foot positions that both players must occupy simultaneously, in either order. The two start positions are the players' feet. Each room enables its own objects by placing them. The return value is ordinary Luau data.

`main.luau` handles jumping, stacking, movable cargo, switches, time shifts and room progression. `view.module.luau` draws the scene with Warp primitives. Edit with `warp run . --live`; restart after changing a room layout.

Run `python scripts/campaign.py path/to/warp.exe` after an edit. Each room has a two-controller solution with expected landings in the test. The test loads a temporary copy in Warp, walks and jumps through the entire campaign and checks that both players finish with zero falls and no script errors. For a render check run `warp script . scripts/capture.json --out captures`.
