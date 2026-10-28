-- The pulse (and a dasher's charge): a deep thump under a rising whoosh. 0.6 s.
length(0.6)
local thump = expr("sin(tau * lerp(110, 40, t / 0.6) * t) * exp(-t * 6) * 0.5")
local whoosh = lowpass(noise(), 2200) * decay(0.5, 2) * 0.35
return expr("tanh((a + b) * 1.4) * 0.7 * (t < 0.6)", { a = thump, b = whoosh })
