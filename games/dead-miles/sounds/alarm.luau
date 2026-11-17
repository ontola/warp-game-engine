-- A car alarm: a siren that whoops up and down. 1.2 s.
length(1.2)
local f = 900 + sine(2.5) * 350
return lowpass(square(f), 2600) * hump(1.2) * 0.22
