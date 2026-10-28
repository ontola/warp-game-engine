-- A power-up or a revive: two bright rising notes. 0.3 s.
length(0.3)
local a = expr("sin(tau * 988 * t) * exp(-t * 16) * 0.35")
local b = expr("sin(tau * 1480 * (t - 0.06)) * exp(-(t - 0.06) * 12) * 0.35 * step(0.06, t)")
return expr("tanh((a + b) * 1.3) * 0.55 * (t < 0.3)", { a = a, b = b })
