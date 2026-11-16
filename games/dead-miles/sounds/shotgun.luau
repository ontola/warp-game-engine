-- A shotgun: a wider, deeper blast than the pistol. 0.6 s.
length(0.6)
local blast = lowpass(noise(), 2400) * decay(0.4, 2.2) * 0.8
local crack = highpass(noise(), 1500) * decay(0.08, 1.5) * 0.6
local kick = sine(sweep(120 * rand(0.9, 1.1), 40, 30)) * decay(0.3, 2) * 0.6
return (blast + crack + kick) * 0.9
