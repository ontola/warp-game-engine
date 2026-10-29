-- A shot thuds into cover or a raised shield: a dull knock. 0.08 s.
length(0.08)
local knock = sine(sweep(320 * rand(0.9, 1.1), 120, 60)) * decay(0.07, 2) * 0.35
return knock + lowpass(noise(), 2000) * decay(0.05, 3) * 0.3
