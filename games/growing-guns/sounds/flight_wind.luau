-- Close rushing air, with shorter sleeve swishes as the arms windmill.
length(1.95)
local air = godot_lp(white44(), expr("0.024 + 0.05 * sin(pi * t / 1.95)"))
local cloth = godot_hp(godot_lp(white44(), 0.25), 0.035)
local gust = expr("max(0, sin(pi * clamp(t / 1.95, 0, 1))) ^ 0.7")
local sweeps = expr("max(0, sin(tau * (1.85 * t / 1.95 + 0.12))) ^ 5 + 0.65 * max(0, sin(tau * (1.85 * t / 1.95 + 0.55))) ^ 7")
return expr("g * (0.95 * a + 0.85 * s * c)", { g = gust, a = air, s = sweeps, c = cloth })
