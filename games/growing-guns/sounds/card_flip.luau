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
-- Godot's one-pole lerp(y, x, k) at 44.1 kHz levels off near its Nyquist at k / (2 - k);
-- a one-pole at 48 kHz with the same cutoff keeps falling there, which darkens it by 1-2%
-- in centroid for these large k. A small direct term c and a slightly lower cutoff fit the
-- 44.1 kHz power response over 0-20 kHz to 0.03 dB (fitted at 48 kHz, exact at 44.1 kHz).
local function lp44(x, k)
  local s = expr("clamp((sr - 44100) / 3900, 0, 1)")
  local c = expr("(0.025 * k + 0.06 * k * k) * s", { k = k, s = s })
  local fc = expr("-log(1 - k) * 44100 / tau * (1 - 0.025 * k * s)", { k = k, s = s })
  return expr("c * x + (1 - c) * y", { c = c, x = x, y = lowpass1(x, fc) })
end

local dur = rand(0.16, 0.22)
local nd = dur * rand(0.65, 0.80)
local ts = dur * rand(0.78, 0.90)
local td = rand(180, 320)
local wash_k = rand(0.20, 0.45)
local tick_k = rand(0.30, 0.55)

-- Godot only runs each filter inside its window, from an empty state; gate the inputs likewise.
local wash = lp44(expr("w * (t < d)", { w = white(), d = nd }), wash_k)
  * expr("sin(pi * clamp(t / d, 0, 1)) * 0.2 * (t < d)", { d = nd })
local tick_in = expr("w * step(s, t)", { w = white(), s = ts })
local tick = (tick_in - lp44(tick_in, tick_k))
  * expr("exp(-max(t - s, 0) * k) * 0.5 * step(s, t)", { s = ts, k = td })
return expr("(a + b) * (t < d)", { a = wash, b = tick, d = dur })
