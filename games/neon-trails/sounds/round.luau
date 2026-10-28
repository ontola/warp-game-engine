-- The last rider standing: three rising square-ish notes. 0.6 s.
length(0.6)
local a = expr("sin(tau * 523 * t) * exp(-t * 10) * 0.3")
local b = expr("sin(tau * 659 * (t - 0.12)) * exp(-(t - 0.12) * 10) * 0.3 * step(0.12, t)")
local c = expr("sin(tau * 784 * (t - 0.24)) * exp(-(t - 0.24) * 6) * 0.35 * step(0.24, t)")
return expr("tanh((a + b + c) * 1.6) * 0.5 * (t < 0.6)", { a = a, b = b, c = c })
