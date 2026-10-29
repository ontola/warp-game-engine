-- Rapid picked up: a stuttering burst of blips that speeds up and climbs. 0.4 s.
length(0.4)
local gate = expr("step(0, sin(tau * lerp(18, 40, t / 0.4) * t))")
local tone = expr("tanh(sin(tau * lerp(660, 1320, t / 0.4) * t) * 3) * 0.3")
return expr("a * b * exp(-t * 3) * 0.9 * (t < 0.4)", { a = gate, b = tone })
