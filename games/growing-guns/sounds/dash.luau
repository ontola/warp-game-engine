-- Dash air whoosh, after sfx.gd _synth_dash (1808-1827).
-- White noise through a one-pole low-pass opening over the dash (Godot k 0.08 -> 0.22,
-- 585 -> 1750 Hz), under a sin^1.6 swell. Fixed 0.16 s.
--
-- Params: none.
-- Game (dash 1039-1040, player.gd:1772): -24 dB, raytraced 3D at the player (room reverb),
--   unit 12 m, SPL 75, big-tail send -24 dB.
length(0.16)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

-- lerp(lp, noise, k) with k = lerp(0.08, 0.22, t / dur), as its exact 44.1 kHz cutoff.
local hz = expr("-log(1 - lerp(0.08, 0.22, t / 0.16)) * 44100 / tau")
local lp = lowpass1(white(), hz)
return expr("x * max(sin(pi * clamp(t / 0.16, 0, 1)), 0) ^ 1.6 * 0.18 * (t < 0.16)", { x = lp })
