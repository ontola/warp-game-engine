-- The sedan's silencer sits between the shared pulse engine and the tailpipe.
loop()
local rpm = param("rpm", 900)
local speed = param("speed", 0)
local load = param("load", 0)
local exhaust = engine("inline_four", rpm, param("throttle", 0), load, param("cut", 0), param("gear", 1), speed)
local road = tyres(speed, param("slip", 0), param("contact", 0), param("loose", 0), param("soft", 0), 0)
local tailpipe = lowpass(exhaust, 820 + 520 * clamp(load, 0, 1), 0.72)
return (tailpipe * 1.10 + road * 0.28) * adsr(0.05, 0.1, 1, 0.12)
