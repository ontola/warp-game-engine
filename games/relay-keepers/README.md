# Relay Keepers

A shared-screen Warp platformer for two people. Move with the stick or D-pad and jump with the south face button. A wing carrier can press jump again in the air. On a striped spring deck, jump within 0.23 seconds of each other for a shared launch. Steer independently in the air. The two lights on an exit belong to the two players. Falls return the player to the room entrance. Back/Select is reserved for GameNight.

Run with `warp run .` or start the game from a two-player GameNight party. Press the north face button (Y on Xbox) to open the wordless room map. Move with the stick or D-pad, confirm with jump, or press north again to return. All 91 rooms are available immediately. Shoulder buttons move 56 tiles. From the opening room, Y, right shoulder and A enter the first ferry room. The first expansion room is the first tile of the third row. After the final room, press jump to play again.

## Editing the campaign

`rooms.module.luau` contains the opening 14 rooms. It appends eleven seven-room districts in order: Drydock, Switchyard, Aviary, Almanac, Foundry, Summit, Narrows, Winding House, Springline, Skyport and Waystations. Each district has its own hand-authored Luau module. Coordinates are measured in tiles, with the origin at the lower left. Each platform has a left edge, a right edge, a top height and an optional material. A numbered material becomes solid after the corresponding pair of relay contacts has been occupied. Each entry in `links` specifies two foot positions that both players must occupy simultaneously, in either order. The two start positions are the players' feet. Each room enables its own objects by placing them. The return value is ordinary Luau data.

`main.luau` handles jumping, stacking, movable cargo, switches, time shifts and room progression. `view.module.luau` draws the scene with Warp primitives. Edit with `warp run . --live`; restart after changing a room layout.

Run `python scripts/campaign.py path/to/warp.exe` after an edit. Each room has a two-controller solution with expected landings in the test. The test loads a temporary copy in Warp, walks and jumps through the entire campaign and checks that both players finish with zero falls and no script errors. For a render check run `warp script . scripts/capture.json --out captures`.

Ferries are authored with `ferry={from={x,y},to={x,y}}`. Set `control={x,y}` for a remote wheel and `coil=true` for a spring deck. A tandem deck leaves when both players board, waits at the far terminal, then returns. After its first crossing it can pick up a lone partner. A wheel drives the carriage out while occupied and brings it home when released. Falling in a ferry room reboards both players at the entrance and preserves items and completed relays.

Run `warp script . scripts/crossings.json --out captures` for the controller, recovery, shared-launch and scrolling-map checks.

## Two-player live playtest

When both players are ready, send **Start live playtest** in the Codex chat that is making the edits. The assistant can launch the desk and monitor its feedback queue for the duration of the session. To open the desk yourself on Windows, run `scripts/Live Playtest.cmd`. Choose **Open GameNight**, join with both controllers and launch Relay Keepers. The desk also supports a standalone window with keyboard and one controller.

Clearing a room holds the scene. Either player can press **RB** after a clear to record a good result and advance. For an issue, press **X** at any time to freeze the room, then Alt+Tab to the playtest desk. Select **Issue** or **Blocked**, describe what happened and choose **Save note**. Each note records the room, number of falls and clear status. Watch the update line, return to the game after selecting **Retry room** and test the change. **Resume** continues an unfinished attempt. **Next room** on an unfinished room records a skip. The checkpoint reopens the same room at its entrance after a full restart.

The queue and checkpoint are stored locally at `%LOCALAPPDATA%/Relay Keepers/Playtest/session.sqlite3`. Notes saved while Codex is away stay pending. The live session needs this computer and the chat to stay available.

### Running the feedback loop

From the installed game's scripts folder, launch `pythonw playtest.py panel` once when requested. While the players are testing, call `python playtest.py pending --seconds 45` repeatedly. It returns pending feedback immediately when a player saves a note. A timeout is an idle session. Keep monitoring until the players stop, finish the review, or ask to pause monitoring. At the last room, handle remaining notes before wrapping up. `python playtest.py report` lists the checkpoint, clear and skip decisions, retries, and all notes with replies.

For an actionable note, inspect the named room in the installed Luau source. Make the targeted change and run `python playtest.py reload`; check the live error list and current room. Update and run the relevant solution test for a gameplay change. Report the concrete result to the players with `python playtest.py respond NOTE_ID "what changed and what to retry"`. Acknowledge good results and ask for clarification on ambiguous notes in the same way. The players decide when to retry or advance. Preserve tested changes in the repository as well as the installed copy.
