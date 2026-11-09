# Startled spectator gestures

9 November 2026, Windows. Compared the procedural crowd with `3cd3cb4c`.
The old fear pose lowered the body and held both arms forward. The new
shoulder and elbow targets let a spectator guard with one arm or reach
overhead with both. Each arm gets its own rate, short hold and seeded
pulse strength. Chest and neck turn slightly with the reach. Local row
displacement and the stadium's smoothed fear envelope control the blend.
The existing visible-seat dispatch and 48-byte pose record carry the result.

A CPU software-renderer check evaluated the production joint shader for
64 identities over four seconds at 120 samples per second. At full fear,
34 identities simultaneously raised both shoulders past 2.15 radians at
some point. For 29, one shoulder passed 2.15 radians while the other
remained below 1.65. The largest frame-to-frame shoulder change was
0.081 radians or less. All 30,720 calm samples matched the baseline.

The 960 × 540 animation preview uses the Microsoft Basic Render Driver
and a staged rise in fear on the same steps as the previous motion clip.
It uses the game's mesh and joint shader. Hardware frame cost was not measured.
