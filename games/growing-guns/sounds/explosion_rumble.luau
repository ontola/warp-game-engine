-- Explosion rumble, after sfx.gd _synth_explosion_rumble (1491-1538).
-- Brown noise (leaky-integrated white), low-passed, with a short linear
-- attack and a power-law fade. Bigger radius: longer (0.33 s at 2 m, 5 s
-- at 24 m), deeper, and a little quieter per sample.
--
-- Params:
--   radius  in m. Explosions pass round(clamp(radius, 2, 24)); a heavy gunshot
--           (damage / 22 > 1.4) passes round(clamp(damage / 22 * 1.5, 2, 12)).
-- Game:
--   explosion: +10 dB, room reverb, unit clamp(4.5 r, 20, 100), big-tail send -14 dB, delayed d / 343 s.
--   shot rumble: -12 dB at the muzzle (2D for the shooter), room reverb,
--                unit clamp(radius * 0.8, 4, 12), the shot's SPL. Same synth, no separate patch.
length(5.0)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local r = param("radius", 6)

local dur = expr("clamp(0.3 + 4.7 * (clamp(r, 2, 24) / 24) ^ 2, 0.3, 5)", { r = r })
local fc = expr("clamp(220 - 7 * r, 55, 200) + j", { r = r, j = rand(-15, 15) })
local k = expr("clamp(tau * f / 44100, 0.005, 0.4)", { f = fc })

-- brown = 0.998 brown + 0.03 white  ==  15 * one-pole LP (k 0.002, 14.05 Hz) of white.
local brown = godot_lp(white44(), 0.002) * 15
local lp = godot_lp(brown, k)

local env = expr("clamp(t / a, 0, 1) * (1 - clamp((t - a) / max(d - a, 0.0001), 0, 1)) ^ p", {
  a = rand(0.05, 0.12), d = dur, p = rand(2.4, 3.0) })
local gain = expr("1.5 * sqrt(6 / max(6, r))", { r = r })
return expr("tanh(x * e * g) * (t < d)", { x = lp, e = env, g = gain, d = dur })
