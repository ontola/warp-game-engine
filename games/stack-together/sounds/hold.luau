-- A piece goes into the shared slot: a quick upward swish and a soft pling. 0.25 s.
length(0.25)
local swish = lowpass(noise(), 2400) * decay(0.12, 3) * 0.18
local pling = expr("sin(tau * (660 + 500 * t) * t) * exp(-t * 16) * 0.3")
return swish + pling
