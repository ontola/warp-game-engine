-- B raises the shield: a quick rising shimmer. 0.3 s.
length(0.3)
local f = expr("lerp(500, 1300, t / 0.3)")
return expr("(sin(tau * f * t) + 0.5 * sin(tau * f * 1.5 * t)) * (1 - exp(-t * 60)) * exp(-t * 8) * 0.22 * (t < 0.3)", { f = f })
