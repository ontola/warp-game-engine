-- A piece lands: a short soft wooden knock. 0.12 s.
length(0.12)
local knock = expr("sin(tau * 190 * t) * exp(-t * 45) * 0.45")
local click = lowpass(noise(), 3200) * decay(0.03, 4) * 0.2
return (knock + click) * 0.8
