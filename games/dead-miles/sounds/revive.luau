-- Back on your feet: a warm rising chord. 0.6 s.
length(0.6)
return (sine(sweep(330, 440, 4)) + sine(sweep(415, 554, 4)) * 0.6) * hump(0.6) * 0.22
