-- Mine plant "thud + arm beep", after sfx.gd _synth_mine_plant (2044-2063).
-- An 80 Hz thud (e^-35t), then from 100 ms a 1200 Hz beep that fades in and out,
-- through tanh. Deterministic in Godot (no random draws). Fixed 0.35 s.
--
-- Params: none.
-- Game (mine_plant 1130-1133; player.gd:3581 on planting, air_strike.gd:67 as the strike's
--   telegraph at the target): -10 dB, raytraced 3D at the mine (room reverb), unit 12 m,
--   SPL 90, big-tail send -24 dB.
length(0.35)

local thud = expr("sin(tau * 80 * t) * exp(-t * 35) * 0.6")
local beep = expr("sin(tau * 1200 * t) * 0.35 * (t > 0.1) * exp(-max(t - 0.1, 0) * 16) * (1 - exp(-max(t - 0.1, 0) * 50))")
return expr("tanh((a + b) * 1.2) * 0.55 * (t < 0.35)", { a = thud, b = beep })
