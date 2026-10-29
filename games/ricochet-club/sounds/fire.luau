-- A tank fires: a short noise crack over a quick falling thump. 0.12 s.
length(0.12)
local thump = sine(sweep(260 * rand(0.92, 1.08), 70, 40)) * decay(0.1, 2) * 0.5
local crack = highpass(noise(), 1800) * decay(0.035, 2) * 0.35
return (thump + crack) * 0.7
