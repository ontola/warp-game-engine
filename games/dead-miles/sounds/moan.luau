-- A ghost's moan: a soft hollow wail. 1.2 s.
length(1.2)
local f = 220 * rand(0.9, 1.1)
return (sine(f) + sine(f * 1.5) * 0.4) * hump(1.2) * 0.25 + bandpass(noise(), 800, 4) * hump(1.2) * 0.1
