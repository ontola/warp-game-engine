-- A dash: a short rising whoosh over a low push. 0.25 s.
length(0.25)
local whoosh = lowpass(noise(), 2600) * expr("sin(pi * min(t / 0.25, 1)) * (t < 0.25)") * 0.4
local push = expr("sin(tau * lerp(140, 260, t / 0.25) * t) * exp(-t * 14) * 0.3")
return expr("tanh((a + b) * 1.3) * 0.6 * (t < 0.25)", { a = whoosh, b = push })
