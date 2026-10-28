-- A new wave: a two-tone alert. 0.5 s.
length(0.5)
local a = expr("sin(tau * 440 * t) * exp(-t * 8) * 0.3")
local b = expr("sin(tau * 660 * (t - 0.18)) * exp(-(t - 0.18) * 7) * 0.3 * step(0.18, t)")
return expr("tanh((a + b) * 1.5) * 0.5 * (t < 0.5)", { a = a, b = b })
