-- Found something: two short soft notes. 0.3 s.
length(0.3)
return sine(520) * decay(0.12, 2) * 0.25 + after(0.12, sine(780) * decay(0.16, 2) * 0.25)
