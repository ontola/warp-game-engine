-- Victory: four rising notes. 1 s.
length(1)
local function note(f, at) return expr("sin(tau * f * (t - s)) * exp(-(t - s) * 5) * 0.3 * step(s, t)", { f = f, s = at }) end
return expr("tanh((a + b + c + d) * 1.4) * 0.5 * (t < 1)", { a = note(523, 0), b = note(659, 0.12), c = note(784, 0.24), d = note(1047, 0.36) })
