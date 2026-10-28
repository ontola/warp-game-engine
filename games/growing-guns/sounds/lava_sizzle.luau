-- Lava sizzle, after sfx.gd _synth_lava_sizzle (1830-1861), contact and fall-death modes.
-- High-passed white hiss (Godot k 0.08, 585 Hz) plus a "bubble" bed: 127 Hz low-passed noise
-- through tanh, amplitude-modulated 55-100% by a ~30 Hz sine whose frequency is redrawn from
-- U(22, 38) Hz every sample (a jittery 30 Hz AM); all through tanh.
--   contact: 0.42-0.58 s, 75 ms smoothstep attack, exp(-4.2 u) body, hiss 0.26, drive 1.35.
--   fall:    1.55-1.95 s, 0.42 s smoothstep attack, smoothstep(0, 0.18, u) rise and a
--            cos^1.65 fade over the last 62%, hiss 0.11, drive 1.05.
-- Godot renders 3 seeded variants per mode (hash([variant, "lava_sizzle", fall_death]),
-- variant = randi() % 3); here every voice draws its own length and bubble phase in the
-- same order (Godot's U(a, b) draws become rand(a, b)). Fixed clip.
--
-- Params:
--   fall_death  0: contact (touching lava), 1: fall death into lava (lava_sizzle's fall_death).
-- Game (lava_sizzle 1053-1060, player.gd:4039): raytraced 3D at the player (room reverb),
--   unit 18 m, no big-tail send (big_tail false), pitch (playback rate) U(0.96, 1.04);
--   contact -23 +-1 dB (lava_sizzle_db - 5), SPL 95; fall -26 +-1 dB, SPL 72.
length(1.95)

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local f = expr("clamp(x, 0, 1) > 0.5", { x = param("fall_death", 0) })
local r = rand(0, 1)
local dur = expr("lerp(lerp(0.42, 0.58, r), lerp(1.55, 1.95, r), f)", { r = r, f = f })
local u = expr("t / d", { d = dur })

local attack = expr("smoothstep(0, lerp(0.075, 0.42, f), t)", { f = f })
local fall_body = expr("smoothstep(0, 0.18, u) * max(cos(clamp((u - 0.38) / 0.62, 0, 1) * pi * 0.5), 0) ^ 1.65", { u = u })
local body = expr("lerp(exp(-u * 4.2), b, f)", { u = u, b = fall_body, f = f })

-- Hiss: x - lerp(h, x, 0.08), i.e. a one-pole high-pass at 585.2 Hz.
local hiss = highpass1(white(), 585.2) * expr("a * b * lerp(0.26, 0.11, f)", { a = attack, b = body, f = f })

-- Bubble: phase += 2 pi U(22, 38) / sr per sample. The per-sample draw is a phase random walk;
-- scaling its spread by sqrt(sr / 44100) keeps Godot's walk per second at any rate.
local bub_hz = expr("30 + 8 * w * sqrt(sr / 44100)", { w = noise() })
local bub = sine(bub_hz, expr("p + 30 / sr", { p = rand(0, 1) }))
-- lerp(lo, y, 0.018): 127.5 Hz.
local low = lowpass1(white(), 127.49)
local bubble = expr("tanh(l * 4) * b * (0.55 + 0.45 * s) * 0.18", { l = low, b = body, s = bub })

return expr("tanh((h + q) * lerp(1.35, 1.05, f)) * (t < d)", { h = hiss, q = bubble, f = f, d = dur })
