-- A shot glances off the arena wall: a bright metallic ping. 0.15 s.
length(0.15)
local ping = expr("sin(tau * f * t) * exp(-t * 32) * 0.25 * (t < 0.15)", { f = 1900 * rand(0.94, 1.06) })
local tick = highpass(noise(), 4000) * decay(0.012, 2) * 0.2
return ping + tick
