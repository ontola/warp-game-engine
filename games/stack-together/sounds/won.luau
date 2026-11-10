-- The target is reached: a little fanfare of four rising notes. 1.1 s.
length(1.1)
local a = expr("sin(tau * 523 * t) * exp(-t * 6) * 0.3")
local b = expr("sin(tau * 659 * (t - 0.14)) * exp(-(t - 0.14) * 6) * 0.3 * step(0.14, t)")
local c = expr("sin(tau * 784 * (t - 0.28)) * exp(-(t - 0.28) * 6) * 0.3 * step(0.28, t)")
local d = expr("(sin(tau * 1047 * (t - 0.42)) + 0.4 * sin(tau * 2094 * (t - 0.42))) * exp(-(t - 0.42) * 3) * 0.3 * step(0.42, t)")
return expr("tanh((a + b + c + d) * 1.3) * 0.55 * (t < 1.1)", { a = a, b = b, c = c, d = d })
