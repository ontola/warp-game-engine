-- Exhaust interior dimensions in metres, from collector to open tailpipe.
local sedan = {
    model = "inline_four",
    muffler = {
        inlet_length = 1.85, inlet_diameter = 0.050,
        chamber_length = 0.46, chamber_diameter = 0.165,
        outlet_length = 0.66, outlet_diameter = 0.045,
        packing = 0.78,
    },
}
loop()
local rpm = param("rpm", 900)
local speed = param("speed", 0)
local load = param("load", 0)
local exhaust = engine(sedan, rpm, param("throttle", 0), load, param("cut", 0), param("gear", 1), speed)
local road = tyres(speed, param("slip", 0), param("contact", 0), param("loose", 0), param("soft", 0), 0)
return (exhaust + road * 0.28) * adsr(0.05, 0.1, 1, 0.12)
