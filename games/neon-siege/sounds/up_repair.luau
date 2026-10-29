-- Repair picked up: a warm, rising heal chime (a major arpeggio). 0.5 s.
length(0.5)
local function note(f, at)
	return expr("sin(tau * f * (t - at)) * exp(-(t - at) * 7) * step(at, t) * 0.3", { f = f, at = at })
end
local shimmer = expr("sin(tau * 2637 * t) * exp(-t * 9) * 0.06 * (0.5 + 0.5 * sin(tau * 12 * t))")
return expr("tanh((a + b + c + d) * 1.2) * 0.6 * (t < 0.5)", { a = note(523, 0), b = note(659, 0.07), c = note(784, 0.14), d = shimmer })
