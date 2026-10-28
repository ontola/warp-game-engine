-- Next round / bolt cycle, after sfx.gd _synth_next_round (1361-1388).
-- Two band-passed noise clicks 22 ms apart (bolt back, bolt home).
--
-- Params: none.
-- Game: after each shot, at the gun, -16 dB, dry (no room reverb), unit 5 m, SPL 65.
length(0.07)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local function hz(k) return expr("-log(1 - " .. k .. ") * 44100 / tau") end

local burst = white() * expr("exp(-t * 260) + step(0.022, t) * 0.75 * exp(-(t - 0.022) * 200)")
-- HP at k 0.04 (287 Hz), then LP at k 0.48 (4.6 kHz).
return lowpass1(highpass1(burst, hz("0.04")), hz("0.48")) * 0.85
