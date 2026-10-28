-- Empty chamber click, after sfx.gd _synth_empty_chamber (1390-1408).
-- One short band-passed noise burst: the firing pin on nothing.
--
-- Params: none.
-- Game: trigger pulled with no round, at the gun, -10 dB, dry, unit 5 m, SPL 55.
length(0.025)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local function hz(k) return expr("-log(1 - " .. k .. ") * 44100 / tau") end

local burst = white() * expr("exp(-t * 280)")
-- HP at k 0.04 (287 Hz), then LP at k 0.42 (3.8 kHz).
return lowpass1(highpass1(burst, hz("0.04")), hz("0.42")) * 0.9
