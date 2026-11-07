# Parenting in four games, 7 November 2026

Reference checkout: `96bf388`. The comparison uses the same software Vulkan
adapter (Mesa llvmpipe, LLVM 21.1.8) under WSL and 480 × 270 output on both
sides. The reference includes the same two rendering repairs: Spellstorm's
surface shader reads scene data through `aux_word`, and the transparent
view layout uses the ten bindings supplied by the renderer. Without these
repairs the reference exits on GPU validation before drawing Spellstorm.

## Scenarios

| Game | Reproduction |
| --- | --- |
| Spellstorm | `spellstorm shot --size 480x270 --seed 7 --ticks 24 --frames 1 --enchant fire --out spell.png` |
| Growing Guns | `growing-guns shot --size 480x270 --views 2 --seconds 1 --out guns.png` |
| Sideways | `sideways shot car.png --size 480x270 --frames 120 --km 1 --at 100 --taa 1` |
| Neon Trails | Run `warp script games/neon-trails` with the game's smoke script at 480 × 270, with an additional capture just after the first turn (tick 14). The second capture and state dump follow at tick 420. |

All five PNGs matched byte for byte. Their SHA-256 prefixes were
`58c870474317d0ce` (Spellstorm), `5c20d06d938b661f` (Growing Guns),
`74e92aa783ea4bb0` (Sideways), `580f142c7dc0232e` (Neon Trails turn) and
`2040089b6c016f53` (Neon Trails tick 420). The seven-second Neon Trails
audio captures also matched. The final state JSON matched after dropping the
old per-rider shell IDs. These are fixed-scene checks on software Vulkan.

## Complexity

Net change in nonblank, noncomment production lines in the edited gameplay
files, excluding tests, type declarations and shader compatibility fixes.
Line count supplements the comparison of what each game has to maintain.

| Game | Lines | Change in responsibility |
| --- | ---: | --- |
| Spellstorm | −38 | Rest and animated poses use one validated hierarchy. Staff light and orbs borrow one socket-position list. Each used to own a copy. |
| Sideways | −5 | Creation and movement use the same ordered list of body and suspension parts. Steering, spin and compression remain in the game. |
| Growing Guns | −4 | Both views use the renderer's shared child-slot writer. Recoil, the pose of each view, and the muzzle calculation remain in the game. |
| Neon Trails | +4 | One placement per moving bike. First placement also adopts a shell from an older hot-reloaded round and preserves the old spawn pose. |

The Luau integration test exercises three generations, a rotating parent,
world-space queries and movement, failed edits, cycle rejection, detachment
and removal. The Sideways regression checks GPU instance rows and the
solid-body flag at creation and after an update, including a missing wheel.
