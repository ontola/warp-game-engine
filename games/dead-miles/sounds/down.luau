-- A survivor falls: a heavy thump and a falling tone. 0.8 s.
length(0.8)
return sine(sweep(110, 40, 3)) * decay(0.7, 1.8) * 0.6 + lowpass(noise(), 600) * decay(0.2, 2) * 0.5
