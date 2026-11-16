-- A zombie goes down: a wet crunch. 0.35 s.
length(0.35)
return lowpass(noise(), 1100) * decay(0.3, 2) * 0.6 + sine(sweep(90, 40, 12)) * decay(0.25, 2) * 0.4
