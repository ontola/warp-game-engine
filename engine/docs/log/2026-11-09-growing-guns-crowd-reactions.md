# Repeated crowd shots and blast reactions

Tested on 9 November 2026 from 4b3b1181 with the reaction changes, Vulkan,
RTX 5070 Ti, full graphics, noon, packed seating, seed 52. The headless game
starts on the tread facing a neighbour. Two individual trigger pulls and
one grenade run in both one-player and two-player layouts at 960 × 540.

| Recorded state | Tick | Living spectators |
| --- | ---: | ---: |
| Before firing | 8 | 10,700 |
| After the first shot | 16 | 10,699 |
| After the second shot | 47 | 10,698 |
| After the explosion | 189 | 10,658 |
| Forty ticks later | 229 | 10,658 |

The shader regression reads back the actual animated body transforms on the
GPU. It checks position continuity at a second shot, a retrigger during the
return, eight unrelated scares, a shot at a vacated seat, a shot at the
displaced body, repeated segments from one projectile, overlapping blasts,
and revival on reset.

The simulation supplies collision-clipped segments and explosions. Event
ordering and stable projectile ids carry through the GPU selection, the death
mask, and the per-spectator flight trajectory. A game-side contact receipt
requests GPU debris and mist and places persistent marks. Receipts are
asynchronous in the match. Earlier recordings still accept the original
untracked bullet segment. New network matches use protocol 7 on both peers.

Reproduce the match using [the crowd scenario](../../../games/growing-guns/scripts/crowd-reactions.json)
and [the launch arguments](../warp-humans.md). Run the GPU regression with
`cargo test --release -p render --lib crowd:: -- --include-ignored --test-threads=1`.
