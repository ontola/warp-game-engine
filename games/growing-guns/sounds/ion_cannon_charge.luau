-- Ion cannon charge pad, after sfx.gd _synth_ion_cannon_charge (1935-1969).
-- An Am7-ish additive chord (220, 261.63, 329.63, 392, 440, 554.37 Hz, slightly detuned,
-- random start phases) that swells from a whisper (u^3.4), blooming 2nd and 3rd harmonics
-- and a 1.37 Hz tremolo in over the second half, over a 99 Hz low-passed noise bed; tanh.
--
-- Fixed clip, as in Godot: Godot renders one clip as long as the charge and plays it once,
-- attached to the beam column; the build-up is t / charge_seconds, not game state. So this
-- patch is not a loop and takes no per-frame param.
--
-- Params:
--   charge_seconds  charge time (ion_cannon.gd CHARGE_SECONDS 3.8). Godot caches
--                   snapped(clamp(s, 2.5, 6.5), 0.1); clamped to 2.5..6.5 here.
-- Game (attach_ion_cannon_charge 1092-1112, ion_cannon.gd:133): at the ground target, attached
--   (no speed-of-sound delay, not HDR-gated), raytraced 3D (room reverb), unit 140 m, -14 dB;
--   big-tail send -28 dB, big-tail intensity >= 0.52. Plus a positional companion copy
--   (_play_stream) at -10 dB, unit 140, SPL 118, which is HDR-gated, delayed by distance and
--   sends -34 dB to the big tail. Godot's right channel is 0.98 x left; this patch is mono.
length(6.5)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local dur = expr("clamp(x, 2.5, 6.5)", { x = param("charge_seconds", 3.8) })
local u = expr("t / d", { d = dur })
local swell = expr("clamp(u, 0, 1) ^ 3.4", { u = u })
local bloom = expr("smoothstep(0.42, 1, u)", { u = u })

local freqs = { 220.0, 261.63, 329.63, 392.0, 440.0, 554.37 }
local gains = { 0.34, 0.28, 0.24, 0.20, 0.18, 0.12 }
local detune = { 1.0, 1.002, 0.998, 1.004, 0.996, 1.001 }
local fund, harm2, harm3 = {}, {}, {}
for j = 1, 6 do
  local hz = freqs[j] * detune[j]
  -- Godot advances each phase (from U(0, 2 pi)) before reading it: phase(t) = p0 + 2 pi hz (t + 1/sr).
  local p = expr("r + h / sr", { r = rand(0, 1), h = hz })
  fund[#fund + 1] = sine(hz, p) * gains[j]
  harm2[#harm2 + 1] = sine(2 * hz, p * 2) * gains[j]
  harm3[#harm3 + 1] = sine(3 * hz, p * 3) * gains[j]
end
local chord = expr("a + b * 0.07 * m + c * 0.03 * m * m", { a = mix(table.unpack(fund)), b = mix(table.unpack(harm2)), c = mix(table.unpack(harm3)), m = bloom })

-- lerp(nl, white, 0.014): 98.96 Hz.
local bed = lowpass1(white(), 98.96) * expr("lerp(0.05, 0.18, s)", { s = swell })
local pulse = expr("1 + sin(t * 8.6) * 0.035 * m", { m = bloom })
return expr("tanh((c * 0.22 + b) * lerp(0.10, 1, s) * p) * 0.68 * (t < d)", { c = chord, b = bed, s = swell, p = pulse, d = dur })
