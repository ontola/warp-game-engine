-- Ion cannon detonation burst, after sfx.gd _synth_ion_cannon_burst (1972-2008).
-- Two naive (aliasing, as in Godot) saws at ~40 Hz and its octave, rounded by a
-- one-pole low-pass, plus a short noise crack; exp * sqrt fade, through tanh.
--
-- Params:
--   radius  blast radius in m (ion_cannon.gd BLAST_RADIUS 38); Godot passes round(clamp(r, 8, 48)).
-- Game (sfx.gd ion_cannon_detonate 1115-1127): first explosion(at, 38) (explosion_bang +
--   explosion_rumble at radius 24), then this at the target, room reverb, unit clamp(1.6 r, 16, 72):
--   one copy at -2 dB (big-tail send -26), one wet copy at 0 dB (send -24), plus a big-tail
--   send at -20 dB; SPL 168 + 4 log2(max(1, r / 6)); big-tail intensity >= 0.62.
--   Godot's right channel is 0.99 x left; this patch is mono.
length(0.95)

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local r = param("radius", 38)

local R = expr("clamp(r / 38, 0.6, 1.4)", { r = r })
local dur = expr("clamp(lerp(0.55, 0.85, R) * j, 0.5, 0.95)", { R = R, j = rand(0.95, 1.05) })
local f0 = expr("40 * j * lerp(1, 0.9, R - 0.6)", { R = R, j = rand(0.97, 1.03) })
local f1 = f0 * 2 * rand(0.992, 1.008)

-- Saws from random start phases (cycles), advanced before use as in Godot.
local saw = expr("(fract(a + f * (t + 1 / sr)) * 2 - 1) * 0.85 + (fract(b + g * (t + 1 / sr)) * 2 - 1) * 0.3", {
  a = rand(0, 1), b = rand(0, 1), f = f0, g = f1 })
local lp = godot_lp(saw, 0.45)
local crack = godot_lp(white44(), 0.5) * expr("exp(-85 * t)")
local env = expr("exp(-t * lerp(4.6, 3.0, R)) * max(1 - t / d, 0) ^ 0.5", { R = R, d = dur })

return expr("tanh((2.4 * a + 1.1 * c) * e) * 0.7 * (t < d)", { a = lp, c = crack, e = env, d = dur })
