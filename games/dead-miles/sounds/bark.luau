-- A dog's bark: two short rough yaps. 0.5 s.
length(0.5)
local f = sweep(900, 420, 18)
local yap = bandpass(saw(f) + noise() * 0.4, 800, 1.2) * decay(0.16, 1.5)
local yap2 = after(0.22, bandpass(saw(sweep(820, 400, 18)) + noise() * 0.4, 760, 1.2) * decay(0.16, 1.5))
return (yap + yap2) * 0.5
