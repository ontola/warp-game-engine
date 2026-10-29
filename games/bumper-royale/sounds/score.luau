-- A knock-out counts: two bright notes. 0.3 s.
length(0.3)
local a = expr("sin(tau * 740 * t) * exp(-t * 16) * 0.35")
local b = expr("sin(tau * 1109 * (t - 0.07)) * exp(-(t - 0.07) * 12) * 0.35 * step(0.07, t)")
return expr("tanh((a + b) * 1.3) * 0.5 * (t < 0.3)", { a = a, b = b })
