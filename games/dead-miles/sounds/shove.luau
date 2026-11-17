-- A shove: a grunt of effort and cloth. 0.2 s.
length(0.2)
return lowpass(noise(), 700) * hump(0.2) * 0.45 + sine(sweep(180, 120, 10)) * hump(0.15) * 0.15
