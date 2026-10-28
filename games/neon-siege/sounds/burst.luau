-- An enemy pops: a noise burst over a falling tone. 0.3 s.
length(0.3)
local tone = expr("sin(tau * lerp(420, 90, t / 0.3) * t) * exp(-t * 14) * 0.3")
local pop = lowpass(noise(), 3200) * decay(0.2, 3) * 0.45
return expr("tanh((a + b) * 1.3) * 0.7 * (t < 0.3)", { a = tone, b = pop })
