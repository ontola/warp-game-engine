-- A radio left on: static with a faint voice-like tone under it. 1.5 s.
length(1.5)
local hiss = bandpass(noise(), 2400, 0.7) * 0.25
local voice = bandpass(saw(180 + sine(4) * 40), 900, 2) * (0.5 + sine(3.3) * 0.5) * 0.3
return (hiss + voice) * hump(1.5) * 0.5
