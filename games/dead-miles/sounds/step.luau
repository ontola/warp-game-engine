-- A footstep on asphalt (the recorded steps replace it once fetched). 0.12 s.
length(0.12)
return lowpass(noise(), 420 * rand(0.8, 1.25)) * decay(0.09, 3) * 0.3
