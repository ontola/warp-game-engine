-- Rummaging a car: a hollow metal thump (the glovebox, the boot lid). 0.35 s.
length(0.35)
local f = rand(0.8, 1.2)
local body = sine(sweep(160 * f, 80 * f, 25)) * decay(0.25, 2) * 0.6
local panel = bandpass(noise(), 420 * f, 3) * decay(0.18, 2) * 0.6
local rattle = bandpass(noise(), 2400, 4) * after(0.03, decay(0.1, 2)) * 0.2
return body + panel + rattle
