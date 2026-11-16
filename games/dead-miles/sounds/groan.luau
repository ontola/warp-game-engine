-- A zombie groan: a low rough voice that slides down. 1.1 s.
length(1.1)
local f0 = 95 * rand(0.85, 1.2)
local voice = saw(sweep(f0 * 1.3, f0 * 0.8, 1.5)) * hump(1.1)
local rough = lowpass(noise(), 500) * hump(1.1) * 0.3
return bandpass(voice + rough, 600 * rand(0.8, 1.2), 1.5) * 0.4
