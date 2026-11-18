-- Broken glass shifting under a hand: bright tinkles. 0.5 s.
length(0.5)
local hit = highpass(noise(), 3000) * decay(0.05, 2) * 0.5
local ring1 = bandpass(noise(), 5200 * rand(0.9, 1.1), 30) * after(0.02, decay(0.25, 2)) * 1.2
local ring2 = bandpass(noise(), 7400 * rand(0.9, 1.1), 30) * after(0.12, decay(0.2, 2)) * 0.9
local ring3 = bandpass(noise(), 4300 * rand(0.9, 1.1), 30) * after(0.24, decay(0.18, 2)) * 0.7
return hit + ring1 + ring2 + ring3
