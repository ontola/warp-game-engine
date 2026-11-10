-- Rows clear together: a bright rising chord. 0.5 s.
length(0.5)
local a = expr("sin(tau * 523 * t) * exp(-t * 7) * 0.25")
local b = expr("sin(tau * 659 * (t - 0.05)) * exp(-(t - 0.05) * 7) * 0.25 * step(0.05, t)")
local c = expr("sin(tau * 1047 * (t - 0.1)) * exp(-(t - 0.1) * 6) * 0.25 * step(0.1, t)")
return expr("tanh((a + b + c) * 1.4) * 0.55 * (t < 0.5)", { a = a, b = b, c = c })
