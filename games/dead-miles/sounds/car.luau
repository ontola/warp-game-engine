-- The car starts and pulls away: a cranking engine. 1.5 s.
length(1.5)
local rpm = sweep(30, 70, 2)
local engine = lowpass(saw(rpm) + noise() * 0.2, 400) * hump(1.5) * 0.6
return engine
