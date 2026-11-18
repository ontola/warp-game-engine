-- Out of breath: one ragged breath, a hard huff out and a sharp pull in,
-- airy and throaty. Played over and over while someone is spent. 0.8 s.
length(0.8)
local f = rand(0.9, 1.1)
local huff = (bandpass(noise(), 1100 * f, 1.4) * 0.7 + bandpass(noise(), 2600 * f, 2) * 0.35) * hump(0.38) * 0.5
local voice = bandpass(saw(sweep(170 * f, 120 * f, 5)), 700, 3) * hump(0.3) * 0.06
local pull = after(0.45, (bandpass(noise(), 1900 * f, 1.8) * 0.6 + highpass(noise(), 3500) * 0.2) * hump(0.28) * 0.4)
return huff + voice + pull
