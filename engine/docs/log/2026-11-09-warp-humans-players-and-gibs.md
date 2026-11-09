# Seeded match players and spectator debris

On 9 November 2026 we tested a Windows release build based on `a57c2f55` on Mancave (RTX 5070 Ti). Both headless Growing Guns scenarios used `--warp-humans --seed 52 --graphics full --sky noon`.

- The 1600 × 900 player capture placed three bots side by side. The yellow, green and blue shirts identify their slots. Face, skin, hair and body shape come from the same generator as the spectators. The models held their weapons using the match rig. A scripted kill put a bot into the normal death path.
- The 960 × 540 in-stands scenario began with 10,700 spectators. The two successive shots each removed one. The explosion removed another 40, and the scenario passed all its assertions at tick 229.
- The close shots showed that the larger pieces needed more surface variation. Three tapered meshes now share a texture containing a marbled dark-red cut surface and a skin surface. Half the pieces carry one skin side. Their eight possible tints are matched to the seat identity returned with the hit.
- A single death budgets twelve principal pieces. Their summed mesh volume is scaled from a 75 litre reference body. The forty-person event assigned four larger pieces per victim under the 192-piece event ceiling. These captures check appearance and behaviour. They do not measure frame-time impact.

The crowd scenario lives at `games/growing-guns/scripts/crowd-reactions.json`. The player identity and volume checks are in the Growing Guns app library tests.
