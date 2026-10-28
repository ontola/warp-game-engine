-- Enemy incoming rift stutter, after sfx.gd _synth_enemy_incoming (2102-2125).
-- Gated white-noise bursts (22 ms in every 70 ms, a 14.3 Hz stutter), a tone falling
-- 210 -> 95 Hz and RC-low-passed noise (180 -> 70 Hz), under e^-5.8t with a 25 ms
-- smoothstep attack, through tanh. As in Godot the tone is sin(2 pi f(u) t), not an
-- integrated phase. Fixed 0.42 s.
--
-- Params: none.
-- Game (enemy_incoming 1161-1162, game.gd:1873): raytraced 3D at the rift (room reverb),
--   unit 12 m, -8.5 dB, SPL 95, big-tail send -24 dB.
length(0.42)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

-- fmod(t, 0.07) < 0.022
local burst = expr("w * 0.55 * (fract(t / 0.07) < 0.022 / 0.07)", { w = white() })
local tone = expr("sin(tau * lerp(210, 95, t / 0.42) * t) * 0.28")
-- Godot's RC one-pole, alpha = dt / (1 / (2 pi fc) + dt) at 44.1 kHz, as its exact cutoff.
local hz = expr("-log(1 - 1 / (1 + 44100 / (tau * lerp(180, 70, t / 0.42)))) * 44100 / tau")
local lp = lowpass1(white(), hz)
local env = expr("exp(-t * 5.8) * smoothstep(0, 0.06, t / 0.42)")
return expr("tanh((b + o + l * 0.35) * e * 1.45) * 0.62 * (t < 0.42)", { b = burst, o = tone, l = lp, e = env })
