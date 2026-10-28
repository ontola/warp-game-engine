-- A shot hits without killing: a dull tick. 0.05 s.
length(0.05)
return lowpass(noise(), 2500) * decay(0.05, 4) * 0.25
