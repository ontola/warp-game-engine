-- A pistol shot: a hard crack over a short low thump. 0.35 s.
length(0.35)
local crack = highpass(noise(), 1800) * decay(0.06, 1.5) * 0.7
local body = lowpass(noise(), 900) * decay(0.25, 2.5) * 0.6
local kick = sine(sweep(160 * rand(0.9, 1.1), 50, 40)) * decay(0.18, 2) * 0.5
return (crack + body + kick) * 0.9
