-- A boss arrives: a deep detuned saw swell. 1.4 s.
length(1.4)
local low = (saw(sweep(55, 50, 1)) + saw(sweep(55.6, 50.4, 1))) * 0.2
local swell = expr("sin(pi * min(t / 1.4, 1)) * (t < 1.4)")
return lowpass(low, 900) * swell * 0.9
