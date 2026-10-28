-- Round-start rocket descent rumble, after sfx.gd _synth_rocket_descent (2011-2041).
-- White noise through two one-pole low-passes opening 70 -> 280 Hz, soft-saturated,
-- swelling 0.45 -> 1 toward the landing. Fixed 0.85 s clip, as in Godot.
--
-- Params: none.
-- Game (rocket_descent 1061-1065, player.gd:4614): 2D (the local player's "cabin"), -4 dB,
--   SPL 130, no big-tail send (2D). The landing thump that follows is SFX.landing (samples).
length(0.85)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local dur = 0.85
-- Godot's RC one-pole, alpha = dt / (1 / (2 pi fc) + dt) at 44.1 kHz, as its exact cutoff.
local hz = expr("-log(1 - 1 / (1 + 44100 / (tau * lerp(70, 280, t / 0.85)))) * 44100 / tau")
local l2 = lowpass1(lowpass1(white(), hz), hz)
return expr("tanh(x * 5.5) * 0.55 * lerp(0.45, 1, (t / 0.85) ^ 2) * (t < 0.85)", { x = l2 })
