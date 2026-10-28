-- Hitmarker "tink", after sfx.gd _synth_hitmarker (2127-2157).
-- A noise click (e^-600t) plus two sines (0.55 f1 + 0.35 f2, e^-100t) through tanh.
-- Heavier hits: lower pitch (r^-0.45), slower decay and longer (50 -> 111 ms at r 5).
--   hitmarker (body): f1, f2 = 2500, 1700 Hz
--   hitmarker_head:   3400, 2250 Hz
--   hitmarker_kill:   1800, 1200 Hz (Godot has the recipe; the game routes kills to
--                     kill_confirm.luau instead, so it is unused in play)
--
-- Params:
--   dmg_ratio  clamp(dmg / 22, 0.5, 5) (Weapon.BASE_DAMAGE 22), or 1 when dmg is 0.
--              Godot caches per round(dmg_ratio * 4); pass the exact ratio.
-- Game (hitmarker 1189-1198, game.gd:4577/4589): 2D, -5 + clamp(2 log2(dmg_ratio), 0, 4) dB,
--   SPL 130.

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local function tink(f1, f2)
  length(0.14)
  local r = expr("max(x, 0.01)", { x = param("dmg_ratio", 1) })
  local df = expr("lerp(1, 0.45, clamp((r - 1) / 4, 0, 1))", { r = r })
  local dur = expr("clamp(0.05 / k, 0.05, 0.14)", { k = df })
  local pf = expr("r ^ -0.45", { r = r })
  local tone = expr("(sin(tau * " .. f1 .. " * p * t) * 0.55 + sin(tau * " .. f2 .. " * p * t) * 0.35) * exp(-t * 100 * k)", { p = pf, k = df })
  local click = expr("n * exp(-t * 600 * k) * 0.5", { n = white(), k = df })
  return expr("tanh((c + a) * 0.9) * (t < d)", { c = click, a = tone, d = dur })
end

sound("hitmarker_head", function() return tink(3400, 2250) end)
sound("hitmarker_kill", function() return tink(1800, 1200) end)
return tink(2500, 1700)
