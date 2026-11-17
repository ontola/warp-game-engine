-- An empty gun: a dry metal click. 0.08 s.
length(0.08)
return highpass(noise(), 3500) * decay(0.02, 2) * 0.35
