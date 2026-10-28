-- Card flip, after sfx.gd _synth_card_flip (1864-1905).
-- A lightly low-passed white-noise wash under a sin swell (the card travelling), then a
-- short high-passed noise tick (the card landing). Pure noise, no tone.
-- Godot renders 3 seeded variants (hash([variant, "card_flip"])) and plays them from a
-- shuffle bag; here every voice draws its own jitter in Godot's order (dur, wash length,
-- tick start, tick decay, wash k, tick k), so successive flips differ as the bag's do.
--
-- Params: none (card_flip's pitch_ratio is ignored by Godot).
-- Game (card_flip 1164-1181, render_player.gd:960): 2D, -10 dB, SPL 60.
length(0.22)
local _keepalive = env{ { 0.22, 0 } } -- the wash can end before the tick starts

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end
-- A one-pole "lerp(y, x, k)" coefficient as its exact 44.1 kHz cutoff.
local function k_hz(k) return expr("-log(1 - k) * 44100 / tau", { k = k }) end

local dur = rand(0.16, 0.22)
local nd = dur * rand(0.65, 0.80)
local ts = dur * rand(0.78, 0.90)
local td = rand(180, 320)
local wash_k = rand(0.20, 0.45)
local tick_k = rand(0.30, 0.55)

-- Godot only runs each filter inside its window, from an empty state; gate the inputs likewise.
local wash = lowpass1(expr("w * (t < d)", { w = white(), d = nd }), k_hz(wash_k))
  * expr("sin(pi * clamp(t / d, 0, 1)) * 0.2 * (t < d)", { d = nd })
local tick = highpass1(expr("w * step(s, t)", { w = white(), s = ts }), k_hz(tick_k))
  * expr("exp(-max(t - s, 0) * k) * 0.5 * step(s, t)", { s = ts, k = td })
return expr("(a + b) * (t < d)", { a = wash, b = tick, d = dur })
