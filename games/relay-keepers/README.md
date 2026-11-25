# Relay Keepers

A shared-screen Warp platformer for two people. Move with the stick or D-pad. Tap the south face button for a short hop or hold it for a high jump. Press just before landing to jump again on touchdown, including on a partner’s head. A wing carrier can press and hold jump again in the air. On a striped spring deck, jump within 0.23 seconds of each other for a shared launch. Steer independently in the air. Checkered flags mark the exits. A shared finish has two landing lights. Split flags need one keeper each. Falls return the player to the room entrance. Back/Select is reserved for GameNight.

Run with `warp run .` or start the game from a two-player GameNight party. Press the north face button (Y on Xbox) to open the wordless atlas. The little harbor chart folds away as either keeper moves. Until the atlas is first opened, a controller beside the chart shows the button when both keepers stand still. Move with the stick or D-pad, then use the bottom face button to enter the selected room. Press the top button again to close the map. All rooms are available immediately. Each row has seven rooms. The shoulder buttons move eight rows at once. From the opening room, Y, right shoulder and A enter the first ferry room. After the final room, press jump to play again.

The campaign climbs fast: five opening rooms, then the four hardest rooms of each district. Right after the aviary comes the hourglass: stepping onto it sends that keeper five seconds back in time. They reappear where they stood five seconds ago while their echo replays what they did, holding plates and lending a head to stand on, until it walks back into the hourglass. Counterpoise introduces bridges powered by a keeper riding on their partner. In a pool, hold down to dive and release to float up. Standing on a sluice raises the water. Spring off a crab's shell from above. A keeper or a crate on the bell stills the patrol and retracts its claws. Walk into a ring to travel to its matching ring. Move clear of the destination to use it again.

## Editing the campaign

`rooms.module.luau` contains the opening rooms and registers the districts. It picks the campaign (the `OPENING` rooms, then the last four rooms of each district in `ORDER`) and gives each room its `index` in the full authored list, which `scripts/campaign.py` uses to find its route. Rooms with the retired past/future `clock` are skipped. Each district has its own hand-authored Luau module. Coordinates are measured in tiles, with the origin at the lower left. Each platform has a left edge, a right edge, a top height and an optional material. A numbered material becomes solid after the corresponding pair of relay contacts has been occupied. Each entry in `links` specifies two foot positions that both players must occupy simultaneously, in either order. The two start positions are the players' feet.

| Room field | Behavior |
| --- | --- |
| `land={{left,right,height,"balance"}}` | The bridge supports a stacked pair and holds for half a second after they separate. |
| `water={left=4,right=16,low=0.1,current=0.4}` | A buoyant surface and horizontal current. Add `control={x,y},high=2.1` for a partner-operated sluice and a platform below the surface for a pool bed. |
| `crabs={{from=5,to=7,y=0,speed=0.7,phase=0.5}}` | A shell patrols the authored stretch. An optional `bell={x,y}` controls every crab in the room. |
| `portals={{{4,0},{16,3}}}` | Two grounded entrances form one bidirectional pair. Place up to three pairs, each identified by its color and number of lights. |

`main.luau` handles jumping, stacking, movable cargo, switches, the echo hourglass (`hourglass={x,y}`) and room progression. After a clear both keepers keep moving while the room celebrates. `view.module.luau` composes the keepers, material-specific decks, painted coastlines and atlas from Warp primitives. Edit with `warp run . --live` and retry the room after changing its layout.

Run `python scripts/campaign.py path/to/warp.exe` to play the complete campaign with two virtual controllers. Each room has a solution with expected landings, the natural clear condition and zero falls. Add `--room 155` to replay one room while iterating. Run `python scripts/test_jump.py path/to/warp.exe` to measure tap, medium and held jumps and exercise edge and landing timing. Run `python scripts/test_mechanics.py path/to/warp.exe` to exercise patrol damage, both bell controls, bridge release, portal return and buoyancy. For a render check run `warp script . scripts/capture.json --out captures`.

Ferries are authored with `ferry={from={x,y},to={x,y}}`. Set `control={x,y}` for a remote wheel and `coil=true` for a spring deck. A tandem deck leaves when both players board, waits at the far terminal, then returns. After its first crossing it can pick up a lone partner. A wheel drives the carriage out while occupied and brings it home when released. Falling in a ferry room reboards both players at the entrance and preserves items and completed relays.

Run `warp script . scripts/crossings.json --out captures` for the controller, recovery, shared-launch and scrolling-map checks.

## Two-player live playtest

When both players are ready, send **Start live playtest** in the Codex chat that is making the edits. The assistant can launch the desk and monitor its feedback queue for the duration of the session. To open the desk yourself on Windows, run `scripts/Live Playtest.cmd`. Choose **Open GameNight**, join with both controllers and launch Relay Keepers. The desk also supports a standalone window with keyboard and one controller.

Clearing a room holds the scene. Either player can press **RB** after a clear to record a good result and advance. For an issue, press **X** at any time to freeze the room, then Alt+Tab to the playtest desk. Select **Issue** or **Blocked**, describe what happened and choose **Save note**. Each note records the room, number of falls and clear status. Watch the update line, return to the game after selecting **Retry room** and test the change. **Resume** continues an unfinished attempt. **Next room** on an unfinished room records a skip. The checkpoint reopens the same room at its entrance after a full restart.

The queue and checkpoint are stored locally at `%LOCALAPPDATA%/Relay Keepers/Playtest/session.sqlite3`. Notes saved while Codex is away stay pending. If you finished the campaign before more rooms were added, the desk opens the first new room and keeps your feedback. The live session needs this computer and the chat to stay available.

### Running the feedback loop

From the installed game's scripts folder, launch `pythonw playtest.py panel` once when requested. While the players are testing, call `python playtest.py pending --seconds 45` repeatedly. It returns pending feedback immediately when a player saves a note. A timeout is an idle session. Keep monitoring until the players stop, finish the review, or ask to pause monitoring. At the last room, handle remaining notes before wrapping up. `python playtest.py report` lists the checkpoint, clear and skip decisions, retries, and all notes with replies.

For an actionable note, inspect the named room in the installed Luau source. Make the targeted change and run `python playtest.py reload`; check the live error list and current room. Update and run the relevant solution test for a gameplay change. Report the concrete result to the players with `python playtest.py respond NOTE_ID "what changed and what to retry"`. Acknowledge good results and ask for clarification on ambiguous notes in the same way. The players decide when to retry or advance. Preserve tested changes in the repository as well as the installed copy.
