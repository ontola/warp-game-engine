-- A rider hits a wall or a trail: a noise burst with a falling buzz. 0.5 s.
length(0.5)
local buzz = saw(sweep(180, 40, 9)) * decay(0.45, 2) * 0.35
local burst = lowpass(noise(), 1800) * decay(0.25, 3) * 0.6
return (buzz + burst) * 0.8
