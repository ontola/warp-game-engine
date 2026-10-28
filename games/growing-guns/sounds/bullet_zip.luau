-- Bullet zip / near miss, after sfx.gd _synth_bullet_zip (1673-1733).
-- Noise through a two-octave band whose centre glides down like a doppler
-- pass (smootherstep from the approach to the recede frequency), with a
-- linear swell then an exponential tail. Fast rounds are shorter and harder.
--
-- Params:
--   speed_factor  clamp(bullet speed / 165 m/s, 0.2, 5) (= bullet_speed_mult; Godot buckets to 0.1)
--   scale_factor  clamp(bullet scale, 0.5, 4) (bullet_scale * sqrt(damage_mult))
-- Game: at the bullet's closest pass when it is within 12 m (32 ms between zips), dry, unit 12 m.
--   gain  lerp(-36, -24, sqrt(1 - d / 12)) + lerp(-10, 6, sn) dB, sn = clamp((speed_factor - 0.2) / 4.8, 0, 1)
--   SPL lerp(95, 135, sn); big-tail send at gain + lerp(-30, -2, sn) when sn > 0.
length(0.48)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local sf = expr("clamp(s, 0.2, 5)", { s = param("speed_factor", 1) })
local sc = expr("clamp(s, 0.5, 4)", { s = param("scale_factor", 1) })

local nominal = expr("clamp(2400 + 1300 * s, 300, 6000) * j", { s = sf, j = rand(0.92, 1.08) })
local v = expr("clamp(s * 165, 10, 280)", { s = sf })
local hi = expr("min(n * 343 / (343 - v), 7000)", { n = nominal, v = v })
local lo = expr("n * 343 / (343 + v)", { n = nominal, v = v })
local dur = expr("clamp(0.06 + 0.06 / s, 0.07, 0.40) * j", { s = sf, j = rand(0.85, 1.18) })
local af = expr("clamp(0.45 / s, 0.04, 0.5)", { s = sf })
local drive = expr("lerp(1, 3.5, clamp((s - 3) / 2, 0, 1))", { s = sf })

local ph = expr("clamp(t / d, 0, 1)", { d = dur })
local c = expr("lerp(h, l, p * p * p * (p * (p * 6 - 15) + 10)) * j", { h = hi, l = lo, p = ph, j = rand(0.85, 1.15) })
-- Band edges at 2c and c/2, from Godot's k = 2π f / 44100 (clamped), as exact cutoffs.
local a = lowpass1(white(), expr("-log(1 - clamp(c * tau * 2 / 44100, 0.04, 0.98)) * 44100 / tau", { c = c }))
local b = lowpass1(a, expr("-log(1 - clamp(c * tau * 0.5 / 44100, 0.003, 0.6)) * 44100 / tau", { c = c }))
local env = expr("(p < f) * p / f + (1 - (p < f)) * exp(-4 * (p - f) / max(1 - f, 0.0001))", { p = ph, f = af })

return expr("tanh(x * e * 0.55 * k * g) * (t < d)", { x = a - b, e = env, k = sc, g = drive, d = dur })
