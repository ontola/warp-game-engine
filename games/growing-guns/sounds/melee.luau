-- Melee swoosh, after sfx.gd _synth_melee (1586-1601).
-- Half-sine swell of noise through a low-pass that falls from about 8 kHz
-- to 1.4 kHz (at pitch_ratio 1) over 0.2 s.
--
-- Params:
--   pitch_ratio  clamp(50 / damage, 0.5, 1.2), damage = melee damage * damage_mult
--                (base melee damage 70 gives 0.71). Higher is brighter.
-- Game: at the swinging player, -6 dB, room reverb, unit 12 m, SPL 90.
length(0.2)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local pr = param("pitch_ratio", 0.71)
local k = expr("-log(1 - clamp(lerp(0.7, 0.18, t / 0.2) * p, 0.01, 0.99)) * 44100 / tau", { p = pr })
return lowpass1(white(), k) * expr("0.45 * sin(pi * clamp(t / 0.2, 0, 1))")
