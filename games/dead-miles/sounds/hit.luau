-- A melee hit: a dull thud with a wet edge. 0.25 s.
length(0.25)
local thud = sine(sweep(140 * rand(0.85, 1.15), 60, 25)) * decay(0.18, 2) * 0.7
local wet = lowpass(noise(), 1400) * decay(0.1, 2) * 0.4
return thud + wet
