-- Melee swoosh, after sfx.gd _synth_melee (1586-1601).
-- Half-sine swell of noise through a low-pass that falls from about 8 kHz
-- to 1.4 kHz (at pitch_ratio 1) over 0.2 s.
--
-- Params:
--   pitch_ratio  clamp(50 / damage, 0.5, 1.2), damage = melee damage * damage_mult
--                (base melee damage 70 gives 0.71). Higher is brighter.
-- Game: at the swinging player, -6 dB, room reverb, unit 12 m, SPL 90.
length(0.2)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local pr = param("pitch_ratio", 0.71)
local k = expr("clamp(lerp(0.7, 0.18, t / 0.2) * p, 0.01, 0.99)", { p = pr })
return godot_lp(white44(), k) * expr("0.45 * sin(pi * clamp(t / 0.2, 0, 1))")
