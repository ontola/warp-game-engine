-- Zombies feeding: wet tearing and chewing (recordings replace it once fetched). 0.6 s.
length(0.6)
local f = rand(0.7, 1.3)
return bandpass(noise(), 900 * f, 2) * hump(0.55) * (0.5 + 0.5 * sine(rand(6, 10))) * 0.35
