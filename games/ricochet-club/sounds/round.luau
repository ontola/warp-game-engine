-- The round ends: three rising notes and a held chord. 0.9 s.
length(0.9)
local a = expr("sin(tau * 523 * t) * exp(-t * 8) * 0.3")
local b = expr("sin(tau * 659 * (t - 0.12)) * exp(-(t - 0.12) * 8) * 0.3 * step(0.12, t)")
local c = expr("(sin(tau * 784 * (t - 0.24)) + 0.5 * sin(tau * 1047 * (t - 0.24))) * exp(-(t - 0.24) * 4) * 0.3 * step(0.24, t)")
return expr("tanh((a + b + c) * 1.5) * 0.5 * (t < 0.9)", { a = a, b = b, c = c })
