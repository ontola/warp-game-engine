-- Dash air whoosh, after sfx.gd _synth_dash (1808-1827).
-- White noise through a one-pole low-pass opening over the dash (Godot k 0.08 -> 0.22,
-- 585 -> 1750 Hz), under a sin^1.6 swell. Fixed 0.16 s.
--
-- Params: none.
-- Game (dash 1039-1040, player.gd:1772): -24 dB, raytraced 3D at the player (room reverb),
--   unit 12 m, SPL 75, big-tail send -24 dB.
length(0.16)

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

-- lerp(lp, noise, k) with k = lerp(0.08, 0.22, t / dur).
local lp = lp44(white(), expr("lerp(0.08, 0.22, t / 0.16)"))
return expr("x * max(sin(pi * clamp(t / 0.16, 0, 1)), 0) ^ 1.6 * 0.18 * (t < 0.16)", { x = lp })
