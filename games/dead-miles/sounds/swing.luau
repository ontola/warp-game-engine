-- A swing that hits nothing: a short whoosh. 0.22 s.
length(0.22)
return bandpass(noise(), sweep(500, 1600, 8), 1.2) * hump(0.22) * 0.35
