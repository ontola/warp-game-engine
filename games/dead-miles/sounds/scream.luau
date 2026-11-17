-- The director's peak: a far scream rising, the street answers. 1.8 s.
length(1.8)
local f = sweep(300, 900, 1.2)
local scream = bandpass(saw(f) + noise() * 0.3, 1200, 2) * hump(1.8) * 0.5
return lowpass(scream, 3000) * 0.7
