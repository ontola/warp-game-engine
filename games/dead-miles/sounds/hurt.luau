-- A survivor is hit: a sharp impact and a short cry. 0.3 s.
length(0.3)
local hit = lowpass(noise(), 1800) * decay(0.08, 2) * 0.6
local cry = bandpass(saw(sweep(320 * rand(0.9, 1.1), 220, 6)), 900, 2) * hump(0.25) * 0.4
return hit + cry
