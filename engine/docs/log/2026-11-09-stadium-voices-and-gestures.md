# Stadium voices and spectator gestures

9 November 2026. Windows desktop, baseline `72f356bf`.

In the previous playtest a missing external recording directory also
disabled the synthesised murmur and applause. The desktop game now carries
20 Ogg recordings (2,204,562 compressed bytes), with their source credits
in the game's crowd asset folder. A valid external bank takes precedence.
The beds and one-shots use the existing crowd mixer and replay audio bank.

A CPU-only render of the bundled bank at 48 kHz stereo measured
−40.1 dBFS RMS in the quiet interval (seconds 1–3), −38.3 dBFS at kickoff
(3–6) and −26.7 dBFS during celebration (6–10). The preview contains
the audience channel alone.

The GPU detail selection computes eight angles per visible seat. Both
shoulders and elbows have independent angles. Spine roll and twist carry
the shoulders, neck and head. The head turns and nods around the neck.
Raising an arm lifts its collarbone. All three representations follow
the same phases and the live crowd mood. The visible-seat record grows
from 16 to 32 bytes, with the same dispatches and draws. The shader was
validated on the CPU. Visual motion and frame cost were not measured.
