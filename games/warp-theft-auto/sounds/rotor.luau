loop()
local speed = param("speed", 0)
local rate = 18 + speed * 0.11
local chop = (sine(rate) * 0.5 + 0.5) * (sine(rate * 2) * 0.25 + 0.75)
local rotor = lowpass1(noise(), 620) * chop * 0.8 + sine(rate * 3) * 0.10
local cabin = lowpass1(param("cabin", 0), 4)
local cockpit = lowpass1(rotor, 230) * 0.84
return (rotor + cabin * (cockpit - rotor)) * adsr(0.3, 0.1, 1, 0.25)
