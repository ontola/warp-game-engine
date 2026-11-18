-- A car door forced open: a rusty creak, then the clunk of it hitting its stop. 0.7 s.
length(0.7)
local f = rand(0.85, 1.15)
local creak = bandpass(saw(sweep(95 * f, 140 * f, 3)) + noise() * 0.3, 900 * f, 6) * hump(0.45) * 0.35
local clunk = after(0.45, lowpass(noise(), 700) * decay(0.12, 2) * 0.9 + sine(sweep(110, 60, 30)) * decay(0.15, 2) * 0.5)
return creak + clunk
