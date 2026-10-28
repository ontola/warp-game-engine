-- The jump: a short upward sine sweep, 260 Hz to 620 Hz. 0.18 s.
length(0.18)

local f = expr("lerp(260, 620, 1 - exp(-t * 25))")
return expr("sin(tau * f * t) * exp(-t * 14) * 0.3 * (t < 0.18)", { f = f })
