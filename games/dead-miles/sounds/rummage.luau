-- Hands going through things: cloth, paper, a box scraped along. 0.45 s.
length(0.45)
local f = rand(0.8, 1.25)
local rustle = bandpass(noise(), 2200 * f, 1.2) * hump(0.4) * (0.6 + 0.4 * sine(rand(9, 15))) * 0.3
local scrape = bandpass(noise(), 700 * f, 2) * after(0.15, hump(0.25)) * 0.2
return rustle + scrape
