-- A puck rides off the edge: a long falling whistle that fades away. 0.9 s.
length(0.9)
local f = expr("lerp(900, 120, (t / 0.9) ^ 0.7)")
local whistle = expr("sin(tau * f * t) * exp(-t * 2.5) * 0.3", { f = f })
local air = lowpass(noise(), 1200) * decay(0.8, 1.5) * 0.2
return expr("tanh((a + b) * 1.2) * 0.7 * (t < 0.9)", { a = whistle, b = air })
