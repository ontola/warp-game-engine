-- A tank is hit or a crate breaks: a noise burst over a falling boom. 0.35 s.
length(0.35)
local boom = sine(sweep(160 * rand(0.9, 1.1), 40, 9)) * decay(0.3, 2) * 0.55
local burst = lowpass(noise(), 2400) * decay(0.22, 3) * 0.55
return expr("tanh((a + b) * 1.4) * 0.75 * (t < 0.35)", { a = boom, b = burst })
