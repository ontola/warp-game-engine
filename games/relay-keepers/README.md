# Relay Keepers

A shared-screen Warp platformer for two people. Move with the stick or D-pad and jump with the south face button. A wing carrier can press jump again in the air. On a striped spring deck, jump within 0.23 seconds of each other for a shared launch. Steer independently in the air. The two lights on an exit belong to the two players. Falls return the player to the room entrance. Back/Select is reserved for GameNight.

Run with `warp run .` or start the game from a two-player GameNight party. Press the north face button (Y on Xbox) to open the wordless room map. Move with the stick or D-pad, confirm with jump, or press north again to return. All 91 rooms are available immediately. Shoulder buttons move 56 tiles. From the opening room, Y, right shoulder and A enter the first ferry room. The first expansion room is the first tile of the third row. After the final room, press jump to play again.

## Editing the campaign

`rooms.module.luau` contains the opening 14 rooms. It appends eleven seven-room districts in order: Drydock, Switchyard, Aviary, Almanac, Foundry, Summit, Narrows, Winding House, Springline, Skyport and Waystations. Each district has its own hand-authored Luau module. Coordinates are measured in tiles, with the origin at the lower left. Each platform has a left edge, a right edge, a top height and an optional material. A numbered material becomes solid after the corresponding pair of relay contacts has been occupied. Each entry in `links` specifies two foot positions that both players must occupy simultaneously, in either order. The two start positions are the players' feet. Each room enables its own objects by placing them. The return value is ordinary Luau data.

`main.luau` handles jumping, stacking, movable cargo, switches, time shifts and room progression. `view.module.luau` draws the scene with Warp primitives. Edit with `warp run . --live`; restart after changing a room layout.

Run `python scripts/campaign.py path/to/warp.exe` after an edit. Each room has a two-controller solution with expected landings in the test. The test loads a temporary copy in Warp, walks and jumps through the entire campaign and checks that both players finish with zero falls and no script errors. For a render check run `warp script . scripts/capture.json --out captures`.

Ferries are authored with `ferry={from={x,y},to={x,y}}`. Set `control={x,y}` for a remote wheel and `coil=true` for a spring deck. A tandem deck leaves when both players board, waits at the far terminal, then returns. After its first crossing it can pick up a lone partner. A wheel drives the carriage out while occupied and brings it home when released. Falling in a ferry room reboards both players at the entrance and preserves items and completed relays.

Run `warp script . scripts/crossings.json --out captures` for the controller, recovery, shared-launch and scrolling-map checks.
