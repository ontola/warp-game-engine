-- Empty chamber click, after sfx.gd _synth_empty_chamber (1390-1408).
-- One short band-passed noise burst: the firing pin on nothing.
--
-- Params: none.
-- Game: trigger pulled with no round, at the gun, -10 dB, dry, unit 5 m, SPL 55.
length(0.025)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local burst = white44() * expr("exp(-t * 280)")
-- HP at k 0.04 (287 Hz), then LP at k 0.42 (3.8 kHz).
return godot_lp(godot_hp(burst, 0.04), 0.42) * 0.9
