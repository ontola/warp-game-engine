-- Air strike inbound whoosh, after sfx.gd _synth_air_strike_inbound (1907-1932).
-- Brown noise (leaky-integrated white) through two one-pole low-passes whose
-- cutoff opens 60 -> 320 Hz over the flight, soft-saturated, swelling toward impact.
--
-- Fixed clip, as in Godot: Godot renders one clip as long as the rocket's flight
-- and plays it once, attached to the rocket; the approach is t / travel_seconds,
-- not game state. So this patch is not a loop and takes no per-frame param.
--
-- Params:
--   travel_seconds  rocket flight time (air_strike.gd _travel_seconds, 2.3 s). Godot caches
--                   snapped(clamp(t, 1.75, 6.5), 0.25), so 2.3 plays as 2.25; pass the bucket
--                   for parity or the real time to end on the impact. Clamped to 1.75..6.5.
--   distant         0 or 1 (attach_air_strike_inbound's `distant`; the game always passes 0).
--                   1: starts at 0.52 instead of 0.46 and 45 Hz instead of 60 Hz.
-- Game (attach_air_strike_inbound 1068-1084, air_strike.gd:96): attached to the rocket (moves
--   with it, no speed-of-sound delay, not HDR-gated), raytraced 3D (room reverb), unit 58 m
--   (88 distant), -4 dB (-18 distant); big-tail send -28 dB (-44 distant) left at the spawn
--   position; big-tail intensity >= 0.42 (0.28 distant). Stop it with the rocket.
length(6.5)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local dur = expr("clamp(x, 1.75, 6.5)", { x = param("travel_seconds", 2.25) })
local far = expr("clamp(x, 0, 1)", { x = param("distant", 0) })
local u = expr("clamp(t / d, 0, 1)", { d = dur })

-- brown = 0.998 brown + 0.04 white  ==  20 * one-pole LP (k 0.002, 14.05 Hz) of white.
local brown = lowpass1(white(), 14.0515) * 20

-- Godot's RC one-pole, alpha = dt / (1 / (2 pi fc) + dt) at 44.1 kHz, as its exact cutoff.
local fc = expr("lerp(lerp(60, 45, f), 320, u)", { f = far, u = u })
local hz = expr("-log(1 - 1 / (1 + 44100 / (tau * c))) * 44100 / tau", { c = fc })
local l2 = lowpass1(lowpass1(brown, hz), hz)

local env = expr("lerp(lerp(0.46, 0.52, f), 1, u * u)", { f = far, u = u })
return expr("tanh(x * 6.5) * 0.62 * e * (t < d)", { x = l2, e = env, d = dur })
