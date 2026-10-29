-- Points for a hit on another tank: two quick rising notes. 0.25 s.
length(0.25)
local a = expr("sin(tau * 784 * t) * exp(-t * 18) * 0.25")
local b = expr("sin(tau * 1175 * (t - 0.08)) * exp(-(t - 0.08) * 14) * 0.28 * step(0.08, t)")
return expr("tanh((a + b) * 1.5) * 0.5 * (t < 0.25)", { a = a, b = b })
