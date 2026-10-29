-- Spread picked up: three bright notes fanning out left, middle, right. 0.35 s.
length(0.35)
local function blip(f, at)
	return expr("tanh(sin(tau * f * (t - at)) * 2) * exp(-(t - at) * 18) * step(at, t) * 0.28", { f = f, at = at })
end
return expr("(a + b + c) * 0.7 * (t < 0.35)", { a = blip(880, 0), b = blip(1175, 0.045), c = blip(1568, 0.09) })
