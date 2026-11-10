-- The stack reaches the top: a falling buzz and a thud. 0.8 s.
length(0.8)
local buzz = saw(sweep(260, 70, 3)) * decay(0.7, 2) * 0.25
local thud = lowpass(noise(), 600) * decay(0.3, 3) * 0.5
return lowpass(buzz, 1800) + thud
