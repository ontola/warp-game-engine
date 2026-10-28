-- A pilot is hit: a harsh buzz dropping an octave. 0.35 s.
length(0.35)
local buzz = saw(sweep(260, 110, 7)) * decay(0.35, 2) * 0.35
return (buzz + lowpass(noise(), 1500) * decay(0.15, 3) * 0.3) * 0.8
