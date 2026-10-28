-- Next round / bolt cycle, after sfx.gd _synth_next_round (1361-1388).
-- Two band-passed noise clicks 22 ms apart (bolt back, bolt home).
--
-- Params: none.
-- Game: after each shot, at the gun, -16 dB, dry (no room reverb), unit 5 m, SPL 65.
length(0.07)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local burst = white44() * expr("exp(-t * 260) + step(0.022, t) * 0.75 * exp(-(t - 0.022) * 200)")
-- HP at k 0.04 (287 Hz), then LP at k 0.48 (4.6 kHz).
return godot_lp(godot_hp(burst, 0.04), 0.48) * 0.85
