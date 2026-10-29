-- A paint burst: a wet splat over a bright rising chirp. 0.35 s.
length(0.35)
local splat = lowpass(noise(), 1800) * decay(0.18, 3) * 0.55
local chirp = expr("sin(tau * lerp(500, 1300, min(t / 0.12, 1)) * t) * exp(-t * 12) * 0.3")
local body = expr("sin(tau * lerp(160, 60, t / 0.35) * t) * exp(-t * 10) * 0.4")
return expr("tanh((a + b + c) * 1.4) * 0.6 * (t < 0.35)", { a = splat, b = chirp, c = body })
