-- Dash air whoosh, after sfx.gd _synth_dash (1808-1827).
-- White noise through a one-pole low-pass opening over the dash (Godot k 0.08 -> 0.22,
-- 585 -> 1750 Hz), under a sin^1.6 swell. Fixed 0.16 s.
--
-- Params: none.
-- Game (dash 1039-1040, player.gd:1772): -24 dB, raytraced 3D at the player (room reverb),
--   unit 12 m, SPL 75, big-tail send -24 dB.
length(0.16)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, which levels off near its Nyquist
-- at k / (2 - k); a plain 48 kHz one-pole of the same cutoff keeps falling there (1-2% lower
-- centroid at these large k). Built in (see the Luau prelude): a small direct term and a
-- slightly lower cutoff fit the 44.1 kHz power response over 0-20 kHz to 0.03 dB.

-- lerp(lp, noise, k) with k = lerp(0.08, 0.22, t / dur).
local lp = godot_lp(white44(), expr("lerp(0.08, 0.22, t / 0.16)"))
return expr("x * max(sin(pi * clamp(t / 0.16, 0, 1)), 0) ^ 1.6 * 0.18 * (t < 0.16)", { x = lp })
