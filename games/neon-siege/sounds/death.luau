-- A pilot goes down: a long falling buzz with rumble. 0.9 s.
length(0.9)
local buzz = saw(sweep(300, 45, 3)) * decay(0.9, 1.5) * 0.35
local rumble = lowpass(noise(), 600) * decay(0.7, 1.5) * 0.5
return (buzz + rumble) * 0.8
