-- Pierce picked up: a sharp rising zap that ends in a hard metallic ping. 0.45 s.
length(0.45)
local zap = expr("tanh(sin(tau * lerp(300, 2400, min(t / 0.16, 1)) * t) * 4) * exp(-t * 9) * 0.25 * (t < 0.18)")
local ping = expr("(sin(tau * 2093 * (t - 0.16)) + 0.5 * sin(tau * 3136 * (t - 0.16))) * exp(-(t - 0.16) * 10) * step(0.16, t) * 0.25")
return expr("tanh((a + b) * 1.3) * 0.6 * (t < 0.45)", { a = zap, b = ping })
