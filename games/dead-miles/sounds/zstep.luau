-- A dragged, uneven footfall of the dead (recordings replace it once fetched). 0.12 s.
length(0.12)
return lowpass(noise(), 420 * rand(0.8, 1.25)) * decay(0.09, 3) * 0.3
