-- A weapon breaks: a sharp splintering crack. 0.4 s.
length(0.4)
local snap = highpass(noise(), 1200) * decay(0.05, 1.2) * 0.8
local rattle = dust(60, 0.6) * decay(0.35, 1.5)
return (snap + bandpass(rattle, 2500, 2) * 0.6) * 0.9
