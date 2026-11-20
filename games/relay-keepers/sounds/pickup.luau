-- The orb pickup: two quick rising sine notes (880 Hz, then 1320 Hz 70 ms
-- later) with a bright overtone, softly saturated. 0.35 s.
-- sound.play("pickup", { at = pos, pitch = 1.1, bright = 0.5 })
--
-- Params:
--   bright  the overtone's level, 0 to 2 (default 1)
-- Render it without speakers: warp audio render sounds/pickup.luau
length(0.35)

local bright = param("bright", 1)
local low = expr("sin(tau * 880 * t) * exp(-t * 18) * 0.4")
local high = expr("sin(tau * 1320 * (t - 0.07)) * exp(-(t - 0.07) * 14) * 0.4 * step(0.07, t)")
local shine = expr("sin(tau * 2640 * (t - 0.07)) * exp(-(t - 0.07) * 30) * 0.12 * k * step(0.07, t)", { k = bright })
return expr("tanh((a + b + c) * 1.2) * 0.6 * (t < 0.35)", { a = low, b = high, c = shine })
