-- Same pulse-exhaust and tyre units as Sideways. Inputs come from the car solver.
loop()
local rpm = param("rpm", 900)
local speed = param("speed", 0)
local exhaust = engine("inline_four", rpm, param("throttle", 0), param("load", 0), param("cut", 0), param("gear", 1), speed)
local road = tyres(speed, param("slip", 0), param("contact", 0), param("loose", 0), param("soft", 0), 0)
return (exhaust + road * 0.28) * adsr(0.05, 0.1, 1, 0.12)
