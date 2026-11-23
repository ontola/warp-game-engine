loop()
local speed = param("speed", 0)
local rate = 18 + speed * 0.11
local chop = (sine(rate) * 0.5 + 0.5) * (sine(rate * 2) * 0.25 + 0.75)
return (lowpass1(noise(), 620) * chop * 0.8 + sine(rate * 3) * 0.10) * adsr(0.3, 0.1, 1, 0.25)
