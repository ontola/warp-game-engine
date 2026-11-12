# SpaceRacer rule calls: where a sim tick went

2026-11-11, on top of main at 29b5711. Machine: cloud VM (4 cores, Xeon,
Linux), release build, headless.

Scene: `Race::new(7, "normal", "", 1, 3)`, one idle player and five AI
ships, 240 ticks warm-up, then 7,200 ticks (60 s at 120 Hz). Rule calls
timed by wrapping `Rules::call` (instrumentation not committed).

Before:

| | µs per tick |
| --- | ---: |
| Whole `Race::step` | 372 |
| `ai` rule (5 calls, 56 µs each) | 281 |
| every other rule together | 2 |

Inside one `ai` call: `to_value` of the argument object 38 µs, the Luau
function 7.7 µs, `from_value` of the result 4.2 µs, `Host::each` 0.8 µs.

The argument was big (the whole `Pilot` struct), but sending only the 11
pilot fields `ai` reads saved just 20%. The real cost was the memory cap:
with `set_memory_limit` set, mlua guards every allocating API call, and a
`raw_set` costs about 400 ns instead of 90 ns. A 15-field nested
`to_value` costs 8.8 µs capped and 4.8 µs uncapped.

Change: `script::unguarded` / `Host::unguarded` lift the cap while Rust
converts values (no Luau runs then), and `Rules::call` converts through
it; `ai` gets the slim pilot.

After, same scene, alternating binaries three times:

| | µs per tick |
| --- | ---: |
| Before | 365, 355, 346 |
| After | 194, 211, 190 |

The state hash after 7,200 ticks is identical (30509787683471500), so the
race plays out the same.
