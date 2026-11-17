-- A church bell, struck once: a low bronze toll that rings on. 3 s.
length(3)
local strike = noise() * decay(0.01, 1)
return modal(strike, 220, 1, { { 1, 2.6, 1 }, { 2.0, 1.8, 0.6 }, { 2.4, 1.4, 0.4 }, { 3.0, 1.0, 0.3 }, { 4.2, 0.7, 0.2 } }) * 0.6
